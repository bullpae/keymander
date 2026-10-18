//! 프롬프트 템플릿 관리 — 저장/조회/적용

use crate::config::PromptTemplate;
use crate::index::{IndexItem, ItemKind, Source};

/// 저장된 템플릿 목록을 IndexItem으로 변환 (`:prompt` 검색용)
pub fn list_templates_as_items(
    templates: &[PromptTemplate],
    filter: &str,
    use_emoji: bool,
) -> Vec<IndexItem> {
    let filter_lower = filter.to_lowercase();
    let mut items: Vec<IndexItem> = templates
        .iter()
        .filter(|t| {
            filter_lower.is_empty()
                || t.name.to_lowercase().contains(&filter_lower)
                || t.body.to_lowercase().contains(&filter_lower)
        })
        .map(|t| IndexItem {
            name: format!(":{:<16} {}", t.name, preview_body(&t.body, 60)),
            path: format!("@ll :{} <query> 형태로 사용", t.name),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: if use_emoji { "\u{1F4DD}" } else { "Pt" }.to_string(),
            keywords: format!("prompt template {}", t.name),
            icon_path: None,
        })
        .collect();

    // 빈 목록이면 안내 항목 추가
    if items.is_empty() && filter_lower.is_empty() {
        items.push(IndexItem {
            name: "저장된 템플릿 없음".to_string(),
            path: ":prompt add <name> <body>  또는  kmd prompt add <name> <body>".to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: if use_emoji { "\u{2139}\u{FE0F}" } else { "[i]" }.to_string(),
            keywords: "prompt template empty".to_string(),
            icon_path: None,
        });
    }

    // :prompt add 힌트
    if filter_lower.is_empty() || "add".contains(&filter_lower) {
        items.push(IndexItem {
            name: ":prompt add <name> <body>".to_string(),
            path: "새 프롬프트 템플릿 추가 (예: :prompt add review 코드를 리뷰해주세요)"
                .to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: if use_emoji { "\u{2795}" } else { "[+]" }.to_string(),
            keywords: PROMPT_ADD_HINT_MARKER.to_string(),
            icon_path: None,
        });
    }

    items
}

/// `@ll :template_name actual question` 에서 템플릿 적용 결과 반환.
/// 템플릿이 없으면 원본 쿼리를 그대로 반환.
pub fn apply_template(templates: &[PromptTemplate], query: &str) -> String {
    let trimmed = query.trim();
    if !trimmed.starts_with(':') {
        return query.to_string();
    }

    // `:name rest...` 파싱
    let (name_part, rest) = match trimmed.find(char::is_whitespace) {
        Some(pos) => (&trimmed[1..pos], trimmed[pos..].trim()),
        None => (&trimmed[1..], ""),
    };

    let Some(template) = templates
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name_part))
    else {
        return query.to_string();
    };

    if template.body.contains("{query}") {
        template.body.replace("{query}", rest)
    } else if rest.is_empty() {
        template.body.clone()
    } else {
        format!("{}\n\n{}", template.body, rest)
    }
}

/// 긴 텍스트를 지정 길이(바이트)로 잘라 미리보기 문자열 생성.
///
/// 문자 경계에서 자른다 — 예전엔 `&text[..n]` 바이트 슬라이스라 한글(3바이트)과
/// ASCII가 섞인 본문이면 경계 중간을 잘라 `:prompt` 목록에서 패닉했다.
pub fn preview_body(text: &str, max_len: usize) -> String {
    if text.len() > max_len {
        let mut cut = max_len.saturating_sub(3);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...", &text[..cut])
    } else {
        text.to_string()
    }
}

/// `:prompt` 입력 중 미리보기 항목의 Enter 마커 — 확정은 여기서만 일어난다.
pub const PROMPT_COMMIT_MARKER: &str = "kmd:prompt:commit";
/// `:prompt add` 힌트 항목 마커 — Enter 시 `:prompt add `로 입력을 채운다.
pub const PROMPT_ADD_HINT_MARKER: &str = "kmd:prompt:add";

