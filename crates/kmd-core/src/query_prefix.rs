//! 검색창 특수 프리픽스 분류 — TUI와 데스크톱이 공유하는 단일 파서.
//!
//! 별칭 매칭 규칙: **완전일치 또는 별칭 바로 뒤 공백**(토큰 경계).
//! 예: `:pt hello` → Prompt, `:pto` → General (별칭 오인 방지),
//! `:settings` → Settings, `:setup` → General.
//!
//! [`COMMANDS`] 레지스트리가 단일 진실 소스다: `prefix_of` 판별과
//! `:help` 목록([`help_items`])·퀵 템플릿 시드([`help_query_seed`])가
//! 모두 여기서 나온다. 새 명령을 추가할 때는 레지스트리에 한 항목만 넣으면 된다.

use crate::index::{IndexItem, ItemKind, Source};

/// 검색창 입력의 특수 프리픽스 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryPrefix {
    /// `@` — 웹/AI 서비스 검색
    Web,
    /// `:t` / `:transform` — 클립보드 변환 (맞춤법/번역)
    Transform,
    /// `:prompt` / `:pt` — 프롬프트 템플릿 관리
    Prompt,
    /// `:calc` — 계산기
    Calc,
    /// `:emoji` / `:e` — 이모지 검색
    Emoji,
    /// `:set` / `:settings` — 설정
    Settings,
    /// `:help` / `:h` — 도움말
    Help,
    /// `:version` / `:ver` / `:v` — 버전 정보
    Version,
    /// `!` — 셸 명령 실행
    Shell,
    /// `:keymap` / `:km` — kanata 키맵 제어
    Keymap,
    /// `:keys` / `:k` — 키 바인딩 치트시트
    Keys,
    /// `:f` — 특정 폴더 내 즉석 파일 검색
    FolderSearch,
    /// `?` / `:grep` — 문서 본문 검색 (FTS5, docs/15)
    ContentSearch,
    /// `;` / `:clip` — 클립보드 히스토리 검색 (docs/12 흐름 B)
    Clipboard,
    /// 일반 검색 (fuzzy / glob / regex / contains / url)
    General,
}

/// `:` 명령 하나의 정의 — 별칭, 도움말 표기, Enter 시 시작 쿼리까지 한 곳에.
pub struct CommandSpec {
    pub prefix: QueryPrefix,
    /// 토큰 경계로 매칭되는 별칭 목록 (긴 형태를 앞에)
    pub aliases: &'static [&'static str],
    /// `:help` 목록에 표시되는 제목
    pub title: &'static str,
    /// `:help` 목록에 표시되는 사용법 설명
    pub usage: &'static str,
    /// 도움말 항목을 Enter로 선택했을 때 검색창에 채울 시작 쿼리
    pub seed: &'static str,
    pub icon_emoji: &'static str,
    pub icon_ascii: &'static str,
}

