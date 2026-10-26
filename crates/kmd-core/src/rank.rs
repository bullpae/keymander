//! 순위 — 앱·실행 파일·폴더·파일 결과를 한 척도로 (docs/22 §3.7)
//!
//! 옛 엔진의 퍼지 점수는 **순위**(`(개수−i)×10`)라 품질을 담지 않았고, 거기에 종류 가중
//! (폴더 80·앱 70)과 실행 이력(최대 200)을 더하면 매칭 품질이 묻혔다. 여기서는 모든 결과를
//! 같은 신호로 다시 매긴다.
//!
//! ```text
//! 점수 = BASE + 이름 일치 품질(정확 100 · 접두 70 · 단어 시작 50 · 부분 30 · 머리글자 25 · 흩어짐 10,
//!                              조상 폴더에만 있는 단어 6)
//!      + 종류(kind_weights ÷ 4: 폴더 20 · 앱 17 · 파일 12 · 실행 파일 10 · 시스템 명령 7)
//!      + 최근 수정(1일 15 · 1주 10 · 1달 5, 파일만)
//!      + 실행 이력(최대 60)
//!      − 깊이(검색 폴더 아래 단계당 2, 최대 10, 파일만)
//! ```
//!
//! 정확 일치가 종류 가중보다 크고, 실행 이력은 두 단계 품질 차이(예: 정확 100 vs 단어 시작 50)
//! 정도를 뒤집을 만큼만 준다 — 자주 연 "Visual Studio Code" 가 `code` 실행 파일을 이기되,
//! 이력이 있는 흩어진 매칭(10)이 이력 없는 정확 일치를 이기지는 않는다.

use crate::config::KindWeights;
use crate::filesearch::query::{query_tokens, token_quality, Q_ANCESTOR, Q_SUBSEQ};
use crate::filesearch::table::fold;
use crate::history::FrecencyMap;
use crate::index::Source;
use crate::search::SearchResult;

/// 모든 점수의 바닥 — 깊이 감점이 0 아래로 내려가지 않게
pub const BASE: u32 = 40;
/// 실행 이력 점수 상한 — 정확 일치와 단어 시작 일치의 차이(50)를 뒤집을 수 있을 만큼
pub const FRECENCY_MAX: u32 = 60;

const DAY_NS: i64 = 86_400 * 1_000_000_000;

pub fn recency_points(age_ns: i64) -> u32 {
    if age_ns < DAY_NS {
        15
    } else if age_ns < 7 * DAY_NS {
        10
    } else if age_ns < 30 * DAY_NS {
        5
    } else {
        0
    }
}

/// 검색 폴더 바로 아래(깊이 1)는 감점 없음
pub fn depth_penalty(depth: u32) -> u32 {
    (depth.saturating_sub(1) * 2).min(10)
}

pub fn kind_points(kind: crate::index::ItemKind, weights: &KindWeights) -> u32 {
    weights.weight_for(kind) / 4
}

/// 실행 이력 점수 — 옛 부스트 값(최대 200)을 0..=60 으로
pub fn frecency_points(freq: u32, executed_at: &str) -> u32 {
    (crate::history::boost_value(freq, executed_at) * 3 / 10).min(FRECENCY_MAX)
}

/// 옛 엔진 결과와 새 파일 엔진 결과를 한 척도로 합친다.
///
/// - 옛 결과의 파일 항목(할당량에 잘린 것)은 버린다 — 파일은 새 엔진이 전부 갖고 있다.
/// - 엔진이 낸 결과(앱·실행 파일·시스템 명령)는 이름 일치 품질로 다시 매긴다.
///   계산기·URL 열기 같은 플러그인 항목(`Source::Plugin`)은 점수를 그대로 둔다.
/// - `files` 는 [`crate::filesearch::FileIndex::search`] 결과 (실행 이력 빼고 같은 공식).
/// - 실행 이력은 여기서 모두에게 한 번만 더한다.
pub fn merge(
    legacy: Vec<SearchResult>,
    files: Vec<SearchResult>,
    query: &str,
    weights: &KindWeights,
    frecency: &FrecencyMap,
) -> Vec<SearchResult> {
    let toks = query_tokens(query);
    let mut out: Vec<SearchResult> = legacy
        .into_iter()
        .filter(|r| r.item.source != Source::FileProvider)
        .map(|mut r| {
            if r.item.source != Source::Plugin {
                r.score = BASE + item_quality(&r.item, &toks) + kind_points(r.item.kind, weights);
            }
            r
        })
        .collect();
    out.extend(files);
    for r in &mut out {
        if r.item.source == Source::Plugin {
            continue;
        }
        if let Some((freq, at)) = frecency.get(&r.item.path) {
            r.score = r.score.saturating_add(frecency_points(*freq, at));
        }
    }
    // 같은 점수는 이름이 짧은 쪽 — "Slack" 이 "Slack Helper" 보다 먼저
    out.sort_by(|a, b| {
        b.score.cmp(&a.score).then_with(|| {
            a.item
                .name
                .chars()
                .count()
                .cmp(&b.item.name.chars().count())
        })
    });
    out
}

