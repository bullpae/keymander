//! 채점 — 입버릇 `src/scoring.ts` 를 그대로 옮겼다.
//!
//! 폰은 음성 인식 결과를, 컴퓨터는 타이핑한 문장을 넣는다. 같은 함수라서 같은 답이면
//! 두 기기의 점수가 같다.

/// 축약형 풀기 — 입버릇과 같은 표
const CONTRACTIONS: &[(&str, &str)] = &[
    ("i'm", "i am"),
    ("you're", "you are"),
    ("we're", "we are"),
    ("they're", "they are"),
    ("he's", "he is"),
    ("she's", "she is"),
    ("it's", "it is"),
    ("that's", "that is"),
    ("what's", "what is"),
    ("there's", "there is"),
    ("let's", "let us"),
    ("i've", "i have"),
    ("you've", "you have"),
    ("we've", "we have"),
    ("i'll", "i will"),
    ("you'll", "you will"),
    ("we'll", "we will"),
    ("it'll", "it will"),
    ("i'd", "i would"),
    ("you'd", "you would"),
    ("don't", "do not"),
    ("doesn't", "does not"),
    ("didn't", "did not"),
    ("can't", "can not"),
    ("cannot", "can not"),
    ("won't", "will not"),
    ("isn't", "is not"),
    ("aren't", "are not"),
    ("wasn't", "was not"),
    ("weren't", "were not"),
    ("haven't", "have not"),
    ("hasn't", "has not"),
    ("couldn't", "could not"),
    ("wouldn't", "would not"),
    ("shouldn't", "should not"),
    ("gonna", "going to"),
    ("wanna", "want to"),
    ("gotta", "got to"),
];

/// 비교용 단어 목록: 소문자, 문장부호 제거, 축약형 풀기
pub fn normalize_words(text: &str) -> Vec<String> {
    let cleaned: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '`' => '\'',
            c if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '\'' || c.is_whitespace() => {
                c
            }
            _ => ' ',
        })
        .collect();
    let mut out = Vec::new();
    for w in cleaned.split_whitespace() {
        match CONTRACTIONS.iter().find(|(k, _)| *k == w) {
            Some((_, full)) => out.extend(full.split(' ').map(String::from)),
            None => out.push(w.replace('\'', "")),
        }
    }
    out
}

/// 원문 단어 하나의 맞음/틀림 (화면 표시용)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordMark {
    pub word: String,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoreResult {
    /// 0~1: 목표 문장 단어 중 순서대로 맞게 친 비율 (군더더기 단어는 감점)
    pub score: f64,
    pub marks: Vec<WordMark>,
}

fn lcs_matches(target: &[String], said: &[String]) -> Vec<bool> {
    let (n, m) = (target.len(), said.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if target[i] == said[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut matched = vec![false; n];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if target[i] == said[j] {
            matched[i] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matched
}

pub fn score_utterance(target_text: &str, said_text: &str) -> ScoreResult {
    let target = normalize_words(target_text);
    let said = normalize_words(said_text);
    if target.is_empty() {
        return ScoreResult {
            score: 0.0,
            marks: Vec::new(),
        };
    }
    let matched = lcs_matches(&target, &said);
    let hits = matched.iter().filter(|m| **m).count();
    let extra = said.len().saturating_sub(hits);
    // 목표 단어 재현율 기준, 쓸데없이 덧붙인 단어는 절반 가중치로 감점
    let score = ((hits as f64 - extra as f64 * 0.5) / target.len() as f64).clamp(0.0, 1.0);

    // 표시용: 원문 토큰(문장부호 포함) 단위로 표시
    let mut marks = Vec::new();
    let mut k = 0;
    for tok in target_text.split_whitespace() {
        let parts = normalize_words(tok).len();
        let ok = parts == 0 || (k..k + parts).all(|idx| matched.get(idx).copied().unwrap_or(false));
        k += parts;
        marks.push(WordMark {
            word: tok.to_string(),
            ok,
        });
    }
    ScoreResult { score, marks }
}

/// 빠진 단어를 `[단어]` 로 감싼 한 줄 — 런처 결과 행은 색을 못 입히므로 괄호로 표시한다
pub fn marked_line(marks: &[WordMark]) -> String {
    marks
        .iter()
        .map(|m| {
            if m.ok {
                m.word.clone()
            } else {
                format!("[{}]", m.word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    // 아래는 입버릇 tests/logic.test.ts 와 같은 기대값이다.

    #[test]
    fn ignores_contractions_and_punctuation() {
        assert_eq!(
            normalize_words("I'm running late!"),
            ["i", "am", "running", "late"]
        );
        assert_eq!(
            score_utterance("I'm running a little late.", "i am running a little late").score,
            1.0
        );
        assert_eq!(
            score_utterance(
                "Could you say that again, please?",
                "could you say that again please"
            )
            .score,
            1.0
        );
    }

    #[test]
    fn missing_word_is_marked() {
        let r = score_utterance("Could you give me a hand?", "could you give me hand");
        assert!(r.score < 1.0 && r.score >= 0.8, "score {}", r.score);
        let oks: Vec<bool> = r.marks.iter().map(|m| m.ok).collect();
        assert_eq!(oks, [true, true, true, true, false, true]);
        assert_eq!(marked_line(&r.marks), "Could you give me [a] hand?");
    }

    #[test]
    fn unrelated_is_low() {
        assert!(score_utterance("Thanks for having me.", "what time is it now").score < 0.3);
        assert_eq!(score_utterance("Thanks for having me.", "").score, 0.0);
    }

    #[test]
    fn curly_apostrophe_and_extra_words() {
        assert_eq!(
            score_utterance("It's up to you.", "it\u{2019}s up to you").score,
            1.0
        );
        // 목표 5단어(it is up to you) 다 맞고 군더더기 2단어 → (5 - 1) / 5
        let r = score_utterance("It's up to you.", "well it's up to you then");
        assert!((r.score - 0.8).abs() < 1e-9, "score {}", r.score);
    }
}