/// `:` 명령 레지스트리 — 배열 순서가 `:help` 표시 순서다.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        prefix: QueryPrefix::Calc,
        aliases: &[":calc"],
        title: ":calc  Calculator",
        usage: "Type :calc expression  (e.g. :calc (2+3)*4)",
        seed: ":calc ",
        icon_emoji: "\u{1F522}",
        icon_ascii: "[CAL]",
    },
    CommandSpec {
        prefix: QueryPrefix::Emoji,
        aliases: &[":emoji", ":e"],
        title: ":emoji  Emoji Search",
        usage: "Type :emoji keyword  or  :e keyword  (e.g. :e fire)",
        seed: ":emoji ",
        icon_emoji: "\u{1F60A}",
        icon_ascii: "[EMO]",
    },
    CommandSpec {
        prefix: QueryPrefix::Settings,
        aliases: &[":settings", ":set"],
        title: ":set  Settings",
        usage: "Type :set or :settings to manage config, themes, index",
        seed: ":set",
        icon_emoji: "\u{2699}\u{FE0F}",
        icon_ascii: "[SET]",
    },
    CommandSpec {
        prefix: QueryPrefix::Transform,
        aliases: &[":transform", ":t"],
        title: ":t  Quick Transform",
        usage: "Type :t spell/tr/trko/tren  (clipboard text → spell/translate)",
        seed: ":t ",
        icon_emoji: "\u{26A1}",
        icon_ascii: "[QT]",
    },
    CommandSpec {
        prefix: QueryPrefix::Prompt,
        aliases: &[":prompt", ":pt"],
        title: ":prompt  Prompt Templates",
        usage: "Type :prompt  (manage reusable prompt templates for @ll)",
        seed: ":prompt",
        icon_emoji: "\u{1F4DD}",
        icon_ascii: "[PT]",
    },
    CommandSpec {
        prefix: QueryPrefix::FolderSearch,
        aliases: &[":f"],
        title: ":f  Folder Search",
        usage: "Type :f /path query  (search inside a specific folder)",
        seed: ":f ",
        icon_emoji: "\u{1F4C2}",
        icon_ascii: "[DIR]",
    },
    CommandSpec {
        prefix: QueryPrefix::ContentSearch,
        aliases: &[":grep"],
        title: "?  Content Search",
        usage: "Type ? keyword  (documents; \"exact phrase\", -exclude, ext:md, path:docs)",
        seed: "?",
        icon_emoji: "\u{1F50E}",
        icon_ascii: "[GRP]",
    },
    CommandSpec {
        prefix: QueryPrefix::Keys,
        aliases: &[":keys", ":k"],
        title: ":keys  Key Mapping Sheet",
        usage: "Type :keys or :k  (show all keybinding cheatsheet)",
        seed: ":keys",
        icon_emoji: "\u{1F5FA}\u{FE0F}",
        icon_ascii: "[KEY]",
    },
    CommandSpec {
        prefix: QueryPrefix::Keymap,
        aliases: &[":keymap", ":km"],
        title: ":keymap  Keymap Control",
        usage: "Type :keymap or :km  (kanata status, on/off, profile switch)",
        seed: ":keymap",
        icon_emoji: "\u{2328}\u{FE0F}",
        icon_ascii: "[KM]",
    },
    CommandSpec {
        prefix: QueryPrefix::Version,
        aliases: &[":version", ":ver", ":v"],
        title: ":version  Version Info",
        usage: "Type :version  (show app/core/target/os versions)",
        seed: ":version",
        icon_emoji: "\u{1F4E6}",
        icon_ascii: "[VER]",
    },
    CommandSpec {
        prefix: QueryPrefix::Clipboard,
        aliases: &[":clip"],
        title: ";  Clipboard History",
        usage: "Type ;query or :clip query  (paste a past copy into the previous app)",
        seed: ";",
        icon_emoji: "\u{1F4CB}",
        icon_ascii: "[CLIP]",
    },
    CommandSpec {
        prefix: QueryPrefix::Help,
        aliases: &[":help", ":h"],
        title: ":help  Help",
        usage: "Type :help or :h  (show this command list)",
        seed: ":help",
        icon_emoji: "\u{2753}",
        icon_ascii: "[?]",
    },
];

/// `:` 명령이 아닌 도움말 항목 (시길 프리픽스, 검색 모드 예시 등)
struct HelpEntry {
    name: &'static str,
    usage: &'static str,
    seed: &'static str,
    icon_emoji: &'static str,
    icon_ascii: &'static str,
    /// true면 검색 모드 예시 항목 (`kmd:help:example` 키워드)
    example: bool,
}

/// `:help` 상단 — 가장 많이 쓰는 시길 프리픽스
const HELP_TOP: &[HelpEntry] = &[HelpEntry {
    name: "@  Web Search",
    usage: "Type @prefix query  (e.g. @g rust, @ai why is the sky blue)",
    seed: "@",
    icon_emoji: "\u{1F310}",
    icon_ascii: "[WEB]",
    example: false,
}];