/// `:prompt`(또는 `:pt`) 하위 명령 해석 결과.
///
/// 예전엔 두 UI가 검색 경로(키 입력마다)에서 바로 add/rm을 **실행**해, 타이핑하는
/// 글자마다 config가 저장되고 `rm ab`를 치는 도중 `a` 템플릿이 먼저 지워졌다.
/// 이제 검색은 [`command_items`]로 미리보기만, Enter에서 [`apply_command`]로 확정한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptCommand {
    /// `:prompt [list] [필터]`
    List(String),
    /// `:prompt add <name> <body>`
    Add { name: String, body: String },
    /// `:prompt add` 뒤에 이름/본문이 아직 없음
    AddUsage,
    /// `:prompt remove|rm|del <name>`
    Remove(String),
    /// `:prompt remove` 뒤에 이름이 아직 없음
    RemoveUsage,
}

/// 전체 질의(`:prompt ...`)를 하위 명령으로 해석한다.
pub fn parse_command(query: &str) -> PromptCommand {
    let sub = query
        .strip_prefix(":prompt")
        .or_else(|| query.strip_prefix(":pt"))
        .unwrap_or("")
        .trim();

    if let Some(rest) = sub.strip_prefix("add ") {
        let rest = rest.trim();
        return match rest.find(char::is_whitespace) {
            Some(pos) => PromptCommand::Add {
                name: rest[..pos].to_string(),
                body: rest[pos..].trim().to_string(),
            },
            None => PromptCommand::AddUsage,
        };
    }
    if sub == "add" {
        return PromptCommand::AddUsage;
    }
    for verb in ["remove", "rm", "del"] {
        if let Some(rest) = sub.strip_prefix(verb) {
            if rest.is_empty() {
                return PromptCommand::RemoveUsage;
            }
            if rest.starts_with(' ') {
                let name = rest.trim();
                return if name.is_empty() {
                    PromptCommand::RemoveUsage
                } else {
                    PromptCommand::Remove(name.to_string())
                };
            }
        }
    }
    PromptCommand::List(sub.strip_prefix("list").unwrap_or(sub).trim().to_string())
}

/// 실행 전 검증 — 실패 사유(제목, 설명)
fn validate(cmd: &PromptCommand, templates: &[PromptTemplate]) -> Result<(), (String, String)> {
    match cmd {
        PromptCommand::Add { name, body } => {
            if !validate_template_name(name) {
                Err((
                    format!("❌ 잘못된 이름: '{name}'"),
                    "영문/숫자/하이픈/언더스코어만, 최대 32자".to_string(),
                ))
            } else if body.is_empty() {
                Err((
                    "❌ 본문이 비어 있습니다".to_string(),
                    ":prompt add <name> <body> 형태로 입력하세요".to_string(),
                ))
            } else {
                Ok(())
            }
        }
        PromptCommand::Remove(name) => {
            if templates.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
                Ok(())
            } else {
                Err((
                    format!("❌ 템플릿 '{name}'을 찾을 수 없습니다"),
                    String::new(),
                ))
            }
        }
        _ => Ok(()),
    }
}

fn notice(name: String, path: String, icon: &str, keywords: &str) -> IndexItem {
    IndexItem {
        name,
        path,
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: icon.to_string(),
        keywords: keywords.to_string(),
        icon_path: None,
    }
}

