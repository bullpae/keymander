//! 파일 이름 질의 — 거르기 → 싼 점수 → (퍼지면) Matcher 다듬기 (docs/22 §3.6)
//!
//! 전 행을 선형으로 훑는다. 40만 개 수준에서는 역색인보다 캐시 친화적 선형 스캔이
//! 단순하고 충분히 빠르다(시제품 4스레드 1~2ms). 행 범위를 몇 조각으로 나눠 짧게만
//! 스레드를 쓴다.
//!
//! - 한 단어: 자기 이름에서만 찾는다(폴더 아래 전부가 걸리지 않게).
//! - 여러 단어: 단어마다 자기 이름 **또는 조상 폴더 이름(검색 폴더 경로 포함)** 에 있어야 한다.
//!   조상 쪽은 부모가 먼저 오는 순서를 이용해 앞에서 뒤로 한 번에 전파한다(폴더 마스크).
//! - 비ASCII 토큰은 바이트 subsequence 로 보지 않는다(UTF-8 연속 바이트가 다른 음절과 엇갈려 맞는다).
//! - 1~2글자 ASCII 토큰은 접두·단어 시작·머리글자만 — subsequence 로는 `2` 가 거의 전부 통과한다.

use std::collections::HashMap;
use std::path::Path;

use memchr::memmem;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config as NucleoConfig, Matcher, Utf32Str};

use super::table::{char_bag, fold, Kind, Table, NO_PARENT};
use crate::history::FrecencyMap;
use crate::search::{GlobMatcher, SearchMode};

/// 한 번에 다룰 최대 토큰 수 (마스크 비트)
const MAX_TOKENS: usize = 32;
/// Matcher 로 다듬을 후보 수 상한 — 그 밖은 싼 점수 순서 그대로
const REFINE_TOP: usize = 5_000;
/// regex·glob 이 볼 최대 후보 — 비싼 패턴이 런처를 오래 붙잡지 않게
const PATTERN_SCAN_CAP: usize = 2_000_000;

// 이름 일치 품질 — 정확 일치가 kind 가중보다 커야 한다 (docs/22 §3.7)
const Q_EXACT: u32 = 100;
const Q_PREFIX: u32 = 70;
const Q_WORD: u32 = 50;
const Q_SUBSTR: u32 = 30;
const Q_ACRONYM: u32 = 25;
const Q_SUBSEQ: u32 = 10;
/// 단어가 자기 이름이 아니라 조상 폴더에만 있을 때
const Q_ANCESTOR: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub row: u32,
    /// 매칭·종류·최근·깊이 점수 (실행 이력 부스트는 빼고 — 합칠 때 한 번만 더한다)
    pub score: u32,
}

/// 실행 이력 — 자주 여는 파일이 후보 상한 밖으로 밀리지 않게 선정 단계에서 쓴다.
/// 행에는 경로가 없으므로 이름으로 먼저 찾고 경로로 확인한다.
#[derive(Default)]
pub struct Boost {
    by_name: HashMap<Vec<u8>, Vec<(String, u32)>>,
}

impl Boost {
    pub fn new(map: &FrecencyMap) -> Self {
        let mut by_name: HashMap<Vec<u8>, Vec<(String, u32)>> = HashMap::new();
        for (path, (freq, at)) in map {
            let Some(name) = Path::new(path).file_name() else {
                continue;
            };
            by_name
                .entry(fold(&name.to_string_lossy()).into_bytes())
                .or_default()
                .push((path.clone(), crate::history::boost_value(*freq, at)));
        }
        Self { by_name }
    }

    fn get(&self, t: &Table<'_>, row: usize) -> u32 {
        let Some(paths) = self.by_name.get(t.folded_name(row)) else {
            return 0;
        };
        let full = t.path(row);
        let full = full.to_string_lossy();
        paths
            .iter()
            .find(|(p, _)| *p == full)
            .map_or(0, |&(_, b)| b)
    }
}