/// `:help` 하단 — @ 계열 확장, 셸, 검색 모드 예시
const HELP_BOTTOM: &[HelpEntry] = &[
    HelpEntry {
        name: "@llm  Multi LLM Compare",
        usage: "Type @ll prompt  (alias: @llm, open selected LLM providers)",
        seed: "@ll ",
        icon_emoji: "\u{1F9E0}",
        icon_ascii: "[LLM]",
        example: false,
    },
    HelpEntry {
        name: "@msearch  Multi Web Search",
        usage: "Type @m query  (alias: @msearch, open selected web engines)",
        seed: "@m ",
        icon_emoji: "\u{1F50E}",
        icon_ascii: "[MWEB]",
        example: false,
    },
    HelpEntry {
        name: "@sp  Spell Check",
        usage: "Type @sp text  (Korean spelling check on selected providers)",
        seed: "@sp ",
        icon_emoji: "\u{270D}\u{FE0F}",
        icon_ascii: "[SPL]",
        example: false,
    },
    HelpEntry {
        name: "@tr  Translate",
        usage: "Type @tr/@trko/@tren text  (auto / en->ko / ko->en)",
        seed: "@tr ",
        icon_emoji: "\u{1F5E3}\u{FE0F}",
        icon_ascii: "[TR]",
        example: false,
    },
    HelpEntry {
        name: "!  Shell Command",
        usage: "Type !command or >command  (e.g. !ip, !hostname, >echo hello)",
        seed: "!",
        icon_emoji: "\u{1F4BB}",
        icon_ascii: "[SHL]",
        example: false,
    },
    HelpEntry {
        name: "Fuzzy Search",
        usage: "Just type to search files, apps, folders  (e.g. firefox)",
        seed: "firefox",
        icon_emoji: "\u{1F50D}",
        icon_ascii: "[FZF]",
        example: true,
    },
    HelpEntry {
        name: "*.ext  Glob Pattern",
        usage: "Use * or ? for glob matching  (e.g. *.pdf, test?.rs)",
        seed: "*.pdf",
        icon_emoji: "\u{1F4C4}",
        icon_ascii: "[GLB]",
        example: true,
    },
    HelpEntry {
        name: "/regex/  Regular Expression",
        usage: "Wrap in /slashes/ for regex  (e.g. /test\\d+/)",
        seed: "/test\\d+/",
        icon_emoji: "\u{1F9EA}",
        icon_ascii: "[RGX]",
        example: true,
    },
];

