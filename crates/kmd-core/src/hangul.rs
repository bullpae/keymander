//! Korean Hangul 2-벌식 (2-set) composition engine
//!
//! Implements full Korean syllable composition and decomposition for terminal
//! environments where the system IME doesn't work properly in raw mode.
//!
//! # 2-벌식 Layout (Standard Korean Keyboard)
//!
//! ```text
//! q=ㅂ  w=ㅈ  e=ㄷ  r=ㄱ  t=ㅅ  y=ㅛ  u=ㅕ  i=ㅑ  o=ㅐ  p=ㅔ
//! a=ㅁ  s=ㄴ  d=ㅇ  f=ㄹ  g=ㅎ  h=ㅗ  j=ㅓ  k=ㅏ  l=ㅣ
//! z=ㅋ  x=ㅌ  c=ㅊ  v=ㅍ  b=ㅠ  n=ㅜ  m=ㅡ
//!
//! Shift: Q=ㅃ  W=ㅉ  E=ㄸ  R=ㄲ  T=ㅆ  O=ㅒ  P=ㅖ
//! ```

// ============================================================================
// Public Types
// ============================================================================

/// A Korean jamo (자모) — either a consonant or vowel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jamo {
    /// 초성/종성 consonant, identified by 초성 index (0-18)
    Consonant(u32),
    /// 중성 vowel, identified by 중성 index (0-20)
    Vowel(u32),
}

/// Result of a composition step
#[derive(Debug, Clone)]
pub struct ComposeResult {
    /// Character to commit to the text buffer (from completing a syllable)
    pub committed: Option<char>,
    /// Currently composing character for display
    pub composing: Option<char>,
}

/// Korean Hangul composition state machine
pub struct HangulComposer {
    state: State,
}

// ============================================================================
// Internal State
// ============================================================================

#[derive(Debug, Clone, Copy)]
enum State {
    /// No composition in progress
    Empty,
    /// Only 초성 (initial consonant) entered
    Choseong { cho: u32 },
    /// Standalone 중성 (vowel without preceding consonant)
    Jungseong { jung: u32 },
    /// 초성 + 중성
    ChoseongJungseong { cho: u32, jung: u32 },
    /// 초성 + 중성 + 종성 (complete syllable with final consonant)
    Full { cho: u32, jung: u32, jong: u32 },
}

// ============================================================================
// Constants — Jamo Tables
// ============================================================================

/// 초성 (initial consonant) compatibility jamo characters for standalone display
const CHO_CHARS: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];

/// 중성 (medial vowel) compatibility jamo characters for standalone display
const JUNG_CHARS: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];

/// 초성 index → 종성 index mapping (None if this consonant can't be 종성)
/// ㄸ(4), ㅃ(8), ㅉ(13) cannot appear as 종성
const CHO_TO_JONG: [Option<u32>; 19] = [
    Some(1),  // 0  ㄱ → jong 1
    Some(2),  // 1  ㄲ → jong 2
    Some(4),  // 2  ㄴ → jong 4
    Some(7),  // 3  ㄷ → jong 7
    None,     // 4  ㄸ — (cannot be 종성)
    Some(8),  // 5  ㄹ → jong 8
    Some(16), // 6  ㅁ → jong 16
    Some(17), // 7  ㅂ → jong 17
    None,     // 8  ㅃ — (cannot be 종성)
    Some(19), // 9  ㅅ → jong 19
    Some(20), // 10 ㅆ → jong 20
    Some(21), // 11 ㅇ → jong 21
    Some(22), // 12 ㅈ → jong 22
    None,     // 13 ㅉ — (cannot be 종성)
    Some(23), // 14 ㅊ → jong 23
    Some(24), // 15 ㅋ → jong 24
    Some(25), // 16 ㅌ → jong 25
    Some(26), // 17 ㅍ → jong 26
    Some(27), // 18 ㅎ → jong 27
];

/// 종성 index → 초성 index mapping (for moving 종성 to new 초성)
fn jong_to_cho(jong: u32) -> Option<u32> {
    match jong {
        1 => Some(0),   // ㄱ
        2 => Some(1),   // ㄲ
        4 => Some(2),   // ㄴ
        7 => Some(3),   // ㄷ
        8 => Some(5),   // ㄹ
        16 => Some(6),  // ㅁ
        17 => Some(7),  // ㅂ
        19 => Some(9),  // ㅅ
        20 => Some(10), // ㅆ
        21 => Some(11), // ㅇ
        22 => Some(12), // ㅈ
        23 => Some(14), // ㅊ
        24 => Some(15), // ㅋ
        25 => Some(16), // ㅌ
        26 => Some(17), // ㅍ
        27 => Some(18), // ㅎ
        _ => None,      // compound 종성 — use decompose_jong instead
    }
}

// ============================================================================
// Compound Jamo Tables
// ============================================================================

