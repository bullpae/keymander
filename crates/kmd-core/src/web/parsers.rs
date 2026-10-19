//! 쿼리 파서 — @prefix 기반 입력 분류와 쿼리 추출

use super::services::*;

/// @ 쿼리의 분류 결과. TUI/Desktop에서 match 한 번으로 처리할 수 있도록 통합.
pub enum WebQueryResult {
    Spell(String),
    Translate(TranslateDirection, String),
    MultiLlm(Vec<&'static WebService>, String),
    MultiWeb(Vec<&'static WebService>, String),
    Single(&'static WebService, String),
    /// @ 필터 브라우징 (빈 쿼리 또는 서비스 미매칭)
    Browse(String),
}

/// 쿼리 분류에 필요한 설정 묶음.
///
/// TUI/Desktop 양쪽에서 `classify_web_query`를 호출할 때
/// 사용자 설정 값을 한데 모아 전달하기 위한 구조체.
pub struct WebQueryConfig<'a> {
    /// 맞춤법 검사 명령에 대한 사용자 별칭 (예: `["@sp", "@spell"]`)
    pub spell_prefixes: &'a [String],
    /// 번역 명령에 대한 사용자 별칭 (예: `["@tr", "@trko", "@tren"]`)
    pub translate_prefixes: &'a [String],
    /// 멀티 LLM 명령에 대한 사용자 별칭 (예: `["@ll", "@llm"]`)
    pub multi_llm_prefixes: &'a [String],
    /// 사용자가 선택한 LLM provider ID 목록 (예: `["chatgpt", "claude"]`)
    pub multi_llm_ids: &'a [String],
    /// 멀티 웹검색 명령에 대한 사용자 별칭 (예: `["@m", "@msearch"]`)
    pub multi_web_prefixes: &'a [String],
    /// 사용자가 선택한 웹 검색 엔진 ID 목록 (예: `["google", "naver_search"]`)
    pub multi_web_ids: &'a [String],
}

impl<'a> WebQueryConfig<'a> {
    /// 런처 설정에서 바로 만든다 — UI가 별칭·선택 목록을 필드로 복사해 두면
    /// 설정 변경 때마다 두 벌을 맞춰야 해 어긋난다(예전 데스크톱/TUI 미러 필드).
    pub fn from_launcher(launcher: &'a crate::config::LauncherConfig) -> Self {
        Self {
            spell_prefixes: &launcher.spell_prefixes,
            translate_prefixes: &launcher.translate_prefixes,
            multi_llm_prefixes: &launcher.multi_llm_prefixes,
            multi_llm_ids: &launcher.multi_llm_providers,
            multi_web_prefixes: &launcher.multi_web_prefixes,
            multi_web_ids: &launcher.multi_web_providers,
        }
    }
}

/// 통합 쿼리 분류기 — if-else 체인을 한 곳에 집약
pub fn classify_web_query(input: &str, cfg: &WebQueryConfig<'_>) -> WebQueryResult {
    if let Some(q) = parse_spell_query_with_prefixes(input, cfg.spell_prefixes) {
        return WebQueryResult::Spell(q);
    }
    if let Some((dir, q)) = parse_translate_query_with_prefixes(input, cfg.translate_prefixes) {
        return WebQueryResult::Translate(dir, q);
    }
    if let Some((svcs, q)) =
        parse_multi_llm_query_with_prefixes(input, cfg.multi_llm_ids, cfg.multi_llm_prefixes)
    {
        return WebQueryResult::MultiLlm(svcs, q);
    }
    if let Some((svcs, q)) =
        parse_multi_web_query_with_prefixes(input, cfg.multi_web_ids, cfg.multi_web_prefixes)
    {
        return WebQueryResult::MultiWeb(svcs, q);
    }
    if let Some((svc, q)) = parse_web_query(input) {
        return WebQueryResult::Single(svc, q);
    }
    let filter = input.trim_start_matches('@').to_string();
    WebQueryResult::Browse(filter)
}

// ── 개별 파서 ─────────────────────────────────────────────────────────────────

/// @prefix 쿼리 → (서비스, 쿼리 텍스트)
pub fn parse_web_query(input: &str) -> Option<(&'static WebService, String)> {
    if !input.starts_with('@') {
        return None;
    }
    let first_space = input.find(' ');
    let prefix_raw = &input[..first_space.unwrap_or(input.len())];
    let query = first_space
        .map(|i| input[i + 1..].trim())
        .unwrap_or("")
        .to_string();
    WEB_SERVICES
        .iter()
        .find(|s| {
            s.prefixes
                .iter()
                .any(|p| p.eq_ignore_ascii_case(prefix_raw))
        })
        .map(|s| (s, query))
}

/// 멀티 LLM 쿼리 파서 (기본 별칭)
pub fn parse_multi_llm_query(
    input: &str,
    selected_ids: &[String],
) -> Option<(Vec<&'static WebService>, String)> {
    parse_multi_llm_query_with_prefixes(input, selected_ids, &[])
}

/// 단일(`@gpt`) 또는 멀티(`@llm`) LLM 쿼리를 통합 파싱 → (LLM 서비스들, 프롬프트).
///
/// 프런트엔드가 오토파일럿 실행 계획을 세우기 위한 진입점. LLM이 아닌 쿼리
/// (일반 웹검색 `@g` 등)는 None.
pub fn parse_any_llm_query(
    input: &str,
    selected_ids: &[String],
    multi_prefixes: &[String],
) -> Option<(Vec<&'static WebService>, String)> {
    // 멀티 우선
    if let Some((svcs, q)) =
        parse_multi_llm_query_with_prefixes(input, selected_ids, multi_prefixes)
    {
        return Some((svcs, q));
    }
    // 단일 — LLM 서비스일 때만
    let (svc, q) = parse_web_query(input)?;
    if is_llm_id(svc.id) {
        Some((vec![svc], q))
    } else {
        None
    }
}

/// 이어서 질문 파서 — `@@ <프롬프트>` → 후속 프롬프트 텍스트.
///
/// 직전에 오토파일럿으로 연 LLM 창들에 이어서 같은 질문을 보낸다 (docs/09).
/// 프롬프트가 비어 있으면 None (브라우징 힌트로 처리되게).
pub fn parse_llm_followup(input: &str) -> Option<String> {
    let (prefix, query) = split_prefix_query(input)?;
    if prefix != "@@" {
        return None;
    }
    if query.trim().is_empty() {
        return None;
    }
    Some(query)
}

/// 멀티 LLM 쿼리 파서 (사용자 별칭 지원)
pub fn parse_multi_llm_query_with_prefixes(
    input: &str,
    selected_ids: &[String],
    prefixes: &[String],
) -> Option<(Vec<&'static WebService>, String)> {
    let (prefix, query) = split_prefix_query(input)?;
    let aliases = normalized_aliases(prefixes, &["@llm", "@ll", "@multi", "@cmp", "@compare"]);
    if !aliases.iter().any(|p| p == &prefix) {
        return None;
    }
    Some((selected_llm_services(selected_ids), query))
}

/// 멀티 웹 쿼리 파서 (기본 별칭)
pub fn parse_multi_web_query(
    input: &str,
    selected_ids: &[String],
) -> Option<(Vec<&'static WebService>, String)> {
    parse_multi_web_query_with_prefixes(input, selected_ids, &[])
}

/// 멀티 웹 쿼리 파서 (사용자 별칭 지원)
pub fn parse_multi_web_query_with_prefixes(
    input: &str,
    selected_ids: &[String],
    prefixes: &[String],
) -> Option<(Vec<&'static WebService>, String)> {
    let (prefix, query) = split_prefix_query(input)?;
    let aliases = normalized_aliases(
        prefixes,
        &[
            "@m",
            "@mw",
            "@msearch",
            "@multisearch",
            "@searchall",
            "@krsearch",
        ],
    );
    if !aliases.iter().any(|p| p == &prefix) {
        return None;
    }
    Some((selected_multi_web_services(selected_ids), query))
}

/// 맞춤법 검사 쿼리 파서
pub fn parse_spell_query_with_prefixes(input: &str, prefixes: &[String]) -> Option<String> {
    let (prefix, query) = split_prefix_query(input)?;
    let aliases = normalized_aliases(prefixes, &["@sp", "@spell"]);
    if !aliases.iter().any(|p| p == &prefix) {
        return None;
    }
    Some(query)
}

/// 번역 쿼리 파서
pub fn parse_translate_query_with_prefixes(
    input: &str,
    prefixes: &[String],
) -> Option<(TranslateDirection, String)> {
    let (prefix, query) = split_prefix_query(input)?;
    let aliases = normalized_aliases(prefixes, &["@tr", "@trko", "@tren"]);
    if !aliases.iter().any(|p| p == &prefix) {
        return None;
    }
    let direction = if prefix.ends_with("ko") {
        TranslateDirection::EnToKo
    } else if prefix.ends_with("en") {
        TranslateDirection::KoToEn
    } else {
        TranslateDirection::Auto
    };
    Some((direction, query))
}

// ── 공통 유틸 ─────────────────────────────────────────────────────────────────

/// `@prefix query` 형태의 입력에서 `(소문자 prefix, 쿼리 텍스트)` 쌍을 추출.
///
/// - `@` 로 시작하지 않으면 `None` 반환.
/// - 공백 이전까지를 prefix, 이후를 query로 분리.
/// - prefix는 소문자로 정규화, query는 양쪽 공백 제거.
///
/// 예: `"@Tr hello"` → `Some(("@tr", "hello"))`
fn split_prefix_query(input: &str) -> Option<(String, String)> {
    if !input.starts_with('@') {
        return None;
    }
    let first_space = input.find(' ');
    let prefix = input[..first_space.unwrap_or(input.len())].to_lowercase();
    let query = first_space
        .map(|i| input[i + 1..].trim())
        .unwrap_or("")
        .to_string();
    Some((prefix, query))
}

/// 사용자 별칭과 기본 별칭을 병합하여 정규화된 접두사 목록을 반환.
///
/// **동작 흐름**:
/// 1. `aliases`가 비어있으면 `defaults` 그대로 반환.
/// 2. 비어있지 않으면 사용자 별칭을 소문자로 정규화하고,
///    `@`가 없으면 자동으로 앞에 붙여줌.
/// 3. 연속 중복을 제거(`dedup`).
/// 4. 결과가 여전히 비어있으면(예: 모두 공백/빈 문자열) `defaults`로 대체.
///
/// 예: `aliases=["ll", "LLM"]` → `["@ll", "@llm"]`
/// 예: `aliases=[]` → `defaults` 그대로
pub(crate) fn normalized_aliases(aliases: &[String], defaults: &[&str]) -> Vec<String> {
    // 사용자 별칭이 없으면 기본값 사용
    let mut out: Vec<String> = if aliases.is_empty() {
        defaults.iter().map(|s| (*s).to_string()).collect()
    } else {
        // 소문자 정규화 + 빈 문자열 제거 + @ 접두사 보장
        aliases
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with('@') {
                    s
                } else {
                    format!("@{s}")
                }
            })
            .collect()
    };
    out.dedup();
    // 모든 사용자 입력이 필터링된 경우 기본값으로 복원
    if out.is_empty() {
        defaults.iter().map(|s| (*s).to_string()).collect()
    } else {
        out
    }
}