/// query가 별칭 중 하나와 토큰 경계로 일치하는지 검사.
///
/// "완전일치" 또는 "별칭 + 공백"만 인정한다 — `:pto`는 `:pt`에 걸리지 않는다.
pub fn matches_command(query: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| {
        query
            .strip_prefix(alias)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// `/help`, `/set`처럼 `/`로 시작하는 명령 입력을 `:` 형태로 정규화한다.
///
/// Slack/Discord/ChatGPT 계열에 익숙한 사용자를 위한 관용 별칭.
/// 닫힌 `/pattern/` 형태(정규식 검색)와 알 수 없는 `/...` 입력은 `None`을
/// 반환해 일반 검색으로 흘려보낸다.
pub fn normalize_slash_command(query: &str) -> Option<String> {
    let rest = query.strip_prefix('/')?;
    // 닫힌 /pattern/ 형태는 정규식 검색 문법이 우선한다
    if query.len() > 2 && query.ends_with('/') {
        return None;
    }
    for spec in COMMANDS {
        for alias in spec.aliases {
            let name = alias.strip_prefix(':').unwrap_or(alias);
            if let Some(r) = rest.strip_prefix(name) {
                if r.is_empty() || r.starts_with(' ') {
                    return Some(format!(":{name}{r}"));
                }
            }
        }
    }
    None
}

/// 검색창 입력을 프리픽스 종류로 분류한다.
pub fn prefix_of(query: &str) -> QueryPrefix {
    if query.starts_with('@') {
        return QueryPrefix::Web;
    }
    // `>`는 런처 생태계(PowerToys Run, Flow Launcher, Alfred)의 셸 관례 별칭
    if query.starts_with('!') || query.starts_with('>') {
        return QueryPrefix::Shell;
    }
    // `;` — 클립보드 히스토리 (고빈도 기능은 한 글자 시길, docs/12)
    if query.starts_with(';') {
        return QueryPrefix::Clipboard;
    }
    // `?` — 문서 본문 검색 (docs/15). 한 글자 시길이라 타이핑 흐름을 안 끊는다
    if query.starts_with('?') {
        return QueryPrefix::ContentSearch;
    }
    if query.starts_with(':') {
        for spec in COMMANDS {
            if matches_command(query, spec.aliases) {
                return spec.prefix;
            }
        }
    }
    if let Some(normalized) = normalize_slash_command(query) {
        return prefix_of(&normalized);
    }
    QueryPrefix::General
}

fn help_item(name: &str, usage: &str, icon: &str, example: bool) -> IndexItem {
    IndexItem {
        name: name.to_string(),
        path: usage.to_string(),
        icon: icon.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        keywords: if example {
            "kmd:help:example".to_string()
        } else {
            "kmd:help:entry".to_string()
        },
        icon_path: None,
    }
}

/// `:help` 결과 목록 — TUI와 데스크톱이 공유하는 명령 안내 항목.
///
/// 항목을 Enter로 선택하면 [`help_query_seed`]가 주는 시작 쿼리로 전환된다.
pub fn help_items(use_emoji: bool) -> Vec<IndexItem> {
    let icon = |emoji: &'static str, ascii: &'static str| if use_emoji { emoji } else { ascii };

    let mut items: Vec<IndexItem> = Vec::new();
    for e in HELP_TOP {
        items.push(help_item(
            e.name,
            e.usage,
            icon(e.icon_emoji, e.icon_ascii),
            e.example,
        ));
    }
    for spec in COMMANDS {
        items.push(help_item(
            spec.title,
            spec.usage,
            icon(spec.icon_emoji, spec.icon_ascii),
            false,
        ));
    }
    for e in HELP_BOTTOM {
        items.push(help_item(
            e.name,
            e.usage,
            icon(e.icon_emoji, e.icon_ascii),
            e.example,
        ));
    }
    items
}

/// 도움말 항목 이름 → Enter 시 검색창에 채울 시작 쿼리(퀵 템플릿).
pub fn help_query_seed(name: &str) -> Option<&'static str> {
    if let Some(spec) = COMMANDS.iter().find(|s| s.title == name) {
        return Some(spec.seed);
    }
    HELP_TOP
        .iter()
        .chain(HELP_BOTTOM)
        .find(|e| e.name == name)
        .map(|e| e.seed)
}

/// `!g rust`처럼 `!` 뒤에 등록된 웹 서비스 프리픽스가 오면
/// `@g rust` 웹 검색으로 전환하는 안내 항목을 만든다.
///
/// DuckDuckGo bang(`!g`) 습관이 있는 사용자가 셸 모드에 빠졌을 때를 위한 힌트.
/// 항목의 `path`가 전환할 쿼리이며, keywords는 `kmd:bang_hint:<service_id>`.
pub fn bang_web_hint(query: &str, use_emoji: bool) -> Option<IndexItem> {
    let rest = query.strip_prefix('!')?;
    let (word, tail) = match rest.split_once(' ') {
        Some((w, t)) => (w, t.trim()),
        None => (rest, ""),
    };
    if word.is_empty() {
        return None;
    }
    let at_prefix = format!("@{}", word.to_ascii_lowercase());
    let service = crate::web::WEB_SERVICES.iter().find(|s| {
        s.prefixes
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&at_prefix))
    })?;
    let seed = if tail.is_empty() {
        format!("{at_prefix} ")
    } else {
        format!("{at_prefix} {tail}")
    };
    Some(IndexItem {
        name: format!("웹 검색으로 전환: {}", seed.trim_end()),
        path: seed,
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: if use_emoji { "\u{1F310}" } else { "[WEB]" }.to_string(),
        keywords: format!("kmd:bang_hint:{}", service.id),
        icon_path: None,
    })
}

