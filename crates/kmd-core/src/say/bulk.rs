//! 일괄 붙여넣기 — 입버릇 `src/bulk.ts` 와 같은 형식.
//!
//! 한 줄에 `English | 한국어` 또는 탭 구분(엑셀·시트에서 복사). 이 형식으로 내보내면
//! 폰 입버릇의 문장 탭 일괄 붙여넣기에 그대로 들어간다.

use super::Sentence;

/// 줄 단위로 `(영어, 한국어)` 쌍을 읽는다. 둘 중 하나가 비면 그 줄은 건너뛴다.
pub fn parse_bulk(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(['|', '\t']).map(str::trim).collect();
            if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                return None;
            }
            Some((
                parts[0].to_string(),
                parts[1..].join(" ").trim().to_string(),
            ))
        })
        .collect()
}

/// 입버릇 일괄 붙여넣기용 텍스트
pub fn export_bulk(sentences: &[Sentence]) -> String {
    sentences
        .iter()
        .map(|s| format!("{} | {}", s.en, s.ko))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 입버릇 tests/logic.test.ts "일괄 입력 파싱"
    #[test]
    fn parses_pipe_and_tab() {
        let items =
            parse_bulk("I'm on my way. | 가는 중이에요.\n\nbad line\nIt's on me.\t제가 살게요.");
        assert_eq!(
            items,
            [
                ("I'm on my way.".to_string(), "가는 중이에요.".to_string()),
                ("It's on me.".to_string(), "제가 살게요.".to_string()),
            ]
        );
    }

    #[test]
    fn crlf_and_round_trip() {
        let items = parse_bulk("A | 가\r\nB|나\r\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1], ("B".to_string(), "나".to_string()));
        let s: Vec<Sentence> = items
            .iter()
            .map(|(en, ko)| Sentence::new(en, ko, false, 0))
            .collect();
        assert_eq!(parse_bulk(&export_bulk(&s)), items);
    }
}
