//! 런처 공용 판단 — 일반 검색 순위 파이프라인과 Enter 실행 계획.
//!
//! 데스크톱·TUI·CLI 가 같은 파이프라인을 각자 들고 있다가 조금씩 달라졌다
//! (계산기 위치, 한영 오타 판정, 웹 제안 유무 …). 여기 한 벌만 두고 UI 는 결과를 그리기만 한다.

use crate::config::LauncherConfig;
use crate::filesearch::{Boost, FileIndex};
use crate::history::FrecencyMap;
use crate::index::{IndexItem, ItemKind, Source};
use crate::plugin::builtin_calc;
use crate::search::{SearchEngine, SearchMode, SearchResult};

/// 엔진 밖에서 끼운 행의 점수 — `rank::merge` 가 다시 매겨도 맨 위에 남는다 (Plugin 출처는 건너뜀)
pub const SCORE_PINNED: u32 = u32::MAX;
/// 이름 매칭이 약할 때 대체 제안 위에 남겨 둘 퍼지 결과 수.
pub const WEAK_MATCH_KEEP_TOP: usize = 5;
/// 이름으로 친 명령을 끼울 자리 — 앱·파일 상위 결과는 그대로 둔다
pub const NAMED_COMMAND_KEEP_TOP: usize = 3;

/// 일반 검색에 필요한 것 — 모두 호출부가 이미 들고 있는 것을 빌린다 (키 입력마다 DB·디스크 접근 없음).
pub struct RankCtx<'a> {
    pub launcher: &'a LauncherConfig,
    /// 새 파일 검색 엔진(docs/22). 없으면 옛 엔진 결과에 실행 이력만 더한다
    pub file_index: Option<&'a FileIndex>,
    pub file_boost: &'a Boost,
    pub frecency: &'a FrecencyMap,
    pub use_emoji: bool,
    pub limit: usize,
    /// 런처 화면용 행(웹 대체 제안·범위 추가·명령 자동완성·명령 안내). CLI 는 끈다
    pub suggestions: bool,
}

/// 일반 검색 결과 — UI 는 그대로 그린다.
#[derive(Debug)]
pub struct RankOutcome {
    pub results: Vec<SearchResult>,
    pub mode: SearchMode,
    /// 짧게 띄울 알림 (`한글로 바꿔 찾음: …`)
    pub notice: Option<String>,
    /// 퍼지 검색이 시간 안에 안 끝났다 — 잠시 뒤 같은 질의로 다시 묻는다
    pub pending: bool,
}

fn pinned(items: impl IntoIterator<Item = IndexItem>) -> impl Iterator<Item = SearchResult> {
    items.into_iter().map(|item| SearchResult {
        item,
        score: SCORE_PINNED,
    })
}

/// 엔진 검색 + URL 열기 행 + (새 엔진이면) 파일 결과 합치기·순위, 아니면 실행 이력 부스트.
/// 한영 오타 변환이 바꾼 질의로 한 번 더 부르므로 따로 둔다.
fn ranked(
    engine: &mut SearchEngine,
    query: &str,
    ctx: &RankCtx<'_>,
) -> (SearchMode, Vec<SearchResult>) {
    let (mode, mut results) = engine.search(query, ctx.limit);

    // URL로 판정된 입력: 파일 검색은 그대로 두고 "열기" 항목을 맨 위에 세운다.
    // 이게 없으면 example.com 을 쳐도 열 방법이 없다.
    if mode == SearchMode::Url {
        let (_, normalized_url) = SearchMode::detect(query);
        results = engine.search_with_mode(SearchMode::Contains, query.trim(), ctx.limit);
        results.insert(
            0,
            SearchResult {
                item: crate::query_prefix::url_open_item(&normalized_url, ctx.use_emoji),
                score: SCORE_PINNED,
            },
        );
    }

    // 할당량에 잘린 옛 파일 결과를 전체 색인 결과로 바꾸고, 앱·실행 파일까지 한 척도로 다시 매긴다
    if let Some(index) = ctx.file_index {
        let weights = &ctx.launcher.kind_weights;
        let files = index.search(query, ctx.limit, ctx.file_boost, weights, ctx.use_emoji);
        results = crate::rank::merge(results, files, query, weights, ctx.frecency);
        results.truncate(ctx.limit);
    } else {
        crate::history::boost_results_with_map(&mut results, ctx.frecency);
    }
    (mode, results)
}

