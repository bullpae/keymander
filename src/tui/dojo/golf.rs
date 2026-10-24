//! 키 골프 — 에디터 모델·사부의 수(BFS)·홀 진행 (docs/10 §4-1, §7).
//!
//! 커서를 깃발 ⚑ 까지 옮기거나(이동 홀) 군더더기를 지운다(편집 홀). 타수는
//! **도착한 결과 키 이벤트 수**다 — 2탭 수(줄 처음)는 첫 탭의 단어← 와 함께
//! 두 이벤트로 도착하므로 2타다.

use std::time::{Duration, Instant};

use super::action::{Action, Os};
use super::content::{Content, GolfHole, Lesson};
use super::play::{BossState, OutcomeRow, RoundEvent, Summary};
use super::score::{self, HoleResult};

/// 홀 화면에 보이는 줄 수
pub const VIEW: usize = 9;
/// 리플레이 한 수의 간격
pub const REPLAY_STEP: Duration = Duration::from_millis(280);
/// BFS 상태 상한 — 홀 데이터가 너무 크면 테스트가 잡는다
const BFS_LIMIT: usize = 400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub ch: char,
    /// 지워야 하는 군더더기
    pub junk: bool,
    /// 군더더기 번호 (사부의 수 탐색 상태의 비트)
    jid: u8,
}

/// 에디터 버퍼 — 줄 목록 + 커서 + sticky 열
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Buffer {
    pub lines: Vec<Vec<Cell>>,
    pub row: usize,
    pub col: usize,
    /// ↑↓ 이동 때 기억하는 원래 열
    sticky: Option<usize>,
    /// 선택의 닻 (선형 위치) — 커서와 닻 사이가 선택 영역이다
    pub anchor: Option<usize>,
}