/// 검색(키 입력) 경로용 결과 항목 — **부수효과 없음**.
///
/// add/remove는 "Enter로 확정" 미리보기 항목([`PROMPT_COMMIT_MARKER`])을, 잘못된
/// 입력은 안내 항목(noop)을, 목록은 [`list_templates_as_items`]를 돌려준다.
pub fn command_items(
    cmd: &PromptCommand,
    templates: &[PromptTemplate],
    use_emoji: bool,
) -> Vec<IndexItem> {
    let (err_icon, info_icon, ok_icon) = if use_emoji {
        ("\u{274C}", "\u{2139}\u{FE0F}", "\u{21B5}")
    } else {
        ("[!]", "[?]", "[>]")
    };
    if let Err((title, detail)) = validate(cmd, templates) {
        return vec![notice(title, detail, err_icon, "kmd:settings:noop")];
    }
    match cmd {
        PromptCommand::List(filter) => list_templates_as_items(templates, filter, use_emoji),
        PromptCommand::AddUsage => vec![notice(
            "사용법: :prompt add <name> <body>".to_string(),
            "예: :prompt add review 코드를 리뷰해주세요".to_string(),
            info_icon,
            "kmd:settings:noop",
        )],
        PromptCommand::RemoveUsage => vec![notice(
            "사용법: :prompt remove <name>".to_string(),
            "삭제할 템플릿 이름을 입력하세요".to_string(),
            info_icon,
            "kmd:settings:noop",
        )],
        PromptCommand::Add { name, body } => {
            let exists = templates.iter().any(|t| t.name.eq_ignore_ascii_case(name));
            let verb = if exists { "덮어쓰기" } else { "저장" };
            vec![notice(
                format!("Enter: 템플릿 '{name}' {verb}"),
                preview_body(body, 80),
                ok_icon,
                PROMPT_COMMIT_MARKER,
            )]
        }
        PromptCommand::Remove(name) => vec![notice(
            format!("Enter: 템플릿 '{name}' 삭제"),
            String::new(),
            ok_icon,
            PROMPT_COMMIT_MARKER,
        )],
    }
}

/// Enter 확정 — 템플릿 목록에 명령을 적용하고 성공 문구를 돌려준다.
/// 저장은 호출자 몫이다(UI마다 저장 경로가 다름). 목록 명령이면 Err.
pub fn apply_command(
    cmd: &PromptCommand,
    templates: &mut Vec<PromptTemplate>,
) -> Result<String, String> {
    validate(cmd, templates).map_err(|(title, _)| title)?;
    match cmd {
        PromptCommand::Add { name, body } => {
            templates.retain(|t| !t.name.eq_ignore_ascii_case(name));
            templates.push(PromptTemplate {
                name: name.clone(),
                body: body.clone(),
            });
            Ok(format!(
                "✅ 템플릿 '{name}' 저장됨 — @ll :{name} <query> 형태로 사용"
            ))
        }
        PromptCommand::Remove(name) => {
            templates.retain(|t| !t.name.eq_ignore_ascii_case(name));
            Ok(format!("✅ 템플릿 '{name}' 삭제됨"))
        }
        _ => Err("확정할 명령이 아닙니다".to_string()),
    }
}

