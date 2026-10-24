//! 이어 붙이기 — 여러 과의 라운드를 줄여 한 판으로 잇는다 (갈색띠·이탈마·오늘의 수련).
//!
//! 단계마다 판정은 빌려 온 라운드가 그대로 한다. 여기는 단계가 끝날 때마다 성적을
//! 모으고, 관문이면 보스 HP·내 기(氣)를 단계 성적으로 바꿔 준다. 다음 단계의
//! 라운드는 배치·보정·연습 목록을 아는 앱이 만들어 [`Gauntlet::push`] 로 넣는다.

use std::time::{Duration, Instant};

use super::content::{Content, Lesson, Stage};
use super::play::{BossState, Play, Rng, RoundEvent, Summary};
use super::score;

/// 실수 한 번에 깎이는 기
const KI_MISTAKE: i32 = 8;
/// 파를 넘긴 문항 하나에 깎이는 기
const KI_OVER_PAR: i32 = 10;

/// 끝난 단계 하나의 성적
pub struct StageResult {
    pub label: String,
    pub summary: Summary,
}

pub struct Gauntlet {
    pub lesson_id: String,
    pub stages: Vec<Lesson>,
    pub idx: usize,
    pub inner: Box<Play>,
    pub done: Vec<StageResult>,
    pub boss: Option<BossState>,
    pub started: Instant,
    /// 지금 단계가 시작된 순간 (단계 표지 연출)
    pub stage_at: Instant,
    /// 지금 단계가 끝났고 앱이 다음 라운드를 넣어 줘야 한다
    pub pending_next: bool,
}

impl Gauntlet {
    /// `first` 는 `stages[0]` 으로 만든 라운드
    pub fn new(lesson: &Lesson, stages: Vec<Lesson>, first: Play, now: Instant) -> Self {
        let boss = lesson.is_boss().then(|| {
            BossState::new(
                lesson
                    .time_limit_s
                    .map(|s| Duration::from_secs(u64::from(s))),
            )
        });
        Self {
            lesson_id: lesson.id.clone(),
            stages,
            idx: 0,
            inner: Box::new(first),
            done: Vec::new(),
            boss,
            started: now,
            stage_at: now,
            pending_next: false,
        }
    }

    pub fn stage(&self) -> &Lesson {
        &self.stages[self.idx]
    }

    /// 다음 단계 과 — 없으면 `None`
    pub fn next_stage(&self) -> Option<&Lesson> {
        self.stages.get(self.idx + 1)
    }

    /// 다음 단계 라운드를 넣는다
    pub fn push(&mut self, play: Play, now: Instant) {
        self.idx += 1;
        *self.inner = play;
        self.stage_at = now;
        self.pending_next = false;
    }

    /// 다음 단계를 만들 수 없을 때 — 건너뛰고, 마지막이었으면 끝낸다
    pub fn skip(&mut self) -> RoundEvent {
        self.idx += 1;
        self.pending_next = false;
        if self.idx + 1 >= self.stages.len() {
            self.idx = self.stages.len().saturating_sub(1);
            RoundEvent::Finished
        } else {
            self.pending_next = true;
            RoundEvent::None
        }
    }

    pub fn total_mistakes(&self) -> u32 {
        self.done.iter().map(|d| d.summary.mistakes).sum::<u32>() + self.inner.total_mistakes()
    }

    pub fn remaining_time(&self, now: Instant) -> Option<Duration> {
        let limit = self.boss.as_ref()?.time_limit?;
        Some(limit.saturating_sub(now.duration_since(self.started)))
    }

    /// 단계 표지를 보여 줄 때인가 (단계가 바뀐 직후)
    pub fn banner(&self, now: Instant) -> bool {
        now.duration_since(self.stage_at) < Duration::from_millis(1500)
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.inner.animating(now) || self.banner(now) || self.remaining_time(now).is_some()
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        if self.remaining_time(now) == Some(Duration::ZERO) {
            return RoundEvent::Failed;
        }
        let ev = self.inner.tick(now);
        self.absorb(ev, now)
    }