/// 일반(프리픽스 없는) 질의의 결과 전체 — 데스크톱·TUI·CLI 공용.
pub fn rank_general(engine: &mut SearchEngine, query: &str, ctx: &RankCtx<'_>) -> RankOutcome {
    let (mut mode, mut results) = ranked(engine, query, ctx);

    // 계산기는 순위 매긴 뒤 맨 위에 — merge 전에 넣으면 자를 때 자리를 차지하고,
    // 같은 최댓값인 URL 행과 이름 길이로 순서가 흔들린다
    let math = builtin_calc::looks_like_math(query);
    if math {
        let calc = builtin_calc::CalcExtension.search_with_emoji(query, ctx.use_emoji);
        results.splice(0..0, pinned(calc));
    }

    // 미완성이면 빈 결과가 진짜가 아니다 — 완화 검색·웹 제안을 띄우지 않고 재검색을 기다린다
    let pending = engine.is_pending();

    // 한영 오타: 영문 자판으로 친 한글(`qhrhtj` → 보고서). 원래 질의가 그대로 든 결과가 없고,
    // 바꾼 질의는 그대로 든 결과가 있을 때만 위에 올린다 — 영어 질의를 가리지 않게.
    let mut notice = None;
    if !pending && !has_literal_match(&results, query) {
        if let Some(ko) = crate::hangul::qwerty_to_hangul(query) {
            let (ko_mode, ko_results) = ranked(engine, &ko, ctx);
            if has_literal_match(&ko_results, &ko) {
                let mut seen: std::collections::HashSet<String> =
                    ko_results.iter().map(|r| r.item.path.clone()).collect();
                results.retain(|r| seen.insert(r.item.path.clone()));
                results.splice(0..0, ko_results);
                results.truncate(ctx.limit);
                mode = ko_mode;
                notice = Some(format!("한글로 바꿔 찾음: {ko}"));
            }
        }
    }

    // IME 조합 중 끝 자모(`하ㄴ`)로 아무것도 안 걸리면 자모를 뗀 질의로 한 번 더
    if results.is_empty() && !query.is_empty() && !pending {
        if let Some(relaxed) = relaxed_hangul_query(query) {
            let mut relaxed_results =
                engine.search_with_mode(SearchMode::Contains, &relaxed, ctx.limit);
            if !relaxed_results.is_empty() {
                crate::history::boost_results_with_map(&mut relaxed_results, ctx.frecency);
                results = relaxed_results;
                mode = SearchMode::Contains;
            }
        }
    }

    if ctx.suggestions {
        if pending {
            // 계산기·경로 추가 행처럼 엔진 밖 결과만 그대로 둔다
        } else if results.is_empty() && !query.is_empty() {
            results = fallback_suggestions(query, ctx.launcher, ctx.use_emoji);
            mode = SearchMode::Contains;
        } else if mode == SearchMode::Fuzzy && !math && !has_literal_match(&results, query) {
            // 퍼지 결과만 있고 질의가 그대로 든 항목이 없으면 (`gmail` → 글자만 흩어져 걸린 앱들)
            // 결과가 비지 않아 웹 제안이 영영 안 나온다. 약어 매칭(`vsc` → Visual Studio Code)일
            // 수도 있으니 상위 몇 개는 그대로 두고 그 아래에 대체 제안을 끼운다.
            let at = results.len().min(WEAK_MATCH_KEEP_TOP);
            let fallback = fallback_suggestions(query, ctx.launcher, ctx.use_emoji);
            results.splice(at..at, fallback);
        }

        // 범위 밖 폴더 경로를 그대로 쳤으면 맨 위에 "검색 범위에 추가" (docs/18 E1)
        if let Some(item) = crate::scope::path_add_suggestion(ctx.launcher, query, ctx.use_emoji) {
            results.splice(0..0, pinned([item]));
        }

        // `:` · `:ti` — 치는 중인 명령의 자동완성. 일반 검색 결과는 뜻이 없으므로 갈아 끼운다
        let completions = crate::query_prefix::command_completions(query, ctx.use_emoji);
        if !completions.is_empty() {
            results = pinned(completions).collect();
            mode = SearchMode::Contains;
        } else {
            // 이름으로 친 명령(`timer`, `타이머`) — 앱·파일 상위 결과 아래에 끼운다
            let named = crate::query_prefix::command_name_matches(query, ctx.use_emoji);
            let at = results.len().min(NAMED_COMMAND_KEEP_TOP);
            results.splice(at..at, pinned(named));
        }

        // 오타/미지원 : 명령 안내를 최상단에 표시 (검색 폴스루는 유지)
        if let Some(hint) = crate::query_prefix::unknown_command_hint(query, ctx.use_emoji) {
            results.splice(0..0, pinned([hint]));
        }
    }

    RankOutcome {
        results,
        mode,
        notice,
        pending,
    }
}

