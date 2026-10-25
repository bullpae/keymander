//! `:say` — 컴퓨터 앞에서 영어에 스며들기 (입버릇 데스크톱, docs/21).
//!
//! 폰 앱 입버릇(SayHabit)은 말하기를, 컴퓨터는 **노출**을 맡는다. 런처를 열 때마다
//! 한 줄 카드로 문장을 보여 주고(`card`), `:say` 로 타이핑 떠올리기를 한다(`command`).
//!
//! 컴퓨터에서 맞혀도 상자(box)는 올리지 않는다 — 입버릇의 "말해야 통과" 원칙
//! (docs/21 §4). 그래서 컴퓨터는 [`srs::grade_hold`]·[`srs::grade_fail`] 만 쓴다.
//!
//! 저장: `<data_dir>/say/deck.json`(문장, 입버릇과 같은 필드) +
//! `state.json`(카드·연습 중인 문장 같은 화면 상태). 런처는 단축키마다 새 프로세스라
//! 화면 상태도 파일에 둔다.

pub mod bulk;
pub mod command;
pub mod scoring;
pub mod srs;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 명령 별칭 — 영문 자판 그대로 치는 `:say` 가 기본 (`:map` 과 같은 이유)
pub const SAY_ALIASES: &[&str] = &[":say", ":입버릇"];

/// 런처 카드가 같은 문장을 유지하는 시간 — 열 때마다 바뀌면 읽기 전에 넘어간다
pub const CARD_HOLD_MS: i64 = 10 * 60 * 1000;

/// 최근에 보여 준 문장 몇 개를 피할지 (입버릇 planNudges 와 같은 3)
const RECENT_LEN: usize = 3;

/// `[say]` 설정 (docs/06 §4.7)
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SayConfig {
    /// 기본 꺼짐 — 런처는 검색 도구이고 화면을 다른 사람이 볼 수 있다.
    /// `:say` 를 처음 치면 켜는 항목이 나온다.
    pub enabled: bool,
    /// 빈 입력일 때 결과 아래 한 줄 카드
    pub card: bool,
    /// 타이핑 떠올리기 통과 기준 (입버릇 passScore 와 같은 뜻)
    pub pass_score: f64,
}

impl Default for SayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            card: true,
            pass_score: 0.8,
        }
    }
}

/// 문장 하나 — 입버릇 `Sentence` 와 같은 JSON 필드(camelCase).
/// 뒤의 세 필드는 컴퓨터가 더한 것이고 입버릇은 모르는 필드라 무시해도 된다.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sentence {
    pub id: String,
    pub en: String,
    pub ko: String,
    pub builtin: bool,
    /// 간격 반복 상자 (0 = 새 문장/틀린 문장)
    pub r#box: u8,
    /// 다음 복습 시각 (epoch ms)
    pub due_at: i64,
    /// 폰에서 소리 내어 말한 횟수 (컴퓨터는 건드리지 않는다)
    pub spoken_count: u32,
    pub created_at: i64,
    /// 마지막으로 바뀐 시각 — S4 동기화 병합용
    pub updated_at: i64,
    /// 컴퓨터에서 노출된 횟수 (카드) — 통계용, 복습 일정에는 쓰지 않는다
    pub seen_count: u32,
    /// 타이핑 떠올리기 통과 횟수
    pub typed_count: u32,
}

impl Sentence {
    pub fn new(en: &str, ko: &str, builtin: bool, now: i64) -> Self {
        Self {
            id: new_id(now),
            en: en.trim().to_string(),
            ko: ko.trim().to_string(),
            builtin,
            due_at: now,
            created_at: now,
            updated_at: now,
            ..Self::default()
        }
    }
}

/// 입버릇과 같은 모양의 id — `<ms 36진수>-<무작위 6자>`
fn new_id(now: i64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let to36 = |mut n: u64, min: usize| {
        let mut out = Vec::new();
        while n > 0 || out.len() < min {
            out.push(DIGITS[(n % 36) as usize]);
            n /= 36;
        }
        out.reverse();
        String::from_utf8(out).unwrap_or_default()
    };
    let mut buf = [0u8; 8];
    if getrandom::fill(&mut buf).is_err() {
        // 무작위가 없으면 시각의 나노초로 대신한다 — 같은 ms 에 여러 개를 만들 때만 겹칠 수 있다
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        buf[..4].copy_from_slice(&nanos.to_le_bytes());
    }
    let rand = to36(u64::from_le_bytes(buf), 6);
    format!("{}-{}", to36(now.max(0) as u64, 1), &rand[rand.len() - 6..])
}