/// Try to combine two 종성 into a compound 종성
fn compound_jong(first: u32, second: u32) -> Option<u32> {
    match (first, second) {
        (1, 19) => Some(3),   // ㄱ+ㅅ = ㄳ
        (4, 22) => Some(5),   // ㄴ+ㅈ = ㄵ
        (4, 27) => Some(6),   // ㄴ+ㅎ = ㄶ
        (8, 1) => Some(9),    // ㄹ+ㄱ = ㄺ
        (8, 16) => Some(10),  // ㄹ+ㅁ = ㄻ
        (8, 17) => Some(11),  // ㄹ+ㅂ = ㄼ
        (8, 19) => Some(12),  // ㄹ+ㅅ = ㄽ
        (8, 25) => Some(13),  // ㄹ+ㅌ = ㄾ
        (8, 26) => Some(14),  // ㄹ+ㅍ = ㄿ
        (8, 27) => Some(15),  // ㄹ+ㅎ = ㅀ
        (17, 19) => Some(18), // ㅂ+ㅅ = ㅄ
        _ => None,
    }
}

/// Decompose a compound 종성 into (remaining 종성, detached 종성)
/// The detached part becomes a new 초성 via jong_to_cho
fn decompose_jong(compound: u32) -> Option<(u32, u32)> {
    match compound {
        3 => Some((1, 19)),   // ㄳ → ㄱ + ㅅ
        5 => Some((4, 22)),   // ㄵ → ㄴ + ㅈ
        6 => Some((4, 27)),   // ㄶ → ㄴ + ㅎ
        9 => Some((8, 1)),    // ㄺ → ㄹ + ㄱ
        10 => Some((8, 16)),  // ㄻ → ㄹ + ㅁ
        11 => Some((8, 17)),  // ㄼ → ㄹ + ㅂ
        12 => Some((8, 19)),  // ㄽ → ㄹ + ㅅ
        13 => Some((8, 25)),  // ㄾ → ㄹ + ㅌ
        14 => Some((8, 26)),  // ㄿ → ㄹ + ㅍ
        15 => Some((8, 27)),  // ㅀ → ㄹ + ㅎ
        18 => Some((17, 19)), // ㅄ → ㅂ + ㅅ
        _ => None,            // not a compound
    }
}

/// Try to combine two 중성 into a compound 중성
fn compound_jung(first: u32, second: u32) -> Option<u32> {
    match (first, second) {
        (8, 0) => Some(9),    // ㅗ+ㅏ = ㅘ
        (8, 1) => Some(10),   // ㅗ+ㅐ = ㅙ
        (8, 20) => Some(11),  // ㅗ+ㅣ = ㅚ
        (13, 4) => Some(14),  // ㅜ+ㅓ = ㅝ
        (13, 5) => Some(15),  // ㅜ+ㅔ = ㅞ
        (13, 20) => Some(16), // ㅜ+ㅣ = ㅟ
        (18, 20) => Some(19), // ㅡ+ㅣ = ㅢ
        _ => None,
    }
}

/// Decompose a compound 중성 into (first, second) vowel indices
fn decompose_jung(compound: u32) -> Option<(u32, u32)> {
    match compound {
        9 => Some((8, 0)),    // ㅘ → ㅗ + ㅏ
        10 => Some((8, 1)),   // ㅙ → ㅗ + ㅐ
        11 => Some((8, 20)),  // ㅚ → ㅗ + ㅣ
        14 => Some((13, 4)),  // ㅝ → ㅜ + ㅓ
        15 => Some((13, 5)),  // ㅞ → ㅜ + ㅔ
        16 => Some((13, 20)), // ㅟ → ㅜ + ㅣ
        19 => Some((18, 20)), // ㅢ → ㅡ + ㅣ
        _ => None,
    }
}

// ============================================================================
// Character Composition Helpers
// ============================================================================