/// `:clac`처럼 `:`로 시작하지만 알려진 명령이 아닌 입력에 대한 안내 항목.
///
/// 오타나 미지원 명령일 때 결과 최상단에 표시할 힌트 — 일반 검색 폴스루는
/// 유지하되, Vim의 `E492: Not an editor command`처럼 조용히 넘어가지 않는다.
/// Enter 시 `:help`로 이동(keywords: `kmd:unknown_cmd`).
pub fn unknown_command_hint(query: &str, use_emoji: bool) -> Option<IndexItem> {
    if !query.starts_with(':') || query.len() < 2 {
        return None;
    }
    if prefix_of(query) != QueryPrefix::General {
        return None;
    }
    let token = query.split_whitespace().next().unwrap_or(query);
    // 알려진 명령을 입력하는 중(별칭의 접두사)이면 아직 힌트를 띄우지 않는다
    let typing_known = COMMANDS
        .iter()
        .any(|s| s.aliases.iter().any(|a| a.starts_with(token)));
    if typing_known {
        return None;
    }
    Some(IndexItem {
        name: format!("알 수 없는 명령: {token}"),
        path: "Enter로 :help 를 열어 사용 가능한 명령을 확인하세요".to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: if use_emoji { "\u{2753}" } else { "[?]" }.to_string(),
        keywords: "kmd:unknown_cmd".to_string(),
        icon_path: None,
    })
}

/// `:version` 결과 목록 — 앱 이름/버전만 프런트엔드별로 다르다.
pub fn version_items(app_label: &str, app_version: &str, use_emoji: bool) -> Vec<IndexItem> {
    let emoji = use_emoji;
    vec![
        IndexItem {
            name: format!("{app_label} {app_version}"),
            path: "Application version".to_string(),
            icon: if emoji { "\u{1F4E6}" } else { "[VER]" }.to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            keywords: "kmd:settings:noop".to_string(),
            icon_path: None,
        },
        IndexItem {
            name: format!("kmd-core {}", crate::Index::current_version()),
            path: "Search index schema version".to_string(),
            icon: if emoji { "\u{1F9E0}" } else { "[CORE]" }.to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            keywords: "kmd:settings:noop".to_string(),
            icon_path: None,
        },
        IndexItem {
            name: format!("target {}", std::env::consts::ARCH),
            path: format!("os {}", std::env::consts::OS),
            icon: if emoji { "\u{1F5A5}\u{FE0F}" } else { "[SYS]" }.to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            keywords: "kmd:settings:noop".to_string(),
            icon_path: None,
        },
    ]
}

// ── 프리픽스별 결과 생성 (순수) ──────────────────────────────────────────────
//
// 아래 함수들은 **쿼리 → 항목 목록**만 담당한다. 데스크톱과 TUI가 같은
// 프리픽스 문자열(`:emoji`/`:e`, `!`/`>`, `:keymap`/`:km`)을 각자 벗겨내던
// 것을 여기로 모았다 — 별칭 하나를 추가할 때 한쪽만 고치면 두 UI의 동작이
// 갈라지는 것이 실제 위험이었다.
//
// 결과를 상태에 반영하는 방식(점수·정렬·검색 모드)은 UI마다 다르므로
// 여기서 다루지 않는다. 마찬가지로 UI별로 이미 갈라진 핸들러
// (`transform`은 데스크톱=결과 항목/TUI=상태 메시지, `prompt`·`keys`는
// 설정 출처가 다름)는 의도적으로 통합하지 않았다.

/// `:calc <식>` — 계산기 항목.
pub fn calc_items(query: &str, use_emoji: bool) -> Vec<IndexItem> {
    let expr = query.strip_prefix(":calc").unwrap_or("").trim();
    crate::plugin::builtin_calc::CalcExtension.search_with_emoji(expr, use_emoji)
}