/// 앱·실행 파일 이름의 일치 품질. 옛 엔진은 키워드·경로로도 매칭하므로, 이름에 없는 단어는
/// 경로에 있으면 조상 몫(6), 그마저 없으면(흩어진 퍼지·키워드) 최소 품질을 준다.
fn item_quality(item: &crate::index::IndexItem, toks: &[crate::filesearch::query::Token]) -> u32 {
    let name = fold(&item.name);
    let name = name.as_bytes();
    let mut path: Option<String> = None;
    let mut sum = 0;
    for tok in toks {
        let q = token_quality(name, tok);
        if q > 0 {
            sum += q;
            continue;
        }
        let p = path.get_or_insert_with(|| fold(&item.path));
        if memchr::memmem::find(p.as_bytes(), &tok.text).is_some() {
            sum += Q_ANCESTOR;
        }
    }
    sum.max(Q_SUBSEQ)
}

#[cfg(test)]
mod tests {
    //! 골든 셋 — "이 질의면 1위는 이것" (docs/22 §3.7). 순위 공식을 바꾸면 여기서 회귀를 본다.
    use super::*;
    use crate::filesearch::table::{Kind, RootInfo, TableBuilder, NO_PARENT};
    use crate::filesearch::FileIndex;
    use crate::index::{IndexItem, ItemKind};
    use std::ffi::OsStr;

    fn item(name: &str, path: &str, kind: ItemKind, source: Source) -> IndexItem {
        IndexItem {
            name: name.into(),
            path: path.into(),
            kind,
            source,
            icon: String::new(),
            keywords: String::new(),
            icon_path: None,
        }
    }

    /// 옛 엔진이 낼 법한 앱·실행 파일·시스템 명령 (점수는 순위 흉내)
    fn legacy_pool() -> Vec<IndexItem> {
        vec![
            item(
                "Visual Studio Code",
                "/Applications/Visual Studio Code.app",
                ItemKind::App,
                Source::Apps,
            ),
            item(
                "Google Chrome",
                "/Applications/Google Chrome.app",
                ItemKind::App,
                Source::Apps,
            ),
            item(
                "Calculator",
                "/System/Applications/Calculator.app",
                ItemKind::App,
                Source::Apps,
            ),
            item(
                "Slack",
                "/Applications/Slack.app",
                ItemKind::App,
                Source::Apps,
            ),
            item(
                "code",
                "/usr/local/bin/code",
                ItemKind::Executable,
                Source::Path,
            ),
            item(
                "cargo",
                "/Users/me/.cargo/bin/cargo",
                ItemKind::Executable,
                Source::Path,
            ),
            item(
                "Lock Screen",
                "kmd:lock",
                ItemKind::SystemCommand,
                Source::SystemCommand,
            ),
        ]
    }

    fn files() -> FileIndex {
        let mut b = TableBuilder::new();
        let root = b.add_root(RootInfo {
            path: OsStr::new("/Users/me/MyOffice").as_encoded_bytes().to_vec(),
            mtime: 0,
            mtime_reliable: true,
        });
        for year in 2024..=2026 {
            let y = b.push(
                OsStr::new(&format!("{year}년")),
                NO_PARENT,
                root,
                Kind::Dir,
                1,
                0,
            );
            let p = b.push(OsStr::new("고객사_보고"), y, root, Kind::Dir, 2, 0);
            b.push(OsStr::new("보고서_초안.hwp"), p, root, Kind::File, 3, 0);
            b.push(OsStr::new("보고서_최종.hwp"), p, root, Kind::File, 3, 0);
        }
        b.push(OsStr::new("README.md"), NO_PARENT, root, Kind::File, 1, 0);
        let deep = b.push(OsStr::new("archive"), NO_PARENT, root, Kind::Dir, 1, 0);
        let deeper = b.push(OsStr::new("old"), deep, root, Kind::Dir, 2, 0);
        b.push(OsStr::new("README.md"), deeper, root, Kind::File, 3, 0);
        b.push(OsStr::new("calc-notes.txt"), deeper, root, Kind::File, 3, 0);
        b.push(OsStr::new("slack-export"), NO_PARENT, root, Kind::Dir, 1, 0);
        b.push(
            OsStr::new("report-2026.pdf"),
            NO_PARENT,
            root,
            Kind::File,
            1,
            0,
        );
        FileIndex::Memory(b)
    }