/// 결과가 비거나 약할 때의 대체 제안 — 범위 추가 → 웹 검색 → 폴더 지정 검색 순.
pub fn fallback_suggestions(
    query: &str,
    launcher: &LauncherConfig,
    use_emoji: bool,
) -> Vec<SearchResult> {
    use crate::web::{self, WEB_SERVICES};

    const FALLBACK_IDS: &[&str] = &[
        "google",
        "perplexity",
        "chatgpt",
        "claude",
        "gemini",
        "naver_search",
        "youtube",
        "github",
        "stackoverflow",
        "wikipedia",
    ];

    // 검색 결과가 비는 가장 흔한 원인은 "그 폴더가 검색 범위 밖"이다.
    // 웹 검색을 권하기 전에 범위 추가 제안을 먼저 보여준다.
    let mut items = crate::folder_suggest::suggestion_results(launcher, use_emoji, 2);
    // 제안에 원하는 폴더가 없을 때 직접 넣는 길 (docs/18 E4)
    items.push(SearchResult {
        item: crate::scope::empty_result_hint(use_emoji),
        score: 0,
    });

    items.extend(
        FALLBACK_IDS
            .iter()
            .filter_map(|id| WEB_SERVICES.iter().find(|s| s.id == *id))
            .map(|service| SearchResult {
                item: web::search_result_item(service, query, use_emoji),
                score: 0,
            }),
    );

    if let Some(multi_item) =
        web::multi_llm_result_items(query, &launcher.multi_llm_providers, use_emoji)
            .into_iter()
            .next()
    {
        let at = items.len().min(5);
        items.insert(
            at,
            SearchResult {
                item: multi_item,
                score: 0,
            },
        );
    }

    // 폴더 지정 검색 힌트 — 인덱스에 없는 파일을 찾을 때 유용함을 알림
    items.push(SearchResult {
        item: IndexItem {
            name: format!(":f /폴더경로 {query}  — 폴더 직접 지정 검색"),
            path: format!(":f  {query}"),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: "F>".to_string(),
            keywords: "kmd:folder_search:suggest".to_string(),
            icon_path: None,
        },
        score: 0,
    });

    items
}

/// 결과 중 질의 토큰이 모두 (대소문자 무시) 이름이나 경로에 그대로 들어간
/// 항목이 있는가 — 글자만 흩어져 걸린 퍼지 매칭과 구분한다.
pub fn has_literal_match(results: &[SearchResult], query: &str) -> bool {
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return true;
    }
    results.iter().any(|r| {
        // macOS 파일 이름은 NFD(자모 분리)일 수 있다 — 질의(NFC)와 맞춰 비교
        let name = crate::textenc::nfc(&r.item.name).to_lowercase();
        let path = crate::textenc::nfc(&r.item.path).to_lowercase();
        tokens
            .iter()
            .all(|t| name.contains(t.as_str()) || path.contains(t.as_str()))
    })
}

/// IME 조합 중 생기는 끝 자모를 떼어 즉시 검색 반응성을 높인다.
///
/// 예: `하ㄴ` -> `하`, `한ㄱ` -> `한`. 뗄 것이 없거나 자모뿐이면 None.
pub fn relaxed_hangul_query(query: &str) -> Option<String> {
    let trimmed = query.trim_end_matches(crate::hangul::is_jamo);
    (!trimmed.is_empty() && trimmed.len() != query.len()).then(|| trimmed.to_string())
}

// ── Enter 실행 계획 ──────────────────────────────────────────────────────────

/// `@gpt …`·`@llm …` Enter 의 실행 계획 — 오토파일럿 위임 여부와 실패 시 폴백은 UI 가 정한다.
pub struct LlmLaunch {
    pub plan: crate::web::LlmLaunchPlan,
    /// 폴백(오토파일럿 꺼짐·실패) 때 열 전 서비스 URL
    pub all_urls: Vec<String>,
    /// 붙여넣기형 서비스가 있으면 클립보드에 담을 프롬프트
    pub paste: Option<String>,
}

