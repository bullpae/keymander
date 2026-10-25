//! 두벌식 — 한/영 상태를 잘못 둔 채 친 글자를 되짚는다 (파란띠 피드백).
//!
//! 영문 상태로 `fh` 를 쳤다면 한글 상태였을 때 `로` 였다. 거꾸로 한글 상태로
//! `ㅖㅇㄹ` 이 나왔다면 영문 상태의 `Pdf` 다. 판정이 아니라 설명용이라 IME 의
//! 세부 동작(조합 중 백스페이스 등)까지 흉내 내지 않는다.

const CHO: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];
const JUNG: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];
/// 0 = 받침 없음
const JONG: [char; 28] = [
    '\0', 'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ',
    'ㅁ', 'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// 겹모음·겹받침 = (앞, 뒤, 합)
const VOWEL_PAIRS: [(char, char, char); 7] = [
    ('ㅗ', 'ㅏ', 'ㅘ'),
    ('ㅗ', 'ㅐ', 'ㅙ'),
    ('ㅗ', 'ㅣ', 'ㅚ'),
    ('ㅜ', 'ㅓ', 'ㅝ'),
    ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'),
    ('ㅡ', 'ㅣ', 'ㅢ'),
];
const FINAL_PAIRS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];

/// 영문 자판 글자 → 두벌식 자모. Shift 로 달라지는 건 쌍자음과 ㅒ·ㅖ 뿐이다.
fn jamo_of(c: char) -> Option<char> {
    Some(match c {
        'Q' => 'ㅃ',
        'W' => 'ㅉ',
        'E' => 'ㄸ',
        'R' => 'ㄲ',
        'T' => 'ㅆ',
        'O' => 'ㅒ',
        'P' => 'ㅖ',
        _ => {
            let i = "qwertyuiopasdfghjklzxcvbnm".find(c.to_ascii_lowercase())?;
            "ㅂㅈㄷㄱㅅㅛㅕㅑㅐㅔㅁㄴㅇㄹㅎㅗㅓㅏㅣㅋㅌㅊㅍㅠㅜㅡ"
                .chars()
                .nth(i)?
        }
    })
}

fn key_of(j: char) -> Option<char> {
    if let Some(i) = "ㅃㅉㄸㄲㅆㅒㅖ".chars().position(|x| x == j) {
        return "QWERTOP".chars().nth(i);
    }
    let i = "ㅂㅈㄷㄱㅅㅛㅕㅑㅐㅔㅁㄴㅇㄹㅎㅗㅓㅏㅣㅋㅌㅊㅍㅠㅜㅡ"
        .chars()
        .position(|x| x == j)?;
    "qwertyuiopasdfghjklzxcvbnm".chars().nth(i)
}

fn is_vowel(j: char) -> bool {
    JUNG.contains(&j)
}

pub fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{3131}'..='\u{318E}')
}

fn pair(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table.iter().find(|p| p.0 == a && p.1 == b).map(|p| p.2)
}

fn split(table: &[(char, char, char)], c: char) -> Option<(char, char)> {
    table.iter().find(|p| p.2 == c).map(|p| (p.0, p.1))
}

#[derive(Default)]
struct Syllable {
    cho: Option<char>,
    jung: Option<char>,
    jong: Option<char>,
}

impl Syllable {
    fn flush(&mut self, out: &mut String) {
        match (self.cho, self.jung) {
            (Some(c), Some(v)) => {
                let ci = CHO.iter().position(|x| *x == c).unwrap_or(0) as u32;
                let vi = JUNG.iter().position(|x| *x == v).unwrap_or(0) as u32;
                let ji = self
                    .jong
                    .and_then(|j| JONG.iter().position(|x| *x == j))
                    .unwrap_or(0) as u32;
                out.extend(char::from_u32(0xAC00 + (ci * 21 + vi) * 28 + ji));
            }
            _ => out.extend([self.cho, self.jung, self.jong].into_iter().flatten()),
        }
        *self = Self::default();
    }
}

