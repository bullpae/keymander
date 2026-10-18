//! 결과 아이템 빌더 — IndexItem 생성, URL 추출, 서비스 목록

use crate::index::{IndexItem, ItemKind, Source};

use super::services::*;

struct VirtualBrowseEntry {
    hint: &'static str,
    name: &'static str,
    path: &'static str,
    emoji_icon: &'static str,
    ascii_icon: &'static str,
    keywords: &'static str,
    match_terms: &'static [&'static str],
}

const VIRTUAL_BROWSE_ENTRIES: &[VirtualBrowseEntry] = &[
    VirtualBrowseEntry {
        hint: "kmd:web_hint:multi_llm",
        name: "@ll         Compare multiple LLMs with one prompt",
        path: "Open selected LLM providers (some may require paste/Enter)",
        emoji_icon: "\u{1F9E0}",
        ascii_icon: "Ml",
        keywords: "@ll @llm @multi @cmp multi llm compare",
        match_terms: &["@llm", "multi llm compare", "@multi", "@cmp"],
    },
    VirtualBrowseEntry {
        hint: "kmd:web_hint:multi_web",
        name: "@m          Search multiple engines at once",
        path: "Open Google/Naver/Daum in parallel tabs",
        emoji_icon: "\u{1F50E}",
        ascii_icon: "Mw",
        keywords: "@m @mw @msearch @multisearch @searchall @krsearch multi web",
        match_terms: &[
            "@msearch",
            "@multisearch",
            "@searchall",
            "@krsearch",
            "multi web search",
        ],
    },
    VirtualBrowseEntry {
        hint: "kmd:web_hint:spell",
        name: "@sp         Korean spelling check",
        path: "Run spelling check providers with one query",
        emoji_icon: "\u{270D}\u{FE0F}",
        ascii_icon: "Sp",
        keywords: "@sp @spell spelling checker",
        match_terms: &["@sp", "@spell", "spelling"],
    },
    VirtualBrowseEntry {
        hint: "kmd:web_hint:translate",
        name: "@tr         Translate (auto / ko / en)",
        path: "Open selected translate providers in parallel",
        emoji_icon: "\u{1F5E3}\u{FE0F}",
        ascii_icon: "Tr",
        keywords: "@tr @trko @tren translate",
        match_terms: &["@tr", "@trko", "@tren", "translate"],
    },
];

// ── URL 빌드 ──────────────────────────────────────────────────────────────────

/// `@` 브라우징용 가상 항목 — `keywords` 첫 줄이 `kmd:web_hint:*` 이면 입력창에 넣을 프리픽스
pub fn web_browse_hint_seed(keywords: &str) -> Option<&'static str> {
    let first = keywords.lines().next()?.trim();
    match first {
        "kmd:web_hint:multi_llm" => Some("@llm "),
        "kmd:web_hint:multi_web" => Some("@msearch "),
        "kmd:web_hint:spell" => Some("@sp "),
        "kmd:web_hint:translate" => Some("@tr "),
        _ => None,
    }
}

/// 쿼리를 인코딩해 검색 URL 생성
pub fn build_search_url(service: &WebService, query: &str) -> String {
    let encoded = url_encode(query);
    service.url_template.replace("{query}", &encoded)
}

/// LLM 실행 계획 — 데몬 오토파일럿에 보낼 잡과, 그냥 URL로 열 서비스 분리.
pub struct LlmLaunchPlan {
    /// 자동화 대상(chatgpt/claude/gemini) — 데몬 키 주입으로 자동 제출
    pub jobs: Vec<crate::ipc::LlmJob>,
    /// 자동화 불필요(perplexity/grok) — URL만으로 실행되므로 바로 연다
    pub plain_urls: Vec<String>,
}

