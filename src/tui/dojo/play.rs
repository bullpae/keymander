//! 수련 한 판 — 종류별 라운드를 하나로 묶는 공통 틀.
//!
//! 종류마다 판정은 다르지만 앱이 아는 것은 같다: 이벤트를 넘기고, 시계를
//! 돌리고, 끝나면 [`Summary`] 를 받아 기록한다.

use std::time::{Duration, Instant};

use super::golf::GolfRound;
use super::maze::MazeRound;
use super::mouse::MouseRound;
use super::score::HoleResult;
use super::spar::Spar;
use super::summon::Round;

/// 이벤트 하나를 처리한 결과
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundEvent {
    None,
    /// 맞힘 (대련·조준)
    Hit,
    /// 틀림
    Mistake,
    /// 문항(의뢰·홀·미로) 하나 완료
    Cleared {
        over_par: bool,
        sage_equal: bool,
    },
    /// 마지막 문항까지 완료
    Finished,
    /// 패배 (기 소진 · 시간 초과 · 보스 생존)
    Failed,
}

/// 보스전 게이지 (docs/10 §8-3) — 명중은 보스 HP, 실수는 내 기(氣)
#[derive(Debug, Clone)]
pub struct BossState {
    /// 0~100
    pub hp: i32,
    /// 0~100, 0 이 되면 패배
    pub ki: i32,
    pub time_limit: Option<Duration>,
    /// 마지막으로 맞은 순간 (보스 그림 깜빡임)
    pub hurt_at: Option<Instant>,
    /// 마지막으로 내가 맞은 순간 (보스 도발)
    pub struck_at: Option<Instant>,
}

impl BossState {
    pub fn new(time_limit: Option<Duration>) -> Self {
        Self {
            hp: 100,
            ki: 100,
            time_limit,
            hurt_at: None,
            struck_at: None,
        }
    }

    /// 보스를 친다
    pub fn hit(&mut self, dmg: i32) {
        self.hp = (self.hp - dmg).max(0);
        self.hurt_at = Some(Instant::now());
    }

    /// 내가 맞는다 — 기가 다하면 true
    pub fn hurt_me(&mut self, n: i32) -> bool {
        self.ki -= n;
        self.struck_at = Some(Instant::now());
        self.ki <= 0
    }

    pub fn recently_hurt(&self, now: Instant) -> bool {
        self.hurt_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(220))
    }

    pub fn recently_struck(&self, now: Instant) -> bool {
        self.struck_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(1500))
    }
}

/// 결과 화면 스코어카드의 한 줄
pub struct OutcomeRow {
    pub ask: String,
    pub sage: String,
    pub sage_strokes: u32,
    pub par: u32,
    pub strokes: Option<u32>,
}

/// 한 판을 마친 요약 — 기록과 결과 화면의 재료
pub struct Summary {
    pub stars: u8,
    pub score: u32,
    pub strokes: Option<u32>,
    pub par: Option<u32>,
    pub mistakes: u32,
    pub hints: u32,
    pub max_combo: Option<u32>,
    /// 타수 계열(소환·골프·미로)의 스코어카드
    pub rows: Vec<OutcomeRow>,
    /// 반응·조준 계열의 수치 (이름, 값)
    pub stats: Vec<(String, String)>,
    pub ki: Option<i32>,
}

impl Summary {
    /// 타수 계열 공통 — 별·점수는 골프 규칙(docs/10 §8-1)
    pub fn from_holes(
        holes: &[HoleResult],
        rows: Vec<OutcomeRow>,
        cleared: bool,
        mistakes: u32,
        hints: u32,
        boss: Option<&BossState>,
    ) -> Self {
        use super::score;
        let (stars, score) = match (boss, cleared) {
            (_, false) => (0, 0),
            (Some(b), true) => (
                score::boss_stars(b.ki),
                score::stroke_score(holes, mistakes),
            ),
            (None, true) => (
                score::stroke_stars(holes),
                score::stroke_score(holes, mistakes),
            ),
        };
        Self {
            stars,
            score,
            strokes: cleared.then(|| holes.iter().map(|h| h.strokes).sum()),
            par: cleared.then(|| holes.iter().map(|h| h.par).sum()),
            mistakes,
            hints,
            max_combo: None,
            rows,
            stats: Vec::new(),
            ki: boss.map(|b| b.ki.max(0)),
        }
    }
}

/// 점수 → 등급 글자
pub fn rank(score: u32) -> (&'static str, &'static str) {
    match score {
        950.. => ("S", "신기(神技)"),
        800.. => ("A", "고수"),
        600.. => ("B", "숙련"),
        350.. => ("C", "수련생"),
        _ => ("D", "입문"),
    }
}

/// 종류별 라운드
pub enum Play {
    Summon(Round),
    Golf(GolfRound),
    Spar(Spar),
    Maze(MazeRound),
    Mouse(MouseRound),
}

