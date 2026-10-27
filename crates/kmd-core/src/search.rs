//! Search engine — fuzzy, glob, regex, and contains matching over IndexItems

use std::sync::Arc;
use std::time::{Duration, Instant};

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config as NucleoConfig, Nucleo};

use crate::config::KindWeights;
use crate::index::IndexItem;
use crate::textenc::nfc;

/// A search result wrapping an IndexItem with a relevance score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub item: IndexItem,
    pub score: u32,
}

/// Search mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    Fuzzy,
    Glob,
    Regex,
    Contains,
    Url,
}

impl SearchMode {
    /// Auto-detect search mode from query string
    pub fn detect(query: &str) -> (Self, String) {
        let q = query.trim();
        if q.is_empty() {
            return (Self::Fuzzy, String::new());
        }

        // URL detection
        if is_url(q) {
            return (Self::Url, normalize_url(q));
        }

        // Glob: contains * or ?
        if q.contains('*') || q.contains('?') {
            return (Self::Glob, q.to_string());
        }

        // Regex: /pattern/ — 내부에 /가 더 있으면 Unix 경로(/usr/bin/)로 보고 제외
        if q.starts_with('/') && q.len() > 2 && q.ends_with('/') {
            let pattern = &q[1..q.len() - 1];
            if !pattern.contains('/') {
                return (Self::Regex, pattern.to_string());
            }
        }

        // Extension shortcut: .doc -> *.doc
        if q.starts_with('.') && q.len() >= 2 && q[1..].chars().all(|c| c.is_ascii_alphanumeric()) {
            return (Self::Glob, format!("*{}", q));
        }

        // Non-ASCII (CJK etc.) → Contains mode for accurate substring matching
        if !q.is_ascii() {
            return (Self::Contains, q.to_string());
        }

        (Self::Fuzzy, q.to_string())
    }

    /// Display label
    pub fn label(&self) -> &str {
        match self {
            Self::Fuzzy => "fuzzy",
            Self::Glob => "glob",
            Self::Regex => "regex",
            Self::Contains => "contains",
            Self::Url => "url",
        }
    }
}

/// 퍼지 매칭 대상 문자열 — 이름과 사람이 붙인 키워드만 담는다.
///
/// 인덱서들은 keywords에 전체 경로나 앱 ID(AUMID)를 넣는다. 퍼지 매칭은 글자가
/// 순서대로 흩어져 있기만 해도 통과하므로, 긴 경로가 섞이면 거의 모든 항목이
/// 걸린다 — `gmail` 이 `...\Roamin[g]\[M]icrosoft\St[a]rt Menu\...\Access[i]bi[l]ity`
/// 로 매칭돼 시작 메뉴 바로가기가 전부 결과에 뜨던 문제. 크롬 웹앱 AUMID
/// (`Chrome._crx_<무작위 32자>`)도 같은 식으로 오탐을 만든다.
///
/// 경로 조각으로 보고 빼는 토큰:
/// - 경로 구분자(`\`, `/`)가 든 토큰
/// - 항목 경로가 실제 경로/URI(구분자 포함)일 때 그 안에 들어 있는 토큰
///   (`shell:appsFolder\<AUMID>` 의 AUMID, `.desktop` Exec의 `%u` 등)
///
/// 경로로 찾는 질의는 [`SearchEngine::search_fuzzy_with_paths`]가
/// 멀티 토큰 contains 매칭으로 따로 처리한다.
fn fuzzy_haystack(item: &IndexItem) -> String {
    let path_is_path = item.path.contains(['\\', '/']);
    let mut text = nfc(&item.name).into_owned();
    for token in item.keywords.split_whitespace() {
        let path_piece = token.contains(['\\', '/']) || (path_is_path && item.path.contains(token));
        if !path_piece {
            text.push(' ');
            text.push_str(&nfc(token));
        }
    }
    text
}