    /// 빌려 온 라운드의 사건을 이어 붙이기의 사건으로 바꾼다
    pub fn absorb(&mut self, ev: RoundEvent, now: Instant) -> RoundEvent {
        match ev {
            RoundEvent::Finished => {
                let summary = self.inner.summary(true);
                self.done.push(StageResult {
                    label: self.stage().title.clone(),
                    summary,
                });
                let n = self.stages.len() as i32;
                let left = n - self.done.len() as i32;
                if let Some(b) = &mut self.boss {
                    let hp = 100 * left / n;
                    b.hit(b.hp - hp);
                }
                if self.next_stage().is_some() {
                    self.pending_next = true;
                    let _ = now;
                    RoundEvent::Cleared {
                        over_par: false,
                        sage_equal: false,
                    }
                } else {
                    RoundEvent::Finished
                }
            }
            RoundEvent::Mistake => {
                if self.boss.as_mut().is_some_and(|b| b.hurt_me(KI_MISTAKE)) {
                    RoundEvent::Failed
                } else {
                    RoundEvent::Mistake
                }
            }
            RoundEvent::Cleared { over_par: true, .. } => {
                if self.boss.as_mut().is_some_and(|b| b.hurt_me(KI_OVER_PAR)) {
                    RoundEvent::Failed
                } else {
                    ev
                }
            }
            RoundEvent::Hit | RoundEvent::Cleared { .. } => {
                if let Some(b) = &mut self.boss {
                    b.hit(0);
                }
                ev
            }
            RoundEvent::Failed | RoundEvent::None => ev,
        }
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let n = self.done.len().max(1) as u32;
        let avg_score = self.done.iter().map(|d| d.summary.score).sum::<u32>() / n;
        let avg_stars = self
            .done
            .iter()
            .map(|d| u32::from(d.summary.stars))
            .sum::<u32>() as f32
            / n as f32;
        let stars = match (&self.boss, cleared) {
            (_, false) => 0,
            (Some(b), true) => score::boss_stars(b.ki),
            (None, true) => (avg_stars.round() as u8).clamp(1, 3),
        };
        let mut stats: Vec<(String, String)> = self
            .done
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let s = d.summary.stars.min(3) as usize;
                (
                    format!("{} {}", i + 1, d.label),
                    format!(
                        "{}{}  {}점",
                        "★".repeat(s),
                        "☆".repeat(3 - s),
                        d.summary.score
                    ),
                )
            })
            .collect();
        if !cleared {
            stats.push((
                format!("{} {}", self.done.len() + 1, self.stage().title),
                "— 여기서 멈췄다".into(),
            ));
        }
        Summary {
            stars,
            score: if cleared { avg_score } else { 0 },
            strokes: None,
            par: None,
            mistakes: self.total_mistakes(),
            hints: self.done.iter().map(|d| d.summary.hints).sum(),
            max_combo: self.done.iter().filter_map(|d| d.summary.max_combo).max(),
            rows: Vec::new(),
            stats,
            ki: self.boss.as_ref().map(|b| b.ki.max(0)),
        }
    }
}