/// 템플릿 이름 유효성 검사 (영문/숫자/하이픈/언더스코어)
pub fn validate_template_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 미리보기는_한글_혼합_본문을_문자_경계에서_자른다() {
        let body = "a다음 코드를 리뷰해주세요 그리고 개선점을 알려주세요 부탁합니다";
        for max in 1..body.len() {
            let _ = preview_body(body, max); // 패닉하면 실패
        }
    }

    #[test]
    fn 하위_명령_파싱() {
        assert_eq!(parse_command(":prompt"), PromptCommand::List(String::new()));
        assert_eq!(
            parse_command(":pt list rev"),
            PromptCommand::List("rev".into())
        );
        assert_eq!(parse_command(":prompt add"), PromptCommand::AddUsage);
        assert_eq!(parse_command(":prompt add review"), PromptCommand::AddUsage);
        assert_eq!(
            parse_command(":prompt add review 코드 리뷰 {query}"),
            PromptCommand::Add {
                name: "review".into(),
                body: "코드 리뷰 {query}".into()
            }
        );
        assert_eq!(parse_command(":prompt rm"), PromptCommand::RemoveUsage);
        assert_eq!(
            parse_command(":prompt rm ab"),
            PromptCommand::Remove("ab".into())
        );
        // "rmx"는 삭제 명령이 아니라 목록 필터
        assert_eq!(
            parse_command(":prompt rmx"),
            PromptCommand::List("rmx".into())
        );
    }

    #[test]
    fn 입력_중에는_템플릿이_바뀌지_않는다() {
        // `rm ab`를 치는 도중의 모든 접두사에서 command_items는 부수효과가 없다
        let templates = vec![
            PromptTemplate {
                name: "a".into(),
                body: "A".into(),
            },
            PromptTemplate {
                name: "ab".into(),
                body: "AB".into(),
            },
        ];
        let before = templates.clone();
        for q in [":prompt r", ":prompt rm", ":prompt rm a", ":prompt rm ab"] {
            let _ = command_items(&parse_command(q), &templates, false);
        }
        assert_eq!(templates, before);
        let items = command_items(&parse_command(":prompt rm a"), &templates, false);
        assert_eq!(items[0].keywords, PROMPT_COMMIT_MARKER, "확정은 Enter에서");
    }

    #[test]
    fn 확정은_추가_덮어쓰기_삭제를_적용한다() {
        let mut t = Vec::new();
        let add = parse_command(":prompt add Rev 본문1");
        assert!(apply_command(&add, &mut t).is_ok());
        let over = parse_command(":prompt add rev 본문2");
        assert!(apply_command(&over, &mut t).is_ok());
        assert_eq!(t.len(), 1, "대소문자 무시 덮어쓰기");
        assert_eq!(t[0].body, "본문2");
        assert!(apply_command(&parse_command(":prompt rm nope"), &mut t).is_err());
        assert!(apply_command(&parse_command(":prompt add bad!name x"), &mut t).is_err());
        assert!(apply_command(&parse_command(":prompt rm REV"), &mut t).is_ok());
        assert!(t.is_empty());
    }

    fn sample_templates() -> Vec<PromptTemplate> {
        vec![
            PromptTemplate {
                name: "review".to_string(),
                body: "다음 코드를 리뷰해주세요:\n{query}".to_string(),
            },
            PromptTemplate {
                name: "translate".to_string(),
                body: "한국어로 번역해주세요".to_string(),
            },
        ]
    }

    #[test]
    fn test_apply_template_with_placeholder() {
        let templates = sample_templates();
        let result = apply_template(&templates, ":review fn main() {}");
        assert_eq!(result, "다음 코드를 리뷰해주세요:\nfn main() {}");
    }

    #[test]
    fn test_apply_template_without_placeholder() {
        let templates = sample_templates();
        let result = apply_template(&templates, ":translate hello world");
        assert_eq!(result, "한국어로 번역해주세요\n\nhello world");
    }

    #[test]
    fn test_apply_template_not_found() {
        let templates = sample_templates();
        let result = apply_template(&templates, ":unknown test");
        assert_eq!(result, ":unknown test");
    }

    #[test]
    fn test_apply_template_no_prefix() {
        let templates = sample_templates();
        let result = apply_template(&templates, "plain query");
        assert_eq!(result, "plain query");
    }

    #[test]
    fn test_validate_template_name() {
        assert!(validate_template_name("review"));
        assert!(validate_template_name("code-review"));
        assert!(validate_template_name("my_template_1"));
        assert!(!validate_template_name(""));
        assert!(!validate_template_name("with spaces"));
        assert!(!validate_template_name("한글이름"));
    }

    #[test]
    fn test_list_templates_empty() {
        let items = list_templates_as_items(&[], "", false);
        assert!(items.iter().any(|i| i.name.contains("저장된 템플릿 없음")));
    }

    #[test]
    fn test_list_templates_with_data() {
        let templates = sample_templates();
        let items = list_templates_as_items(&templates, "", false);
        assert!(items.iter().any(|i| i.name.contains("review")));
        assert!(items.iter().any(|i| i.name.contains("translate")));
    }
}
