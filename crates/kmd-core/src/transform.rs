//! 클립보드 변환 명령 — `:t` 계열
//!
//! `:t spell <text>` → 맞춤법 검사 서비스로 열기
//! `:t trko <text>`  → 한국어 번역 서비스로 열기
//! `:t tren <text>`  → 영어 번역 서비스로 열기
//! `:t tr <text>`    → 자동 번역으로 열기
//!
//! `<text>` 생략 시 클립보드 내용을 자동으로 사용.

use crate::index::{IndexItem, ItemKind, Source};
use crate::web::services::TranslateDirection;

/// 변환 명령의 종류
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformKind {
    Spell,
    Translate(TranslateDirection),
}

/// `:t` 쿼리 파싱 결과
#[derive(Debug, Clone)]
pub struct TransformQuery {
    pub kind: TransformKind,
    /// 사용자 입력 텍스트 (빈 문자열이면 클립보드 사용)
    pub text: String,
}

/// `:t` / `:transform` 쿼리 파싱
pub fn parse_transform_query(input: &str) -> Option<TransformQuery> {
    let sub = input
        .strip_prefix(":transform")
        .or_else(|| input.strip_prefix(":t"))
        .map(|s| s.trim())?;

    // `:t` 만 입력 → 도움말 모드 (None)
    if sub.is_empty() {
        return None;
    }

    let (cmd, rest) = match sub.find(char::is_whitespace) {
        Some(pos) => (&sub[..pos], sub[pos..].trim()),
        None => (sub, ""),
    };

    let kind = match cmd.to_lowercase().as_str() {
        "spell" | "sp" => TransformKind::Spell,
        "trko" | "ko" => TransformKind::Translate(TranslateDirection::EnToKo),
        "tren" | "en" => TransformKind::Translate(TranslateDirection::KoToEn),
        "tr" | "auto" => TransformKind::Translate(TranslateDirection::Auto),
        _ => return None,
    };

    Some(TransformQuery {
        kind,
        text: rest.to_string(),
    })
}

/// `:t` 명령 도움말 항목 생성
pub fn help_items(use_emoji: bool) -> Vec<IndexItem> {
    let entries: &[(&str, &str, &str)] = &[
        (
            ":t spell <text>",
            "맞춤법 검사 (텍스트 생략 시 클립보드 사용)",
            if use_emoji { "\u{270D}\u{FE0F}" } else { "Sp" },
        ),
        (
            ":t tr <text>",
            "번역 — 자동 감지 (텍스트 생략 시 클립보드 사용)",
            if use_emoji { "\u{1F310}" } else { "Tr" },
        ),
        (
            ":t trko <text>",
            "영어 → 한국어 번역",
            if use_emoji {
                "\u{1F1F0}\u{1F1F7}"
            } else {
                "Ko"
            },
        ),
        (
            ":t tren <text>",
            "한국어 → 영어 번역",
            if use_emoji {
                "\u{1F1FA}\u{1F1F8}"
            } else {
                "En"
            },
        ),
    ];

    entries
        .iter()
        .map(|(name, desc, icon)| IndexItem {
            name: name.to_string(),
            path: desc.to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: icon.to_string(),
            keywords: "transform clipboard".to_string(),
            icon_path: None,
        })
        .collect()
}

/// 변환 결과 URL 목록 생성
pub fn build_transform_urls(
    query: &TransformQuery,
    spell_providers: &[String],
    translate_providers: &[String],
) -> Vec<String> {
    match &query.kind {
        TransformKind::Spell => {
            let services = crate::web::selected_spell_services(spell_providers);
            services
                .iter()
                .map(|svc| {
                    svc.url_template
                        .replace("{query}", &crate::web::url_encode(&query.text))
                })
                .collect()
        }
        TransformKind::Translate(dir) => {
            let services = crate::web::selected_translate_services(translate_providers);
            services
                .iter()
                .map(|svc| crate::web::build_translate_url(svc, &query.text, *dir))
                .collect()
        }
    }
}

/// `:t` 실행 항목의 keywords 마커. Enter에서 이 접두를 보고 URL을 연다.
pub const TRANSFORM_RUN_MARKER: &str = "kmd:transform:run";

/// 클립보드가 비었을 때의 안내 항목 마커 (선택해도 아무 일도 없다).
pub const TRANSFORM_EMPTY_MARKER: &str = "kmd:transform:empty";