/// 질의 하나 실행. `now_ns` 는 최근 수정 가중용(유닉스 시각 ns).
pub fn search(t: &Table<'_>, query: &str, limit: usize, now_ns: i64, boost: &Boost) -> Vec<Hit> {
    if t.is_empty() || limit == 0 {
        return Vec::new();
    }
    let (mode, pattern) = SearchMode::detect(query);
    let scored: Vec<(u32, u32)> = match mode {
        SearchMode::Glob => {
            let pattern = fold(&pattern);
            // 가장 흔한 꼴 `*.hwp`(`.hwp` 입력도 이렇게 바뀐다)은 끝 비교로 — 일반 glob 은 수십 ms
            match pattern.strip_prefix('*') {
                Some(ext) if ext.starts_with('.') && !ext.contains(['*', '?']) => {
                    pattern_scan(t, now_ns, |name| name.ends_with(ext))
                }
                _ => {
                    let glob = GlobMatcher::new(&pattern);
                    pattern_scan(t, now_ns, |name| glob.matches(name))
                }
            }
        }
        SearchMode::Regex => {
            let Ok(re) = regex::RegexBuilder::new(&pattern)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
            else {
                return Vec::new();
            };
            pattern_scan(t, now_ns, |name| re.is_match(name))
        }
        SearchMode::Fuzzy | SearchMode::Contains | SearchMode::Url => {
            token_scan(t, query, mode == SearchMode::Fuzzy, now_ns)
        }
    };
    rank(t, scored, query, mode, limit, boost)
}

