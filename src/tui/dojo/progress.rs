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
    /// 수련할 수 있는 과도 관문도 없다 (전부 공사 중)
    pub building: bool,
}

#[derive(Debug, Clone)]
pub struct Standing {
    pub belts: Vec<BeltState>,
    /// 지금 수련 중인 띠 (첫 미완료 띠). 전부 끝났으면 `belts.len()`.
    pub current: usize,
    /// 들어갈 수 있는 가장 먼 띠 — 공사 중인 띠는 길을 막지 않는다
    pub reach: usize,
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
    let mut stalled = None;
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
        st.completed =
            st.required > 0 && st.done == st.required && st.boss_ready && st.boss_cleared;
        st.building = st.required == 0 && !st.boss_ready;
        stars += st.stars;
        // 전부 공사 중인 띠는 완료로 치지 않지만 길을 막지도 않는다 — 다음 띠가
        // 열려 있으면 거기서 수련한다. 뒤에 수련할 띠가 하나도 없으면 첫 공사 중
        // 띠에서 멈춘다(검은띠로 건너뛰지 않게).
        if st.building {
            stalled.get_or_insert(i);
        } else if !st.completed && current.is_none() {
            current = Some(i);
        }
        belts.push(st);
    }
    let reach = current.unwrap_or(belts.len());
    Standing {
        current: current.or(stalled).unwrap_or(belts.len()),
        reach,
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
        if belt_idx > self.reach {
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
    use crate::tui::dojo::content::{content, Curriculum};

    /// 실제 커리큘럼에서 `belt` 띠만 공사 중으로 바꾼 사본 — 콘텐츠가 채워져도
    /// "공사 중 띠는 길을 막지 않는다" 규칙을 계속 시험한다.
    fn with_building(belt: &str) -> Curriculum {
        let mut c: Curriculum =
            toml::from_str(include_str!("../../../assets/dojo/curriculum.toml")).unwrap();
        let b = c.belt.iter_mut().find(|b| b.id == belt).unwrap();
        for l in b.lesson.iter_mut().chain(b.boss.iter_mut()) {
            l.kind = "wip".into();
        }
        c
    }

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
        assert_eq!(st.access(1, &yellow.lesson[0], ALL), Access::LockedBelt);
        let cur = with_building("green");
        let st = standing(&cur, &BTreeMap::new(), ALL);
        let green = &cur.belt[3];
        assert_eq!(st.access(3, &green.lesson[1], ALL), Access::Building);
    }

    #[test]
    fn 수가_없는_과는_미보유로_표시된다() {
        let c = content();
        let has = |a| a != Action::PageUp;
        let st = standing(&c.curriculum, &BTreeMap::new(), &has);
        let page = c.lesson("orange.3").unwrap().2;
        assert_eq!(
            st.access(0, page, &has),
            Access::Missing(vec![Action::PageUp])
        );
    }

    struct Wrap {
        curriculum: Curriculum,
    }

    /// 주황띠까지 끝내면 공사 중인 초록·파란띠를 건너 보라띠에서 수련한다
    #[test]
    fn 공사_중인_띠는_건너뛰고_끝에서는_멈춘다() {
        let mut cur = with_building("green");
        for b in cur.belt.iter_mut().filter(|b| b.id == "blue") {
            for l in b.lesson.iter_mut().chain(b.boss.iter_mut()) {
                l.kind = "wip".into();
            }
        }
        let c = Wrap { curriculum: cur };
        let mut entries = Vec::new();
        for b in &c.curriculum.belt[..3] {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                entries.push((l.id.as_str(), 1));
            }
        }
        let st = standing(&c.curriculum, &prog(&entries), ALL);
        assert_eq!(c.curriculum.belt[st.current].id, "purple");
        assert!(st.belts[3].building, "초록띠는 공사 중");

        for l in c.curriculum.belt[5]
            .lesson
            .iter()
            .chain(c.curriculum.belt[5].boss.iter())
        {
            entries.push((l.id.as_str(), 1));
        }
        let st = standing(&c.curriculum, &prog(&entries), ALL);
        assert_eq!(
            c.curriculum.belt[st.current].id, "green",
            "뒤에 수련할 띠가 없으면 첫 공사 중 띠에서 멈춘다"
        );
        let purple = &c.curriculum.belt[5].lesson[0];
        assert_eq!(
            st.access(5, purple, ALL),
            Access::Open,
            "끝낸 띠는 다시 할 수 있다"
        );
    }
}