/// 군더더기가 아닌 글자를 지우려 했다
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotJunk;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Buffer {
    fn len(&self, row: usize) -> usize {
        self.lines[row].len()
    }

    fn last_row(&self) -> usize {
        self.lines.len() - 1
    }

    pub fn junk_left(&self) -> usize {
        self.lines.iter().flatten().filter(|c| c.junk).count()
    }

    /// (row, col) → 줄바꿈을 한 글자로 친 선형 위치
    fn linear_at(&self, row: usize, col: usize) -> usize {
        self.lines[..row].iter().map(|l| l.len() + 1).sum::<usize>() + col
    }

    fn pos_at(&self, mut i: usize) -> (usize, usize) {
        for (r, l) in self.lines.iter().enumerate() {
            if i <= l.len() {
                return (r, i);
            }
            i -= l.len() + 1;
        }
        let r = self.last_row();
        (r, self.len(r))
    }

    fn linear_end(&self) -> usize {
        self.linear_at(self.last_row(), self.len(self.last_row()))
    }

    /// 선형 위치의 글자 — 줄 끝은 '\n'
    fn char_at(&self, i: usize) -> char {
        let (r, c) = self.pos_at(i);
        self.lines[r].get(c).map_or('\n', |cell| cell.ch)
    }

    fn vertical(&mut self, row: usize) {
        let want = self.sticky.unwrap_or(self.col);
        self.row = row;
        self.col = want.min(self.len(row));
        self.sticky = Some(want);
    }

    pub fn cursor_linear(&self) -> usize {
        self.linear_at(self.row, self.col)
    }

    /// 선택 영역 `[시작, 끝)` (선형). 비었으면 `None`.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let here = self.cursor_linear();
        let a = self.anchor?;
        (a != here).then(|| (a.min(here), a.max(here)))
    }

    /// 수 하나를 적용한다. 군더더기가 아닌 글자 삭제는 거절한다(버퍼는 그대로).
    ///
    /// 선택 수(Shift+이동)는 닻을 내리고 커서만 옮긴다. 선택이 있을 때 ←/→ 는
    /// 선택의 앞/뒤로 접히고, 지우기는 선택 영역을 통째로 지운다(전부 군더더기일 때만).
    pub fn apply(&mut self, a: Action, os: Os, page: usize) -> Result<(), NotJunk> {
        if let Some(base) = a.unshifted() {
            let anchor = self.anchor.unwrap_or_else(|| self.cursor_linear());
            self.step(base, os, page)?;
            self.anchor = (anchor != self.cursor_linear()).then_some(anchor);
            return Ok(());
        }
        let Some((lo, hi)) = self.selection() else {
            self.anchor = None;
            return self.step(a, os, page);
        };
        match a {
            Action::Left | Action::Right => {
                let to = if a == Action::Left { lo } else { hi };
                (self.row, self.col) = self.pos_at(to);
                self.sticky = None;
            }
            Action::Backspace | Action::Delete => {
                let (r, c0) = self.pos_at(lo);
                let (r1, c1) = self.pos_at(hi);
                if r != r1 || !self.lines[r][c0..c1].iter().all(|c| c.junk) {
                    return Err(NotJunk);
                }
                self.lines[r].drain(c0..c1);
                (self.row, self.col) = (r, c0);
                self.sticky = None;
            }
            _ => self.step(a, os, page)?,
        }
        self.anchor = None;
        Ok(())
    }

    fn step(&mut self, a: Action, os: Os, page: usize) -> Result<(), NotJunk> {
        let keep_sticky = matches!(
            a,
            Action::Up | Action::Down | Action::PageUp | Action::PageDown
        );
        match a {
            Action::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.len(self.row);
                }
            }
            Action::Right => {
                if self.col < self.len(self.row) {
                    self.col += 1;
                } else if self.row < self.last_row() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            Action::Up if self.row > 0 => self.vertical(self.row - 1),
            Action::Down if self.row < self.last_row() => self.vertical(self.row + 1),
            Action::PageUp => self.vertical(self.row.saturating_sub(page)),
            Action::PageDown => self.vertical((self.row + page).min(self.last_row())),
            Action::LineStart => self.col = 0,
            Action::LineEnd => self.col = self.len(self.row),
            Action::WordLeft => {
                let mut i = self.linear_at(self.row, self.col);
                while i > 0 && !is_word(self.char_at(i - 1)) {
                    i -= 1;
                }
                while i > 0 && is_word(self.char_at(i - 1)) {
                    i -= 1;
                }
                (self.row, self.col) = self.pos_at(i);
            }
            Action::WordRight => {
                let end = self.linear_end();
                let mut i = self.linear_at(self.row, self.col);
                if os == Os::Windows {
                    // Windows: 다음 단어의 시작
                    while i < end && is_word(self.char_at(i)) {
                        i += 1;
                    }
                    while i < end && !is_word(self.char_at(i)) {
                        i += 1;
                    }
                } else {
                    // macOS · Linux(GTK): 현재/다음 단어의 끝
                    while i < end && !is_word(self.char_at(i)) {
                        i += 1;
                    }
                    while i < end && is_word(self.char_at(i)) {
                        i += 1;
                    }
                }
                (self.row, self.col) = self.pos_at(i);
            }
            Action::Backspace => {
                if self.col == 0 {
                    // 줄 합치기 — 줄바꿈은 군더더기가 아니다
                    if self.row > 0 {
                        return Err(NotJunk);
                    }
                } else if self.lines[self.row][self.col - 1].junk {
                    self.lines[self.row].remove(self.col - 1);
                    self.col -= 1;
                } else {
                    return Err(NotJunk);
                }
            }
            Action::Delete => {
                if self.col >= self.len(self.row) {
                    if self.row < self.last_row() {
                        return Err(NotJunk);
                    }
                } else if self.lines[self.row][self.col].junk {
                    self.lines[self.row].remove(self.col);
                } else {
                    return Err(NotJunk);
                }
            }
            _ => {}
        }
        if !keep_sticky {
            self.sticky = None;
        }
        Ok(())
    }
}

/// 홀 — 파싱된 버퍼와 목표
#[derive(Debug, Clone)]
pub struct Hole {
    pub title: String,
    pub start: Buffer,
    /// 이동 홀의 깃발
    pub flag: Option<(usize, usize)>,
    /// 선택 홀의 과녁 — 이 영역을 정확히 선택하면 홀 아웃 (선형 `[시작, 끝)`)
    pub target: Option<(usize, usize)>,
    pub moves: Vec<Action>,
    pub par: Option<u32>,
    pub page: usize,
    /// 군더더기 덩어리(«…» 하나)마다 그 글자들의 비트
    runs: Vec<u32>,
}