/// 문장 묶음 파일 — 입버릇 백업 형식과 같게 둔다 (docs/21 §5.2)
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Deck {
    pub format: String,
    pub version: u32,
    pub sentences: Vec<Sentence>,
}

pub const DECK_FORMAT: &str = "sayhabit-deck";

impl Deck {
    pub fn new(sentences: Vec<Sentence>) -> Self {
        Self {
            format: DECK_FORMAT.to_string(),
            version: 1,
            sentences,
        }
    }

    /// 처음 켤 때 넣는 기본 문장 — 한꺼번에 몰리지 않게 1분씩 시차를 둔다 (입버릇과 같음)
    pub fn seeded(now: i64) -> Self {
        Self::new(
            DEFAULT_SENTENCES
                .iter()
                .enumerate()
                .map(|(i, (en, ko))| {
                    let mut s = Sentence::new(en, ko, true, now);
                    s.due_at = now + i as i64 * 60_000;
                    s
                })
                .collect(),
        )
    }

    pub fn get(&self, id: &str) -> Option<&Sentence> {
        self.sentences.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Sentence> {
        self.sentences.iter_mut().find(|s| s.id == id)
    }

    /// 같은 영어 문장이 이미 있는지 (대소문자·문장부호·축약형 무시)
    pub fn contains_en(&self, en: &str) -> bool {
        let key = scoring::normalize_words(en);
        !key.is_empty()
            && self
                .sentences
                .iter()
                .any(|s| scoring::normalize_words(&s.en) == key)
    }

    /// 여러 쌍을 담는다 — (담은 수, 중복으로 건너뛴 수)
    pub fn add_pairs(&mut self, pairs: &[(String, String)], now: i64) -> (usize, usize) {
        let (mut added, mut dup) = (0, 0);
        for (en, ko) in pairs {
            if self.contains_en(en) {
                dup += 1;
            } else {
                self.sentences.push(Sentence::new(en, ko, false, now));
                added += 1;
            }
        }
        (added, dup)
    }
}

/// 카드에 지금 걸린 문장
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CardPin {
    pub id: String,
    /// 이 문장을 처음 건 시각 (epoch ms)
    pub since: i64,
}

/// `:say` 에서 연습 중인 문장 — 틀리면 정답을 보고 한 번 따라 치게 붙잡아 둔다
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PracticePin {
    pub id: String,
    /// 이번 문장을 이미 틀렸는지 — 그 뒤의 입력은 채점이 아니라 따라 치기다
    pub failed: bool,
}

/// 화면 상태 — 문장 파일과 따로 둔다 (동기화 대상이 아니다)
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct SayState {
    pub card: Option<CardPin>,
    pub practice: Option<PracticePin>,
    /// 최근에 보여 준 문장 id (새 것이 뒤)
    pub recent: Vec<String>,
}

impl SayState {
    pub fn push_recent(&mut self, id: &str) {
        self.recent.retain(|r| r != id);
        self.recent.push(id.to_string());
        let over = self.recent.len().saturating_sub(RECENT_LEN);
        self.recent.drain(..over);
    }
}

/// 파일 위치 묶음. 테스트는 임시 폴더를 넘긴다.
#[derive(Debug, Clone)]
pub struct SayStore {
    dir: PathBuf,
}

impl SayStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<data_dir>/say`
    pub fn default_location() -> Self {
        Self::new(crate::Config::default_data_dir().join("say"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn deck_path(&self) -> PathBuf {
        self.dir.join("deck.json")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    /// 문장 묶음 읽기. 파일이 없으면 기본 문장을 만들어 **바로 저장한다** — 저장 전에는
    /// 읽을 때마다 id 가 새로 생겨, 미리보기에서 고른 문장을 Enter 에서 못 찾는다.
    /// (켜기 전에는 부르지 않으므로 꺼진 사용자에게 파일이 생기지 않는다.)
    ///
    /// 파일이 깨져 있으면 오류다 — 기본 문장으로 덮어쓰면 사용자가 모은 문장이 사라진다.
    pub fn load_deck(&self, now: i64) -> Result<Deck, String> {
        match std::fs::read_to_string(self.deck_path()) {
            Ok(text) => serde_json::from_str::<Deck>(&text).map_err(|e| {
                format!(
                    "{} 을 읽지 못했습니다 ({e}) — 파일을 고치거나 옮긴 뒤 다시 여세요",
                    self.deck_path().display()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let deck = Deck::seeded(now);
                // 저장에 실패해도 이번 화면은 그릴 수 있다 — 다음 저장에서 다시 시도한다
                if let Err(e) = self.save_deck(&deck) {
                    tracing::warn!("입버릇 기본 문장 저장 실패: {e}");
                }
                Ok(deck)
            }
            Err(e) => Err(format!("{} 읽기 실패: {e}", self.deck_path().display())),
        }
    }

    pub fn save_deck(&self, deck: &Deck) -> Result<(), String> {
        self.write_json(&self.deck_path(), deck)
    }

    /// 화면 상태 읽기 — 없거나 깨졌으면 빈 상태 (잃어도 되는 정보다)
    pub fn load_state(&self) -> SayState {
        std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_state(&self, state: &SayState) -> Result<(), String> {
        self.write_json(&self.state_path(), state)
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("{} 만들기 실패: {e}", self.dir.display()))?;
        let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        crate::fsutil::write_atomic(path, json.as_bytes(), true)
            .map_err(|e| format!("{} 저장 실패: {e}", path.display()))
    }
}

/// 지금 시각 (epoch ms)
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 런처 한 줄 카드에 보일 내용
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: String,
    pub en: String,
    pub ko: String,
    /// 떠올리기 단계 — 한국어만 보이고 Tab 으로 영어를 연다
    pub recall: bool,
}

impl Card {
    /// 카드 한 줄. `revealed` 면 떠올리기 단계여도 영어를 보인다.
    pub fn line(&self, revealed: bool, use_emoji: bool) -> String {
        let icon = if use_emoji { "\u{1F4AC}" } else { "[EN]" };
        if self.recall && !revealed {
            format!("{icon} {}  \u{2192}  ?", self.ko)
        } else if self.recall {
            format!("{icon} {}  \u{2192}  {}", self.ko, self.en)
        } else {
            format!("{icon} {}   {}", self.en, self.ko)
        }
    }
}

/// 카드에 걸 문장을 고른다 — 걸린 지 [`CARD_HOLD_MS`] 안이면 그대로, 아니면 새로.
/// 상태가 바뀌었으면 true (호출자가 저장한다).
pub fn pick_card(deck: &Deck, state: &mut SayState, now: i64) -> (Option<Card>, bool) {
    let held = state
        .card
        .as_ref()
        .filter(|pin| now - pin.since < CARD_HOLD_MS && now >= pin.since)
        .and_then(|pin| deck.get(&pin.id));
    let (sentence, changed) = match held {
        Some(s) => (Some(s), false),
        None => match srs::pick_for_time(&deck.sentences, now, &state.recent) {
            Some(s) => {
                state.card = Some(CardPin {
                    id: s.id.clone(),
                    since: now,
                });
                state.push_recent(&s.id);
                (Some(s), true)
            }
            None => (None, false),
        },
    };
    let card = sentence.map(|s| Card {
        id: s.id.clone(),
        en: s.en.clone(),
        ko: s.ko.clone(),
        recall: srs::is_recall_stage(s),
    });
    (card, changed)
}

/// 런처를 열 때 카드 하나를 준비한다 — 읽고, 고르고, 노출 횟수를 센다.
///
/// 저장은 호출자가 [`record_card_seen`] 으로 따로 한다(첫 화면을 그리는 길에서 디스크 쓰기를
/// 빼려고). 파일이 깨졌으면 카드를 띄우지 않는다 — 검색을 막을 일은 아니다.
pub fn load_card(store: &SayStore, now: i64) -> Option<Card> {
    let deck = store.load_deck(now).ok()?;
    let mut state = store.load_state();
    pick_card(&deck, &mut state, now).0
}

/// 카드가 한 번 보였음을 기록한다 (노출 수 +1, 카드 고정 상태 저장).
pub fn record_card_seen(store: &SayStore, now: i64) -> Result<(), String> {
    let mut deck = store.load_deck(now)?;
    let mut state = store.load_state();
    let (card, changed) = pick_card(&deck, &mut state, now);
    let Some(card) = card else {
        return Ok(());
    };
    if let Some(s) = deck.get_mut(&card.id) {
        s.seen_count += 1;
    }
    store.save_deck(&deck)?;
    if changed {
        store.save_state(&state)?;
    }
    Ok(())
}

/// 기본 문장 40개 — 입버릇 `src/defaultSentences.ts` 와 같다
pub const DEFAULT_SENTENCES: &[(&str, &str)] = &[
    (
        "Could you say that again, please?",
        "다시 한 번 말씀해 주시겠어요?",
    ),
    (
        "I'm not sure I understand.",
        "제가 이해한 건지 잘 모르겠어요.",
    ),
    ("What do you mean by that?", "그게 무슨 뜻이에요?"),
    ("Let me think about it.", "생각 좀 해볼게요."),
    ("That makes sense.", "말이 되네요. / 이해가 돼요."),
    ("I didn't mean to do that.", "그러려던 건 아니었어요."),
    (
        "I'll get back to you on that.",
        "그건 나중에 다시 알려드릴게요.",
    ),
    ("Do you have a minute?", "잠깐 시간 괜찮으세요?"),
    ("I'm running a little late.", "저 조금 늦을 것 같아요."),
    (
        "It's up to you.",
        "당신이 정하세요. / 당신 마음대로 하세요.",
    ),
    ("I'm looking forward to it.", "그거 기대하고 있어요."),
    ("Sorry to bother you.", "귀찮게 해서 죄송해요."),
    ("Can I ask you something?", "뭐 하나 물어봐도 돼요?"),
    (
        "I've been meaning to ask you.",
        "안 그래도 물어보려고 했어요.",
    ),
    ("How was your weekend?", "주말 어떻게 보냈어요?"),
    ("What do you do for a living?", "무슨 일 하세요?"),
    ("I'm into hiking these days.", "요즘 등산에 빠져 있어요."),
    ("That sounds like a good idea.", "좋은 생각 같아요."),
    ("I couldn't agree more.", "전적으로 동의해요."),
    (
        "I see what you mean, but I'm not sure.",
        "무슨 말인지는 알겠는데, 잘 모르겠어요.",
    ),
    ("Could you give me a hand?", "좀 도와주실 수 있어요?"),
    ("No worries, take your time.", "괜찮아요, 천천히 하세요."),
    ("I'll be there in ten minutes.", "10분 뒤에 도착해요."),
    (
        "Where is the nearest subway station?",
        "가장 가까운 지하철역이 어디예요?",
    ),
    ("Can I get this to go?", "이거 포장해 갈 수 있어요?"),
    ("I'd like to make a reservation.", "예약하고 싶어요."),
    ("How much does this cost?", "이거 얼마예요?"),
    (
        "Do you have this in a different size?",
        "이거 다른 사이즈 있어요?",
    ),
    (
        "I'm just looking, thanks.",
        "그냥 둘러보는 중이에요, 감사합니다.",
    ),
    ("Could we split the bill?", "계산 따로 할 수 있을까요?"),
    ("I have a meeting at three.", "3시에 회의가 있어요."),
    ("Let's touch base next week.", "다음 주에 다시 이야기해요."),
    (
        "I'll send you the file by tomorrow.",
        "내일까지 파일 보내드릴게요.",
    ),
    (
        "Can we push the meeting back an hour?",
        "회의를 한 시간 미룰 수 있을까요?",
    ),
    ("I'm working on it right now.", "지금 그거 작업하고 있어요."),
    ("It took longer than I expected.", "생각보다 오래 걸렸어요."),
    (
        "I'm sorry, I forgot all about it.",
        "죄송해요, 완전히 깜빡했어요.",
    ),
    ("It's not a big deal.", "별일 아니에요."),
    (
        "I'm so tired I could sleep all day.",
        "너무 피곤해서 하루 종일 잘 수도 있을 것 같아요.",
    ),
    ("Thanks for having me.", "초대해 주셔서 감사해요."),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> SayStore {
        let dir = std::env::temp_dir().join(format!("kmd-say-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SayStore::new(dir)
    }

    #[test]
    fn deck_json_uses_sayhabit_field_names() {
        let s = Sentence::new("Hi", "안녕", false, 5);
        let v = serde_json::to_value(&s).unwrap();
        for key in [
            "id",
            "en",
            "ko",
            "builtin",
            "box",
            "dueAt",
            "spokenCount",
            "createdAt",
        ] {
            assert!(v.get(key).is_some(), "{key} 없음: {v}");
        }
        // 입버릇이 쓴 파일(컴퓨터 필드 없음)도 읽힌다
        let phone = r#"{"id":"a","en":"Hi","ko":"안녕","builtin":true,"box":3,"dueAt":9,"spokenCount":2,"createdAt":1}"#;
        let s: Sentence = serde_json::from_str(phone).unwrap();
        assert_eq!((s.r#box, s.due_at, s.seen_count), (3, 9, 0));
    }

    #[test]
    fn ids_look_like_sayhabit_and_differ() {
        let a = new_id(1_700_000_000_000);
        let b = new_id(1_700_000_000_000);
        assert_ne!(a, b);
        let (time, rand) = a.split_once('-').unwrap();
        assert_eq!(rand.len(), 6);
        assert_eq!(u64::from_str_radix(time, 36).unwrap(), 1_700_000_000_000);
    }

    #[test]
    fn seeded_deck_and_dedupe() {
        let mut deck = Deck::seeded(1000);
        assert_eq!(deck.sentences.len(), 40);
        assert_eq!(deck.sentences[1].due_at, 61_000);
        assert!(deck.contains_en("it's UP to you"));
        let (added, dup) = deck.add_pairs(
            &[
                ("It is up to you!".into(), "x".into()),
                ("I'm on it.".into(), "하고 있어요".into()),
            ],
            2000,
        );
        assert_eq!((added, dup), (1, 1));
    }

    #[test]
    fn store_round_trip_and_corrupt_is_error() {
        let store = temp_store("rt");
        let deck = store.load_deck(0).unwrap();
        assert_eq!(deck.sentences.len(), 40, "없으면 기본 문장");
        assert_eq!(
            store.load_deck(0).unwrap(),
            deck,
            "처음 읽을 때 저장해 id 가 그대로다"
        );
        std::fs::write(store.deck_path(), "{ broken").unwrap();
        assert!(
            store.load_deck(0).is_err(),
            "깨진 파일을 기본값으로 덮지 않는다"
        );
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn card_holds_for_ten_minutes_then_rotates() {
        let deck = Deck::seeded(0);
        let mut state = SayState::default();
        let (c1, changed) = pick_card(&deck, &mut state, 100);
        assert!(changed);
        let c1 = c1.unwrap();
        let (c2, changed) = pick_card(&deck, &mut state, 100 + CARD_HOLD_MS - 1);
        assert!(!changed);
        assert_eq!(c2.unwrap().id, c1.id);
        let (c3, changed) = pick_card(&deck, &mut state, 100 + CARD_HOLD_MS);
        assert!(changed);
        assert_ne!(c3.unwrap().id, c1.id, "최근 문장은 피한다");
    }

    #[test]
    fn card_line_hides_english_in_recall_stage() {
        let card = Card {
            id: "a".into(),
            en: "It's up to you.".into(),
            ko: "마음대로 하세요".into(),
            recall: true,
        };
        assert!(!card.line(false, true).contains("It's"));
        assert!(card.line(true, true).contains("It's"));
        let expo = Card {
            recall: false,
            ..card
        };
        assert!(expo.line(false, false).starts_with("[EN] It's up to you."));
    }

    #[test]
    fn record_card_seen_counts_and_pins() {
        let store = temp_store("seen");
        record_card_seen(&store, 1000).unwrap();
        record_card_seen(&store, 2000).unwrap();
        let deck = store.load_deck(0).unwrap();
        let pin = store.load_state().card.unwrap();
        assert_eq!(deck.get(&pin.id).unwrap().seen_count, 2);
        assert_eq!(pin.since, 1000);
        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[test]
    fn recent_keeps_last_three() {
        let mut st = SayState::default();
        for id in ["a", "b", "c", "d", "b"] {
            st.push_recent(id);
        }
        assert_eq!(st.recent, ["c", "d", "b"]);
    }
}
