//! 결과 항목 실행(Enter) 분류 — `kmd:*` 마커 해석의 정본 (REF-06)
//!
//! 예전엔 데스크톱 `launch_selected`와 TUI `execute_selected`가 마커 문자열을
//! 각자 if 사슬로 해석해 이미 어긋나 있었다 — TUI는 `@` 브라우징 가상 항목과
//! 폴더 검색 시드를 몰라 Enter가 "열 수 있는 URL이 없습니다" 오류로 끝났고,
//! 데스크톱은 모르는 `kmd:*` 안내 항목을 시스템 명령으로 실행하려 했다.
//!
//! 이제 "이 항목을 실행하면 무엇을 해야 하는가"는 [`classify`] 한 곳에서 정하고,
//! UI는 결과 [`Activation`]에 맞는 효과(창 닫기, 상태 표시, 비동기 작업)만 적용한다.
//! 오토파일럿·클립보드처럼 UI 문맥이 필요한 단계는 분류만 하고 실행은 UI 몫이다.

use crate::index::{IndexItem, ItemKind};
use crate::plugin::builtin_shell::ShellExtension;

/// 항목 실행 시 UI가 할 일
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// 입력을 이 문자열로 바꾸고 다시 검색 (도움말 시드, `!bang` 힌트, 미지 명령 →
    /// `:help`, 폴더 검색 시드, `@` 브라우징 힌트, `:prompt add` 힌트)
    SetQuery(String),
    /// 클립보드 히스토리 붙여넣기 (`kmd:clip:*`) — 데스크톱 흐름 B
    ClipPaste,
    /// `:t` 실행 — 현재 질의로 변환 URL을 연다
    TransformRun,
    /// `:prompt add/rm` 확정 — 현재 질의를 다시 해석해 적용·저장
    PromptCommit,
    /// 폴더 제안 → search_paths에 추가 (마커는 항목 keywords)
    FolderSuggest,
    /// `:scope` 추가 — 이 폴더를 검색 범위에 넣고 바로 인덱싱 (docs/18)
    ScopeAdd(String),
    /// `:scope` 목록의 범위 폴더 — Enter는 탐색기에서 열기 (Ctrl+Del 제거는 UI 단축키)
    ScopeFolder(String),
    /// 데스크톱 설정 항목 (`kmd:settings:*`)
    Settings,
    /// 키맵 항목 (`kmd:keymap:*`, noop 제외)
    Keymap,
    /// TUI 설정 모달 (`kmd:tui:open_settings`)
    OpenSettingsModal,
    /// 셸 quick action — 백그라운드 실행 후 결과 복사
    ShellQuickAction,
    /// 셸 사용자 명령 — 새 터미널 창에서 실행
    ShellTerminal,
    /// 값 복사 (계산기·이모지)
    Copy(String),
    /// 웹 결과 — LLM 라우팅 → URL 일괄 열기 → 기본 실행 순으로 UI가 처리
    Web,
    /// 안내 항목 — 아무것도 하지 않는다
    Noop,
    /// 기본 실행 (`action::execute` + 실행 이력 기록)
    Execute,
}

/// TUI 설정 모달 마커 (`:set`)
pub const TUI_OPEN_SETTINGS_MARKER: &str = "kmd:tui:open_settings";
/// 미지 명령 안내 마커
pub const UNKNOWN_CMD_MARKER: &str = "kmd:unknown_cmd";

