//! 미로 탈출 — 레이어 방향키만으로 격자를 빠져나간다 (docs/10 §6-4 미로).
//!
//! 엽전 ◆ 을 모두 주워야 문 門 이 열린다. 벽에 부딪히면 한 타가 날아가고 화면이
//! 흔들린다. 사부의 수 = (위치, 주운 엽전) 상태 위의 BFS 최단 경로.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::time::{Duration, Instant};

use super::action::Action;
use super::content::{Content, Lesson, MazeDef};
use super::play::{later_tag, OutcomeRow, RoundEvent, Summary};
use super::score::{self, HoleResult};

/// 탈출 연출 시간 — 끝나면 다음 미로로
const ESCAPE_PAUSE: Duration = Duration::from_millis(900);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Wall,
    Floor,
    Exit,
}

#[derive(Debug, Clone)]
pub struct Maze {
    pub title: String,
    pub grid: Vec<Vec<Tile>>,
    pub start: (usize, usize),
    pub coins: Vec<(usize, usize)>,
    /// 시야가 좁은 어둠 미로
    pub fog: bool,
}

impl Maze {
    pub fn parse(def: &MazeDef) -> Result<Self, String> {
        let mut grid = Vec::new();
        let mut start = None;
        let mut coins = Vec::new();
        let mut exit = false;
        for (y, line) in def.map.trim_end_matches('\n').lines().enumerate() {
            let mut row = Vec::new();
            for (x, ch) in line.chars().enumerate() {
                row.push(match ch {
                    '#' => Tile::Wall,
                    'S' => {
                        start = Some((x, y));
                        Tile::Floor
                    }
                    'E' => {
                        exit = true;
                        Tile::Exit
                    }
                    '$' => {
                        coins.push((x, y));
                        Tile::Floor
                    }
                    ' ' | '.' => Tile::Floor,
                    other => return Err(format!("{}: 모르는 칸 {other:?}", def.title)),
                });
            }
            grid.push(row);
        }
        if !exit {
            return Err(format!("{}: 문 E 가 없다", def.title));
        }
        if coins.len() > 6 {
            return Err(format!("{}: 엽전은 6개까지", def.title));
        }
        Ok(Self {
            title: def.title.clone(),
            grid,
            start: start.ok_or_else(|| format!("{}: 시작 S 가 없다", def.title))?,
            coins,
            fog: def.fog,
        })
    }

    pub fn tile(&self, x: usize, y: usize) -> Tile {
        self.grid
            .get(y)
            .and_then(|r| r.get(x))
            .copied()
            .unwrap_or(Tile::Wall)
    }

    pub fn width(&self) -> usize {
        self.grid.iter().map(Vec::len).max().unwrap_or(0)
    }

    fn step(&self, (x, y): (usize, usize), a: Action) -> Option<(usize, usize)> {
        let (nx, ny) = match a {
            Action::Left => (x.checked_sub(1)?, y),
            Action::Right => (x + 1, y),
            Action::Up => (x, y.checked_sub(1)?),
            Action::Down => (x, y + 1),
            _ => return None,
        };
        (self.tile(nx, ny) != Tile::Wall).then_some((nx, ny))
    }

