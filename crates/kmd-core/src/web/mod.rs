//! Web service integration — @prefix 기반 웹/LLM/맞춤법/번역 쿼리 처리
//!
//! 서브모듈 구조:
//! - `services`: 서비스 정의, 상수, provider 선택
//! - `parsers`: @ 쿼리 파싱, 통합 분류기 (`classify_web_query`)
//! - `items`: IndexItem 생성, URL 추출, 목록

pub mod items;
pub mod parsers;
pub mod services;

// ── re-export — 기존 `web::*` 임포트를 깨뜨리지 않도록 ──────────────────────

pub use items::{
    browse_items, build_llm_launch_plan, build_search_url, build_translate_url, extract_batch_urls,
    extract_multi_llm_urls, extract_multi_web_urls, extract_spell_urls, extract_translate_urls,
    list_services_as_items, llm_plan_all_urls, multi_llm_result_items, multi_web_result_items,
    search_result_item, spell_result_items, translate_result_items, url_encode,
    web_browse_hint_seed, web_query_items, LlmLaunchPlan,
};

pub use parsers::{
    classify_web_query, parse_any_llm_query, parse_llm_followup, parse_multi_llm_query,
    parse_multi_llm_query_with_prefixes, parse_multi_web_query,
    parse_multi_web_query_with_prefixes, parse_spell_query_with_prefixes,
    parse_translate_query_with_prefixes, parse_web_query, WebQueryConfig, WebQueryResult,
};