/// 결과 항목을 실행 동작으로 분류한다.
pub fn classify(item: &IndexItem) -> Activation {
    let kw = item.keywords.as_str();

    if kw.starts_with("kmd:clip:") {
        return Activation::ClipPaste;
    }
    if kw.starts_with("kmd:help:") {
        return match crate::query_prefix::help_query_seed(&item.name) {
            Some(seed) => Activation::SetQuery(seed.to_string()),
            None => Activation::Noop,
        };
    }
    if kw.starts_with("kmd:bang_hint:") {
        return Activation::SetQuery(item.path.clone());
    }
    if kw == crate::transform::TRANSFORM_RUN_MARKER {
        return Activation::TransformRun;
    }
    if kw == crate::transform::TRANSFORM_EMPTY_MARKER {
        return Activation::Noop;
    }
    if kw == UNKNOWN_CMD_MARKER {
        return Activation::SetQuery(":help".to_string());
    }
    if kw == crate::prompt::PROMPT_COMMIT_MARKER {
        return Activation::PromptCommit;
    }
    if kw == crate::prompt::PROMPT_ADD_HINT_MARKER {
        return Activation::SetQuery(":prompt add ".to_string());
    }
    if kw.starts_with(crate::folder_suggest::SUGGEST_MARKER) {
        return Activation::FolderSuggest;
    }
    if let Some(path) = kw.strip_prefix(crate::scope::SCOPE_ADD_MARKER) {
        return Activation::ScopeAdd(path.to_string());
    }
    if let Some(path) = kw.strip_prefix(crate::scope::SCOPE_FOLDER_MARKER) {
        return Activation::ScopeFolder(path.to_string());
    }
    if let Some(path) = kw.strip_prefix(crate::scope::SCOPE_BROWSE_MARKER) {
        return Activation::SetQuery(crate::scope::browse_seed(path));
    }
    if kw == crate::scope::SCOPE_OPEN_MARKER {
        return Activation::SetQuery(":scope ".to_string());
    }
    if kw == crate::scope::SCOPE_NOOP_MARKER {
        return Activation::Noop;
    }
    if kw.starts_with("kmd:folder_search:") {
        let seed = item.path.trim();
        return if seed.starts_with(":f") {
            Activation::SetQuery(seed.to_string())
        } else {
            Activation::Noop
        };
    }
    if kw == TUI_OPEN_SETTINGS_MARKER {
        return Activation::OpenSettingsModal;
    }
    if item.kind == ItemKind::SystemCommand && kw.starts_with("kmd:") {
        if kw.ends_with(":noop") || kw.contains(":noop:") {
            return Activation::Noop;
        }
        if kw.starts_with("kmd:settings:") {
            return Activation::Settings;
        }
        if kw.starts_with("kmd:keymap:") {
            return Activation::Keymap;
        }
        // 그 밖의 kmd:* 시스템 항목은 안내용 — 시스템 명령으로 실행하면 안 된다
        return Activation::Noop;
    }

    match item.kind {
        ItemKind::Shell => {
            if ShellExtension::is_quick_action(&item.path) {
                Activation::ShellQuickAction
            } else {
                Activation::ShellTerminal
            }
        }
        ItemKind::Calculator | ItemKind::Emoji => Activation::Copy(item.path.clone()),
        ItemKind::WebSearch => match crate::web::web_browse_hint_seed(kw) {
            Some(seed) => Activation::SetQuery(seed.to_string()),
            None => Activation::Web,
        },
        _ => Activation::Execute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Source;

    fn item(kind: ItemKind, name: &str, path: &str, keywords: &str) -> IndexItem {
        IndexItem {
            name: name.into(),
            path: path.into(),
            kind,
            source: Source::Plugin,
            icon: String::new(),
            keywords: keywords.into(),
            icon_path: None,
        }
    }

    #[test]
    fn 마커별_분류() {
        use Activation::*;
        let sys = ItemKind::SystemCommand;
        assert_eq!(classify(&item(sys, "", "", "kmd:clip:3:1")), ClipPaste);
        assert_eq!(
            classify(&item(sys, "", "@g ", "kmd:bang_hint:g")),
            SetQuery("@g ".into())
        );
        assert_eq!(
            classify(&item(sys, "", "", UNKNOWN_CMD_MARKER)),
            SetQuery(":help".into())
        );
        assert_eq!(
            classify(&item(sys, "", "", crate::prompt::PROMPT_ADD_HINT_MARKER)),
            SetQuery(":prompt add ".into())
        );
        assert_eq!(
            classify(&item(sys, "", "", crate::prompt::PROMPT_COMMIT_MARKER)),
            PromptCommit
        );
        assert_eq!(classify(&item(sys, "", "", "kmd:settings:theme")), Settings);
        assert_eq!(classify(&item(sys, "", "", "kmd:settings:noop")), Noop);
        assert_eq!(classify(&item(sys, "", "", "kmd:keymap:use:x")), Keymap);
        assert_eq!(classify(&item(sys, "", "", "kmd:keymap:noop")), Noop);
        assert_eq!(
            classify(&item(sys, "", "", TUI_OPEN_SETTINGS_MARKER)),
            OpenSettingsModal
        );
        assert_eq!(
            classify(&item(sys, "", ":f  docs", "kmd:folder_search:hint")),
            SetQuery(":f  docs".into())
        );
        assert_eq!(
            classify(&item(sys, "", "", "kmd:folder_search:empty")),
            Noop
        );
    }

    #[test]
    fn 모르는_kmd_시스템_항목은_실행하지_않는다() {
        // 예전 데스크톱은 이런 안내 항목을 action::execute(시스템 명령)로 넘겼다
        let hint = item(
            ItemKind::SystemCommand,
            "저장된 템플릿 없음",
            "",
            "kmd:content:hint",
        );
        assert_eq!(classify(&hint), Activation::Noop);
    }

    #[test]
    fn 웹_브라우징_가상_항목은_해당_모드로_전환한다() {
        // 예전 TUI는 이 항목을 action::execute로 넘겨 "URL이 없습니다" 오류가 났다
        let browse = item(
            ItemKind::WebSearch,
            "@llm",
            "설명",
            "kmd:web_hint:multi_llm",
        );
        assert_eq!(classify(&browse), Activation::SetQuery("@llm ".into()));
        let web = item(ItemKind::WebSearch, "Google", "https://g", "");
        assert_eq!(classify(&web), Activation::Web);
    }

    #[test]
    fn 종류별_기본_분류() {
        let calc = item(ItemKind::Calculator, "1+1", "2", "");
        assert_eq!(classify(&calc), Activation::Copy("2".into()));
        let app = item(ItemKind::App, "Safari", "/Applications/Safari.app", "");
        assert_eq!(classify(&app), Activation::Execute);
        let hit = item(ItemKind::File, "a.md", "/a.md", "kmd:content:hit");
        assert_eq!(
            classify(&hit),
            Activation::Execute,
            "본문 검색 결과는 파일 열기"
        );
    }
}