/// 질의를 소문자 토큰으로 나눈다 — 공백과 경로 구분자 모두 경계로 본다.
fn split_query_tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .flat_map(|word| word.split(['\\', '/']))
        .filter(|s| !s.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Pre-lowercased fields for efficient case-insensitive substring/glob/regex matching.
struct LowercaseCache {
    /// Lowercased name, path, and keywords for each item (same order as `all_items`)
    entries: Vec<LowercaseEntry>,
}

struct LowercaseEntry {
    name: String,
    path: String,
    keywords: String,
}

impl LowercaseCache {
    fn build(items: &[IndexItem]) -> Self {
        let entries = items
            .iter()
            // NFC로 맞춘 뒤 소문자화 — macOS의 NFD 한글 파일명이 타이핑한 NFC
            // 질의와 매칭되도록 (textenc::nfc 참조). 원본 item은 건드리지 않는다.
            .map(|item| LowercaseEntry {
                name: nfc(&item.name).to_lowercase(),
                path: nfc(&item.path).to_lowercase(),
                keywords: nfc(&item.keywords).to_lowercase(),
            })
            .collect();
        Self { entries }
    }
}

/// The search engine wrapping Nucleo fuzzy matcher + other modes
pub struct SearchEngine {
    nucleo: Nucleo<IndexItem>,
    all_items: Vec<IndexItem>,
    lowercase_cache: LowercaseCache,
    kind_weights: KindWeights,
    /// 퍼지 검색이 끝나기를 기다리는 최대 시간 — 넘기면 [`Self::is_pending`]
    fuzzy_wait: Duration,
    /// 마지막 퍼지 검색이 시간 안에 끝나지 않았다 — UI 는 잠시 뒤 같은 질의로 다시 묻는다
    pending: bool,
}

/// UI 기본값 — 키 입력 한 번에 이보다 오래 멈추지 않는다
const FUZZY_WAIT_UI: Duration = Duration::from_millis(50);

impl SearchEngine {
    /// Create a new empty search engine
    pub fn new() -> Self {
        let config = NucleoConfig::DEFAULT;
        let nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);
        Self {
            nucleo,
            all_items: Vec::new(),
            lowercase_cache: LowercaseCache {
                entries: Vec::new(),
            },
            kind_weights: KindWeights::default(),
            fuzzy_wait: FUZZY_WAIT_UI,
            pending: false,
        }
    }

    /// 퍼지 검색을 기다리는 최대 시간. 한 번 묻고 끝나는 호출(`kmd search`·데몬 IPC)은
    /// 다시 물을 기회가 없으므로 길게 잡는다.
    pub fn set_fuzzy_wait(&mut self, wait: Duration) {
        self.fuzzy_wait = wait;
    }

    /// 마지막 검색 결과가 미완성인지 — 참이면 같은 질의로 다시 검색해 완성된 결과를 받는다.
    /// 항목이 많으면(수십만) 퍼지 매칭이 [`Self::set_fuzzy_wait`] 안에 끝나지 않을 수 있다.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// 엔진을 거치지 않는 질의(`:say` 등)로 바뀌었을 때 — 이전 미완성 표시를 지워
    /// UI 가 엉뚱한 질의를 계속 다시 묻지 않게 한다.
    pub fn clear_pending(&mut self) {
        self.pending = false;
    }

    /// kind 가중치를 적용한 뒤 항목을 적재한 엔진을 만든다.
    ///
    /// 데몬·데스크톱·TUI가 각자 `new()` → `set_kind_weights()` → `load()`
    /// 세 줄을 적고 있었다. 가중치 설정을 빠뜨려도 컴파일되므로 **한 곳에서만
    /// 조용히 다르게 동작**할 수 있었다(docs/11 R3-3). 한 호출로 묶어 그
    /// 가능성을 없앤다.
    pub fn with_items(weights: &KindWeights, items: Vec<IndexItem>) -> Self {
        let mut engine = Self::new();
        engine.reload(weights, items);
        engine
    }

    /// 이미 만들어진 엔진의 가중치·항목을 한 번에 교체한다.
    ///
    /// 인덱스 리빌드 경로용 — 엔진이 `Mutex` 안에 있어 새로 만들 수 없을 때 쓴다.
    pub fn reload(&mut self, weights: &KindWeights, items: Vec<IndexItem>) {
        self.set_kind_weights(weights.clone());
        self.load(items);
    }

    /// 로드된 아이템 수
    pub fn len(&self) -> usize {
        self.all_items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all_items.is_empty()
    }

    /// `root` 아래에 인덱싱된 항목 수 — `:scope` 목록의 폴더별 개수 (docs/18).
    pub fn count_within(&self, root: &std::path::Path) -> usize {
        self.all_items
            .iter()
            .filter(|i| crate::fsutil::path_within(std::path::Path::new(&i.path), root))
            .count()
    }

    /// Set the kind weights for score boosting
    pub fn set_kind_weights(&mut self, weights: KindWeights) {
        self.kind_weights = weights;
    }

    /// Load items into the search engine (consumes the item list)
    ///
    /// 재로드(인덱스 리빌드) 시 이전 아이템을 반드시 비운다 — restart 없이
    /// push만 하면 리빌드마다 삭제된 항목이 fuzzy 결과에 남고 중복이 쌓인다.
    pub fn load(&mut self, items: Vec<IndexItem>) {
        self.nucleo.restart(true);
        let injector = self.nucleo.injector();
        for item in &items {
            injector.push(item.clone(), |item, cols| {
                cols[0] = fuzzy_haystack(item).into();
            });
        }
        self.lowercase_cache = LowercaseCache::build(&items);
        self.all_items = items;
    }

    /// Search with automatic mode detection
    pub fn search(&mut self, query: &str, limit: usize) -> (SearchMode, Vec<SearchResult>) {
        let (mode, pattern) = SearchMode::detect(query);
        let results = self.search_with_mode(mode, &pattern, limit);
        (mode, results)
    }

    /// Search with a specific mode
    pub fn search_with_mode(
        &mut self,
        mode: SearchMode,
        pattern: &str,
        limit: usize,
    ) -> Vec<SearchResult> {
        // 질의도 NFC로 — 매칭 대상(LowercaseCache·fuzzy_haystack)과 같은 정규형.
        // UI가 이 함수를 직접 부르기도 해서 search()가 아니라 여기서 한다.
        let pattern = nfc(pattern);
        let pattern = pattern.as_ref();
        self.pending = false;
        let mut results = match mode {
            SearchMode::Fuzzy => self.search_fuzzy_with_paths(pattern, limit),
            SearchMode::Glob => self.filter_glob(pattern, limit),
            SearchMode::Regex => self.filter_regex(pattern, limit),
            SearchMode::Contains | SearchMode::Url => self.filter_contains(pattern, limit),
        };

        // Apply kind weight boost and re-sort
        self.apply_kind_boost(&mut results);
        results
    }

    /// Apply kind_weights boost to search results and re-sort by score descending
    fn apply_kind_boost(&self, results: &mut [SearchResult]) {
        for result in results.iter_mut() {
            let boost = self.kind_weights.weight_for(result.item.kind);
            result.score = result.score.saturating_add(boost);
        }
        results.sort_by_key(|entry| std::cmp::Reverse(entry.score));
    }

    /// 퍼지 검색 + (여러 토큰일 때) 경로 세그먼트 매칭.
    ///
    /// 퍼지 매칭은 이름 위주([`fuzzy_haystack`])라 `dev rust` 같은 경로 조각
    /// 질의는 못 찾는다. 토큰이 둘 이상이면 [`Self::filter_contains`]의 AND +
    /// 세그먼트 점수 결과를 합쳐, 경로로 찾던 기존 동작을 유지한다.
    /// 양쪽에 모두 잡힌 항목은 점수를 더해 위로 올린다.
    fn search_fuzzy_with_paths(&mut self, pattern: &str, limit: usize) -> Vec<SearchResult> {
        let mut results = self.search_fuzzy(pattern, limit);
        if split_query_tokens(pattern).len() < 2 {
            return results;
        }

        for hit in self.filter_contains(pattern, limit) {
            match results.iter_mut().find(|r| r.item.path == hit.item.path) {
                Some(existing) => existing.score = existing.score.saturating_add(hit.score),
                None => results.push(hit),
            }
        }
        results.sort_by_key(|entry| std::cmp::Reverse(entry.score));
        results.truncate(limit);
        results
    }

    /// Fuzzy search using Nucleo
    fn search_fuzzy(&mut self, pattern: &str, limit: usize) -> Vec<SearchResult> {
        self.nucleo
            .pattern
            .reparse(0, pattern, CaseMatching::Smart, Normalization::Never, false);
        // 매칭은 워커 스레드에서 돈다. tick 한 번(10ms)만 기다리면 항목이 많을 때
        // 끝나기 전에 스냅숏을 읽어, 결과가 비거나 **이전 질의의 결과**가 그대로 나왔다
        // (40만 개에서 `2` → 0건, 이어 `zzzz` → 이전 50건, docs/22 §2.2).
        // 끝날 때까지 tick 을 반복하고, 시간을 넘기면 미완성으로 표시해 다시 묻게 한다.
        const NUCLEO_TICK_TIMEOUT_MS: u64 = 10;
        let deadline = Instant::now() + self.fuzzy_wait;
        while self.nucleo.tick(NUCLEO_TICK_TIMEOUT_MS).running {
            if Instant::now() >= deadline {
                // 스냅숏은 아직 이전 질의 것일 수 있다 — 틀린 결과 대신 빈 결과
                self.pending = true;
                return Vec::new();
            }
        }

        let snapshot = self.nucleo.snapshot();
        let count = snapshot.matched_item_count().min(limit as u32);
        snapshot
            .matched_items(..count)
            .enumerate()
            .map(|(i, item)| SearchResult {
                item: item.data.clone(),
                // Higher rank = higher score (first result gets highest)
                score: count.saturating_sub(i as u32) * 10,
            })
            .collect()
    }

    /// Glob pattern filter
    fn filter_glob(&self, pattern: &str, limit: usize) -> Vec<SearchResult> {
        let pattern_lower = pattern.to_lowercase();
        let matcher = GlobMatcher::new(&pattern_lower);

        self.all_items
            .iter()
            .zip(self.lowercase_cache.entries.iter())
            .filter(|(_, lc)| matcher.matches(&lc.name) || matcher.matches(&lc.path))
            .take(limit)
            .map(|(item, _)| SearchResult {
                item: item.clone(),
                score: 0,
            })
            .collect()
    }

    /// Regex filter (with ReDoS protection)
    fn filter_regex(&self, pattern: &str, limit: usize) -> Vec<SearchResult> {
        const MAX_REGEX_PATTERN_LEN: usize = 200;
        const REGEX_SIZE_LIMIT: usize = 1 << 20; // 1 MiB

        if pattern.len() > MAX_REGEX_PATTERN_LEN {
            return Vec::new();
        }

        let Ok(re) = regex::RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
        else {
            return Vec::new();
        };

        self.all_items
            .iter()
            .filter(|item| re.is_match(&item.name) || re.is_match(&item.path))
            .take(limit)
            .map(|item| SearchResult {
                item: item.clone(),
                score: 0,
            })
            .collect()
    }

    /// Substring contains filter (case-insensitive, good for CJK)
    ///
    /// 멀티 토큰 지원: 공백으로 구분된 2개 이상의 토큰이 입력되면
    /// 모든 토큰이 name/path/keywords 중 하나에 포함되어야 매칭 (AND 조건).
    /// 경로 세그먼트 정확 일치에 높은 점수를 부여하여 디렉토리 점프에 활용.
    fn filter_contains(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let tokens = split_query_tokens(query);

        // 단일 토큰: 기존 동작 100% 보존 (score: 0, substring match)
        if tokens.len() <= 1 {
            let query_lower = tokens.first().map(|s| s.as_str()).unwrap_or("");
            // 초성 질의(`ㅋㅋㅇㅌ` → 카카오톡) — 이름의 초성 문자열에서 찾는다 (docs/22 §3.10)
            if crate::hangul::is_choseong_query(query_lower) {
                return self
                    .all_items
                    .iter()
                    .zip(self.lowercase_cache.entries.iter())
                    .filter(|(_, lc)| {
                        crate::hangul::choseong_string(&lc.name).contains(query_lower)
                    })
                    .take(limit)
                    .map(|(item, _)| SearchResult {
                        item: item.clone(),
                        score: 0,
                    })
                    .collect();
            }
            // 치는 중인 마지막 음절(`보곳` → 보고서) — 이름에서만 본다
            let partial = crate::hangul::split_partial(query_lower);
            return self
                .all_items
                .iter()
                .zip(self.lowercase_cache.entries.iter())
                .filter(|(_, lc)| {
                    lc.name.contains(query_lower)
                        || lc.path.contains(query_lower)
                        || lc.keywords.contains(query_lower)
                        || partial.as_ref().is_some_and(|p| {
                            crate::hangul::partial_matches(&lc.name, p).next().is_some()
                        })
                })
                .take(limit)
                .map(|(item, _)| SearchResult {
                    item: item.clone(),
                    score: 0,
                })
                .collect();
        }

        // ── 멀티 토큰: AND 매칭 + 가중 스코어링 ──
        // 점수 상수 — Score Pollution 방어를 위해 search_score 범위를 넓게 설정
        const SEGMENT_EXACT: u32 = 60; // 경로 세그먼트 정확 일치
        const PATH_CONTAINS: u32 = 30; // 경로 내 substring
        const NAME_CONTAINS: u32 = 15; // 이름 매칭
        const KW_CONTAINS: u32 = 5; // 키워드 매칭
        const ALL_SEGMENTS_BONUS: u32 = 80; // 전 토큰 세그먼트 정확 일치 보너스

        let token_count = tokens.len() as u32;
        let last_partial = tokens.last().and_then(|t| crate::hangul::split_partial(t));

        // 점수는 행 번호와 함께만 모으고, 항목 복제는 상위 `limit` 개만 —
        // 흔한 단어("보고서")면 수십만 개가 통과해 전부 복제하는 비용이 컸다.
        let mut scored: Vec<(u32, usize)> = self
            .lowercase_cache
            .entries
            .iter()
            .enumerate()
            .filter_map(|(row, lc)| {
                let mut total_score: u32 = 0;
                let mut exact_segments: u32 = 0;

                for (i, token) in tokens.iter().enumerate() {
                    let t = token.as_str();
                    // 우선순위 기반 점수 — 최고 점수만 적용.
                    // 세그먼트 정확 일치는 경로 포함의 부분집합이라 포함부터 본다 — 예전에는
                    // 항목마다 경로 조각 HashSet 을 먼저 만들어 대부분 탈락할 항목에도
                    // 할당을 치렀다(40만 개 "보고서 최종" 188ms, docs/22 §2.2).
                    if lc.path.contains(t) {
                        if lc.path.split(['\\', '/']).any(|seg| seg == t) {
                            total_score += SEGMENT_EXACT;
                            exact_segments += 1;
                        } else {
                            total_score += PATH_CONTAINS;
                        }
                    } else if lc.name.contains(t) {
                        total_score += NAME_CONTAINS;
                    } else if lc.keywords.contains(t) {
                        total_score += KW_CONTAINS;
                    } else if i + 1 == tokens.len()
                        && last_partial.as_ref().is_some_and(|p| {
                            crate::hangul::partial_matches(&lc.name, p).next().is_some()
                        })
                    {
                        total_score += NAME_CONTAINS;
                    } else {
                        return None; // AND 조건: 하나라도 미매칭이면 제외
                    }
                }

                // 모든 토큰이 세그먼트 정확 일치 → 완전한 경로 의도 매칭 보너스
                if exact_segments == token_count {
                    total_score += ALL_SEGMENTS_BONUS;
                }

                Some((total_score, row))
            })
            .collect();

        // 안정 정렬 — 같은 점수는 색인 순서 그대로 (예전 동작과 같다)
        scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
        scored
            .into_iter()
            .take(limit)
            .map(|(score, row)| SearchResult {
                item: self.all_items[row].clone(),
                score,
            })
            .collect()
    }

    /// Total loaded items
    pub fn total_items(&self) -> usize {
        self.all_items.len()
    }
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── URL helpers ─────────────────────────────────────────

fn is_url(s: &str) -> bool {
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("www.")
        || (s.contains('.')
            && !s.contains(' ')
            && !s.contains('*')
            && !s.starts_with('.')
            && matches_domain_pattern(s))
}

/// 통용 TLD 화이트리스트 — "알파벳 2~6자"만으로 판정하면 report.pdf,
/// readme.md, config.toml 같은 파일명이 전부 URL로 오판정되어 파일 검색이
/// 막힌다. md/rs/sh/ts/zip/mov 처럼 파일 확장자와 충돌하는 TLD는 의도적으로
/// 제외한다 (명시적으로 열려면 https:// 또는 www. 접두사 사용).
const KNOWN_TLDS: &[&str] = &[
    "com", "net", "org", "io", "dev", "app", "ai", "co", "kr", "jp", "cn", "us", "uk", "de", "fr",
    "it", "nl", "es", "se", "no", "fi", "pl", "ch", "at", "be", "dk", "cz", "pt", "gr", "ru", "br",
    "in", "au", "ca", "mx", "tw", "hk", "sg", "id", "th", "vn", "ph", "my", "nz", "tr", "il", "za",
    "eu", "tv", "me", "cc", "gg", "fm", "to", "ly", "info", "biz", "xyz", "site", "online",
    "store", "tech", "blog", "news", "wiki", "cloud", "edu", "gov", "mil", "int",
];

fn matches_domain_pattern(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let tld = parts.last().unwrap_or(&"");
    let tld_part = tld.split('/').next().unwrap_or(tld);
    KNOWN_TLDS.contains(&tld_part.to_ascii_lowercase().as_str())
}

fn normalize_url(s: &str) -> String {
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{}", s)
    }
}