pub use services::{
    is_llm_id, is_multi_web_id, is_spell_id, is_translate_id, selected_llm_services,
    selected_multi_web_services, selected_spell_services, selected_translate_services, HasId,
    LlmAutomation, SpellService, TranslateDirection, TranslateService, WebService, SPELL_SERVICES,
    TRANSLATE_SERVICES, WEB_SERVICES,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 브라우징_목록은_멀티_진입점을_필터와_무관하게_보인다() {
        let items = browse_items("google", false);
        for hint in ["kmd:web_hint:multi_llm", "kmd:web_hint:multi_web"] {
            let n = items
                .iter()
                .filter(|i| i.keywords.lines().next() == Some(hint))
                .count();
            assert_eq!(n, 1, "{hint}는 정확히 한 번: {items:?}");
        }
        // 빈 필터(모든 가상 항목이 이미 포함)에서도 중복되지 않는다
        let all = browse_items("", false);
        let n = all
            .iter()
            .filter(|i| i.keywords.starts_with("kmd:web_hint:multi_llm"))
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn 웹_질의_항목은_목록_여부를_알려준다() {
        let launcher = crate::config::LauncherConfig::default();
        let (_, listing) = web_query_items("@", &launcher, false);
        assert!(listing);
        let (items, listing) = web_query_items("@g rust", &launcher, false);
        assert!(!listing);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_parse_google() {
        let result = parse_web_query("@g rust tutorial");
        assert!(result.is_some());
        let (service, query) = result.unwrap();
        assert_eq!(service.name, "Google");
        assert_eq!(query, "rust tutorial");
    }

    #[test]
    fn test_parse_web_query_prefix_case_insensitive() {
        let result = parse_web_query("@AI why rust");
        assert!(result.is_some());
        let (service, query) = result.unwrap();
        assert_eq!(service.id, "perplexity");
        assert_eq!(query, "why rust");
    }

    #[test]
    fn test_web_browse_hint_seed() {
        let items = list_services_as_items("", false);
        let hint_item = items
            .iter()
            .find(|i| i.name.starts_with("@ll "))
            .expect("ll hint row");
        assert_eq!(web_browse_hint_seed(&hint_item.keywords), Some("@llm "));
    }

    #[test]
    fn test_parse_no_query() {
        let result = parse_web_query("@yt");
        assert!(result.is_some());
        let (_, query) = result.unwrap();
        assert_eq!(query, "");
    }

    #[test]
    fn test_unknown_prefix() {
        assert!(parse_web_query("@unknown test").is_none());
    }

    #[test]
    fn test_build_url() {
        let service = &WEB_SERVICES[0]; // Google
        let url = build_search_url(service, "rust tutorial");
        assert_eq!(url, "https://google.com/search?q=rust+tutorial");
    }

    #[test]
    fn test_parse_multi_llm_query() {
        let selected = vec!["chatgpt".to_string(), "claude".to_string()];
        let parsed = parse_multi_llm_query("@llm compare rust ownership", &selected);
        assert!(parsed.is_some());
        let (services, q) = parsed.unwrap();
        assert_eq!(q, "compare rust ownership");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].id, "chatgpt");
        assert_eq!(services[1].id, "claude");
    }

    #[test]
    fn test_parse_multi_llm_custom_prefix() {
        let selected = vec!["chatgpt".to_string()];
        let prefixes = vec!["@askllm".to_string()];
        let parsed =
            parse_multi_llm_query_with_prefixes("@askllm rust async", &selected, &prefixes);
        assert!(parsed.is_some());
        let (_, q) = parsed.unwrap();
        assert_eq!(q, "rust async");
    }

    #[test]
    fn test_parse_multi_web_query() {
        let selected = vec!["google".to_string(), "daum".to_string()];
        let parsed = parse_multi_web_query("@msearch rust ownership", &selected);
        assert!(parsed.is_some());
        let (services, q) = parsed.unwrap();
        assert_eq!(q, "rust ownership");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].id, "google");
        assert_eq!(services[1].id, "daum");
    }

    #[test]
    fn test_parse_multi_web_custom_prefix() {
        let selected = vec!["google".to_string()];
        let prefixes = vec!["@searchx".to_string()];
        let parsed = parse_multi_web_query_with_prefixes("@searchx rust", &selected, &prefixes);
        assert!(parsed.is_some());
        let (_, q) = parsed.unwrap();
        assert_eq!(q, "rust");
    }

    #[test]
    fn test_parse_spell_query() {
        let prefixes = vec!["@sp".to_string(), "@spell".to_string()];
        let parsed = parse_spell_query_with_prefixes("@sp 안녕 하세요", &prefixes);
        assert_eq!(parsed, Some("안녕 하세요".to_string()));
    }

    #[test]
    fn test_parse_translate_query() {
        let prefixes = vec!["@tr".to_string(), "@trko".to_string(), "@tren".to_string()];
        let parsed = parse_translate_query_with_prefixes("@trko hello world", &prefixes).unwrap();
        assert_eq!(parsed.0, TranslateDirection::EnToKo);
        assert_eq!(parsed.1, "hello world");
    }

    #[test]
    fn test_multi_llm_item_urls_roundtrip() {
        // chatgpt(프리필 URL) + grok(프리필 URL) — 둘 다 쿼리를 URL에 담는다.
        // (gemini는 URL 파라미터를 무시하므로 프리필 없이 앱만 여는 것으로 바뀜)
        let selected = vec!["chatgpt".to_string(), "grok".to_string()];
        let items = multi_llm_result_items("rust lifetimes", &selected, false);
        assert!(!items.is_empty());
        let urls = extract_multi_llm_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("rust+lifetimes"));
        assert!(urls[1].contains("rust+lifetimes"));
    }

    #[test]
    fn test_gemini_multi_url_has_no_query() {
        // gemini는 URL 파라미터 미지원 → 프리필 없이 앱만 (프롬프트는 클립보드/주입)
        let selected = vec!["gemini".to_string()];
        let items = multi_llm_result_items("rust lifetimes", &selected, false);
        let urls = extract_multi_llm_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 1);
        assert!(!urls[0].contains("rust"), "gemini URL에 쿼리가 없어야 함");
    }

    #[test]
    fn test_multi_web_item_urls_roundtrip() {
        let selected = vec!["google".to_string(), "naver_search".to_string()];
        let items = multi_web_result_items("러스트 소유권", &selected, false);
        assert!(!items.is_empty());
        let urls = extract_multi_web_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("%EB%9F%AC%EC%8A%A4%ED%8A%B8"));
    }

    #[test]
    fn test_spell_item_urls_roundtrip() {
        let selected = vec!["naver_spell".to_string(), "pusan_spell".to_string()];
        let items = spell_result_items("문장 검사", &selected, false);
        let urls = extract_spell_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_translate_item_urls_roundtrip() {
        let selected = vec!["google_translate".to_string(), "papago".to_string()];
        let items = translate_result_items("hello", TranslateDirection::EnToKo, &selected, false);
        let urls = extract_translate_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("sl=en"));
    }

    #[test]
    fn test_list_services_contains_llm_hint() {
        let items = list_services_as_items("", false);
        assert!(items
            .iter()
            .any(|item| item.name.starts_with("@ll") || item.name.starts_with("@llm")));
    }

    #[test]
    fn test_extract_batch_urls_works() {
        let selected = vec!["chatgpt".to_string(), "gemini".to_string()];
        let items = multi_llm_result_items("test", &selected, false);
        let urls = extract_batch_urls(&items[0]).unwrap();
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_parse_llm_followup() {
        assert_eq!(
            parse_llm_followup("@@ 그럼 요약해줘"),
            Some("그럼 요약해줘".into())
        );
        assert_eq!(parse_llm_followup("@@   trimmed  "), Some("trimmed".into()));
        assert!(parse_llm_followup("@@").is_none(), "빈 후속은 None");
        assert!(
            parse_llm_followup("@gpt hi").is_none(),
            "다른 프리픽스 무시"
        );
        assert!(parse_llm_followup("no prefix").is_none());
    }

    #[test]
    fn test_build_llm_launch_plan_splits_automation() {
        // chatgpt(자동화), gemini(자동화), perplexity(plain)
        let services = selected_llm_services(&[
            "chatgpt".to_string(),
            "gemini".to_string(),
            "perplexity".to_string(),
        ]);
        let plan = build_llm_launch_plan(&services, "why rust");

        // 자동화 잡: chatgpt(EnterOnly), gemini(PasteEnter)
        assert_eq!(plan.jobs.len(), 2);
        let gpt = plan
            .jobs
            .iter()
            .find(|j| j.service_id == "chatgpt")
            .unwrap();
        assert_eq!(gpt.method, crate::ipc::LlmInject::EnterOnly);
        assert!(gpt.url.contains("why+rust"), "gpt는 프리필 URL");
        assert_eq!(gpt.title_markers, vec!["ChatGPT".to_string()]);

        let gem = plan.jobs.iter().find(|j| j.service_id == "gemini").unwrap();
        assert_eq!(gem.method, crate::ipc::LlmInject::PasteEnter);
        assert!(!gem.url.contains("why"), "gemini는 프리필 없이 앱만 연다");
        assert_eq!(gem.prompt, "why rust", "붙여넣기용 원문 보존");

        // plain: perplexity
        assert_eq!(plan.plain_urls.len(), 1);
        assert!(plan.plain_urls[0].contains("perplexity"));

        // 폴백: 전부 URL로 환원
        assert_eq!(llm_plan_all_urls(&plan).len(), 3);
    }

    #[test]
    fn test_classify_web_query() {
        let cfg = WebQueryConfig {
            spell_prefixes: &[],
            translate_prefixes: &[],
            multi_llm_prefixes: &[],
            multi_llm_ids: &["chatgpt".to_string()],
            multi_web_prefixes: &[],
            multi_web_ids: &["google".to_string()],
        };

        match classify_web_query("@sp 테스트", &cfg) {
            WebQueryResult::Spell(q) => assert_eq!(q, "테스트"),
            _ => panic!("expected Spell"),
        }

        match classify_web_query("@g hello", &cfg) {
            WebQueryResult::Single(svc, q) => {
                assert_eq!(svc.id, "google");
                assert_eq!(q, "hello");
            }
            _ => panic!("expected Single"),
        }

        match classify_web_query("@llm test prompt", &cfg) {
            WebQueryResult::MultiLlm(svcs, q) => {
                assert!(!svcs.is_empty());
                assert_eq!(q, "test prompt");
            }
            _ => panic!("expected MultiLlm"),
        }
    }
}