/// 선택된 LLM 서비스 + 프롬프트로 실행 계획을 만든다.
///
/// `automation` 메타가 있는 서비스는 [`crate::ipc::LlmJob`]으로, 없는 서비스는
/// 프리필 URL로 분류한다. 프런트엔드는 데몬 오토파일럿이 가능하면 jobs를 IPC로
/// 보내고, 불가하면 jobs의 URL도 plain처럼 열어 폴백한다 (docs/09).
pub fn build_llm_launch_plan(services: &[&WebService], query: &str) -> LlmLaunchPlan {
    let mut jobs = Vec::new();
    let mut plain_urls = Vec::new();
    for svc in services {
        let url = build_search_url(svc, query);
        match &svc.automation {
            Some(auto) => jobs.push(crate::ipc::LlmJob {
                service_id: svc.id.to_string(),
                url,
                prompt: query.to_string(),
                method: auto.method,
                title_markers: auto.title_markers.iter().map(|s| s.to_string()).collect(),
            }),
            None => plain_urls.push(url),
        }
    }
    LlmLaunchPlan { jobs, plain_urls }
}

/// 오토파일럿이 불가능할 때(데몬 미실행/기능 off) jobs를 URL 리스트로 환원.
/// 프리필 URL을 그냥 여는 현행 폴백 동작.
pub fn llm_plan_all_urls(plan: &LlmLaunchPlan) -> Vec<String> {
    plan.jobs
        .iter()
        .map(|j| j.url.clone())
        .chain(plan.plain_urls.iter().cloned())
        .collect()
}

/// 번역 서비스 URL 생성
pub fn build_translate_url(
    service: &TranslateService,
    query: &str,
    direction: TranslateDirection,
) -> String {
    let encoded = url_encode(query);
    let (sl, tl) = match direction {
        TranslateDirection::Auto => ("auto", "ko"),
        TranslateDirection::EnToKo => ("en", "ko"),
        TranslateDirection::KoToEn => ("ko", "en"),
    };
    match service.id {
        "google_translate" => {
            format!("https://translate.google.com/?sl={sl}&tl={tl}&text={encoded}&op=translate")
        }
        "papago" => format!("https://papago.naver.com/?sk={sl}&tk={tl}&st={encoded}"),
        "deepl" => format!("https://www.deepl.com/translator#{sl}/{tl}/{encoded}"),
        _ => format!("https://translate.google.com/?sl={sl}&tl={tl}&text={encoded}&op=translate"),
    }
}

// ── @ 목록 / 검색 결과 아이템 ────────────────────────────────────────────────

/// @ 프리픽스 브라우징용 서비스 목록
pub fn list_services_as_items(filter: &str, use_emoji: bool) -> Vec<IndexItem> {
    let filter_lower = filter.to_lowercase();

    let mut items: Vec<IndexItem> = WEB_SERVICES
        .iter()
        .filter(|s| {
            filter_lower.is_empty()
                || s.name.to_lowercase().contains(&filter_lower)
                || s.prefixes.iter().any(|p| p.contains(&filter_lower))
                || s.description.to_lowercase().contains(&filter_lower)
        })
        .map(|s| {
            let prefix = s.prefixes.first().unwrap_or(&"@");
            IndexItem {
                name: format!("{:<12} {}", prefix, s.description),
                path: s.url_template.to_string(),
                kind: ItemKind::WebSearch,
                source: Source::Plugin,
                icon: s.pick_icon(use_emoji).to_string(),
                keywords: format!("{} {}", s.prefixes.join(" "), s.description),
                icon_path: None,
            }
        })
        .collect();

    // 가상 항목 — @ll, @m, @sp, @tr 발견용 (path는 안내 문구이므로 URL로 열리면 안 됨)
    for entry in VIRTUAL_BROWSE_ENTRIES {
        let matched =
            filter_lower.is_empty() || entry.match_terms.iter().any(|t| t.contains(&filter_lower));
        if matched {
            items.push(virtual_entry_item(entry, use_emoji));
        }
    }

    items
}