/// Compose a Korean syllable block from 초성, 중성, 종성 indices
/// Formula: 0xAC00 + (cho * 21 + jung) * 28 + jong
fn compose_syllable(cho: u32, jung: u32, jong: u32) -> char {
    let code = 0xAC00 + (cho * 21 + jung) * 28 + jong;
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

/// Get the compatibility jamo character for a standalone 초성
fn cho_to_char(cho: u32) -> char {
    CHO_CHARS.get(cho as usize).copied().unwrap_or('\u{FFFD}')
}

/// Get the compatibility jamo character for a standalone 중성
fn jung_to_char(jung: u32) -> char {
    JUNG_CHARS.get(jung as usize).copied().unwrap_or('\u{FFFD}')
}

// ============================================================================
// Key-to-Jamo Mapping
// ============================================================================

/// Map a QWERTY key to a Korean jamo (2-벌식 standard layout)
///
/// Returns `None` if the key doesn't correspond to any Korean jamo.
/// For shifted keys that don't produce a different jamo (e.g. Shift+K),
/// the lowercase mapping is used.
pub fn key_to_jamo(c: char) -> Option<Jamo> {
    match c {
        // ── Consonants (초성) ──
        'r' => Some(Jamo::Consonant(0)),        // ㄱ
        'R' => Some(Jamo::Consonant(1)),        // ㄲ
        's' | 'S' => Some(Jamo::Consonant(2)),  // ㄴ
        'e' => Some(Jamo::Consonant(3)),        // ㄷ
        'E' => Some(Jamo::Consonant(4)),        // ㄸ
        'f' | 'F' => Some(Jamo::Consonant(5)),  // ㄹ
        'a' | 'A' => Some(Jamo::Consonant(6)),  // ㅁ
        'q' => Some(Jamo::Consonant(7)),        // ㅂ
        'Q' => Some(Jamo::Consonant(8)),        // ㅃ
        't' => Some(Jamo::Consonant(9)),        // ㅅ
        'T' => Some(Jamo::Consonant(10)),       // ㅆ
        'd' | 'D' => Some(Jamo::Consonant(11)), // ㅇ
        'w' => Some(Jamo::Consonant(12)),       // ㅈ
        'W' => Some(Jamo::Consonant(13)),       // ㅉ
        'c' | 'C' => Some(Jamo::Consonant(14)), // ㅊ
        'z' | 'Z' => Some(Jamo::Consonant(15)), // ㅋ
        'x' | 'X' => Some(Jamo::Consonant(16)), // ㅌ
        'v' | 'V' => Some(Jamo::Consonant(17)), // ㅍ
        'g' | 'G' => Some(Jamo::Consonant(18)), // ㅎ

        // ── Vowels (중성) ──
        'k' | 'K' => Some(Jamo::Vowel(0)),  // ㅏ
        'o' => Some(Jamo::Vowel(1)),        // ㅐ
        'i' | 'I' => Some(Jamo::Vowel(2)),  // ㅑ
        'O' => Some(Jamo::Vowel(3)),        // ㅒ
        'j' | 'J' => Some(Jamo::Vowel(4)),  // ㅓ
        'p' => Some(Jamo::Vowel(5)),        // ㅔ
        'u' | 'U' => Some(Jamo::Vowel(6)),  // ㅕ
        'P' => Some(Jamo::Vowel(7)),        // ㅖ
        'h' | 'H' => Some(Jamo::Vowel(8)),  // ㅗ
        'y' | 'Y' => Some(Jamo::Vowel(12)), // ㅛ
        'n' | 'N' => Some(Jamo::Vowel(13)), // ㅜ
        'b' | 'B' => Some(Jamo::Vowel(17)), // ㅠ
        'm' | 'M' => Some(Jamo::Vowel(18)), // ㅡ
        'l' | 'L' => Some(Jamo::Vowel(20)), // ㅣ

        _ => None,
    }
}

// ============================================================================
// HangulComposer Implementation
// ============================================================================

impl HangulComposer {
    /// Create a new composer with empty state
    pub fn new() -> Self {
        Self {
            state: State::Empty,
        }
    }

    /// Process a jamo input and return the composition result.
    ///
    /// The caller should:
    /// 1. If `committed` is Some, push that char to the text buffer
    /// 2. Display `composing` (if any) at the cursor with a highlight style
    pub fn process(&mut self, jamo: Jamo) -> ComposeResult {
        match (self.state, jamo) {
            // ─── From Empty ────────────────────────────────────────
            (State::Empty, Jamo::Consonant(cho)) => {
                self.state = State::Choseong { cho };
                ComposeResult {
                    committed: None,
                    composing: Some(cho_to_char(cho)),
                }
            }
            (State::Empty, Jamo::Vowel(jung)) => {
                self.state = State::Jungseong { jung };
                ComposeResult {
                    committed: None,
                    composing: Some(jung_to_char(jung)),
                }
            }

            // ─── From Choseong (초성만 있음) ──────────────────────
            (State::Choseong { cho }, Jamo::Consonant(new_cho)) => {
                // Can't combine two consonants in 초성 → commit first, start new
                let committed = cho_to_char(cho);
                self.state = State::Choseong { cho: new_cho };
                ComposeResult {
                    committed: Some(committed),
                    composing: Some(cho_to_char(new_cho)),
                }
            }
            (State::Choseong { cho }, Jamo::Vowel(jung)) => {
                // 초성 + 중성 → syllable (no 종성 yet)
                self.state = State::ChoseongJungseong { cho, jung };
                ComposeResult {
                    committed: None,
                    composing: Some(compose_syllable(cho, jung, 0)),
                }
            }

            // ─── From Jungseong (홀로 쓴 모음) ───────────────────
            (State::Jungseong { jung }, Jamo::Vowel(new_jung)) => {
                // Try compound vowel
                if let Some(compound) = compound_jung(jung, new_jung) {
                    self.state = State::Jungseong { jung: compound };
                    ComposeResult {
                        committed: None,
                        composing: Some(jung_to_char(compound)),
                    }
                } else {
                    // Can't compound → commit current vowel, start new
                    let committed = jung_to_char(jung);
                    self.state = State::Jungseong { jung: new_jung };
                    ComposeResult {
                        committed: Some(committed),
                        composing: Some(jung_to_char(new_jung)),
                    }
                }
            }
            (State::Jungseong { jung }, Jamo::Consonant(cho)) => {
                // Vowel then consonant → commit vowel, start new 초성
                let committed = jung_to_char(jung);
                self.state = State::Choseong { cho };
                ComposeResult {
                    committed: Some(committed),
                    composing: Some(cho_to_char(cho)),
                }
            }

            // ─── From ChoseongJungseong (초성+중성) ──────────────
            (State::ChoseongJungseong { cho, jung }, Jamo::Vowel(new_jung)) => {
                // Try compound vowel
                if let Some(compound) = compound_jung(jung, new_jung) {
                    self.state = State::ChoseongJungseong {
                        cho,
                        jung: compound,
                    };
                    ComposeResult {
                        committed: None,
                        composing: Some(compose_syllable(cho, compound, 0)),
                    }
                } else {
                    // Can't compound → commit current syllable, start standalone vowel
                    let committed = compose_syllable(cho, jung, 0);
                    self.state = State::Jungseong { jung: new_jung };
                    ComposeResult {
                        committed: Some(committed),
                        composing: Some(jung_to_char(new_jung)),
                    }
                }
            }
            (State::ChoseongJungseong { cho, jung }, Jamo::Consonant(new_cho)) => {
                // Try adding as 종성
                if let Some(jong) = CHO_TO_JONG[new_cho as usize] {
                    self.state = State::Full { cho, jung, jong };
                    ComposeResult {
                        committed: None,
                        composing: Some(compose_syllable(cho, jung, jong)),
                    }
                } else {
                    // ㄸ, ㅃ, ㅉ can't be 종성 → commit syllable, start new 초성
                    let committed = compose_syllable(cho, jung, 0);
                    self.state = State::Choseong { cho: new_cho };
                    ComposeResult {
                        committed: Some(committed),
                        composing: Some(cho_to_char(new_cho)),
                    }
                }
            }

            // ─── From Full (초성+중성+종성) ──────────────────────
            (State::Full { cho, jung, jong }, Jamo::Consonant(new_cho)) => {
                // Try compound 종성
                if let Some(new_jong_idx) = CHO_TO_JONG[new_cho as usize] {
                    if let Some(compound) = compound_jong(jong, new_jong_idx) {
                        self.state = State::Full {
                            cho,
                            jung,
                            jong: compound,
                        };
                        return ComposeResult {
                            committed: None,
                            composing: Some(compose_syllable(cho, jung, compound)),
                        };
                    }
                }
                // Can't compound → commit current syllable, start new 초성
                let committed = compose_syllable(cho, jung, jong);
                self.state = State::Choseong { cho: new_cho };
                ComposeResult {
                    committed: Some(committed),
                    composing: Some(cho_to_char(new_cho)),
                }
            }
            (State::Full { cho, jung, jong }, Jamo::Vowel(new_jung)) => {
                // Vowel after 종성 → the 종성 moves to become 초성 of new syllable
                if let Some((remaining, detached)) = decompose_jong(jong) {
                    // Compound 종성: split it
                    let committed = compose_syllable(cho, jung, remaining);
                    let new_cho = jong_to_cho(detached).unwrap_or(0);
                    self.state = State::ChoseongJungseong {
                        cho: new_cho,
                        jung: new_jung,
                    };
                    ComposeResult {
                        committed: Some(committed),
                        composing: Some(compose_syllable(new_cho, new_jung, 0)),
                    }
                } else {
                    // Simple 종성 → whole thing moves
                    let committed = compose_syllable(cho, jung, 0);
                    let new_cho = jong_to_cho(jong).unwrap_or(0);
                    self.state = State::ChoseongJungseong {
                        cho: new_cho,
                        jung: new_jung,
                    };
                    ComposeResult {
                        committed: Some(committed),
                        composing: Some(compose_syllable(new_cho, new_jung, 0)),
                    }
                }
            }
        }
    }

    /// Handle backspace in composition.
    ///
    /// Returns `true` if the backspace was consumed (composing was modified).
    /// Returns `false` if there was nothing to decompose — caller should
    /// perform a normal backspace (remove last char from query).
    pub fn backspace(&mut self) -> bool {
        match self.state {
            State::Empty => false,

            State::Choseong { .. } => {
                self.state = State::Empty;
                true
            }

            State::Jungseong { jung } => {
                // Try decomposing compound vowel
                if let Some((first, _)) = decompose_jung(jung) {
                    self.state = State::Jungseong { jung: first };
                } else {
                    self.state = State::Empty;
                }
                true
            }

            State::ChoseongJungseong { cho, jung } => {
                // Try decomposing compound vowel
                if let Some((first, _)) = decompose_jung(jung) {
                    self.state = State::ChoseongJungseong { cho, jung: first };
                } else {
                    self.state = State::Choseong { cho };
                }
                true
            }

            State::Full { cho, jung, jong } => {
                // Try decomposing compound 종성
                if let Some((first, _)) = decompose_jong(jong) {
                    self.state = State::Full {
                        cho,
                        jung,
                        jong: first,
                    };
                } else {
                    self.state = State::ChoseongJungseong { cho, jung };
                }
                true
            }
        }
    }

    /// Flush (commit) any composing character and reset to Empty state.
    ///
    /// Returns the character to commit, or None if nothing was composing.
    pub fn flush(&mut self) -> Option<char> {
        let result = self.composing();
        self.state = State::Empty;
        result
    }

    /// Get the current composing character for display.
    ///
    /// Returns None if no composition is in progress.
    pub fn composing(&self) -> Option<char> {
        match self.state {
            State::Empty => None,
            State::Choseong { cho } => Some(cho_to_char(cho)),
            State::Jungseong { jung } => Some(jung_to_char(jung)),
            State::ChoseongJungseong { cho, jung } => Some(compose_syllable(cho, jung, 0)),
            State::Full { cho, jung, jong } => Some(compose_syllable(cho, jung, jong)),
        }
    }

    /// Check if there's an active composition in progress
    pub fn is_composing(&self) -> bool {
        !matches!(self.state, State::Empty)
    }
}

impl Default for HangulComposer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Utility
// ============================================================================

/// 낱자로 커밋된 호환 자모를 2벌식 오토마타로 재조합해 정규화한다.
///
/// macOS에서 런처가 새 프로세스로 뜬 직후 TSM이 한글 입력기를 연결하기 전에
/// 입력된 키는 조합 없이 낱자 자모(예: `ㅎㅏㄴ`)로 커밋된다. 이 함수는
/// 문자열 전체를 자모 스트림으로 분해한 뒤 오토마타로 다시 조합해 IME가
/// 조합했을 결과(`한`)를 복원한다.
///
/// - 정상 한글 텍스트는 분해→재조합이 항등이므로 변화 없음 → `None`
/// - 자음만 나열된 검색어(`ㅅㅅ` 등)도 조합 불가라 그대로 유지 → `None`
/// - 변경이 있을 때만 `Some(정규화된 문자열)` 반환
pub fn recompose_jamo(text: &str) -> Option<String> {
    if !text.chars().any(|c| compat_char_to_jamo(c).is_some()) {
        return None;
    }

    let mut out = String::with_capacity(text.len());
    let mut composer = HangulComposer::new();

    let feed = |composer: &mut HangulComposer, out: &mut String, jamo: Jamo| {
        let res = composer.process(jamo);
        if let Some(c) = res.committed {
            out.push(c);
        }
    };

    for ch in text.chars() {
        if let Some(jamo) = compat_char_to_jamo(ch) {
            feed(&mut composer, &mut out, jamo);
        } else if let Some((cho, jung, jong)) = decompose_syllable(ch) {
            // 완성 음절도 자모로 분해해 스트림에 합류시킨다. 그래야
            // "하" + 낱자 "ㄴ"처럼 앞서 재조합된 결과에 뒤따르는 자모가
            // 받침으로 올바르게 붙는다 (하ㄴ → 한).
            feed(&mut composer, &mut out, Jamo::Consonant(cho));
            feed(&mut composer, &mut out, Jamo::Vowel(jung));
            if jong != 0 {
                if let Some((first, second)) = decompose_jong(jong) {
                    if let (Some(c1), Some(c2)) = (jong_to_cho(first), jong_to_cho(second)) {
                        feed(&mut composer, &mut out, Jamo::Consonant(c1));
                        feed(&mut composer, &mut out, Jamo::Consonant(c2));
                    }
                } else if let Some(c) = jong_to_cho(jong) {
                    feed(&mut composer, &mut out, Jamo::Consonant(c));
                }
            }
        } else {
            if let Some(c) = composer.flush() {
                out.push(c);
            }
            out.push(ch);
        }
    }
    if let Some(c) = composer.flush() {
        out.push(c);
    }

    if out == text {
        None
    } else {
        Some(out)
    }
}

/// 호환 자모 문자 → Jamo (초성 테이블/중성 테이블 역조회)
fn compat_char_to_jamo(ch: char) -> Option<Jamo> {
    if let Some(i) = CHO_CHARS.iter().position(|&c| c == ch) {
        return Some(Jamo::Consonant(i as u32));
    }
    if let Some(i) = JUNG_CHARS.iter().position(|&c| c == ch) {
        return Some(Jamo::Vowel(i as u32));
    }
    None
}

/// 완성 음절(U+AC00..U+D7A3)을 (초성, 중성, 종성) 인덱스로 분해
fn decompose_syllable(ch: char) -> Option<(u32, u32, u32)> {
    let code = ch as u32;
    if !(0xAC00..=0xD7A3).contains(&code) {
        return None;
    }
    let idx = code - 0xAC00;
    Some((idx / (21 * 28), (idx % (21 * 28)) / 28, idx % 28))
}

/// Check if a character is already a Korean character (syllable or jamo).
/// Used to detect when the system IME is active and we should pass through.
pub fn is_korean_char(c: char) -> bool {
    // Hangul Syllables: U+AC00..U+D7AF
    (0xAC00..=0xD7AF).contains(&(c as u32)) || is_jamo(c)
}

/// 완성 음절이 아닌 한글 자모인지 (IME 조합 중간 상태 판정용).
///
/// - Hangul Jamo: U+1100..U+11FF
/// - Hangul Compatibility Jamo: U+3131..U+318E (옛한글 포함 — IME가 낼 수 있다)
/// - Hangul Jamo Extended-A: U+A960..U+A97F
/// - Hangul Jamo Extended-B: U+D7B0..U+D7FF
pub fn is_jamo(c: char) -> bool {
    let code = c as u32;
    (0x1100..=0x11FF).contains(&code)
        || (0x3131..=0x318E).contains(&code)
        || (0xA960..=0xA97F).contains(&code)
        || (0xD7B0..=0xD7FF).contains(&code)
}

// ============================================================================
// 검색용 — 한영 오타 변환 · 초성 (docs/22 §3.10)
// ============================================================================

/// 영문 자판 상태로 친 질의를 두벌식 한글로 바꾼다 — `qhrhtj` → `보고서`.
///
/// 한글 입력기를 켜지 않고 친 경우(Windows 에서 특히 흔하다)를 살리기 위한 것이다.
/// 영어 단어를 엉뚱한 한글로 바꾸지 않도록, 모든 글자가 **완성 음절로만** 조합될 때만
/// 돌려준다 — `test` 는 `ㅅㄷㄴㅅ`(자음만)이라 `None`.
pub fn qwerty_to_hangul(s: &str) -> Option<String> {
    if !s.is_ascii() || !s.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = String::with_capacity(s.len() * 2);
    let mut composer = HangulComposer::new();
    for ch in s.chars() {
        match key_to_jamo(ch) {
            Some(jamo) => {
                if let Some(c) = composer.process(jamo).committed {
                    out.push(c);
                }
            }
            None => {
                if let Some(c) = composer.flush() {
                    out.push(c);
                }
                out.push(ch);
            }
        }
    }
    if let Some(c) = composer.flush() {
        out.push(c);
    }
    let has_syllable = out.chars().any(|c| decompose_syllable(c).is_some());
    (has_syllable && !out.chars().any(is_jamo)).then_some(out)
}

/// 초성 자음 → 초성 번호 (`ㄱ`=0 … `ㅎ`=18). 받침 전용 겹자음(`ㄳ` 등)은 `None`.
pub fn choseong_index(c: char) -> Option<u32> {
    CHO_CHARS.iter().position(|&x| x == c).map(|i| i as u32)
}

/// 초성 질의인지 — 초성 자음으로만 2글자 이상 (`ㅂㄱㅅ`). 한 글자는 너무 많이 걸린다.
pub fn is_choseong_query(s: &str) -> bool {
    s.chars().count() >= 2 && s.chars().all(|c| choseong_index(c).is_some())
}

/// 이름의 초성 문자열 — 완성 음절은 초성 자모로, 나머지는 그대로 (`보고서_2026` → `ㅂㄱㅅ_2026`)
pub fn choseong_string(s: &str) -> String {
    s.chars()
        .map(|c| match decompose_syllable(c) {
            Some((cho, _, _)) => CHO_CHARS[cho as usize],
            None => c,
        })
        .collect()
}

/// 치는 중인 마지막 음절 — `보고ㅅ`·`보곳` 은 둘 다 "보고 + 초성 ㅅ" 으로 `보고서` 를 찾는다.
///
/// 두벌식 입력기로 `보고서` 를 치면 `보곳` 을 거친다(ㅅ 이 받침에 붙었다가 ㅓ 를 치면 다음 음절로 간다).
/// 그 사이에 결과가 비지 않게 질의 끝의 받침·자음을 "다음 글자의 초성" 으로도 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialSyllable {
    /// 마지막 글자 앞부분 + 받침을 뗀 마지막 음절 (`보곳` → `보고`, `닭` → `달`)
    pub head: String,
    /// 다음 글자의 초성 번호 (`ㄱ`=0 … `ㅎ`=18)
    pub cho: u32,
}