impl Hole {
    /// 홀 텍스트 표기: `‸` 시작 커서, `⚑` 깃발, `«…»` 군더더기
    pub fn parse(h: &GolfHole, moves: &[String], page: usize) -> Result<Self, String> {
        let mut lines = Vec::new();
        let mut start = None;
        let mut flag = None;
        let mut mark = (None, None);
        let mut junk = false;
        let mut jid = 0u8;
        let mut runs: Vec<u32> = Vec::new();
        for (r, raw) in h.text.trim_end_matches('\n').lines().enumerate() {
            let mut line = Vec::new();
            for ch in raw.chars() {
                match ch {
                    '‸' => start = Some((r, line.len())),
                    '⚑' => flag = Some((r, line.len())),
                    '⟦' => mark.0 = Some((r, line.len())),
                    '⟧' => mark.1 = Some((r, line.len())),
                    '«' => {
                        junk = true;
                        runs.push(0);
                    }
                    '»' => junk = false,
                    _ => {
                        line.push(Cell { ch, junk, jid });
                        if junk {
                            if let Some(r) = runs.last_mut() {
                                *r |= 1 << jid;
                            }
                            jid += 1;
                        }
                    }
                }
            }
            lines.push(line);
        }
        let (row, col) = start.ok_or_else(|| format!("{}: 시작 커서 ‸ 가 없다", h.title))?;
        let moves = h
            .moves
            .as_deref()
            .unwrap_or(moves)
            .iter()
            .map(|m| Action::from_id(m).ok_or_else(|| format!("{}: 모르는 수 {m}", h.title)))
            .collect::<Result<Vec<_>, _>>()?;
        let buf = Buffer {
            lines,
            row,
            col,
            sticky: None,
            anchor: None,
        };
        let target = match mark {
            (Some((r0, c0)), Some((r1, c1))) => {
                Some((buf.linear_at(r0, c0), buf.linear_at(r1, c1)))
            }
            (None, None) => None,
            _ => return Err(format!("{}: 과녁 ⟦⟧ 짝이 맞지 않는다", h.title)),
        };
        if target.is_some() && (flag.is_some() || buf.junk_left() > 0) {
            return Err(format!(
                "{}: 과녁 홀에는 깃발·군더더기를 두지 않는다",
                h.title
            ));
        }
        if flag.is_none() && target.is_none() && buf.junk_left() == 0 {
            return Err(format!("{}: 깃발도 군더더기도 과녁도 없다", h.title));
        }
        if jid > 16 {
            return Err(format!("{}: 군더더기는 16글자까지", h.title));
        }
        Ok(Self {
            title: h.title.clone(),
            start: buf,
            flag,
            target,
            moves,
            par: h.par,
            page: h.page.unwrap_or(page),
            runs,
        })
    }

    /// 지금 커서에 닻을 내려 쓸모가 있는가 — 사부의 수 탐색 가지치기.
    /// 선택은 과녁의 양 끝, 또는 군더더기 덩어리의 경계에서 시작해야만 득이 된다
    /// (군더더기가 아닌 글자가 섞인 선택은 지울 수 없다).
    fn anchor_useful(&self, buf: &Buffer) -> bool {
        let here = buf.cursor_linear();
        if let Some((a, b)) = self.target {
            return here == a || here == b;
        }
        let line = &buf.lines[buf.row];
        line.get(buf.col).is_some_and(|c| c.junk) || (buf.col > 0 && line[buf.col - 1].junk)
    }

    pub fn done(&self, buf: &Buffer) -> bool {
        buf.junk_left() == 0
            && self.flag.is_none_or(|f| (buf.row, buf.col) == f)
            && self.target.is_none_or(|t| buf.selection() == Some(t))
    }
}

/// 수 하나를 누르면 도착하는 이벤트들 — 2탭 수는 `[단어←, 줄 처음]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub action: Action,
    pub seq: Vec<Action>,
}

impl Edge {
    pub fn single(a: Action) -> Self {
        Self {
            action: a,
            seq: vec![a],
        }
    }
}

/// 사부의 수 — 최소 타수 경로
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagePath {
    pub moves: Vec<Action>,
    pub strokes: u32,
}

impl SagePath {
    pub fn display(&self) -> String {
        if self.moves.is_empty() {
            return "—".into();
        }
        self.moves
            .iter()
            .map(|a| a.glyph())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 탐색 상태 — 남은 군더더기 비트 + 커서. 버퍼 통째 대신 이것만 해시한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    mask: u32,
    row: u16,
    col: u16,
    sticky: u16,
    anchor: u16,
}

const NO_STICKY: u16 = u16::MAX;

impl Buffer {
    fn junk_mask(&self) -> u32 {
        self.lines
            .iter()
            .flatten()
            .filter(|c| c.junk)
            .fold(0, |m, c| m | (1 << c.jid))
    }

    fn key(&self, junky: bool) -> Key {
        Key {
            mask: if junky { self.junk_mask() } else { 0 },
            row: self.row as u16,
            col: self.col as u16,
            sticky: self.sticky.map_or(NO_STICKY, |s| s as u16),
            anchor: self.anchor.map_or(NO_STICKY, |s| s as u16),
        }
    }