/// `:t` 질의를 **실행 항목 하나**로 만든다.
///
/// 검색 중에 브라우저를 열면 안 된다 — 타이핑하는 동안 키를 누를 때마다 탭이
/// 열린다(실제 발생한 버그). 다른 모든 프리픽스처럼 "검색은 순수하게, 실행은
/// Enter에서"를 지키기 위해, 검색 단계에서는 이 항목만 보여주고 실제 열기는
/// 호출자가 Enter에서 [`build_transform_urls`]로 수행한다.
///
/// `text`가 비어 있으면(클립보드도 비었으면) 실행 불가 안내 항목을 돌려준다.
pub fn run_item(query: &TransformQuery, service_count: usize, use_emoji: bool) -> IndexItem {
    if query.text.trim().is_empty() {
        return IndexItem {
            name: "클립보드가 비어 있습니다".to_string(),
            path: "텍스트를 복사한 뒤 다시 실행하세요".to_string(),
            kind: ItemKind::SystemCommand,
            source: Source::Plugin,
            icon: if use_emoji { "\u{2139}\u{FE0F}" } else { "[!]" }.to_string(),
            keywords: TRANSFORM_EMPTY_MARKER.to_string(),
            icon_path: None,
        };
    }

    let (label, icon_emoji, icon_ascii) = match &query.kind {
        TransformKind::Spell => ("맞춤법 검사", "\u{270D}\u{FE0F}", "[SPL]"),
        TransformKind::Translate(TranslateDirection::EnToKo) => {
            ("번역 (→ 한국어)", "\u{1F5E3}\u{FE0F}", "[TR]")
        }
        TransformKind::Translate(TranslateDirection::KoToEn) => {
            ("번역 (→ 영어)", "\u{1F5E3}\u{FE0F}", "[TR]")
        }
        TransformKind::Translate(TranslateDirection::Auto) => {
            ("번역 (자동)", "\u{1F5E3}\u{FE0F}", "[TR]")
        }
    };

    // 무엇을 보낼지 눈으로 확인하고 Enter를 누를 수 있게 대상 텍스트를 보여준다.
    let preview: String = query.text.chars().take(40).collect();
    let ellipsis = if query.text.chars().count() > 40 {
        "…"
    } else {
        ""
    };

    IndexItem {
        name: format!("{label} 실행 — {service_count}개 서비스 열기"),
        path: format!("\"{preview}{ellipsis}\""),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: if use_emoji { icon_emoji } else { icon_ascii }.to_string(),
        keywords: TRANSFORM_RUN_MARKER.to_string(),
        icon_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 실행_항목은_열지_않고_선택_가능한_항목만_만든다() {
        let q = parse_transform_query(":t spell 안녕").unwrap();
        let item = run_item(&q, 2, false);
        assert_eq!(item.keywords, TRANSFORM_RUN_MARKER);
        assert!(item.name.contains("맞춤법"), "{}", item.name);
        assert!(item.name.contains('2'), "서비스 개수 표시: {}", item.name);
        assert!(
            item.path.contains("안녕"),
            "대상 텍스트 표시: {}",
            item.path
        );
    }

    #[test]
    fn 텍스트가_없으면_실행_불가_안내() {
        let q = TransformQuery {
            kind: TransformKind::Spell,
            text: String::new(),
        };
        let item = run_item(&q, 2, false);
        assert_eq!(item.keywords, TRANSFORM_EMPTY_MARKER);
    }

    #[test]
    fn 긴_텍스트는_잘라서_보여준다() {
        let q = TransformQuery {
            kind: TransformKind::Translate(TranslateDirection::Auto),
            text: "가".repeat(60),
        };
        let item = run_item(&q, 3, true);
        assert!(item.path.ends_with("…\""), "말줄임: {}", item.path);
    }

    #[test]
    fn test_parse_spell() {
        let q = parse_transform_query(":t spell 안녕하세요").unwrap();
        assert_eq!(q.kind, TransformKind::Spell);
        assert_eq!(q.text, "안녕하세요");
    }

    #[test]
    fn test_parse_translate_auto() {
        let q = parse_transform_query(":t tr hello world").unwrap();
        assert_eq!(q.kind, TransformKind::Translate(TranslateDirection::Auto));
        assert_eq!(q.text, "hello world");
    }

    #[test]
    fn test_parse_translate_ko() {
        let q = parse_transform_query(":t trko hello").unwrap();
        assert_eq!(q.kind, TransformKind::Translate(TranslateDirection::EnToKo));
    }

    #[test]
    fn test_parse_empty_returns_none() {
        assert!(parse_transform_query(":t").is_none());
    }

    #[test]
    fn test_parse_unknown_subcmd() {
        assert!(parse_transform_query(":t foobar test").is_none());
    }

    #[test]
    fn test_parse_clipboard_mode() {
        let q = parse_transform_query(":t spell").unwrap();
        assert_eq!(q.kind, TransformKind::Spell);
        assert_eq!(q.text, ""); // 클립보드 모드
    }

    #[test]
    fn test_build_spell_urls() {
        let q = TransformQuery {
            kind: TransformKind::Spell,
            text: "테스트".to_string(),
        };
        let urls = build_transform_urls(&q, &[], &[]);
        assert!(!urls.is_empty());
        assert!(urls[0].contains("%ED%85%8C%EC%8A%A4%ED%8A%B8"));
    }

    #[test]
    fn test_build_translate_urls() {
        let q = TransformQuery {
            kind: TransformKind::Translate(TranslateDirection::EnToKo),
            text: "hello".to_string(),
        };
        let urls = build_transform_urls(&q, &[], &[]);
        assert!(!urls.is_empty());
        assert!(urls[0].contains("sl=en"));
    }

    #[test]
    fn test_help_items() {
        let items = help_items(false);
        assert_eq!(items.len(), 4);
    }
}