    /// 앱 쪽은 "질의가 이름 어딘가에 흩어져 맞는" 항목을 모두 후보로 — 옛 엔진(nucleo)과 비슷하게
    fn legacy_hits(query: &str) -> Vec<SearchResult> {
        let q: Vec<char> = query
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        legacy_pool()
            .into_iter()
            .filter(|i| {
                let name = i.name.to_lowercase();
                let mut it = name.chars();
                q.iter().all(|c| it.any(|x| x == *c))
            })
            .enumerate()
            .map(|(i, item)| SearchResult {
                item,
                score: 500 - i as u32 * 10,
            })
            .collect()
    }

    fn top(query: &str, frecency: &FrecencyMap) -> String {
        let w = KindWeights::default();
        let idx = files();
        let f = idx.search(
            query,
            50,
            &crate::filesearch::Boost::new(frecency),
            &w,
            false,
        );
        let merged = merge(legacy_hits(query), f, query, &w, frecency);
        merged
            .first()
            // 기대값은 `/` 로 적는다 — Windows 는 검색 폴더 아래를 `\` 로 잇는다
            .map(|r| r.item.path.replace('\\', "/"))
            .unwrap_or_default()
    }

    #[test]
    fn 골든_셋() {
        let none = FrecencyMap::new();
        let cases = [
            ("vsc", "/Applications/Visual Studio Code.app"),
            ("chrome", "/Applications/Google Chrome.app"),
            ("calc", "/System/Applications/Calculator.app"),
            ("slack", "/Applications/Slack.app"),
            ("cargo", "/Users/me/.cargo/bin/cargo"),
            ("2026년", "/Users/me/MyOffice/2026년"),
            (
                "보고서 최종",
                "/Users/me/MyOffice/2024년/고객사_보고/보고서_최종.hwp",
            ),
            (
                "2026년 최종",
                "/Users/me/MyOffice/2026년/고객사_보고/보고서_최종.hwp",
            ),
            ("readme", "/Users/me/MyOffice/README.md"),
            ("report", "/Users/me/MyOffice/report-2026.pdf"),
        ];
        let mut fails = Vec::new();
        for (q, want) in cases {
            let got = top(q, &none);
            if got != want {
                fails.push(format!("{q:?}: 1위 {got:?}, 기대 {want:?}"));
            }
        }
        assert!(fails.is_empty(), "\n{}", fails.join("\n"));
    }

    #[test]
    fn 실행_이력은_두_단계_품질_차이까지_뒤집는다() {
        // "code" 는 이름이 정확히 같은 실행 파일이 먼저지만, VS Code 를 자주 열었으면 그쪽
        let none = FrecencyMap::new();
        assert_eq!(top("code", &none), "/usr/local/bin/code");
        let mut used = FrecencyMap::new();
        used.insert(
            "/Applications/Visual Studio Code.app".into(),
            (30, chrono_now()),
        );
        assert_eq!(top("code", &used), "/Applications/Visual Studio Code.app");
        // 깊은 README 도 자주 열면 얕은 README 를 이긴다
        let mut used = FrecencyMap::new();
        used.insert(
            std::path::Path::new("/Users/me/MyOffice")
                .join("archive")
                .join("old")
                .join("README.md")
                .to_string_lossy()
                .into_owned(),
            (30, chrono_now()),
        );
        assert_eq!(
            top("readme", &used),
            "/Users/me/MyOffice/archive/old/README.md"
        );
    }

    fn chrono_now() -> String {
        // history 는 "YYYY-MM-DD HH:MM:SS" 로컬 시각 문자열 — 아주 최근으로 보이게 먼 미래
        "2999-01-01 00:00:00".into()
    }

    #[test]
    fn 플러그인_항목은_점수를_건드리지_않는다() {
        let plugin = SearchResult {
            item: item("= 4", "calc:4", ItemKind::Calculator, Source::Plugin),
            score: u32::MAX,
        };
        let merged = merge(
            vec![plugin],
            Vec::new(),
            "2+2",
            &KindWeights::default(),
            &FrecencyMap::new(),
        );
        assert_eq!(merged[0].score, u32::MAX);
    }
}