/// 오늘의 수련 (docs/10 §4-7) — 날짜를 씨앗으로, 열린 과에서 골프 세 홀 + 대련 하나 +
/// 소환 하나를 뽑는다. 골프가 아직 안 열렸으면 그 자리는 소환이 채운다.
pub fn daily_stages(content: &Content, open: &dyn Fn(&Lesson) -> bool, day: &str) -> Vec<Lesson> {
    let mut rng = Rng::seeded(&format!("daily.{day}"));
    let pool = |kind: &str| -> Vec<&Lesson> {
        content
            .curriculum
            .belt
            .iter()
            .flat_map(|b| b.lesson.iter())
            .filter(|l| l.kind == kind && open(l))
            .collect()
    };
    let golf = pool("golf");
    let summon = pool("summon");
    // 대련은 카드만 — 화살비는 2분 안에 넣기엔 길다
    let spar: Vec<&Lesson> = pool("spar")
        .into_iter()
        .filter(|l| l.style.as_deref() != Some("rain"))
        .collect();

    let size = |l: &Lesson| -> usize {
        let set = l.set.as_deref().unwrap_or("");
        match l.kind.as_str() {
            "golf" => content.golf_course(set).map_or(1, |c| c.hole.len()),
            "summon" => content.summon_set(set).map_or(1, |s| s.requests.len()),
            _ => 1,
        }
    };
    let pick = |from: &[&Lesson], label: String, rng: &mut Rng| -> Option<Lesson> {
        let src = from.get(rng.below(from.len() as u64) as usize)?;
        let n = size(src) as u64;
        Some(src.staged(&Stage {
            from: src.id.clone(),
            label: Some(label),
            skip: Some(rng.below(n) as usize),
            take: Some(1),
            count: Some(8),
        }))
    };

    let mut out = Vec::new();
    for i in 1..=3 {
        let golf_stage = pick(&golf, format!("골프 {i}홀"), &mut rng);
        match golf_stage {
            Some(l) => out.push(l),
            None => out.extend(pick(&summon, format!("소환 {i}"), &mut rng)),
        }
    }
    out.extend(pick(&spar, "대련".into(), &mut rng));
    out.extend(pick(&summon, "소환".into(), &mut rng));
    // 소환이 여럿이면 번호를 다시 매긴다
    let summons = out.iter().filter(|l| l.kind == "summon").count();
    if summons > 1 {
        for (i, l) in out.iter_mut().filter(|l| l.kind == "summon").enumerate() {
            l.title = format!("소환 {}", i + 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;
    use crate::tui::dojo::spar::Spar;

    #[test]
    fn 오늘의_수련은_날마다_다르고_같은_날엔_같다() {
        let c = content();
        let all = |_: &Lesson| true;
        let a = daily_stages(c, &all, "2026-10-23");
        let b = daily_stages(c, &all, "2026-10-23");
        let d = daily_stages(c, &all, "2026-10-24");
        let ids = |v: &[Lesson]| v.iter().map(|l| (l.id.clone(), l.skip)).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
        assert_ne!(ids(&a), ids(&d));
        assert_eq!(a.len(), 5);
        assert_eq!(a.iter().filter(|l| l.kind == "golf").count(), 3);
        assert!(a.iter().all(|l| l.take == Some(1)));
    }

    #[test]
    fn 흰띠만_열렸으면_소환으로_채운다() {
        let c = content();
        let white = |l: &Lesson| l.id.starts_with("white.");
        let v = daily_stages(c, &white, "2026-10-23");
        assert_eq!(v.len(), 4, "골프 셋 자리를 소환이, 대련은 없다");
        assert!(v.iter().all(|l| l.kind == "summon"));
    }

    #[test]
    fn 관문은_단계가_끝날_때마다_꺾이고_실수는_기를_깎는다() {
        let c = content();
        let (_, _, boss) = c.lesson("brown.boss").unwrap();
        let stages: Vec<Lesson> = boss
            .stage
            .iter()
            .map(|st| c.lesson(&st.from).unwrap().2.staged(st))
            .collect();
        let now = Instant::now();
        let first = Play::Spar(Spar::new(&stages[0], now));
        let mut g = Gauntlet::new(boss, stages, first, now);
        assert_eq!(g.absorb(RoundEvent::Mistake, now), RoundEvent::Mistake);
        assert_eq!(g.boss.as_ref().unwrap().ki, 100 - KI_MISTAKE);
        let ev = g.absorb(RoundEvent::Finished, now);
        assert!(matches!(ev, RoundEvent::Cleared { .. }));
        assert!(g.pending_next);
        let hp = g.boss.as_ref().unwrap().hp;
        assert!(hp < 100 && hp > 0, "{hp}");
    }
}