fn virtual_entry_item(entry: &VirtualBrowseEntry, use_emoji: bool) -> IndexItem {
    IndexItem {
        name: entry.name.to_string(),
        path: entry.path.to_string(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: if use_emoji {
            entry.emoji_icon
        } else {
            entry.ascii_icon
        }
        .to_string(),
        keywords: format!("{}\n{}", entry.hint, entry.keywords),
        icon_path: None,
    }
}

/// `@` 브라우징 목록 — 필터와 무관하게 멀티 LLM·멀티 웹검색 진입점은 항상 보인다.
///
/// 두 기능은 프리픽스를 모르면 발견할 길이 없어, 필터에 걸리지 않아도 목록 끝에 붙인다.
/// (예전엔 데스크톱이 같은 문구를 손으로 복제해 붙였고 문구가 이미 어긋나 있었다.)
pub fn browse_items(filter: &str, use_emoji: bool) -> Vec<IndexItem> {
    let mut items = list_services_as_items(filter, use_emoji);
    for hint in ["kmd:web_hint:multi_llm", "kmd:web_hint:multi_web"] {
        let present = items
            .iter()
            .any(|it| it.keywords.lines().next() == Some(hint));
        if !present {
            if let Some(entry) = VIRTUAL_BROWSE_ENTRIES.iter().find(|e| e.hint == hint) {
                items.push(virtual_entry_item(entry, use_emoji));
            }
        }
    }
    items
}

/// `@...` 질의의 결과 항목 — 두 UI가 각자 `classify_web_query`를 match하던 것을 모았다.
/// 두 번째 값은 서비스 목록(브라우징)인지 여부 — UI가 점수 정책을 달리 줄 때 쓴다.
pub fn web_query_items(
    query: &str,
    launcher: &crate::config::LauncherConfig,
    use_emoji: bool,
) -> (Vec<IndexItem>, bool) {
    use super::parsers::{classify_web_query, WebQueryConfig, WebQueryResult};
    let cfg = WebQueryConfig::from_launcher(launcher);
    match classify_web_query(query, &cfg) {
        WebQueryResult::Spell(q) => (
            spell_result_items(&q, &launcher.spell_providers, use_emoji),
            false,
        ),
        WebQueryResult::Translate(dir, q) => (
            translate_result_items(&q, dir, &launcher.translate_providers, use_emoji),
            false,
        ),
        WebQueryResult::MultiLlm(_, q) => (
            multi_llm_result_items(&q, &launcher.multi_llm_providers, use_emoji),
            false,
        ),
        WebQueryResult::MultiWeb(_, q) => (
            multi_web_result_items(&q, &launcher.multi_web_providers, use_emoji),
            false,
        ),
        WebQueryResult::Single(_, q) if q.is_empty() => (browse_items("", use_emoji), true),
        WebQueryResult::Single(service, q) => {
            (vec![search_result_item(service, &q, use_emoji)], false)
        }
        WebQueryResult::Browse(filter) => (browse_items(&filter, use_emoji), true),
    }
}

/// 단일 서비스 검색 결과 아이템
pub fn search_result_item(service: &WebService, query: &str, use_emoji: bool) -> IndexItem {
    let url = build_search_url(service, query);
    let name = format!("{}: \"{}\"", service.name, query);
    IndexItem {
        name,
        path: url.clone(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: service.pick_icon(use_emoji).to_string(),
        keywords: url,
        icon_path: None,
    }
}

// ── 멀티 결과 아이템 빌더 ────────────────────────────────────────────────────

pub fn multi_llm_result_items(
    query: &str,
    selected_ids: &[String],
    use_emoji: bool,
) -> Vec<IndexItem> {
    let services = selected_llm_services(selected_ids);
    if query.is_empty() {
        return services_to_browse_items(
            &services,
            "@llm",
            9,
            use_emoji,
            |s| s.url_template.to_string(),
            |s| format!("{} {}", s.id, s.prefixes.join(" ")),
        );
    }
    build_batch_items(
        services
            .iter()
            .map(|svc| build_search_url(svc, query))
            .collect(),
        &services.iter().map(|s| s.name).collect::<Vec<_>>(),
        "kmd:multi_llm_urls",
        &format!("Multi LLM compare ({})", services.len()),
        &format!(
            "Open all selected LLMs for: \"{}\" (some providers need paste/Enter)",
            query
        ),
        if use_emoji { "\u{1F9E0}" } else { "Ml" },
        services
            .iter()
            .map(|s| search_result_item(s, query, use_emoji))
            .collect(),
    )
}

pub fn multi_web_result_items(
    query: &str,
    selected_ids: &[String],
    use_emoji: bool,
) -> Vec<IndexItem> {
    let services = selected_multi_web_services(selected_ids);
    if query.is_empty() {
        return services_to_browse_items(
            &services,
            "@msearch",
            12,
            use_emoji,
            |s| s.url_template.to_string(),
            |s| format!("{} {}", s.id, s.prefixes.join(" ")),
        );
    }
    build_batch_items(
        services
            .iter()
            .map(|svc| build_search_url(svc, query))
            .collect(),
        &services.iter().map(|s| s.name).collect::<Vec<_>>(),
        "kmd:multi_web_urls",
        &format!("Multi Web search ({})", services.len()),
        &format!("Open selected search engines for: \"{}\"", query),
        if use_emoji { "\u{1F50E}" } else { "Mw" },
        services
            .iter()
            .map(|s| search_result_item(s, query, use_emoji))
            .collect(),
    )
}

pub fn spell_result_items(query: &str, selected_ids: &[String], use_emoji: bool) -> Vec<IndexItem> {
    let services = selected_spell_services(selected_ids);
    if query.is_empty() {
        return services_to_browse_items(
            &services,
            "@sp",
            14,
            use_emoji,
            |s| s.url_template.to_string(),
            |s| format!("{} spell", s.id),
        );
    }
    let urls: Vec<String> = services
        .iter()
        .map(|svc| svc.url_template.replace("{query}", &url_encode(query)))
        .collect();
    let individual: Vec<IndexItem> = services
        .iter()
        .zip(urls.iter())
        .map(|(svc, url)| IndexItem {
            name: format!("{}: \"{}\"", svc.name, query),
            path: url.clone(),
            kind: ItemKind::WebSearch,
            source: Source::Plugin,
            icon: svc.pick_icon(use_emoji).to_string(),
            keywords: url.clone(),
            icon_path: None,
        })
        .collect();
    build_batch_items(
        urls,
        &services.iter().map(|s| s.name).collect::<Vec<_>>(),
        "kmd:spell_urls",
        &format!("Spell check ({})", services.len()),
        &format!("Run spelling check for: \"{}\"", query),
        if use_emoji { "\u{270D}\u{FE0F}" } else { "Sp" },
        individual,
    )
}

pub fn translate_result_items(
    query: &str,
    direction: TranslateDirection,
    selected_ids: &[String],
    use_emoji: bool,
) -> Vec<IndexItem> {
    let services = selected_translate_services(selected_ids);
    if query.is_empty() {
        return services_to_browse_items(
            &services,
            "@tr",
            16,
            use_emoji,
            |s| s.description.to_string(),
            |s| format!("{} translate", s.id),
        );
    }
    let urls: Vec<String> = services
        .iter()
        .map(|svc| build_translate_url(svc, query, direction))
        .collect();
    let individual: Vec<IndexItem> = services
        .iter()
        .zip(urls.iter())
        .map(|(svc, url)| IndexItem {
            name: format!("{}: \"{}\"", svc.name, query),
            path: url.clone(),
            kind: ItemKind::WebSearch,
            source: Source::Plugin,
            icon: svc.pick_icon(use_emoji).to_string(),
            keywords: url.clone(),
            icon_path: None,
        })
        .collect();
    build_batch_items(
        urls,
        &services.iter().map(|s| s.name).collect::<Vec<_>>(),
        "kmd:translate_urls",
        &format!("Translate ({})", services.len()),
        &format!("Translate: \"{}\"", query),
        if use_emoji { "\u{1F5E3}\u{FE0F}" } else { "Tr" },
        individual,
    )
}

// ── URL 추출 ──────────────────────────────────────────────────────────────────

/// 통합 URL 추출 — 4개 extract 함수를 한 번에 시도
pub fn extract_batch_urls(item: &IndexItem) -> Option<Vec<String>> {
    extract_urls_with_prefix(item, "kmd:translate_urls\n")
        .or_else(|| extract_urls_with_prefix(item, "kmd:spell_urls\n"))
        .or_else(|| extract_urls_with_prefix(item, "kmd:multi_web_urls\n"))
        .or_else(|| extract_urls_with_prefix(item, "kmd:multi_llm_urls\n"))
}

pub fn extract_multi_llm_urls(item: &IndexItem) -> Option<Vec<String>> {
    extract_urls_with_prefix(item, "kmd:multi_llm_urls\n")
}

pub fn extract_multi_web_urls(item: &IndexItem) -> Option<Vec<String>> {
    extract_urls_with_prefix(item, "kmd:multi_web_urls\n")
}

pub fn extract_spell_urls(item: &IndexItem) -> Option<Vec<String>> {
    extract_urls_with_prefix(item, "kmd:spell_urls\n")
}

pub fn extract_translate_urls(item: &IndexItem) -> Option<Vec<String>> {
    extract_urls_with_prefix(item, "kmd:translate_urls\n")
}

/// 공통 URL 추출 로직 — 중복 제거
fn extract_urls_with_prefix(item: &IndexItem, prefix: &str) -> Option<Vec<String>> {
    if !item.keywords.starts_with(prefix) {
        return None;
    }
    let urls = item.keywords[prefix.len()..]
        .lines()
        .take_while(|line| !line.starts_with("providers:"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if urls.is_empty() {
        None
    } else {
        Some(urls)
    }
}

// ── 내부 헬퍼 ─────────────────────────────────────────────────────────────────

/// empty-query 시 서비스 목록을 IndexItem으로 변환하는 공용 헬퍼
fn services_to_browse_items<T: HasId>(
    services: &[&'static T],
    prefix_label: &str,
    id_width: usize,
    use_emoji: bool,
    path_fn: impl Fn(&T) -> String,
    keywords_fn: impl Fn(&T) -> String,
) -> Vec<IndexItem> {
    services
        .iter()
        .map(|s| IndexItem {
            name: format!(
                "{} {:width$} {}",
                prefix_label,
                s.service_id(),
                s.service_description(),
                width = id_width
            ),
            path: path_fn(s),
            kind: ItemKind::WebSearch,
            source: Source::Plugin,
            icon: s.pick_icon(use_emoji).to_string(),
            keywords: keywords_fn(s),
            icon_path: None,
        })
        .collect()
}

/// 배치 결과 아이템 (첫 번째 = 전체 실행, 이후 = 개별 서비스)
fn build_batch_items(
    urls: Vec<String>,
    provider_names: &[&str],
    marker_prefix: &str,
    name: &str,
    path: &str,
    icon: &str,
    mut individual: Vec<IndexItem>,
) -> Vec<IndexItem> {
    let providers_str = provider_names.join(", ");
    let marker = format!("{}\n{}", marker_prefix, urls.join("\n"));
    let mut items = vec![IndexItem {
        name: name.to_string(),
        path: path.to_string(),
        kind: ItemKind::WebSearch,
        source: Source::Plugin,
        icon: icon.to_string(),
        keywords: format!("{}\nproviders: {}", marker, providers_str),
        icon_path: None,
    }];
    items.append(&mut individual);
    items
}

/// 간단한 URL 퍼센트 인코딩
pub fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push_str(&format!("{:02X}", byte));
            }
        }
    }
    result
}