impl Play {
    pub fn lesson_id(&self) -> &str {
        match self {
            Play::Summon(r) => &r.lesson_id,
            Play::Golf(r) => &r.lesson_id,
            Play::Spar(r) => &r.lesson_id,
            Play::Maze(r) => &r.lesson_id,
            Play::Mouse(r) => &r.lesson_id,
        }
    }

    pub fn started(&self) -> Instant {
        match self {
            Play::Summon(r) => r.started,
            Play::Golf(r) => r.started,
            Play::Spar(r) => r.started,
            Play::Maze(r) => r.started,
            Play::Mouse(r) => r.started,
        }
    }

    pub fn fail_streak(&self) -> u32 {
        match self {
            Play::Summon(r) => r.fail_streak,
            Play::Golf(r) => r.fail_streak,
            Play::Spar(r) => r.fail_streak,
            Play::Maze(r) => r.fail_streak,
            Play::Mouse(r) => r.fail_streak,
        }
    }

    pub fn total_mistakes(&self) -> u32 {
        match self {
            Play::Summon(r) => r.total_mistakes,
            Play::Golf(r) => r.total_mistakes,
            Play::Spar(r) => r.total_mistakes,
            Play::Maze(r) => r.total_mistakes,
            Play::Mouse(r) => r.total_mistakes,
        }
    }

    /// 화면 흔들림 — 마지막 실수 직후 잠깐
    pub fn flash(&self) -> Option<(bool, Instant)> {
        match self {
            Play::Summon(r) => r.flash,
            Play::Golf(r) => r.flash,
            Play::Spar(r) => r.flash,
            Play::Maze(r) => r.flash,
            Play::Mouse(r) => r.flash,
        }
    }

    pub fn clear_flash(&mut self) {
        match self {
            Play::Summon(r) => r.flash = None,
            Play::Golf(r) => r.flash = None,
            Play::Spar(r) => r.flash = None,
            Play::Maze(r) => r.flash = None,
            Play::Mouse(r) => r.flash = None,
        }
    }

    pub fn boss(&self) -> Option<&BossState> {
        match self {
            Play::Summon(r) => r.boss.as_ref(),
            Play::Golf(r) => r.boss.as_ref(),
            Play::Spar(r) => r.boss.as_ref(),
            Play::Maze(_) => None,
            Play::Mouse(r) => r.boss.as_ref(),
        }
    }

    /// 마우스 캡처가 필요한가
    pub fn wants_mouse(&self) -> bool {
        matches!(self, Play::Mouse(_))
    }

    /// 프레임 단위로 다시 그려야 하는가 (떨어지는 화살·달리는 쥐·리플레이)
    pub fn animating(&self, now: Instant) -> bool {
        match self {
            Play::Summon(r) => r.boss.is_some(),
            Play::Golf(r) => r.animating(now),
            Play::Spar(r) => r.animating(),
            Play::Maze(r) => r.animating(),
            Play::Mouse(r) => r.animating(),
        }
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        match self {
            Play::Summon(r) => r.tick(now),
            Play::Golf(_) => RoundEvent::None,
            Play::Spar(r) => r.tick(now),
            Play::Maze(r) => r.tick(now),
            Play::Mouse(r) => r.tick(now),
        }
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        match self {
            Play::Summon(r) => r.summary(cleared),
            Play::Golf(r) => r.summary(cleared),
            Play::Spar(r) => r.summary(cleared),
            Play::Maze(r) => r.summary(cleared),
            Play::Mouse(r) => r.summary(cleared),
        }
    }
}

/// 결정론적 의사 난수 — 같은 과는 같은 악보·과녁 배치 (공정한 기록 비교)
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: &str) -> Self {
        let mut h: u64 = 0x9e37_79b9_7f4a_7c15;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Self(h | 1)
    }

    pub fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// 0..n
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 난수는_씨앗이_같으면_같다() {
        let mut a = Rng::seeded("yellow.3");
        let mut b = Rng::seeded("yellow.3");
        let va: Vec<u64> = (0..5).map(|_| a.below(4)).collect();
        let vb: Vec<u64> = (0..5).map(|_| b.below(4)).collect();
        assert_eq!(va, vb);
        assert!(va.iter().all(|v| *v < 4));
    }

    #[test]
    fn 보스는_기가_다하면_진다() {
        let mut b = BossState::new(None);
        b.hit(30);
        assert_eq!(b.hp, 70);
        assert!(!b.hurt_me(60));
        assert!(b.hurt_me(40));
    }

    #[test]
    fn 등급() {
        assert_eq!(rank(1000).0, "S");
        assert_eq!(rank(820).0, "A");
        assert_eq!(rank(10).0, "D");
    }
}