/// 현재 질의가 LLM 실행이면 그 계획. 프롬프트 템플릿(`/요약` 등)은 여기서 적용한다.
pub fn llm_launch(query: &str, launcher: &LauncherConfig) -> Option<LlmLaunch> {
    let (services, prompt) = crate::web::parse_any_llm_query(
        query,
        &launcher.multi_llm_providers,
        &launcher.multi_llm_prefixes,
    )?;
    if services.is_empty() {
        return None;
    }
    let final_prompt = crate::prompt::apply_template(&launcher.prompt_templates, &prompt);
    let plan = crate::web::build_llm_launch_plan(&services, &final_prompt);
    let has_paste = plan
        .jobs
        .iter()
        .any(|j| matches!(j.method, crate::ipc::LlmInject::PasteEnter));
    Some(LlmLaunch {
        all_urls: crate::web::llm_plan_all_urls(&plan),
        paste: (has_paste && !final_prompt.is_empty()).then_some(final_prompt),
        plan,
    })
}

/// `:t` Enter 의 실행 계획 — 무엇을 했는지 알릴 종류와 열 URL.
#[derive(Debug, Clone)]
pub struct TransformLaunch {
    pub kind: crate::transform::TransformKind,
    pub urls: Vec<String>,
}

/// 현재 질의(`/t …` 정규화 포함)로 변환 URL 을 만든다. 텍스트가 비면 클립보드 내용을 쓴다.
pub fn transform_launch(query: &str, launcher: &LauncherConfig) -> Option<TransformLaunch> {
    let trimmed = query.trim();
    let normalized = crate::query_prefix::normalize_slash_command(trimmed);
    let dispatch = normalized.as_deref().unwrap_or(trimmed);
    let tq = crate::transform::parse_transform_query_or_clipboard(dispatch)?;
    let urls = crate::transform::build_transform_urls(
        &tq,
        &launcher.spell_providers,
        &launcher.translate_providers,
    );
    Some(TransformLaunch {
        kind: tq.kind,
        urls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, path: &str, kind: ItemKind) -> IndexItem {
        IndexItem {
            name: name.to_string(),
            path: path.to_string(),
            kind,
            source: Source::Apps,
            icon: String::new(),
            keywords: String::new(),
            icon_path: None,
        }
    }

    fn run(items: Vec<IndexItem>, query: &str) -> RankOutcome {
        run_with(items, query, &FrecencyMap::default(), true)
    }

    fn run_with(
        items: Vec<IndexItem>,
        query: &str,
        frecency: &FrecencyMap,
        suggestions: bool,
    ) -> RankOutcome {
        let launcher = LauncherConfig::default();
        let mut engine = SearchEngine::with_items(&launcher.kind_weights, items);
        engine.set_fuzzy_wait(std::time::Duration::from_secs(5));
        let boost = Boost::default();
        let ctx = RankCtx {
            launcher: &launcher,
            file_index: None,
            file_boost: &boost,
            frecency,
            use_emoji: false,
            limit: 50,
            suggestions,
        };
        rank_general(&mut engine, query, &ctx)
    }

    fn names(o: &RankOutcome) -> Vec<&str> {
        o.results.iter().map(|r| r.item.name.as_str()).collect()
    }

    #[test]
    fn url_열기_행은_자주_연_같은_이름_폴더보다_위에_있다() {
        let folder = "/Users/me/go/src/github.com";
        let mut frecency = FrecencyMap::default();
        frecency.insert(folder.into(), (50, "2026-11-28 09:00:00".into()));
        let o = run_with(
            vec![item("github.com", folder, ItemKind::Directory)],
            "github.com",
            &frecency,
            true,
        );
        assert_eq!(
            o.results[0].item.kind,
            ItemKind::WebSearch,
            "{:?}",
            names(&o)
        );
        assert!(o.results.iter().any(|r| r.item.path == folder));
    }

    #[test]
    fn 계산기는_순위를_매긴_뒤_맨_위에_선다() {
        let o = run(
            vec![item("2+3 notes", "/n/2+3 notes.txt", ItemKind::File)],
            "2+3",
        );
        assert_eq!(
            o.results[0].item.kind,
            ItemKind::Calculator,
            "{:?}",
            names(&o)
        );
        assert_eq!(o.results[0].item.path, "5");
        // 계산 질의에는 약한 매칭 웹 제안을 끼우지 않는다
        assert!(!o.results.iter().any(|r| r.item.kind == ItemKind::WebSearch));
    }

    #[test]
    fn 영문_자판으로_친_한글은_바꿔_찾고_알린다() {
        let items = vec![
            item("카카오톡", "/Applications/카카오톡.app", ItemKind::App),
            item("Chrome", "/Applications/Chrome.app", ItemKind::App),
        ];
        let o = run(items.clone(), "zkzkdhxhr");
        assert_eq!(names(&o).first(), Some(&"카카오톡"));
        assert_eq!(o.notice.as_deref(), Some("한글로 바꿔 찾음: 카카오톡"));

        // 영어 질의는 그대로 — 바꾸지 않는다
        let o = run(items, "chrome");
        assert_eq!(names(&o).first(), Some(&"Chrome"));
        assert!(o.notice.is_none());
    }

    #[test]
    fn 한영_판정은_경로에_든_토큰도_그대로_든_것으로_본다() {
        // 이름엔 없지만 경로에 낱말마다 그대로 있다 — 한글로 바꾸지 않는다.
        // (질의 전체가 이름에 들었는지만 보던 TUI 는 여기서 바꿔 버렸다)
        let o = run(
            vec![
                item("Report", "/docs/qhrhtj/dlfwl/Report.pdf", ItemKind::File),
                item("보고서 일지", "/docs/보고서 일지.hwp", ItemKind::File),
            ],
            "qhrhtj dlfwl",
        );
        assert_eq!(names(&o).first(), Some(&"Report"));
        assert!(o.notice.is_none(), "{:?}", names(&o));
    }

    #[test]
    fn 이름으로_친_명령은_상위_세_개_아래에_낀다() {
        let items = (0..5)
            .map(|i| {
                item(
                    &format!("timer app {i}"),
                    &format!("/a/timer{i}"),
                    ItemKind::App,
                )
            })
            .collect();
        let o = run(items, "timer");
        let at = o
            .results
            .iter()
            .position(|r| r.item.name.starts_with(":timer"))
            .expect("이름으로 친 명령이 있어야 한다");
        assert_eq!(at, NAMED_COMMAND_KEEP_TOP, "{:?}", names(&o));
    }

    #[test]
    fn 결과가_없으면_웹_제안을_보여준다() {
        let o = run(Vec::new(), "zzqqxx");
        assert!(o.results.iter().any(|r| r.item.kind == ItemKind::WebSearch));
        assert_eq!(o.mode, SearchMode::Contains);
        // CLI(제안 끔)는 비운 채로
        let o = run_with(Vec::new(), "zzqqxx", &FrecencyMap::default(), false);
        assert!(o.results.is_empty());
    }

    #[test]
    fn 조합_중_끝_자모는_떼고_다시_찾는다() {
        let o = run(vec![item("하늘", "/a/하늘", ItemKind::File)], "하ㄴ");
        assert_eq!(names(&o).first(), Some(&"하늘"));
        assert_eq!(relaxed_hangul_query("한ㄱ").as_deref(), Some("한"));
        assert_eq!(relaxed_hangul_query("한"), None);
        assert_eq!(relaxed_hangul_query("ㄱㄴ"), None);
        assert_eq!(relaxed_hangul_query(""), None);
    }

    #[test]
    fn 콜론_자동완성은_결과를_갈아_끼운다() {
        let o = run(vec![item("timer", "/a/timer", ItemKind::App)], ":ti");
        assert_eq!(names(&o), [":timer  Timer (타이머)"]);
    }

    #[test]
    fn llm_실행_계획은_모든_서비스_url과_붙여넣을_프롬프트를_담는다() {
        let launcher = LauncherConfig::default();
        let launch = llm_launch("@gpt hello", &launcher).expect("LLM 질의");
        assert!(!launch.all_urls.is_empty());
        // LLM 질의가 아니면 계획이 없다 — 일반 웹 결과 경로로 간다
        assert!(llm_launch("hello", &launcher).is_none());
    }

    #[test]
    fn 변환_실행_계획은_슬래시_명령도_같게_읽는다() {
        let launcher = LauncherConfig::default();
        let colon = transform_launch(":t spell 안녕", &launcher).expect(":t");
        let slash = transform_launch("/t spell 안녕", &launcher).expect("/t");
        assert!(matches!(colon.kind, crate::transform::TransformKind::Spell));
        assert_eq!(colon.urls, slash.urls);
        assert!(transform_launch("hello", &launcher).is_none());
    }
}