/// glob·regex — 접은 이름 전체에 패턴, 점수는 종류·최근·깊이만
fn pattern_scan(t: &Table<'_>, now_ns: i64, is_match: impl Fn(&str) -> bool) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for row in 0..t.len().min(PATTERN_SCAN_CAP) {
        let Ok(name) = std::str::from_utf8(t.folded_name(row)) else {
            continue;
        };
        if is_match(name) {
            out.push((Q_SUBSTR + extras(t, row, now_ns), row as u32));
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum TokKind {
    /// 비ASCII 또는 contains 모드 — 부분 문자열
    Substr,
    /// ASCII 3글자 이상 퍼지 — 부분 문자열이 없으면 subsequence
    Fuzzy,
    /// ASCII 1~2글자 — 접두·단어 시작·머리글자만
    Short,
}

struct Token {
    text: Vec<u8>,
    kind: TokKind,
    bag: u64,
    finder: memmem::Finder<'static>,
}

fn tokens_of(query: &str, fuzzy_mode: bool) -> Vec<Token> {
    query
        .split_whitespace()
        .take(MAX_TOKENS)
        .map(|w| {
            let text = fold(w).into_bytes();
            let kind = if !text.is_ascii() || !fuzzy_mode {
                TokKind::Substr
            } else if text.len() <= 2 {
                TokKind::Short
            } else {
                TokKind::Fuzzy
            };
            let bag = char_bag(&String::from_utf8_lossy(&text));
            let finder = memmem::Finder::new(&text).into_owned();
            Token {
                text,
                kind,
                bag,
                finder,
            }
        })
        .collect()
}

fn token_scan(t: &Table<'_>, query: &str, fuzzy_mode: bool, now_ns: i64) -> Vec<(u32, u32)> {
    let toks = tokens_of(query, fuzzy_mode);
    if toks.is_empty() {
        return Vec::new();
    }
    let n = t.len();
    let all: u32 = if toks.len() == 32 {
        u32::MAX
    } else {
        (1u32 << toks.len()) - 1
    };

    // 1) 행마다 자기 이름에 맞는 토큰 비트와 품질 합 — 가장 무거운 부분이라 나눠 돈다
    let mut own = vec![0u32; n];
    let mut quality = vec![0u16; n];
    let threads = worker_count(n);
    let chunk = n.div_ceil(threads);
    std::thread::scope(|s| {
        for (i, (own_c, q_c)) in own
            .chunks_mut(chunk)
            .zip(quality.chunks_mut(chunk))
            .enumerate()
        {
            let toks = &toks;
            s.spawn(move || {
                let base = i * chunk;
                for (j, (o, q)) in own_c.iter_mut().zip(q_c.iter_mut()).enumerate() {
                    let row = base + j;
                    let name = t.folded_name(row);
                    let bag = t.bag[row];
                    let mut sum = 0u32;
                    for (k, tok) in toks.iter().enumerate() {
                        if tok.bag & bag != tok.bag {
                            continue;
                        }
                        let got = match_quality(name, tok);
                        if got > 0 {
                            *o |= 1 << k;
                            sum += got;
                        }
                    }
                    *q = sum.min(u16::MAX as u32) as u16;
                }
            });
        }
    });

    let mut out = Vec::new();
    if toks.len() == 1 {
        for row in 0..n {
            if own[row] != 0 {
                out.push((quality[row] as u32 + extras(t, row, now_ns), row as u32));
            }
        }
        return out;
    }

    // 2) 조상 마스크 — 검색 폴더 경로 이름도 조상으로 친다 ("MyOffice 2026년")
    let root_mask: Vec<u32> = t
        .roots
        .iter()
        .map(|r| {
            let path = fold(&r.path_buf().to_string_lossy()).into_bytes();
            toks.iter()
                .enumerate()
                .filter(|(_, tok)| tok.finder.find(&path).is_some())
                .fold(0, |m, (k, _)| m | 1 << k)
        })
        .collect();
    let mut anc = vec![0u32; n];
    for row in 0..n {
        let p = t.parent[row];
        anc[row] = if p == NO_PARENT {
            root_mask[t.root_id[row] as usize]
        } else {
            own[p as usize] | anc[p as usize]
        };
        // 자기 이름에 하나도 없으면(폴더 아래 아무 파일) 결과로 내지 않는다
        if own[row] != 0 && (own[row] | anc[row]) == all {
            let ancestor_only = (all & !own[row]).count_ones();
            let score = quality[row] as u32 + ancestor_only * Q_ANCESTOR + extras(t, row, now_ns);
            out.push((score, row as u32));
        }
    }
    out
}

/// 짧게 쓰는 작업 스레드 수 — 코어 절반, 최대 4 (작은 표는 1)
fn worker_count(rows: usize) -> usize {
    if rows < 20_000 {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 4))
}

/// 이름 하나에 토큰 하나 — 0 이면 불일치
fn match_quality(name: &[u8], tok: &Token) -> u32 {
    let t = tok.text.as_slice();
    if name == t || stem(name) == t {
        return Q_EXACT;
    }
    if name.starts_with(t) {
        return Q_PREFIX;
    }
    let mut found = false;
    for pos in tok.finder.find_iter(name) {
        if is_word_start(name, pos) {
            return Q_WORD;
        }
        found = true;
    }
    match tok.kind {
        TokKind::Substr if found => Q_SUBSTR,
        TokKind::Substr => 0,
        TokKind::Fuzzy if found => Q_SUBSTR,
        TokKind::Fuzzy if acronym(name, t) => Q_ACRONYM,
        TokKind::Fuzzy if subsequence(name, t) => Q_SUBSEQ,
        TokKind::Fuzzy => 0,
        TokKind::Short if acronym(name, t) => Q_ACRONYM,
        TokKind::Short => 0,
    }
}

fn stem(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// `pos` 가 단어 시작인지 — 구분자 뒤, 숫자↔글자 경계, ASCII↔비ASCII 경계
fn is_word_start(name: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = name[pos - 1];
    let cur = name[pos];
    if prev.is_ascii() && !prev.is_ascii_alphanumeric() {
        return true;
    }
    (prev.is_ascii_digit() && cur.is_ascii_alphabetic())
        || (prev.is_ascii_alphabetic() && cur.is_ascii_digit())
        || (prev.is_ascii() != cur.is_ascii() && (cur & 0xC0) != 0x80)
}

/// 토큰 글자가 모두 단어 시작에 차례로 있는지 — `vsc` → Visual Studio Code
fn acronym(name: &[u8], t: &[u8]) -> bool {
    let mut k = 0;
    for pos in 0..name.len() {
        if k < t.len() && name[pos] == t[k] && is_word_start(name, pos) {
            k += 1;
        }
    }
    k == t.len()
}

fn subsequence(name: &[u8], t: &[u8]) -> bool {
    let mut it = name.iter();
    t.iter().all(|c| it.any(|x| x == c))
}

/// 종류·최근 수정·깊이 가중 (docs/22 §3.7)
fn extras(t: &Table<'_>, row: usize, now_ns: i64) -> u32 {
    const DAY: i64 = 86_400 * 1_000_000_000;
    let kind: u32 = if t.kind(row) == Kind::Dir { 15 } else { 0 };
    let age = now_ns.saturating_sub(t.mtime[row]);
    let recency = if age < DAY {
        15
    } else if age < 7 * DAY {
        10
    } else if age < 30 * DAY {
        5
    } else {
        0
    };
    let depth_penalty = (t.depth(row).saturating_sub(1) * 2).min(10);
    (40 + kind + recency).saturating_sub(depth_penalty)
}

/// 후보 → 상위 `limit`. 실행 이력은 선정에만 쓰고 점수에는 넣지 않는다.
fn rank(
    t: &Table<'_>,
    mut scored: Vec<(u32, u32)>,
    query: &str,
    mode: SearchMode,
    limit: usize,
    boost: &Boost,
) -> Vec<Hit> {
    if !boost.by_name.is_empty() {
        for (score, row) in &mut scored {
            *score += boost.get(t, *row as usize);
        }
    }
    let key = |&(score, row): &(u32, u32)| (std::cmp::Reverse(score), row);
    if scored.len() > REFINE_TOP {
        scored.select_nth_unstable_by_key(REFINE_TOP, key);
        scored.truncate(REFINE_TOP);
    }

    // 한 단어 퍼지: subsequence 처럼 거칠게 맞은 것들의 순서를 nucleo 로 다듬는다
    if mode == SearchMode::Fuzzy && query.split_whitespace().count() == 1 {
        let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
        let pat = Pattern::parse(query, CaseMatching::Ignore, Normalization::Never);
        let mut buf = Vec::new();
        for (score, row) in &mut scored {
            let name = String::from_utf8_lossy(t.folded_name(*row as usize));
            if let Some(s) = pat.score(Utf32Str::new(&name, &mut buf), &mut matcher) {
                *score += (s / 10).min(40);
            }
        }
    }

    scored.sort_unstable_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| {
                t.folded_name(a.1 as usize)
                    .len()
                    .cmp(&t.folded_name(b.1 as usize).len())
            })
            .then_with(|| t.mtime[b.1 as usize].cmp(&t.mtime[a.1 as usize]))
            .then(a.1.cmp(&b.1))
    });
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(score, row)| Hit {
            row,
            score: score.saturating_sub(boost.get(t, row as usize)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::table::{RootInfo, TableBuilder};
    use super::*;
    use std::ffi::OsStr;

    /// /office/{2020..=2026}년/프로젝트/보고서_N.hwp + 몇 가지 이름
    fn office() -> TableBuilder {
        let mut b = TableBuilder::new();
        let root = b.add_root(RootInfo {
            path: OsStr::new("/Users/me/MyOffice").as_encoded_bytes().to_vec(),
            mtime: 0,
            mtime_reliable: true,
        });
        for year in 2020..=2026 {
            let y = b.push(
                OsStr::new(&format!("{year}년")),
                NO_PARENT,
                root,
                Kind::Dir,
                1,
                0,
            );
            let p = b.push(OsStr::new("프로젝트"), y, root, Kind::Dir, 2, 0);
            for n in 0..3 {
                b.push(
                    OsStr::new(&format!("보고서_{n}.hwp")),
                    p,
                    root,
                    Kind::File,
                    3,
                    0,
                );
            }
        }
        b.push(
            OsStr::new("Visual Studio Code.app"),
            NO_PARENT,
            root,
            Kind::Dir,
            1,
            0,
        );
        b.push(
            OsStr::new("report-2026.pdf"),
            NO_PARENT,
            root,
            Kind::File,
            1,
            0,
        );
        b.push(OsStr::new("README.md"), NO_PARENT, root, Kind::File, 1, 0);
        b
    }

    fn names(b: &TableBuilder, q: &str) -> Vec<String> {
        let t = b.table();
        search(&t, q, 50, 0, &Boost::default())
            .into_iter()
            .map(|h| String::from_utf8_lossy(t.raw_name(h.row as usize)).into_owned())
            .collect()
    }

    #[test]
    fn 연도_폴더는_모두_나오고_정확_일치가_먼저() {
        let b = office();
        let got = names(&b, "2026년");
        assert_eq!(got.first().map(String::as_str), Some("2026년"));
        let got = names(&b, "202");
        for year in 2020..=2026 {
            assert!(got.contains(&format!("{year}년")), "{year}년: {got:?}");
        }
    }

    #[test]
    fn 한_글자는_단어_시작만_퍼지는_흩어진_글자도() {
        let b = office();
        // "2" 는 2020년(접두)·report-2026·보고서_2(단어 시작)엔 맞고, 숫자 중간(2020 의 끝 0 등)에는 안 맞는다
        let got = names(&b, "2");
        assert!(got.contains(&"report-2026.pdf".to_string()));
        assert!(got.contains(&"보고서_2.hwp".to_string()));
        assert!(
            !got.iter().any(|n| n == "보고서_0.hwp" || n == "README.md"),
            "{got:?}"
        );
        assert!(
            names(&b, "0").iter().all(|n| n == "보고서_0.hwp"),
            "2020 안의 0 은 단어 시작 아님"
        );
        // 머리글자
        assert_eq!(
            names(&b, "vsc").first().map(String::as_str),
            Some("Visual Studio Code.app")
        );
        // 흩어진 글자(subsequence)
        assert!(names(&b, "rdme").contains(&"README.md".to_string()));
    }

    #[test]
    fn 여러_단어는_조상_폴더와_검색_폴더_경로까지() {
        let b = office();
        let got = names(&b, "2026년 보고서");
        assert_eq!(got.len(), 3, "{got:?}");
        let t = b.table();
        let hits = search(&t, "2026년 보고서", 50, 0, &Boost::default());
        assert!(hits
            .iter()
            .all(|h| t.path(h.row as usize).to_string_lossy().contains("2026년")));
        // 검색 폴더 경로의 이름도 조상
        assert_eq!(names(&b, "myoffice 2024년"), vec!["2024년".to_string()]);
        // 폴더 아래 아무 파일이나 내지 않는다
        assert!(names(&b, "2026년 프로젝트").iter().all(|n| n == "프로젝트"));
    }

    #[test]
    fn 한글_부분_문자열과_glob() {
        let b = office();
        assert_eq!(names(&b, "고서").len(), 21);
        let got = names(&b, "*.pdf");
        assert_eq!(got, vec!["report-2026.pdf".to_string()]);
        assert!(names(&b, "zzzz").is_empty());
    }

    #[test]
    fn 실행_이력은_선정에만_쓰고_점수에는_안_넣는다() {
        let b = office();
        let t = b.table();
        let target = t.path(13).to_string_lossy().into_owned(); // 2022년/프로젝트/보고서_1
        let mut map = FrecencyMap::new();
        map.insert(target.clone(), (50, "2026-10-26 10:00:00".into()));
        let boost = Boost::new(&map);
        let with = search(&t, "보고서", 1, 0, &boost);
        assert_eq!(t.path(with[0].row as usize).to_string_lossy(), target);
        let without = search(&t, "보고서", 50, 0, &Boost::default());
        let plain = without.iter().find(|h| h.row == with[0].row).unwrap();
        assert_eq!(with[0].score, plain.score);
    }
}