// ── Glob matcher ────────────────────────────────────────

/// `*`/`?` 글롭 매처 — 반복형(백트래킹 지점 하나) O(n·m).
///
/// 예전 구현은 `*`마다 재귀로 모든 분할을 시도해 `*a*a*a*a*b` 같은 질의에서 항목마다
/// 지수적으로 느려졌다. 키 입력 경로라 정규식처럼(ReDoS 방어) 최악 비용을 막아야 한다.
pub(crate) struct GlobMatcher {
    toks: Vec<GlobTok>,
}

#[derive(Clone, Copy, PartialEq)]
enum GlobTok {
    Char(char),
    /// `?` — 임의의 한 문자
    Any,
    /// `*` — 임의 길이 (연속 `*`는 하나로 합친다)
    Star,
}

impl GlobMatcher {
    pub(crate) fn new(pattern: &str) -> Self {
        let mut toks: Vec<GlobTok> = Vec::new();
        for ch in pattern.chars() {
            let tok = match ch {
                '*' => GlobTok::Star,
                '?' => GlobTok::Any,
                c => GlobTok::Char(c),
            };
            if tok == GlobTok::Star && toks.last() == Some(&GlobTok::Star) {
                continue;
            }
            toks.push(tok);
        }
        Self { toks }
    }