/// 질의 끝이 받침 있는 음절(`보곳`·`닭`)이거나 음절 뒤 자음(`보고ㅅ`)이면 [`PartialSyllable`].
/// 초성만인 질의(`ㅂㄱㅅ`)·자음 하나는 `None` — 초성 검색이 맡는다.
pub fn split_partial(s: &str) -> Option<PartialSyllable> {
    let mut chars = s.chars();
    let last = chars.next_back()?;
    let rest = chars.as_str();
    if let Some(cho) = choseong_index(last) {
        let prev = rest.chars().next_back()?;
        decompose_syllable(prev)?;
        return Some(PartialSyllable {
            head: rest.to_string(),
            cho,
        });
    }
    let (l, v, t) = decompose_syllable(last)?;
    let (keep, moved) = match decompose_jong(t) {
        Some((keep, moved)) => (keep, moved),
        None => (0, t),
    };
    let cho = jong_to_cho(moved)?;
    let mut head = rest.to_string();
    head.push(compose_syllable(l, v, keep));
    Some(PartialSyllable { head, cho })
}

/// `hay` 안에서 [`PartialSyllable`] 이 맞는 자리들 — (시작 바이트, 끝 바이트), 끝은 초성이 맞은 음절 뒤
pub fn partial_matches<'a>(
    hay: &'a str,
    p: &'a PartialSyllable,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    hay.match_indices(p.head.as_str())
        .filter_map(move |(pos, h)| {
            let end = pos + h.len();
            let next = hay[end..].chars().next()?;
            let (cho, _, _) = decompose_syllable(next)?;
            (cho == p.cho).then_some((pos, end + next.len_utf8()))
        })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {

    #[test]
    fn 한영_오타_변환은_완성_음절일_때만() {
        assert_eq!(qwerty_to_hangul("qhrhtj").as_deref(), Some("보고서"));
        assert_eq!(
            qwerty_to_hangul("dkssud gktpdy").as_deref(),
            Some("안녕 하세요")
        );
        assert_eq!(qwerty_to_hangul("2026sus").as_deref(), Some("2026년"));
        assert_eq!(qwerty_to_hangul("Tkdgks").as_deref(), Some("쌍한"));
        assert_eq!(qwerty_to_hangul("test"), None, "자음만 — 영어 단어");
        assert_eq!(qwerty_to_hangul("chrome"), None, "끝이 자음 하나로 남음");
        assert_eq!(qwerty_to_hangul("123"), None);
        assert_eq!(qwerty_to_hangul("보고서"), None);
    }

    #[test]
    fn 초성() {
        assert!(is_choseong_query("ㅂㄱㅅ"));
        assert!(!is_choseong_query("ㅂ"));
        assert!(!is_choseong_query("ㅂㄱ서"));
        assert!(!is_choseong_query("ㄳㄱ"), "받침 전용 겹자음");
        assert_eq!(choseong_string("보고서_2026.hwp"), "ㅂㄱㅅ_2026.hwp");
        assert_eq!(choseong_index('ㅎ'), Some(18));
    }

    #[test]
    fn 치는_중인_마지막_음절() {
        let p = |head: &str, cho: char| {
            Some(PartialSyllable {
                head: head.to_string(),
                cho: choseong_index(cho).unwrap(),
            })
        };
        assert_eq!(split_partial("보곳"), p("보고", 'ㅅ'));
        assert_eq!(split_partial("보고ㅅ"), p("보고", 'ㅅ'));
        assert_eq!(split_partial("복"), p("보", 'ㄱ'));
        assert_eq!(
            split_partial("닭"),
            p("달", 'ㄱ'),
            "겹받침은 뒤 자음만 넘긴다"
        );
        assert_eq!(split_partial("2026 있"), p("2026 이", 'ㅆ'));
        assert_eq!(split_partial("보고"), None, "받침 없음");
        assert_eq!(split_partial("ㅂㄱㅅ"), None, "초성 검색");
        assert_eq!(split_partial("ㅅ"), None);
        assert_eq!(split_partial("abc"), None);
        assert_eq!(split_partial(""), None);

        let q = split_partial("보곳").unwrap();
        let hits: Vec<_> = partial_matches("주간_보고서.hwp", &q).collect();
        assert_eq!(hits, vec![(7, 16)]);
        assert_eq!(
            partial_matches("보고_최종", &q).count(),
            0,
            "다음 글자 초성이 다름"
        );
        assert_eq!(partial_matches("보고", &q).count(), 0, "다음 글자 없음");
    }

    use super::*;

    /// Helper: process a sequence of keys and return (committed_string, composing_char)
    fn compose_keys(keys: &str) -> (String, Option<char>) {
        let mut composer = HangulComposer::new();
        let mut committed = String::new();

        for c in keys.chars() {
            if let Some(jamo) = key_to_jamo(c) {
                let result = composer.process(jamo);
                if let Some(ch) = result.committed {
                    committed.push(ch);
                }
            }
        }

        // Flush remaining
        if let Some(ch) = composer.flush() {
            committed.push(ch);
        }

        (committed, None) // after flush, composing is None
    }

    #[test]
    fn test_simple_syllable() {
        // 한 = ㅎ(g) + ㅏ(k) + ㄴ(s)
        let (result, _) = compose_keys("gks");
        assert_eq!(result, "한");
    }

    #[test]
    fn test_hangul_word() {
        // 한글 = ㅎ+ㅏ+ㄴ+ㄱ+ㅡ+ㄹ
        let (result, _) = compose_keys("gksrmf");
        assert_eq!(result, "한글");
    }

    #[test]
    fn test_compound_jongseong() {
        // 읽 = ㅇ+ㅣ+ㄹ+ㄱ
        let (result, _) = compose_keys("dlfr");
        assert_eq!(result, "읽");
    }

    #[test]
    fn test_jongseong_to_choseong() {
        // 모음 = ㅁ+ㅗ+ㅇ+ㅡ+ㅁ
        let (result, _) = compose_keys("ahdma");
        assert_eq!(result, "모음");
    }

    #[test]
    fn test_compound_vowel() {
        // 과 = ㄱ+ㅘ(ㅗ+ㅏ)
        let (result, _) = compose_keys("rhk");
        assert_eq!(result, "과");
    }

    #[test]
    fn test_compound_jong_then_vowel() {
        // 읽어 = ㅇ+ㅣ+ㄹ+ㄱ / ㅇ+ㅓ
        let (result, _) = compose_keys("dlfrdj");
        assert_eq!(result, "읽어");
    }

    #[test]
    fn test_double_consonant() {
        // 빠 = ㅃ(Q) + ㅏ(k)
        let (result, _) = compose_keys("Qk");
        assert_eq!(result, "빠");
    }

    #[test]
    fn test_standalone_vowel() {
        // ㅏ = k
        let (result, _) = compose_keys("k");
        assert_eq!(result, "ㅏ");
    }

    #[test]
    fn test_standalone_consonant() {
        // ㄱ = r
        let (result, _) = compose_keys("r");
        assert_eq!(result, "ㄱ");
    }

    #[test]
    fn test_backspace_jongseong() {
        let mut composer = HangulComposer::new();
        let mut committed = String::new();

        // Type: ㅎ+ㅏ+ㄴ = 한
        for c in "gks".chars() {
            let result = composer.process(key_to_jamo(c).unwrap());
            if let Some(ch) = result.committed {
                committed.push(ch);
            }
        }
        assert_eq!(composer.composing(), Some('한'));

        // Backspace: 한 → 하
        assert!(composer.backspace());
        assert_eq!(composer.composing(), Some('하'));

        // Backspace: 하 → ㅎ
        assert!(composer.backspace());
        assert_eq!(composer.composing(), Some('ㅎ'));

        // Backspace: ㅎ → empty
        assert!(composer.backspace());
        assert_eq!(composer.composing(), None);

        // Backspace: empty → false (caller handles)
        assert!(!composer.backspace());
    }

    #[test]
    fn test_backspace_compound_jongseong() {
        let mut composer = HangulComposer::new();

        // Type: ㅇ+ㅣ+ㄹ+ㄱ = 읽 (compound 종성 ㄺ)
        for c in "dlfr".chars() {
            composer.process(key_to_jamo(c).unwrap());
        }
        assert_eq!(composer.composing(), Some('읽'));

        // Backspace: 읽 → 일 (remove ㄱ from ㄺ, leaving ㄹ)
        assert!(composer.backspace());
        assert_eq!(composer.composing(), Some('일'));
    }

    #[test]
    fn test_backspace_compound_vowel() {
        let mut composer = HangulComposer::new();

        // Type: ㄱ+ㅘ (= ㄱ+ㅗ+ㅏ) = 과
        for c in "rhk".chars() {
            composer.process(key_to_jamo(c).unwrap());
        }
        assert_eq!(composer.composing(), Some('과'));

        // Backspace: 과 → 고 (ㅘ decomposes to ㅗ)
        assert!(composer.backspace());
        assert_eq!(composer.composing(), Some('고'));
    }

    #[test]
    fn test_mixed_korean_and_flush() {
        let mut composer = HangulComposer::new();
        let mut result = String::new();

        // Type "안녕" = ㅇ(d)+ㅏ(k)+ㄴ(s) + ㄴ(s)+ㅕ(u)+ㅇ(d)
        // Key sequence: d k s s u d (6 keys)
        for c in "dkssud".chars() {
            let r = composer.process(key_to_jamo(c).unwrap());
            if let Some(ch) = r.committed {
                result.push(ch);
            }
        }
        // d(ㅇ) → Choseong
        // k(ㅏ) → 아 (ChoseongJungseong)
        // s(ㄴ) → 안 (Full, jong=ㄴ)
        // s(ㄴ) → commit '안', composing ㄴ (Choseong)
        // u(ㅕ) → 녀 (ChoseongJungseong)
        // d(ㅇ) → 녕 (Full, jong=ㅇ)
        // committed = "안", composing = "녕"

        if let Some(ch) = composer.flush() {
            result.push(ch);
        }
        assert_eq!(result, "안녕");
    }

    #[test]
    fn test_key_to_jamo_coverage() {
        // All lowercase keys should map
        for c in "qwertyuiopasdfghjklzxcvbnm".chars() {
            assert!(key_to_jamo(c).is_some(), "key '{}' should map to jamo", c);
        }
        // Shifted variants that produce different jamo
        for c in "QWERTOP".chars() {
            assert!(key_to_jamo(c).is_some(), "shifted '{}' should map", c);
        }
        // Non-jamo keys
        assert!(key_to_jamo('1').is_none());
        assert!(key_to_jamo(' ').is_none());
        assert!(key_to_jamo('.').is_none());
    }

    #[test]
    fn test_compose_syllable_formula() {
        // 가 = cho:ㄱ(0) + jung:ㅏ(0) + jong:0
        assert_eq!(compose_syllable(0, 0, 0), '가');
        // 힣 = cho:ㅎ(18) + jung:ㅣ(20) + jong:ㅎ(27)
        assert_eq!(compose_syllable(18, 20, 27), '힣');
    }

    // ── recompose_jamo (TSM 미연결 낱자 커밋 복원) ──

    #[test]
    fn test_recompose_jamo_basic() {
        assert_eq!(recompose_jamo("ㅎㅏㄴ"), Some("한".to_string()));
        assert_eq!(recompose_jamo("ㅎㅏㄴㄱㅡㄹ"), Some("한글".to_string()));
        assert_eq!(recompose_jamo("ㅗㅏ"), Some("ㅘ".to_string()));
    }

    #[test]
    fn test_recompose_jamo_incremental() {
        // 앞서 재조합된 음절 뒤에 낱자가 이어지는 경우 (키 하나씩 도착)
        assert_eq!(recompose_jamo("하ㄴ"), Some("한".to_string()));
        assert_eq!(recompose_jamo("하ㄴㄱㅡ"), Some("한그".to_string()));
        assert_eq!(recompose_jamo("한ㄱㅡㄹ"), Some("한글".to_string()));
    }

    #[test]
    fn test_recompose_jamo_identity_returns_none() {
        // 정상 텍스트는 분해→재조합이 항등 → None
        assert_eq!(recompose_jamo("한글"), None);
        assert_eq!(recompose_jamo("값"), None, "겹받침 왕복 항등");
        assert_eq!(recompose_jamo("safari"), None);
        assert_eq!(recompose_jamo(""), None);
        // 조합 불가능한 자음 나열(의도적 자모 검색)도 유지
        assert_eq!(recompose_jamo("ㅅㅅ"), None);
        assert_eq!(recompose_jamo("ㄱ"), None);
    }

    #[test]
    fn test_recompose_jamo_mixed() {
        assert_eq!(recompose_jamo("aㅎㅏn"), Some("a하n".to_string()));
        assert_eq!(recompose_jamo("ㅎㅏ ㄴ"), Some("하 ㄴ".to_string()));
        assert_eq!(recompose_jamo("김 부장"), None, "공백 포함 정상 텍스트");
    }
}