/// 영문 상태로 친 글자를 한글 상태였다면 나왔을 글자로 — `fh` → `로`
pub fn to_hangul(keys: &str) -> String {
    let mut out = String::new();
    let mut s = Syllable::default();
    for c in keys.chars() {
        let Some(j) = jamo_of(c) else {
            s.flush(&mut out);
            out.push(c);
            continue;
        };
        if is_vowel(j) {
            if let Some(jong) = s.jong.take() {
                // 받침이 다음 글자의 초성으로 넘어간다 (겹받침이면 뒤쪽만)
                let (keep, moved) = match split(&FINAL_PAIRS, jong) {
                    Some((a, b)) => (Some(a), b),
                    None => (None, jong),
                };
                s.jong = keep;
                s.flush(&mut out);
                s.cho = Some(moved);
                s.jung = Some(j);
            } else if let Some(v) = s.jung {
                match pair(&VOWEL_PAIRS, v, j) {
                    Some(m) => s.jung = Some(m),
                    None => {
                        s.flush(&mut out);
                        s.jung = Some(j);
                    }
                }
            } else {
                s.jung = Some(j);
            }
        } else {
            match (s.cho, s.jung, s.jong) {
                (Some(_), Some(_), None) if JONG.contains(&j) => s.jong = Some(j),
                (Some(_), Some(_), Some(prev)) => match pair(&FINAL_PAIRS, prev, j) {
                    Some(m) => s.jong = Some(m),
                    None => {
                        s.flush(&mut out);
                        s.cho = Some(j);
                    }
                },
                _ => {
                    s.flush(&mut out);
                    s.cho = Some(j);
                }
            }
        }
    }
    s.flush(&mut out);
    out
}

/// 한글 상태로 친 글자를 영문 자판 글자로 — `ㅖㅇㄹ` → `Pdf`, `로` → `fh`
pub fn to_keys(text: &str) -> String {
    let mut out = String::new();
    let push_jamo = |j: char, out: &mut String| {
        let parts = split(&VOWEL_PAIRS, j)
            .or_else(|| split(&FINAL_PAIRS, j))
            .map_or(vec![j], |(a, b)| vec![a, b]);
        for p in parts {
            out.push(key_of(p).unwrap_or(p));
        }
    };
    for c in text.chars() {
        if let '\u{AC00}'..='\u{D7A3}' = c {
            let n = c as u32 - 0xAC00;
            let (ci, vi, ji) = (n / (21 * 28), (n / 28) % 21, n % 28);
            push_jamo(CHO[ci as usize], &mut out);
            push_jamo(JUNG[vi as usize], &mut out);
            if ji > 0 {
                push_jamo(JONG[ji as usize], &mut out);
            }
        } else if is_hangul(c) {
            push_jamo(c, &mut out);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 영문으로_친_자판을_한글로_되짚는다() {
        assert_eq!(to_hangul("fh"), "로");
        assert_eq!(to_hangul("dkssudgktpdy"), "안녕하세요");
        assert_eq!(to_hangul("ckwsmsek"), "찾는다");
        assert_eq!(to_hangul("rhkswjd"), "관정");
        assert_eq!(to_hangul("dlfr wkd"), "읽 장");
        assert_eq!(to_hangul("dlfrdj"), "읽어");
        assert_eq!(to_hangul("Tkd"), "쌍");
    }

    #[test]
    fn 한글로_친_글자를_자판으로_되짚는다() {
        assert_eq!(to_keys("ㅖㅇㄹ"), "Pdf");
        assert_eq!(to_keys("로"), "fh");
        assert_eq!(to_keys("읽어"), "dlfrdj");
        assert_eq!(to_keys("kmd"), "kmd");
    }

    #[test]
    fn 왕복() {
        for w in ["한영", "전환", "키맨더", "홈로우", "괜찮다", "없앤다"] {
            assert_eq!(to_hangul(&to_keys(w)), w);
        }
    }
}
