//! 띠 진행 — 저장된 과별 최고 기록에서 매번 계산한다 (docs/10 §8-2).
//! 띠는 저장하지 않는다(진실 소스 하나).

use std::collections::BTreeMap;

use kmd_core::dojo::DojoProgress;

use super::action::Action;
use super::content::{Curriculum, Lesson};

/// 과에 들어갈 수 있는가
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Open,
    /// 앞 띠를 먼저 넘어야 한다
    LockedBelt,
    /// 관문 — 이 띠의 과를 모두 ★1 이상 받아야 한다
    LockedBoss,
    /// 아직 구현되지 않은 수련 종류
    Building,
    /// 이 배치에 필요한 수가 없다 — 승급 조건에서 빠진다
    Missing(Vec<Action>),
}

#[derive(Debug, Clone, Default)]
pub struct BeltState {
    /// 승급에 필요한 과 (구현됐고 필요한 수가 있는 과)
    pub required: usize,
    /// 그중 ★1 이상
    pub done: usize,
    pub boss_ready: bool,
    pub boss_cleared: bool,
    pub stars: u32,
    pub completed: bool,
}

#[derive(Debug, Clone)]
pub struct Standing {
    pub belts: Vec<BeltState>,
    /// 지금 수련 중인 띠 (첫 미완료 띠). 전부 끝났으면 `belts.len()`.
    pub current: usize,
    pub stars: u32,
}

fn stars_of(progress: &BTreeMap<String, DojoProgress>, id: &str) -> u8 {
    progress.get(id).map_or(0, |p| p.best_stars)
}

fn missing(lesson: &Lesson, has: &dyn Fn(Action) -> bool) -> Vec<Action> {
    lesson
        .needed_actions()
        .into_iter()
        .filter(|a| !has(*a))
        .collect()
}

pub fn standing(
    cur: &Curriculum,
    progress: &BTreeMap<String, DojoProgress>,
    has: &dyn Fn(Action) -> bool,
) -> Standing {
    let mut belts = Vec::with_capacity(cur.belt.len());
    let mut current = None;
    let mut stars = 0;
    for (i, belt) in cur.belt.iter().enumerate() {
        let mut st = BeltState::default();
        for l in &belt.lesson {
            let s = stars_of(progress, &l.id);
            st.stars += u32::from(s);
            if l.is_ready() && missing(l, has).is_empty() {
                st.required += 1;
                if s > 0 {
                    st.done += 1;
                }
            }
        }
        if let Some(boss) = &belt.boss {
            st.boss_ready = boss.is_ready() && missing(boss, has).is_empty();
            st.boss_cleared = stars_of(progress, &boss.id) > 0;
        }
        // 수련할 수 있는 것이 하나도 없는 띠(공사 중)는 완료로 치지 않는다 — 거기서 멈춘다
        st.completed =
            st.required > 0 && st.done == st.required && st.boss_ready && st.boss_cleared;
        stars += st.stars;
        if !st.completed && current.is_none() {
            current = Some(i);
        }
        belts.push(st);
    }
    Standing {
        current: current.unwrap_or(belts.len()),
        belts,
        stars,
    }
}

impl Standing {
    pub fn access(&self, belt_idx: usize, lesson: &Lesson, has: &dyn Fn(Action) -> bool) -> Access {
        if !lesson.is_ready() {
            return Access::Building;
        }
        let miss = missing(lesson, has);
        if !miss.is_empty() {
            return Access::Missing(miss);
        }
        if belt_idx > self.current {
            return Access::LockedBelt;
        }
        if lesson.is_boss() {
            let st = &self.belts[belt_idx];
            if st.done < st.required {
                return Access::LockedBoss;
            }
        }
        Access::Open
    }

    /// 다음 띠까지 남은 과 수 (관문 포함)
    pub fn remaining(&self) -> Option<usize> {
        let st = self.belts.get(self.current)?;
        Some(st.required - st.done + usize::from(st.boss_ready && !st.boss_cleared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn prog(entries: &[(&str, u8)]) -> BTreeMap<String, DojoProgress> {
        entries
            .iter()
            .map(|(id, s)| {
                (
                    id.to_string(),
                    DojoProgress {
                        best_stars: *s,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    const ALL: &dyn Fn(Action) -> bool = &|_| true;

    #[test]
    fn 처음엔_흰띠_수련_중이고_관문은_잠겨_있다() {
        let c = content();
        let st = standing(&c.curriculum, &BTreeMap::new(), ALL);
        assert_eq!(st.current, 0);
        let white = &c.curriculum.belt[0];
        assert_eq!(st.access(0, &white.lesson[0], ALL), Access::Open);
        assert_eq!(
            st.access(0, white.boss.as_ref().unwrap(), ALL),
            Access::LockedBoss
        );
        assert_eq!(st.remaining(), Some(5));
    }

    #[test]
    fn 과를_모두_넘으면_관문이_열리고_격파하면_승급한다() {
        let c = content();
        let p = prog(&[
            ("white.1", 1),
            ("white.2", 3),
            ("white.3", 2),
            ("white.4", 1),
        ]);
        let st = standing(&c.curriculum, &p, ALL);
        assert_eq!(st.current, 0);
        assert_eq!(
            st.access(0, c.curriculum.belt[0].boss.as_ref().unwrap(), ALL),
            Access::Open
        );
        assert_eq!(st.remaining(), Some(1));

        let mut p = p;
        p.insert(
            "white.boss".into(),
            DojoProgress {
                best_stars: 1,
                ..Default::default()
            },
        );
        let st = standing(&c.curriculum, &p, ALL);
        assert_eq!(st.current, 1, "노란띠로 승급");
        assert_eq!(st.stars, 7);
    }

    #[test]
    fn 공사_중인_과와_다음_띠는_잠긴다() {
        let c = content();
        let st = standing(&c.curriculum, &BTreeMap::new(), ALL);
        let yellow = &c.curriculum.belt[1];
        assert_eq!(st.access(1, &yellow.lesson[0], ALL), Access::Building);
    }

    #[test]
    fn 수가_없는_과는_미보유로_표시된다() {
        let c = content();
        let st = standing(&c.curriculum, &BTreeMap::new(), &|a| a != Action::PageUp);
        let page = c.lesson("orange.3").unwrap().2;
        // 공사 중 판정이 먼저다 — 구현되면 Missing 이 된다
        assert_eq!(
            st.access(2, page, &|a| a != Action::PageUp),
            Access::Building
        );
        assert_eq!(
            missing(page, &|a| a != Action::PageUp),
            vec![Action::PageUp]
        );
    }
}