    /// 최단 경로 — 엽전을 모두 줍고 문까지
    pub fn sage(&self) -> Option<Vec<Action>> {
        let full = (1u8 << self.coins.len()) - 1;
        let pick = |p: (usize, usize), mask: u8| -> u8 {
            self.coins
                .iter()
                .position(|c| *c == p)
                .map_or(mask, |i| mask | (1 << i))
        };
        let start = (self.start, pick(self.start, 0));
        let mut seen = HashSet::from([start]);
        let mut prev: Vec<((usize, usize), u8, usize, Action)> = Vec::new();
        let mut q = VecDeque::from([(start.0, start.1, usize::MAX)]);
        while let Some((p, mask, from)) = q.pop_front() {
            if mask == full && self.tile(p.0, p.1) == Tile::Exit {
                let mut path = Vec::new();
                let mut cur = from;
                while cur != usize::MAX {
                    let (_, _, back, a) = prev[cur];
                    path.push(a);
                    cur = back;
                }
                path.reverse();
                return Some(path);
            }
            for a in [Action::Left, Action::Down, Action::Up, Action::Right] {
                if let Some(n) = self.step(p, a) {
                    let m = pick(n, mask);
                    if seen.insert((n, m)) {
                        prev.push((n, m, from, a));
                        q.push_back((n, m, prev.len() - 1));
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Play,
    /// 탈출 연출 중
    Escaped(Instant),
}

pub struct MazeRound {
    pub lesson_id: String,
    pub mazes: Vec<Maze>,
    pub sages: Vec<Vec<Action>>,
    pub pars: Vec<u32>,
    pub idx: usize,
    pub pos: (usize, usize),
    pub coins_left: BTreeSet<(usize, usize)>,
    pub trail: HashSet<(usize, usize)>,
    pub strokes: u32,
    pub results: Vec<HoleResult>,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub hint_shown: bool,
    pub hints: u32,
    pub phase: Phase,
    pub started: Instant,
    pub flash: Option<(bool, Instant)>,
    pub feedback: Option<(String, Instant)>,
}

impl MazeRound {
    pub fn new(content: &Content, lesson: &Lesson, now: Instant) -> Option<Self> {
        let set = content.maze_set(lesson.set.as_deref()?)?;
        let mazes: Vec<Maze> = lesson
            .window(&set.maze)
            .iter()
            .map(Maze::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let sages: Vec<Vec<Action>> = mazes.iter().map(Maze::sage).collect::<Option<_>>()?;
        let pars = sages
            .iter()
            .map(|p| score::par_from_sage(p.len() as u32))
            .collect();
        let mut r = Self {
            lesson_id: lesson.id.clone(),
            pos: (0, 0),
            mazes,
            sages,
            pars,
            idx: 0,
            coins_left: BTreeSet::new(),
            trail: HashSet::new(),
            strokes: 0,
            results: Vec::new(),
            total_mistakes: 0,
            fail_streak: 0,
            hint_shown: false,
            hints: 0,
            phase: Phase::Play,
            started: now,
            flash: None,
            feedback: None,
        };
        r.load(0);
        Some(r)
    }

    fn load(&mut self, idx: usize) {
        self.idx = idx;
        let m = &self.mazes[idx];
        self.pos = m.start;
        self.coins_left = m.coins.iter().copied().collect();
        self.coins_left.remove(&m.start);
        self.trail = HashSet::from([m.start]);
        self.strokes = 0;
        self.hint_shown = false;
        self.phase = Phase::Play;
    }

    pub fn maze(&self) -> &Maze {
        &self.mazes[self.idx]
    }

    pub fn par(&self) -> u32 {
        self.pars[self.idx]
    }

    pub fn sage_len(&self) -> u32 {
        self.sages[self.idx].len() as u32
    }

    pub fn hint(&mut self) {
        if !self.hint_shown && self.phase == Phase::Play {
            self.hint_shown = true;
            self.hints += 1;
        }
    }

    /// 힌트로 보여 줄 사부의 길 (출발점 기준 — 지금 위치와 무관하게 정석을 보인다)
    pub fn sage_cells(&self) -> HashSet<(usize, usize)> {
        let m = self.maze();
        let mut p = m.start;
        let mut out = HashSet::new();
        for a in &self.sages[self.idx] {
            if let Some(n) = m.step(p, *a) {
                p = n;
                out.insert(p);
            }
        }
        out
    }

    /// 어둠 미로에서 보이는 칸인가
    pub fn visible(&self, x: usize, y: usize) -> bool {
        if !self.maze().fog {
            return true;
        }
        let dx = x.abs_diff(self.pos.0);
        let dy = y.abs_diff(self.pos.1);
        dx * dx + dy * dy * 4 <= 18 || self.trail.contains(&(x, y))
    }

    pub fn animating(&self) -> bool {
        matches!(self.phase, Phase::Escaped(_))
    }

    pub fn on_action(&mut self, a: Action, now: Instant) -> RoundEvent {
        if self.phase != Phase::Play {
            return RoundEvent::None;
        }
        if !matches!(a, Action::Left | Action::Down | Action::Up | Action::Right) {
            return RoundEvent::None;
        }
        self.strokes += 1;
        let Some(next) = self.maze().step(self.pos, a) else {
            // 쿵 — 벽
            self.total_mistakes += 1;
            self.fail_streak += 1;
            self.flash = Some((false, now));
            self.feedback = Some(("쿵! 벽이다 — 한 타가 날아갔다".into(), now));
            return RoundEvent::Mistake;
        };
        self.fail_streak = 0;
        self.pos = next;
        self.trail.insert(next);
        if self.coins_left.remove(&next) {
            self.feedback = Some((
                if self.coins_left.is_empty() {
                    "◆ 마지막 엽전 — 문이 열렸다!".into()
                } else {
                    format!("◆ 엽전! 남은 {}", self.coins_left.len())
                },
                now,
            ));
        }
        if self.maze().tile(next.0, next.1) == Tile::Exit {
            if !self.coins_left.is_empty() {
                self.feedback = Some(("문이 잠겼다 — 엽전을 모두 주워라".into(), now));
                return RoundEvent::None;
            }
            let hole = HoleResult {
                strokes: self.strokes,
                par: self.par(),
                sage: self.sage_len(),
                hinted: self.hint_shown,
            };
            let over_par = hole.strokes > hole.par;
            let sage_equal = hole.strokes <= hole.sage;
            self.results.push(hole);
            self.flash = Some((true, now));
            self.phase = Phase::Escaped(now);
            return RoundEvent::Cleared {
                over_par,
                sage_equal,
            };
        }
        RoundEvent::None
    }

    /// 일시정지한 만큼 시계를 민다
    pub fn shift(&mut self, d: Duration) {
        self.started += d;
        later_tag(&mut self.flash, d);
        later_tag(&mut self.feedback, d);
        if let Phase::Escaped(t) = &mut self.phase {
            *t += d;
        }
    }

    pub fn tick(&mut self, now: Instant) -> RoundEvent {
        if let Phase::Escaped(at) = self.phase {
            if now.duration_since(at) >= ESCAPE_PAUSE {
                if self.idx + 1 >= self.mazes.len() {
                    return RoundEvent::Finished;
                }
                self.load(self.idx + 1);
                return RoundEvent::Hit;
            }
        }
        RoundEvent::None
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let rows = self
            .mazes
            .iter()
            .enumerate()
            .map(|(i, m)| OutcomeRow {
                ask: m.title.clone(),
                sage: format!("{}걸음", self.sages[i].len()),
                sage_strokes: self.sages[i].len() as u32,
                par: self.pars[i],
                strokes: self.results.get(i).map(|h| h.strokes),
            })
            .collect();
        Summary::from_holes(
            &self.results,
            rows,
            cleared,
            self.total_mistakes,
            self.hints,
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    #[test]
    fn 모든_미로는_탈출할_수_있다() {
        for set in &content().maze.set {
            for def in &set.maze {
                let m = Maze::parse(def).unwrap_or_else(|e| panic!("{e}"));
                let path = m
                    .sage()
                    .unwrap_or_else(|| panic!("{}: 출구 없음", def.title));
                assert!(path.len() >= 6, "{}: 너무 짧다", def.title);
                assert!(m.width() <= 38, "{}: 화면(38칸)을 넘는다", def.title);
                assert!(m.grid.len() <= 13, "{}: 화면(13줄)을 넘는다", def.title);
            }
        }
    }

    #[test]
    fn 사부의_길대로_가면_엽전을_다_줍고_탈출한다() {
        let c = content();
        let lesson = c.lesson("yellow.2").unwrap().2;
        let now = Instant::now();
        let mut r = MazeRound::new(c, lesson, now).unwrap();
        let n = r.mazes.len();
        for i in 0..n {
            let path = r.sages[i].clone();
            let mut ev = RoundEvent::None;
            for a in path {
                ev = r.on_action(a, now);
            }
            assert!(matches!(
                ev,
                RoundEvent::Cleared {
                    sage_equal: true,
                    ..
                }
            ));
            let ev = r.tick(now + Duration::from_secs(1));
            if i + 1 == n {
                assert_eq!(ev, RoundEvent::Finished);
            }
        }
        assert_eq!(r.summary(true).stars, 3);
    }

    #[test]
    fn 벽은_한_타와_실수다() {
        let c = content();
        let lesson = c.lesson("yellow.2").unwrap().2;
        let now = Instant::now();
        let mut r = MazeRound::new(c, lesson, now).unwrap();
        // 사방 중 벽인 방향을 찾아 부딪힌다
        let wall = [Action::Left, Action::Down, Action::Up, Action::Right]
            .into_iter()
            .find(|a| r.maze().step(r.pos, *a).is_none())
            .unwrap();
        assert_eq!(r.on_action(wall, now), RoundEvent::Mistake);
        assert_eq!(r.strokes, 1);
        assert_eq!(r.total_mistakes, 1);
    }
}