/// `:emoji <키워드>` / `:e <키워드>` — 이모지 항목.
pub fn emoji_items(query: &str) -> Vec<IndexItem> {
    let keyword = query
        .strip_prefix(":emoji")
        .or_else(|| query.strip_prefix(":e"))
        .unwrap_or("")
        .trim();
    crate::plugin::builtin_emoji::EmojiExtension.search_emoji(keyword)
}

/// `!<명령>` / `><명령>` — 셸 항목. `!g rust`처럼 웹 검색으로 보이는 입력이면
/// 전환 힌트를 두 번째 자리에 끼운다.
pub fn shell_items(query: &str, use_emoji: bool) -> Vec<IndexItem> {
    let shell_query = query.strip_prefix(['!', '>']).unwrap_or("").trim();
    use crate::plugin::Extension as _;
    let mut items = crate::plugin::builtin_shell::ShellExtension.search(shell_query);
    if let Some(hint) = bang_web_hint(query, use_emoji) {
        let pos = items.len().min(1);
        items.insert(pos, hint);
    }
    items
}

/// `:keymap <하위명령>` / `:km <하위명령>` — 키맵 항목.
pub fn keymap_query_items(config: &crate::Config, query: &str, use_emoji: bool) -> Vec<IndexItem> {
    let sub = query
        .strip_prefix(":keymap")
        .or_else(|| query.strip_prefix(":km"))
        .unwrap_or("")
        .trim();
    crate::keymap::keymap_items(config, sub, use_emoji)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 프리픽스 결과 생성 (데스크톱·TUI 공용) ─────────────────────────
    //
    // 이 별칭 파싱이 예전에는 두 UI에 각각 복사돼 있었다. 별칭을 하나 늘릴 때
    // 한쪽만 고치면 같은 입력에 다른 결과가 나온다.

    #[test]
    fn emoji_별칭_두_가지가_같은_키워드를_준다() {
        let long = emoji_items(":emoji heart");
        let short = emoji_items(":e heart");
        assert!(!long.is_empty(), ":emoji 결과 있음");
        assert_eq!(
            long.iter().map(|i| &i.name).collect::<Vec<_>>(),
            short.iter().map(|i| &i.name).collect::<Vec<_>>(),
            ":emoji 와 :e 는 같은 결과여야 한다"
        );
    }

    #[test]
    fn shell_프리픽스_두_가지가_같은_결과를_준다() {
        let bang = shell_items("!echo", false);
        let gt = shell_items(">echo", false);
        assert_eq!(
            bang.iter().map(|i| &i.name).collect::<Vec<_>>(),
            gt.iter().map(|i| &i.name).collect::<Vec<_>>(),
            "! 와 > 는 같은 결과여야 한다"
        );
    }

    #[test]
    fn shell은_웹검색처럼_보이면_전환_힌트를_끼운다() {
        // `!g rust` — 셸 명령이 아니라 웹 검색 의도로 보이는 입력
        let items = shell_items("!g rust", false);
        let hint_pos = items
            .iter()
            .position(|i| i.name.contains('@'))
            .expect("전환 힌트가 있어야 한다");
        assert!(
            hint_pos <= 1,
            "힌트는 최상단 또는 두 번째 자리 (실제 {hint_pos})"
        );

        // 평범한 셸 입력에는 힌트가 없다
        assert!(
            !shell_items("!echo hi", false)
                .iter()
                .any(|i| i.name.contains('@')),
            "일반 셸 입력에는 전환 힌트를 넣지 않는다"
        );
    }

    #[test]
    fn calc는_프리픽스만_벗기고_식을_넘긴다() {
        let items = calc_items(":calc 2+3", false);
        assert!(
            items.iter().any(|i| i.name.contains('5')),
            "2+3 결과 5가 있어야 한다: {items:?}"
        );
    }

    #[test]
    fn 프리픽스만_있고_인자가_없어도_패닉하지_않는다() {
        // 사용자는 ":e"까지만 친 상태를 반드시 거친다 — 매 키 입력마다 호출된다
        for q in [":calc", ":emoji", ":e", "!", ">", ":keymap", ":km"] {
            let _ = calc_items(q, false);
            let _ = emoji_items(q);
            let _ = shell_items(q, false);
            let _ = keymap_query_items(&crate::Config::default(), q, false);
        }
    }

    #[test]
    fn sigil_prefixes() {
        assert_eq!(prefix_of("@g rust"), QueryPrefix::Web);
        assert_eq!(prefix_of("@"), QueryPrefix::Web);
        assert_eq!(prefix_of("!ip"), QueryPrefix::Shell);
        assert_eq!(prefix_of(">ip"), QueryPrefix::Shell);
        // 클립보드 히스토리 (docs/12 흐름 B) — 시길 `;`와 `:clip` 별칭
        assert_eq!(prefix_of(";"), QueryPrefix::Clipboard);
        assert_eq!(prefix_of(";계좌"), QueryPrefix::Clipboard);
        assert_eq!(prefix_of(":clip"), QueryPrefix::Clipboard);
        assert_eq!(prefix_of(":clip foo"), QueryPrefix::Clipboard);
        // 토큰 경계 — :clipboard 는 :clip 오인이 아니어야 (별칭 뒤 공백/끝만 매칭)
        assert_ne!(prefix_of(":clipx"), QueryPrefix::Clipboard);
    }

    #[test]
    fn bang_web_hint_for_ddg_habit() {
        // !g rust → @g rust 전환 힌트 (DuckDuckGo bang 습관)
        let hint = bang_web_hint("!g rust tutorial", false).expect("힌트가 있어야 함");
        assert_eq!(hint.path, "@g rust tutorial");
        assert!(hint.keywords.starts_with("kmd:bang_hint:"));
        // 쿼리가 없어도 프리픽스만 맞으면 힌트 제공
        assert_eq!(bang_web_hint("!yt", false).unwrap().path, "@yt ");
        // 등록된 웹 프리픽스가 아니면 힌트 없음
        assert!(bang_web_hint("!echo hello", false).is_none());
        assert!(bang_web_hint("!ip", false).is_none());
        assert!(bang_web_hint("!", false).is_none());
    }

    #[test]
    fn command_aliases_match_on_token_boundary() {
        assert_eq!(prefix_of(":t spell hello"), QueryPrefix::Transform);
        assert_eq!(prefix_of(":transform"), QueryPrefix::Transform);
        assert_eq!(prefix_of(":pt"), QueryPrefix::Prompt);
        assert_eq!(prefix_of(":prompt add x y"), QueryPrefix::Prompt);
        assert_eq!(prefix_of(":calc (2+3)*4"), QueryPrefix::Calc);
        assert_eq!(prefix_of(":e fire"), QueryPrefix::Emoji);
        assert_eq!(prefix_of(":emoji"), QueryPrefix::Emoji);
        assert_eq!(prefix_of(":set"), QueryPrefix::Settings);
        assert_eq!(prefix_of(":settings theme"), QueryPrefix::Settings);
        assert_eq!(prefix_of(":h"), QueryPrefix::Help);
        assert_eq!(prefix_of(":version"), QueryPrefix::Version);
        assert_eq!(prefix_of(":km on"), QueryPrefix::Keymap);
        assert_eq!(prefix_of(":k"), QueryPrefix::Keys);
        assert_eq!(prefix_of(":f /tmp report"), QueryPrefix::FolderSearch);
    }

    #[test]
    fn no_false_prefix_matches() {
        // 토큰 경계 규칙: 별칭 뒤에 공백 없이 글자가 이어지면 명령이 아니다
        assert_eq!(prefix_of(":pto"), QueryPrefix::General);
        assert_eq!(prefix_of(":setup"), QueryPrefix::General);
        assert_eq!(prefix_of(":verbose"), QueryPrefix::General);
        assert_eq!(prefix_of(":ex"), QueryPrefix::General);
        assert_eq!(prefix_of(":helpme"), QueryPrefix::General);
        assert_eq!(prefix_of(":keysx"), QueryPrefix::General);
    }

    #[test]
    fn slash_command_aliases() {
        // Slack/Discord 스타일 / 명령도 : 명령과 동일하게 동작
        assert_eq!(prefix_of("/help"), QueryPrefix::Help);
        assert_eq!(prefix_of("/h"), QueryPrefix::Help);
        assert_eq!(prefix_of("/set"), QueryPrefix::Settings);
        assert_eq!(prefix_of("/calc (2+3)*4"), QueryPrefix::Calc);
        assert_eq!(prefix_of("/t spell hello"), QueryPrefix::Transform);
        assert_eq!(prefix_of("/km on"), QueryPrefix::Keymap);
        assert_eq!(
            normalize_slash_command("/emoji fire"),
            Some(":emoji fire".to_string())
        );
        // 닫힌 /pattern/ 형태는 정규식이 우선한다
        assert_eq!(prefix_of("/test\\d+/"), QueryPrefix::General);
        assert_eq!(normalize_slash_command("/e fire/"), None);
        // 명령이 아닌 / 입력(경로, 미지 명령)은 일반 검색
        assert_eq!(prefix_of("/usr"), QueryPrefix::General);
        assert_eq!(prefix_of("/unknown thing"), QueryPrefix::General);
        assert_eq!(prefix_of("/"), QueryPrefix::General);
    }

    #[test]
    fn general_queries() {
        assert_eq!(prefix_of("firefox"), QueryPrefix::General);
        assert_eq!(prefix_of("*.pdf"), QueryPrefix::General);
        assert_eq!(prefix_of("/test\\d+/"), QueryPrefix::General);
        assert_eq!(prefix_of(":"), QueryPrefix::General);
        assert_eq!(prefix_of(""), QueryPrefix::General);
    }

    #[test]
    fn unknown_command_feedback() {
        // 오타/미지원 : 명령 → 안내 항목
        let hint = unknown_command_hint(":clac 2+3", false).expect("힌트가 있어야 함");
        assert!(hint.name.contains(":clac"));
        assert_eq!(hint.keywords, "kmd:unknown_cmd");
        assert!(unknown_command_hint(":pto", false).is_some());
        // 정상 명령, 일반 검색, 미완성 입력에는 힌트 없음
        assert!(unknown_command_hint(":calc 2+3", false).is_none());
        assert!(unknown_command_hint(":e fire", false).is_none());
        assert!(unknown_command_hint("firefox", false).is_none());
        assert!(unknown_command_hint(":", false).is_none());
        assert!(unknown_command_hint("/unknown", false).is_none());
        // 알려진 명령을 입력하는 중이면 힌트를 띄우지 않는다 (:cal → :calc)
        assert!(unknown_command_hint(":cal", false).is_none());
        assert!(unknown_command_hint(":se", false).is_none());
        assert!(unknown_command_hint(":keym", false).is_none());
    }

    #[test]
    fn every_help_entry_has_a_seed() {
        for item in help_items(false) {
            assert!(
                help_query_seed(&item.name).is_some(),
                "도움말 항목에 시드가 없음: {}",
                item.name
            );
        }
    }

    #[test]
    fn registry_covers_all_command_prefixes() {
        // General/Web/Shell을 제외한 모든 프리픽스가 레지스트리에 있어야 한다
        let covered: Vec<QueryPrefix> = COMMANDS.iter().map(|s| s.prefix).collect();
        for p in [
            QueryPrefix::Transform,
            QueryPrefix::Prompt,
            QueryPrefix::Calc,
            QueryPrefix::Emoji,
            QueryPrefix::Settings,
            QueryPrefix::Help,
            QueryPrefix::Version,
            QueryPrefix::Keymap,
            QueryPrefix::Keys,
            QueryPrefix::FolderSearch,
        ] {
            assert!(covered.contains(&p), "레지스트리에 없는 명령: {p:?}");
        }
    }
}