    /// 시작 버퍼에서 `key` 상태를 다시 만든다. 군더더기가 없으면 커서만 옮긴다.
    fn load(&mut self, start: &Buffer, k: Key, junky: bool) {
        if junky {
            self.lines = start
                .lines
                .iter()
                .map(|l| {
                    l.iter()
                        .filter(|c| !c.junk || k.mask & (1 << c.jid) != 0)
                        .copied()
                        .collect()
                })
                .collect();
        }
        self.row = usize::from(k.row);
        self.col = usize::from(k.col);
        self.sticky = (k.sticky != NO_STICKY).then_some(usize::from(k.sticky));
        self.anchor = (k.anchor != NO_STICKY).then_some(usize::from(k.anchor));
    }
}

#[cfg(test)]
thread_local! {
    static EXPANDED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 사부의 수를 구한다 (docs/10 §7-2). 간선 비용 = 도착 이벤트 수(1 또는 2)라
/// 순수 BFS 대신 다익스트라를 쓴다. 상태는 (남은 군더더기, 커서, 닻) 으로 압축하고,
/// 이동 홀은 글자가 바뀌지 않으니 버퍼를 복제하지 않고 커서만 옮겨 본다.
///
/// 군더더기 덩어리가 여럿이면 부분 삭제 조합이 곱으로 늘어난다. 덩어리 둘을
/// 번갈아 조금씩 지우는 수는 득이 없으니, **덜 지운 덩어리는 한 번에 하나**만
/// 허용해 상태를 줄인다. 같은 타수면 선택 수를 덜 쓴 길을 고른다(보이는 수가 자연스럽다).
pub fn sage(hole: &Hole, edges: &[Edge], os: Os) -> Option<SagePath> {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};

    if hole.done(&hole.start) {
        return Some(SagePath {
            moves: Vec::new(),
            strokes: 0,
        });
    }
    let junky = hole.start.junk_left() > 0;
    let start = hole.start.key(junky);
    // 덜 지운 덩어리는 하나만, 그리고 남은 부분이 이어져 있어야 한다
    // (가운데를 파내 두 조각으로 남기는 수는 득이 없다)
    let partial_ok = |mask: u32| {
        let mut partial = 0;
        for r in &hole.runs {
            let left = mask & *r;
            if left != 0 && left != *r {
                partial += 1;
                let bits = left >> left.trailing_zeros();
                if partial > 1 || bits & (bits + 1) != 0 {
                    return false;
                }
            }
        }
        true
    };
    // A* 하한 — 남은 군더더기 덩어리마다 적어도 한 타(지우기)는 든다
    let h = |mask: u32| hole.runs.iter().filter(|r| mask & **r != 0).count() as u32;
    // 비용 = (타수, 선택 수 사용 횟수) — 사전순. 힙에는 (타수 + 하한) 으로 넣는다.
    let mut scratch = hole.start.clone();
    let mut base = hole.start.clone();
    let mut dist: HashMap<Key, (u32, u32)> = HashMap::from([(start, (0, 0))]);
    let mut prev: HashMap<Key, (Key, Action)> = HashMap::new();
    let mut heap = BinaryHeap::from([Reverse(((h(start.mask), 0u32), start))]);
    while let Some(Reverse((f, k))) = heap.pop() {
        let cost = dist[&k];
        if f != (cost.0 + h(k.mask), cost.1) {
            continue; // 더 싼 길로 이미 갱신된 낡은 항목
        }
        base.load(&hole.start, k, junky);
        if hole.done(&base) {
            let mut moves = Vec::new();
            let mut cur = k;
            while let Some((p, a)) = prev.get(&cur) {
                moves.push(*a);
                cur = *p;
            }
            moves.reverse();
            return Some(SagePath {
                moves,
                strokes: cost.0,
            });
        }
        if dist.len() > BFS_LIMIT {
            return None;
        }
        #[cfg(test)]
        EXPANDED.with(|c| c.set(c.get() + 1));
        for e in edges {
            // 이동만 하는 수는 글자를 바꾸지 않는다 — 복제 없이 커서만 옮겼다 되돌린다
            let edits = e
                .seq
                .iter()
                .any(|a| matches!(a, Action::Backspace | Action::Delete));
            base.load(&hole.start, k, false);
            let buf = if edits {
                scratch.clone_from(&base);
                &mut scratch
            } else {
                &mut base
            };
            if e.seq.iter().any(|a| buf.apply(*a, os, hole.page).is_err()) {
                continue;
            }
            let next = if edits {
                buf.key(junky)
            } else {
                Key {
                    mask: k.mask,
                    ..buf.key(false)
                }
            };
            if next == k || (next.mask != k.mask && !partial_ok(next.mask)) {
                continue;
            }
            if k.anchor == NO_STICKY && next.anchor != NO_STICKY {
                base.load(&hole.start, k, false);
                if !hole.anchor_useful(&base) {
                    continue;
                }
            }
            let nc = (
                cost.0 + e.seq.len() as u32,
                cost.1 + u32::from(e.action.unshifted().is_some()),
            );
            if dist.get(&next).is_none_or(|d| nc < *d) {
                dist.insert(next, nc);
                prev.insert(next, (k, e.action));
                heap.push(Reverse(((nc.0 + h(next.mask), nc.1), next)));
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Play,
    /// 홀 아웃 — 결과 오버레이. `replay`면 사부의 수를 다시 보여 주는 중
    HoleOut {
        replay: Option<Instant>,
    },
}

/// 코스 한 판
pub struct GolfRound {
    pub lesson_id: String,
    pub course_name: String,
    pub holes: Vec<Hole>,
    pub sages: Vec<SagePath>,
    pub pars: Vec<u32>,
    /// 홀별 허용 수 (배치·관측 가능 여부 반영)
    pub allowed: Vec<Vec<Edge>>,
    pub idx: usize,
    pub buf: Buffer,
    pub used: Vec<Action>,
    pub strokes: u32,
    pub results: Vec<HoleResult>,
    /// 홀별 내 수 기록 (스코어카드·리플레이 비교)
    pub my_moves: Vec<Vec<Action>>,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub hint_shown: bool,
    pub hints: u32,
    pub phase: Phase,
    pub boss: Option<BossState>,
    pub started: Instant,
    pub flash: Option<(bool, Instant)>,
    /// 화면 아래 한 줄 피드백
    pub feedback: Option<(String, Instant)>,
    /// 이 터미널에서 관측 불가라 뺀 수
    pub unobservable: Vec<Action>,
    os: Os,
}

impl GolfRound {
    /// `edge_for`: 배치에서 그 수를 누르는 법(2탭이면 이벤트 둘). 없거나 관측 불가면 `None`.
    pub fn new(
        content: &Content,
        lesson: &Lesson,
        edge_for: &dyn Fn(Action) -> Option<Edge>,
        unobservable: Vec<Action>,
        now: Instant,
    ) -> Option<Self> {
        let course = content.golf_course(lesson.set.as_deref()?)?;
        let os = Os::current();
        let holes: Vec<Hole> = lesson
            .window(&course.hole)
            .iter()
            .map(|h| Hole::parse(h, &course.moves, course.page.unwrap_or(8)))
            .collect::<Result<_, _>>()
            .ok()?;
        let mut allowed = Vec::new();
        let mut sages = Vec::new();
        let mut pars = Vec::new();
        for h in &holes {
            let mut edges: Vec<Edge> = h.moves.iter().filter_map(|a| edge_for(*a)).collect();
            // 기본 이동이 하나도 없으면 풀 수 없다 — 화살표는 늘 쓸 수 있다고 본다
            for a in [Action::Left, Action::Right, Action::Up, Action::Down] {
                if h.moves.contains(&a) && !edges.iter().any(|e| e.action == a) {
                    edges.push(Edge::single(a));
                }
            }
            let path = sage(h, &edges, os)?;
            pars.push(
                h.par
                    .unwrap_or_else(|| score::par_from_sage(path.strokes))
                    .max(path.strokes),
            );
            sages.push(path);
            allowed.push(edges);
        }
        let boss = lesson.is_boss().then(|| BossState::new(None));
        Some(Self {
            lesson_id: lesson.id.clone(),
            course_name: course.name.clone(),
            buf: holes[0].start.clone(),
            holes,
            sages,
            pars,
            allowed,
            idx: 0,
            used: Vec::new(),
            strokes: 0,
            results: Vec::new(),
            my_moves: Vec::new(),
            total_mistakes: 0,
            fail_streak: 0,
            hint_shown: false,
            hints: 0,
            phase: Phase::Play,
            boss,
            started: now,
            flash: None,
            feedback: None,
            unobservable,
            os,
        })
    }

    pub fn hole(&self) -> &Hole {
        &self.holes[self.idx]
    }

    pub fn sage(&self) -> &SagePath {
        &self.sages[self.idx]
    }

    pub fn par(&self) -> u32 {
        self.pars[self.idx]
    }

    fn arrivals(&self) -> Vec<Action> {
        let mut v: Vec<Action> = self.allowed[self.idx]
            .iter()
            .flat_map(|e| e.seq.iter().copied())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn hint(&mut self) {
        if !self.hint_shown && self.phase == Phase::Play {
            self.hint_shown = true;
            self.hints += 1;
        }
    }

    /// 홀을 처음부터 — 타수는 0으로, 대신 실수 하나
    pub fn retry(&mut self, now: Instant) {
        if self.phase != Phase::Play || self.strokes == 0 {
            return;
        }
        self.buf = self.hole().start.clone();
        self.used.clear();
        self.strokes = 0;
        self.total_mistakes += 1;
        self.feedback = Some(("다시 친다 — 실수 하나".into(), now));
    }

    /// 도착한 수 하나
    pub fn on_action(&mut self, a: Action, now: Instant) -> RoundEvent {
        if self.phase != Phase::Play {
            return RoundEvent::None;
        }
        if !self.arrivals().contains(&a) {
            self.feedback = Some((format!("{} — 이 홀에서는 쓰지 않는 수다", a.label()), now));
            return RoundEvent::None;
        }
        self.strokes += 1;
        self.used.push(a);
        let page = self.hole().page;
        let had_selection = self.buf.selection().is_some();
        if self.buf.apply(a, self.os, page).is_err() {
            self.total_mistakes += 1;
            self.fail_streak += 1;
            self.flash = Some((false, now));
            let msg = if had_selection {
                "선택에 성한 글자가 섞였다 — 칼이 튕긴다"
            } else {
                "군더더기가 아니다 — 그 글자는 살린다"
            };
            self.feedback = Some((msg.into(), now));
            if let Some(b) = &mut self.boss {
                if b.hurt_me(5) {
                    return RoundEvent::Failed;
                }
            }
            return RoundEvent::Mistake;
        }
        if self.hole().done(&self.buf) {
            return self.hole_out(now);
        }
        RoundEvent::None
    }

    fn hole_out(&mut self, now: Instant) -> RoundEvent {
        let hole = HoleResult {
            strokes: self.strokes,
            par: self.par(),
            sage: self.sage().strokes,
            hinted: self.hint_shown,
        };
        let over_par = hole.strokes > hole.par;
        let sage_equal = hole.strokes <= hole.sage;
        let over = hole.strokes.saturating_sub(hole.par) as i32;
        self.results.push(hole);
        self.my_moves.push(std::mem::take(&mut self.used));
        self.fail_streak = 0;
        self.flash = Some((true, now));
        self.phase = Phase::HoleOut { replay: None };
        let total = self.holes.len() as i32;
        if let Some(b) = &mut self.boss {
            b.hit(100 / total + i32::from(sage_equal) * 2);
            if self.results.len() as i32 == total {
                b.hp = 0;
            }
            if over > 0 && b.hurt_me(8 * over) {
                return RoundEvent::Failed;
            }
        }
        RoundEvent::Cleared {
            over_par,
            sage_equal,
        }
    }

    /// 홀 아웃 오버레이에서 Enter
    pub fn next(&mut self) -> RoundEvent {
        if !matches!(self.phase, Phase::HoleOut { .. }) {
            return RoundEvent::None;
        }
        if self.idx + 1 >= self.holes.len() {
            return RoundEvent::Finished;
        }
        self.idx += 1;
        self.buf = self.hole().start.clone();
        self.strokes = 0;
        self.hint_shown = false;
        self.phase = Phase::Play;
        RoundEvent::None
    }

    pub fn replay(&mut self, now: Instant) {
        if let Phase::HoleOut { replay } = &mut self.phase {
            *replay = Some(now);
        }
    }

    /// 리플레이 중이면 사부의 수를 k 수까지 적용한 버퍼
    pub fn replay_buffer(&self, now: Instant) -> Option<(Buffer, usize)> {
        let Phase::HoleOut {
            replay: Some(start),
        } = self.phase
        else {
            return None;
        };
        // 홀 아웃 직후 idx 는 아직 그 홀이다
        let hole = self.hole();
        let k = (now.duration_since(start).as_millis() / REPLAY_STEP.as_millis()) as usize;
        let mut buf = hole.start.clone();
        let edges = &self.allowed[self.idx];
        let path = &self.sage().moves;
        for a in path.iter().take(k) {
            if let Some(e) = edges.iter().find(|e| e.action == *a) {
                for s in &e.seq {
                    let _ = buf.apply(*s, self.os, hole.page);
                }
            }
        }
        Some((buf, k.min(path.len())))
    }

    pub fn animating(&self, now: Instant) -> bool {
        matches!(self.replay_buffer(now), Some((_, k)) if k < self.sage().moves.len() + 1)
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let rows = self
            .holes
            .iter()
            .enumerate()
            .map(|(i, h)| OutcomeRow {
                ask: format!("{}번 홀 · {}", i + 1, h.title),
                sage: self.sages[i].display(),
                sage_strokes: self.sages[i].strokes,
                par: self.pars[i],
                strokes: self.results.get(i).map(|r| r.strokes),
            })
            .collect();
        Summary::from_holes(
            &self.results,
            rows,
            cleared,
            self.total_mistakes,
            self.hints,
            self.boss.as_ref(),
        )
    }

    /// 재사용 — 버퍼 선형 위치 계산이 화면에도 필요하다
    pub fn view_top(&self, buf: &Buffer) -> usize {
        let n = buf.lines.len();
        if n <= VIEW {
            return 0;
        }
        buf.row.saturating_sub(VIEW / 2).min(n - VIEW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;

    fn hole(text: &str, moves: &[&str]) -> Hole {
        let h = GolfHole {
            title: "t".into(),
            text: text.into(),
            moves: None,
            par: None,
            page: None,
        };
        let m: Vec<String> = moves.iter().map(|s| s.to_string()).collect();
        Hole::parse(&h, &m, 8).unwrap()
    }

    fn edges(moves: &[Action]) -> Vec<Edge> {
        moves.iter().map(|a| Edge::single(*a)).collect()
    }

    #[test]
    fn 단어_이동은_os_규칙을_따른다() {
        let h = hole("‸let config = load()⚑", &["Left"]);
        let mut mac = h.start.clone();
        mac.apply(Action::WordRight, Os::Mac, 8).unwrap();
        assert_eq!(mac.col, 3, "macOS: 단어의 끝");
        mac.apply(Action::WordRight, Os::Mac, 8).unwrap();
        assert_eq!(mac.col, 10);
        let mut win = h.start.clone();
        win.apply(Action::WordRight, Os::Windows, 8).unwrap();
        assert_eq!(win.col, 4, "Windows: 다음 단어의 시작");
        let mut back = mac.clone();
        back.apply(Action::WordLeft, Os::Mac, 8).unwrap();
        assert_eq!(back.col, 4);
    }

    #[test]
    fn 위아래는_sticky_열을_기억한다() {
        let h = hole("abcdef‸g\nab\nabcdefgh⚑", &["Left"]);
        let mut b = h.start.clone();
        b.apply(Action::Down, Os::Mac, 8).unwrap();
        assert_eq!((b.row, b.col), (1, 2));
        b.apply(Action::Down, Os::Mac, 8).unwrap();
        assert_eq!((b.row, b.col), (2, 6));
    }

    #[test]
    fn 군더더기가_아닌_글자는_지워지지_않는다() {
        let h = hole("ab«x»‸c", &["Left"]);
        let mut b = h.start.clone();
        assert_eq!(b.apply(Action::Delete, Os::Mac, 8), Err(NotJunk));
        b.apply(Action::Backspace, Os::Mac, 8).unwrap();
        assert_eq!(b.junk_left(), 0);
        assert_eq!(b.apply(Action::Backspace, Os::Mac, 8), Err(NotJunk));
    }

    #[test]
    fn 선택은_닻과_커서_사이다() {
        let h = hole("‸ab ⟦cd⟧", &["Left"]);
        let mut b = h.start.clone();
        b.apply(Action::Right, Os::Mac, 8).unwrap();
        b.apply(Action::SelRight, Os::Mac, 8).unwrap();
        assert_eq!(b.selection(), Some((1, 2)));
        b.apply(Action::SelLeft, Os::Mac, 8).unwrap();
        assert_eq!(b.selection(), None, "닻으로 돌아오면 선택이 사라진다");
        // 과녁 cd 를 단어 선택으로
        let mut b = h.start.clone();
        b.apply(Action::WordRight, Os::Mac, 8).unwrap();
        b.apply(Action::Right, Os::Mac, 8).unwrap();
        b.apply(Action::SelWordRight, Os::Mac, 8).unwrap();
        assert!(h.done(&b));
        // ← 는 선택의 앞으로 접힌다
        b.apply(Action::Left, Os::Mac, 8).unwrap();
        assert_eq!((b.col, b.selection()), (3, None));
    }

    #[test]
    fn 선택_지우기는_군더더기만_통째로() {
        let h = hole("‸the «the »cat", &["Left"]);
        let mut b = h.start.clone();
        b.apply(Action::WordRight, Os::Mac, 8).unwrap();
        b.apply(Action::SelWordRight, Os::Mac, 8).unwrap();
        // " the" — 앞 공백은 군더더기가 아니다
        assert_eq!(b.apply(Action::Backspace, Os::Mac, 8), Err(NotJunk));
        assert_eq!(b.junk_left(), 4);
        let mut b = h.start.clone();
        for a in [Action::Right; 4] {
            b.apply(a, Os::Mac, 8).unwrap();
        }
        b.apply(Action::SelWordRight, Os::Mac, 8).unwrap();
        b.apply(Action::SelRight, Os::Mac, 8).unwrap();
        b.apply(Action::Delete, Os::Mac, 8).unwrap();
        assert!(h.done(&b));
        assert_eq!(b.cursor_linear(), 4);
    }

    #[test]
    fn 선택_수가_있으면_사부는_통째로_지운다() {
        let h = hole("‸the «quick »brown fox", &[]);
        let mut moves = vec![
            Action::Left,
            Action::Right,
            Action::WordRight,
            Action::Backspace,
            Action::Delete,
        ];
        let plain = sage(&h, &edges(&moves), Os::Mac).unwrap();
        moves.extend([Action::SelRight, Action::SelWordRight]);
        let sel = sage(&h, &edges(&moves), Os::Mac).unwrap();
        assert!(sel.strokes < plain.strokes, "{sel:?} vs {plain:?}");
    }

    #[test]
    fn 사부의_수는_단어_점프가_화살표보다_싸다() {
        let h = hole("‸let config = load_config(path);⚑", &[]);
        let arrows = sage(&h, &edges(&[Action::Left, Action::Right]), Os::Mac).unwrap();
        assert_eq!(arrows.strokes, 31);
        let words = sage(
            &h,
            &edges(&[Action::Left, Action::Right, Action::WordRight]),
            Os::Mac,
        )
        .unwrap();
        assert!(words.strokes < 8, "{words:?}");
    }

    #[test]
    fn 이탭_수는_두_타로_센다() {
        let h = hole("one two three‸\n⚑x", &[]);
        let ls = Edge {
            action: Action::LineStart,
            seq: vec![Action::WordLeft, Action::LineStart],
        };
        let mut e = edges(&[Action::Down]);
        e.push(ls);
        let p = sage(&h, &e, Os::Mac).unwrap();
        // ↓ 로 2행(열 1) → 줄 처음(2타) = 3타, 또는 줄 처음(2) + ↓ = 3타
        assert_eq!(p.strokes, 3, "{p:?}");
    }

    /// docs/10 §7-3 — 모든 홀은 깃발에 닿고, 기본 이동만으로도 풀리고, Par ≥ 사부의 수
    #[test]
    fn 모든_홀은_풀리고_기본_수만으로도_풀린다() {
        let c = content();
        for course in &c.golf.course {
            for h in &course.hole {
                let hole = Hole::parse(h, &course.moves, course.page.unwrap_or(8))
                    .unwrap_or_else(|e| panic!("{}: {e}", course.id));
                // 군더더기가 길면 부분 삭제 상태가 2^n 으로 늘어 라운드 시작이 느려진다.
                // 선택 수가 있는 코스는 통째로 지우니 상태가 덜 늘어 더 허용한다.
                let selecting = hole.moves.iter().any(|a| a.unshifted().is_some());
                let cap = if selecting { 12 } else { 6 };
                assert!(
                    hole.start.junk_left() <= cap,
                    "{}: 군더더기는 홀당 {cap}글자까지",
                    h.title
                );
                EXPANDED.with(|c| c.set(0));
                let full = sage(&hole, &edges(&hole.moves), Os::Mac)
                    .unwrap_or_else(|| panic!("{} / {}: 풀리지 않는다", course.id, h.title));
                // 디버그 빌드 라운드 시작이 굼뜨지 않게 — 홀당 탐색 상태 상한
                let n = EXPANDED.with(|c| c.get());
                assert!(n < 20_000, "{} / {}: 탐색 상태 {n}", course.id, h.title);
                assert!(full.strokes >= 1, "{}: 이미 끝난 홀", h.title);
                let basic: Vec<Action> = hole
                    .moves
                    .iter()
                    .copied()
                    .filter(|a| {
                        matches!(
                            a,
                            Action::Left
                                | Action::Right
                                | Action::Up
                                | Action::Down
                                | Action::Backspace
                                | Action::Delete
                                | Action::SelLeft
                                | Action::SelRight
                                | Action::SelUp
                                | Action::SelDown
                        )
                    })
                    .collect();
                let b = sage(&hole, &edges(&basic), Os::Mac)
                    .unwrap_or_else(|| panic!("{}: 기본 수로 풀리지 않는다", h.title));
                assert!(b.strokes >= full.strokes);
                if let Some(p) = h.par {
                    assert!(p >= full.strokes, "{}: Par < 사부의 수", h.title);
                }
                let win = sage(&hole, &edges(&hole.moves), Os::Windows).unwrap();
                assert!(win.strokes >= 1, "{}: Windows 규칙에서도 풀린다", h.title);
                // 주제 수를 쓰면 화살표만보다 확실히 싸야 가르칠 가치가 있다
                if hole.flag.is_some() && hole.moves.len() > 4 {
                    assert!(
                        full.strokes < b.strokes,
                        "{} / {}: 주제 수가 이득이 없다 ({} vs {})",
                        course.id,
                        h.title,
                        full.strokes,
                        b.strokes
                    );
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn dump_sages() {
        let c = content();
        for course in &c.golf.course {
            for h in &course.hole {
                let hole = Hole::parse(h, &course.moves, course.page.unwrap_or(8)).unwrap();
                let t = std::time::Instant::now();
                EXPANDED.with(|c| c.set(0));
                let p = sage(&hole, &edges(&hole.moves), Os::Mac).unwrap();
                println!(
                    "{} / {}: {} {}  ({:?}, {} 상태)",
                    course.id,
                    h.title,
                    p.strokes,
                    p.display(),
                    t.elapsed(),
                    EXPANDED.with(|c| c.get())
                );
            }
        }
    }
}