    pub(crate) fn matches(&self, text: &str) -> bool {
        let (mut p, mut ti) = (0usize, 0usize);
        // 마지막 `*`의 (토큰 위치, 그 `*`가 지금까지 삼킨 끝 바이트 위치)
        let mut star: Option<(usize, usize)> = None;
        while let Some(ch) = text[ti..].chars().next() {
            match self.toks.get(p) {
                Some(GlobTok::Star) => {
                    star = Some((p, ti));
                    p += 1;
                    continue;
                }
                Some(GlobTok::Any) => {
                    p += 1;
                    ti += ch.len_utf8();
                    continue;
                }
                Some(GlobTok::Char(c)) if *c == ch => {
                    p += 1;
                    ti += ch.len_utf8();
                    continue;
                }
                _ => {}
            }
            // 불일치 — 마지막 `*`가 한 글자 더 삼키게 하고 그 뒤부터 다시 맞춘다
            let Some((sp, st)) = star else {
                return false;
            };
            let Some(skipped) = text[st..].chars().next() else {
                return false;
            };
            let next = st + skipped.len_utf8();
            star = Some((sp, next));
            p = sp + 1;
            ti = next;
        }
        // 텍스트를 다 소비했다 — 남은 토큰은 `*`뿐이어야 한다
        self.toks[p..].iter().all(|t| *t == GlobTok::Star)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_반복형_매칭_의미() {
        let cases = [
            ("*.doc", "report.doc", true),
            ("*.doc", "report.docx", false),
            ("re?ort*", "report.pdf", true),
            ("*보고서*", "2026 보고서 최종.hwp", true),
            ("a*b*c", "axxbyyc", true),
            ("a*b*c", "axxbyy", false),
            ("**", "", true),
            ("?", "", false),
            ("?", "한", true),
            ("*a", "ba", true),
            ("a*", "b", false),
        ];
        for (pat, text, want) in cases {
            assert_eq!(GlobMatcher::new(pat).matches(text), want, "{pat} vs {text}");
        }
    }

    #[test]
    fn glob_최악_입력도_빠르게_끝난다() {
        // 예전 재귀 구현은 이런 입력에서 지수 시간이었다
        let m = GlobMatcher::new("*a*a*a*a*a*a*a*a*b");
        let text = "a".repeat(200);
        let started = std::time::Instant::now();
        assert!(!m.matches(&text));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn test_search_mode_detect() {
        assert_eq!(SearchMode::detect("hello").0, SearchMode::Fuzzy);
        assert_eq!(SearchMode::detect("*.doc").0, SearchMode::Glob);
        assert_eq!(SearchMode::detect("test*").0, SearchMode::Glob);
        assert_eq!(SearchMode::detect("/test\\d+/").0, SearchMode::Regex);
        // Unix 절대경로 형태는 정규식으로 오판하지 않는다
        assert_eq!(SearchMode::detect("/usr/bin/").0, SearchMode::Fuzzy);
        assert_eq!(SearchMode::detect("/home/user/docs/").0, SearchMode::Fuzzy);
        assert_eq!(SearchMode::detect(".pdf").0, SearchMode::Glob);
    }

    #[test]
    fn test_search_mode_cjk() {
        assert_eq!(SearchMode::detect("한글").0, SearchMode::Contains);
    }

    #[test]
    fn test_search_mode_url() {
        assert_eq!(SearchMode::detect("google.com").0, SearchMode::Url);
        assert_eq!(SearchMode::detect("https://example.com").0, SearchMode::Url);
        assert_eq!(SearchMode::detect("www.example.abcxyz").0, SearchMode::Url);
    }

    #[test]
    fn test_filenames_are_not_urls() {
        // 파일 확장자가 URL(TLD)로 오판정되면 파일 검색이 불가능해진다
        for name in [
            "report.pdf",
            "readme.md",
            "config.toml",
            "main.rs",
            "run.sh",
            "notes.txt",
            "photo.jpeg",
            "archive.zip",
        ] {
            assert_ne!(
                SearchMode::detect(name).0,
                SearchMode::Url,
                "{name}은 URL이 아니라 파일명으로 취급되어야 함"
            );
        }
    }

    #[test]
    fn test_glob_matcher() {
        let m = GlobMatcher::new("*.doc");
        assert!(m.matches("report.doc"));
        assert!(!m.matches("report.pdf"));

        let m2 = GlobMatcher::new("test*");
        assert!(m2.matches("test_file.rs"));
        assert!(!m2.matches("my_test"));

        let m3 = GlobMatcher::new("*report*");
        assert!(m3.matches("my_report_2024.doc"));
    }

    #[test]
    fn test_search_engine_basic() {
        use crate::index::{ItemKind, Source};

        let mut engine = SearchEngine::new();
        engine.load(vec![
            IndexItem {
                name: "Firefox".to_string(),
                path: "/usr/bin/firefox".to_string(),
                kind: ItemKind::App,
                source: Source::Apps,
                icon: "\u{1F4E6}".to_string(),
                keywords: "firefox browser web".to_string(),
                icon_path: None,
            },
            IndexItem {
                name: "VS Code".to_string(),
                path: "/usr/bin/code".to_string(),
                kind: ItemKind::App,
                source: Source::Apps,
                icon: "\u{1F4E6}".to_string(),
                keywords: "code editor vscode".to_string(),
                icon_path: None,
            },
        ]);

        let (mode, results) = engine.search("fire", 10);
        assert_eq!(mode, SearchMode::Fuzzy);
        assert!(!results.is_empty());
        assert_eq!(results[0].item.name, "Firefox");
    }

    /// 실사례(2026-10-07): ~/Documents/미닉스 청소기1.jpg가 `미닉스`로 0건이었다.
    /// macOS가 파일명을 NFD(자모 분리)로 저장해 타이핑한 NFC 질의와 달랐다.
    #[test]
    fn nfd_파일명도_nfc_질의로_찾는다() {
        use crate::index::{ItemKind, Source};
        // "미닉스 청소기1.jpg" — 디스크에 실제로 있던 NFD 형태
        let nfd_name = "\u{1106}\u{1175}\u{1102}\u{1175}\u{11A8}\u{1109}\u{1173} \
                        \u{110E}\u{1165}\u{11BC}\u{1109}\u{1169}\u{1100}\u{1175}1.jpg";
        let path = format!("/Users/me/Documents/{nfd_name}");
        let mut engine = SearchEngine::with_items(
            &KindWeights::default(),
            vec![IndexItem {
                name: nfd_name.to_string(),
                path: path.clone(),
                kind: ItemKind::File,
                source: Source::FileProvider,
                icon: String::new(),
                keywords: path.clone(),
                icon_path: None,
            }],
        );

        for q in ["미닉스", "미닉스 청소", "청소기"] {
            let (_, results) = engine.search(q, 10);
            assert_eq!(results.len(), 1, "질의 {q:?}가 NFD 파일명을 못 찾음");
            assert_eq!(results[0].item.path, path, "열 때 쓰는 경로는 원본 그대로");
        }
        // 반대 방향(NFD 질의)도
        let nfd_query = "\u{1106}\u{1175}\u{1102}\u{1175}\u{11A8}\u{1109}\u{1173}";
        let (_, results) = engine.search(nfd_query, 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn with_items_적용_가중치와_항목() {
        use crate::index::{ItemKind, Source};

        let weights = KindWeights {
            app: 999, // 기본값과 확실히 다른 값
            ..KindWeights::default()
        };

        let mut engine = SearchEngine::with_items(
            &weights,
            vec![IndexItem {
                name: "Firefox".to_string(),
                path: "/usr/bin/firefox".to_string(),
                kind: ItemKind::App,
                source: Source::Apps,
                icon: String::new(),
                keywords: "browser".to_string(),
                icon_path: None,
            }],
        );

        // 항목이 적재됐고, 가중치도 함께 적용됐다 — 둘 중 하나만 되는 일이
        // 생기지 않게 하는 것이 이 API의 목적이다.
        assert_eq!(engine.len(), 1);
        assert_eq!(engine.kind_weights.app, 999);
        let (_, results) = engine.search("firefox", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn reload_가중치와_항목_동시_교체() {
        use crate::index::{ItemKind, Source};

        let item = |name: &str| IndexItem {
            name: name.to_string(),
            path: format!("/usr/bin/{name}"),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: String::new(),
            keywords: String::new(),
            icon_path: None,
        };

        let mut engine = SearchEngine::with_items(&KindWeights::default(), vec![item("Firefox")]);

        let weights = KindWeights {
            app: 42,
            ..KindWeights::default()
        };
        engine.reload(&weights, vec![item("Chrome")]);

        assert_eq!(engine.kind_weights.app, 42);
        assert_eq!(engine.len(), 1);
        let (_, results) = engine.search("firefox", 10);
        assert!(
            results.is_empty(),
            "reload는 이전 항목을 비워야 한다 (load와 같은 보장)"
        );
    }

    #[test]
    fn test_reload_replaces_previous_items() {
        use crate::index::{ItemKind, Source};

        let firefox = IndexItem {
            name: "Firefox".to_string(),
            path: "/usr/bin/firefox".to_string(),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: "\u{1F4E6}".to_string(),
            keywords: "firefox browser web".to_string(),
            icon_path: None,
        };

        let mut engine = SearchEngine::new();
        engine.load(vec![firefox.clone()]);
        // 인덱스 리빌드 시나리오: 같은 아이템으로 재로드
        engine.load(vec![firefox]);

        let (_, results) = engine.search("firefox", 10);
        assert_eq!(
            results.len(),
            1,
            "재로드 후 fuzzy 결과에 중복 아이템이 남으면 안 됨"
        );

        // 아이템이 제거된 리빌드 반영 확인
        engine.load(vec![]);
        let (_, results) = engine.search("firefox", 10);
        assert!(
            results.is_empty(),
            "재로드로 제거된 아이템은 검색되지 않아야 함"
        );
    }

    #[test]
    fn test_kind_weight_boost() {
        use crate::index::{ItemKind, Source};

        let mut engine = SearchEngine::new();
        engine.set_kind_weights(KindWeights {
            directory: 80,
            app: 70,
            file: 50,
            ..Default::default()
        });

        engine.load(vec![
            IndexItem {
                name: "Downloads".to_string(),
                path: "/home/user/Downloads".to_string(),
                kind: ItemKind::Directory,
                source: Source::FileProvider,
                icon: ">>".to_string(),
                keywords: "downloads".to_string(),
                icon_path: None,
            },
            IndexItem {
                name: "download.txt".to_string(),
                path: "/home/user/download.txt".to_string(),
                kind: ItemKind::File,
                source: Source::FileProvider,
                icon: "\u{1F4DD}".to_string(),
                keywords: "download text".to_string(),
                icon_path: None,
            },
        ]);

        let results = engine.search_with_mode(SearchMode::Contains, "download", 10);
        assert_eq!(results.len(), 2);
        // Directory should be first due to higher weight
        assert_eq!(results[0].item.kind, ItemKind::Directory);
    }

    // ── 멀티 토큰 매칭 테스트 ──────────────────────────────

    /// 테스트용 IndexItem 생성 헬퍼
    fn make_item(name: &str, path: &str, keywords: &str) -> IndexItem {
        use crate::index::{ItemKind, Source};
        IndexItem {
            name: name.to_string(),
            path: path.to_string(),
            kind: ItemKind::Directory,
            source: Source::FileProvider,
            icon: ">>".to_string(),
            keywords: keywords.to_string(),
            icon_path: None,
        }
    }

    #[test]
    fn 초성_질의는_이름의_초성에서_찾는다() {
        let mut engine = SearchEngine::new();
        engine.load(vec![
            make_item("카카오톡", "/Applications/KakaoTalk.app", ""),
            make_item("카카오맵", "/Applications/KakaoMap.app", ""),
            make_item("메모", "/Applications/Notes.app", ""),
        ]);
        let (mode, r) = engine.search("ㅋㅋㅇㅌ", 10);
        assert_eq!(mode, SearchMode::Contains);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.name, "카카오톡");
        assert_eq!(engine.search("ㅋㅋㅇ", 10).1.len(), 2);
    }

    #[test]
    fn 치는_중인_마지막_음절도_찾는다() {
        let mut engine = SearchEngine::new();
        engine.load(vec![
            make_item("카카오톡", "/Applications/KakaoTalk.app", ""),
            make_item("카카오맵", "/Applications/KakaoMap.app", ""),
            make_item("보고서.hwp", "/docs/2026/보고서.hwp", ""),
        ]);
        // `카카오톡` 을 치는 도중의 `카카옽` → 카카오 + 초성 ㅌ
        let r = engine.search("카카옽", 10).1;
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.name, "카카오톡");
        assert_eq!(engine.search("카카오ㅁ", 10).1[0].item.name, "카카오맵");
        // 여러 단어: 마지막 토큰만
        assert_eq!(engine.search("2026 보곳", 10).1.len(), 1);
    }

    #[test]
    fn 퍼지가_덜_끝나면_이전_질의_결과를_내지_않고_미완성으로_표시() {
        // docs/22 §2.2 재현: 항목이 많으면 tick 한 번 안에 매칭이 안 끝나 `2` → 0건,
        // 이어 `zzzz` → 이전 결과 50건이 나왔다.
        let items: Vec<IndexItem> = (0..200_000)
            .map(|i| make_item(&format!("file{i}.txt"), &format!("/d/file{i}.txt"), ""))
            .collect();
        let mut engine = SearchEngine::new();
        engine.load(items);
        engine.set_fuzzy_wait(Duration::ZERO);

        let first = engine.search_with_mode(SearchMode::Fuzzy, "2", 50);
        if engine.is_pending() {
            assert!(first.is_empty(), "미완성이면 빈 결과");
        }
        let stale = engine.search_with_mode(SearchMode::Fuzzy, "zzzz", 50);
        assert!(
            stale.is_empty(),
            "z 가 든 항목은 없다 — 이전 질의 결과가 남으면 안 된다"
        );

        // 다시 물으면(UI 재검색) 완성된다
        engine.set_fuzzy_wait(Duration::from_secs(10));
        let done = engine.search_with_mode(SearchMode::Fuzzy, "123", 50);
        assert!(!engine.is_pending());
        assert_eq!(done.len(), 50);
        engine.clear_pending();
        assert!(!engine.is_pending());
    }

    #[test]
    fn test_multi_token_korean_path_segments() {
        // "2026 출장이력" → c:\2026\work\출장이력 매칭, 세그먼트 정확 일치 보너스
        let mut engine = SearchEngine::new();
        engine.load(vec![
            make_item("출장이력", r"c:\2026\work\출장이력", "출장이력"),
            make_item("출장이력_old", r"c:\2025\archive\출장이력_old", "출장이력"),
            make_item("readme", r"c:\2026\docs\readme.txt", "문서"),
        ]);

        let results = engine.search_with_mode(SearchMode::Contains, "2026 출장이력", 10);
        assert!(!results.is_empty(), "결과가 있어야 함");
        // 첫 번째 결과: 두 토큰 모두 세그먼트 정확 일치 → 최고 점수
        assert_eq!(results[0].item.path, r"c:\2026\work\출장이력");
        // "readme"는 "출장이력" 토큰 미매칭이므로 제외
        assert!(
            results.iter().all(|r| r.item.name != "readme"),
            "readme는 AND 조건 불충족으로 제외"
        );
    }

    #[test]
    fn test_multi_token_order_independent() {
        // "출장이력 2026" → 토큰 순서 무관, 동일 결과
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "출장이력",
            r"c:\2026\work\출장이력",
            "출장이력",
        )]);

        let r1 = engine.search_with_mode(SearchMode::Contains, "2026 출장이력", 10);
        let r2 = engine.search_with_mode(SearchMode::Contains, "출장이력 2026", 10);

        assert_eq!(r1.len(), r2.len(), "순서와 무관하게 같은 수의 결과");
        assert_eq!(r1[0].score, r2[0].score, "순서와 무관하게 같은 점수");
    }

    #[test]
    fn test_single_token_preserves_original_behavior() {
        // 단일 토큰은 기존 동작 보존: score=0, substring match
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "출장이력",
            r"c:\2026\work\출장이력",
            "출장이력",
        )]);

        let results = engine.search_with_mode(SearchMode::Contains, "출장이력", 10);
        assert_eq!(results.len(), 1);
        // 단일 토큰은 score: 0 (kind_boost 적용 전)
        // apply_kind_boost는 search_with_mode에서 적용되므로 여기서는 kind_boost만 반영됨
        // filter_contains 자체는 score 0 반환
    }

    #[test]
    fn test_multi_token_segment_exact_vs_substring() {
        // 세그먼트 정확 일치(+60)와 path substring(+30) 점수 차이 확인
        let mut engine = SearchEngine::new();
        engine.load(vec![
            // "출장" 토큰이 세그먼트가 아닌 substring으로 매칭
            make_item("출장이력_보고서", r"c:\2026\work\출장이력_보고서", "출장"),
            // "출장이력" 토큰이 세그먼트 정확 일치
            make_item("출장이력", r"c:\2026\work\출장이력", "출장이력"),
        ]);

        let results = engine.search_with_mode(SearchMode::Contains, "2026 출장이력", 10);
        assert!(!results.is_empty());
        // 세그먼트 정확 일치 항목이 더 높은 점수
        assert_eq!(
            results[0].item.path, r"c:\2026\work\출장이력",
            "세그먼트 정확 일치가 우선"
        );
    }

    #[test]
    fn test_multi_token_windows_backslash() {
        // Windows 경로 backslash 세그먼트 분리 정상 동작
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "projects",
            r"D:\dev\projects\rust",
            "rust dev",
        )]);

        let results = engine.search_with_mode(SearchMode::Contains, "dev rust", 10);
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0, "멀티 토큰은 0 이상의 점수");
    }

    #[test]
    fn test_multi_token_mixed_korean_english() {
        // 한영 혼합 쿼리: "project 보고서"
        let mut engine = SearchEngine::new();
        engine.load(vec![
            make_item("보고서", r"c:\project\보고서", "보고서 project"),
            make_item("readme", r"c:\project\readme.md", "project"),
        ]);

        let results = engine.search_with_mode(SearchMode::Contains, "project 보고서", 10);
        assert_eq!(results.len(), 1, "readme는 '보고서' 토큰 미매칭으로 제외");
        assert_eq!(results[0].item.name, "보고서");
    }

    #[test]
    fn test_multi_token_no_match_returns_empty() {
        // 매칭되는 항목이 없으면 빈 결과
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "출장이력",
            r"c:\2026\work\출장이력",
            "출장이력",
        )]);

        let results = engine.search_with_mode(SearchMode::Contains, "2025 회의록", 10);
        assert!(results.is_empty(), "매칭 없으면 빈 결과");
    }

    #[test]
    fn test_multi_token_input_with_path_separator() {
        // 입력에 경로 구분자가 포함된 경우 토큰으로 분리
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "출장이력",
            r"c:\2026\work\출장이력",
            "출장이력",
        )]);

        // "work\출장이력" → ["work", "출장이력"] 으로 분리되어 매칭
        let results = engine.search_with_mode(SearchMode::Contains, r"work\출장이력", 10);
        assert_eq!(results.len(), 1, "경로 구분자가 토큰 구분자로 처리됨");
    }

    #[test]
    fn test_all_segments_exact_bonus() {
        // 모든 토큰이 세그먼트 정확 일치 → ALL_SEGMENTS_BONUS(+80) 적용
        let mut engine = SearchEngine::new();
        engine.load(vec![
            // "2026"과 "work" 모두 세그먼트 정확 일치
            make_item("work", r"c:\2026\work", "작업폴더"),
            // "2026"은 정확, "work"는 path substring (workforce의 일부)
            make_item("workforce", r"c:\2026\workforce_data", "인력"),
        ]);

        let results = engine.search_with_mode(SearchMode::Contains, "2026 work", 10);
        assert!(!results.is_empty());
        // 두 토큰 모두 세그먼트 정확 일치한 항목이 보너스로 1등
        assert_eq!(results[0].item.name, "work");
        if results.len() >= 2 {
            assert!(
                results[0].score > results[1].score,
                "전 세그먼트 일치 보너스로 점수 차이 발생"
            );
        }
    }

    // ── 퍼지 오탐 방지 (경로가 keywords에 들어간 항목) ──────────

    fn make_app(name: &str, path: &str, keywords: &str) -> IndexItem {
        use crate::index::{ItemKind, Source};
        IndexItem {
            name: name.to_string(),
            path: path.to_string(),
            kind: ItemKind::App,
            source: Source::Apps,
            icon: String::new(),
            keywords: keywords.to_string(),
            icon_path: None,
        }
    }

    /// 시작 메뉴 .lnk 처럼 keywords = 전체 경로인 앱 목록
    fn start_menu_apps() -> Vec<IndexItem> {
        let base = r"C:\Users\mspma\AppData\Roaming\Microsoft\Windows\Start Menu\Programs";
        ["Magnify", "Narrator", "VoiceAccess", "On-Screen Keyboard"]
            .iter()
            .map(|name| {
                let path = format!(r"{base}\Accessibility\{name}.lnk");
                make_app(name, &path, &path)
            })
            .collect()
    }

    #[test]
    fn 퍼지는_경로_글자로_매칭하지_않는다() {
        // `gmail` 이 경로의 흩어진 글자(Roamin[g]\[M]icrosoft\St[a]rt...)로
        // 시작 메뉴 바로가기 전부에 매칭되던 회귀 방지
        let mut engine = SearchEngine::new();
        engine.load(start_menu_apps());

        let (mode, results) = engine.search("gmail", 50);
        assert_eq!(mode, SearchMode::Fuzzy);
        assert!(
            results.is_empty(),
            "경로 글자만으로 매칭되면 안 됨: {:?}",
            results.iter().map(|r| &r.item.name).collect::<Vec<_>>()
        );

        // 이름 매칭은 그대로
        let (_, results) = engine.search("magn", 50);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.name, "Magnify");
    }

    #[test]
    fn 퍼지는_aumid로_매칭하지_않는다() {
        // Get-StartApps 항목: keywords = "shell:appsFolder\<AUMID> <AUMID>"
        let aumid = "Chrome._crx_fmgjjmmmlfnkbppncabfkddbjimcfncm";
        let path = format!(r"shell:appsFolder\{aumid}");
        let mut engine = SearchEngine::new();
        engine.load(vec![make_app("YouTube", &path, &format!("{path} {aumid}"))]);

        let (_, results) = engine.search("gmail", 50);
        assert!(results.is_empty(), "AUMID 글자로 매칭되면 안 됨");
    }

    #[test]
    fn 퍼지는_사람이_붙인_키워드는_유지한다() {
        // 시스템 명령: path가 경로가 아니므로 keywords가 전부 유지된다
        let mut engine = SearchEngine::new();
        engine.load(vec![make_app(
            "Recycle Bin",
            "explorer",
            "trash, recyclebin, 휴지통",
        )]);

        let (_, results) = engine.search("trash", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn 퍼지_멀티토큰은_경로_세그먼트로도_찾는다() {
        // 이름에는 없는 경로 조각(`dev rust`)으로 찾던 기존 동작 유지
        let mut engine = SearchEngine::new();
        engine.load(vec![
            make_item("projects", r"D:\dev\projects\rust", r"D:\dev\projects\rust"),
            make_item("notes", r"D:\docs\notes", r"D:\docs\notes"),
        ]);

        let (mode, results) = engine.search("dev rust", 10);
        assert_eq!(mode, SearchMode::Fuzzy);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item.name, "projects");

        // 경로 구분자 입력도 토큰으로 나뉘어 매칭
        let (_, results) = engine.search(r"dev\projects", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn 퍼지_멀티토큰은_중복없이_합친다() {
        // 이름 퍼지와 경로 세그먼트 양쪽에 걸린 항목은 한 번만 나온다
        let mut engine = SearchEngine::new();
        engine.load(vec![make_item(
            "rust book",
            r"D:\rust\book",
            r"D:\rust\book",
        )]);

        let (_, results) = engine.search("rust book", 10);
        assert_eq!(results.len(), 1);
    }
}
