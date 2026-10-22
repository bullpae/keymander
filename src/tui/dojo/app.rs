//! dojo 앱 상태와 화면 전환·키 처리

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kmd_core::dojo::{DojoProgress, DojoRun, DojoScroll};
use kmd_core::Database;

use super::action::Action;
use super::binding::Bindings;
use super::content::{content, Content, Lesson, SenseiLines};
use super::precheck::{self, Report};
use super::progress::{self, Access, Standing};
use super::score::{self, Mark};
use super::summon::{Fixture, Round, RoundEvent};

/// kv_store 키 — 입문 의식을 마쳤는가
const KV_INITIATED: &str = "dojo.initiated";
/// kv_store 키 — 마지막으로 본 배치 지문 (docs/10 §6-5)
const KV_KEYMAP_FP: &str = "dojo.keymap_fp";

pub enum Screen {
    Precheck,
    Lobby,
    Map,
    Intro(String),
    Round(Box<Round>),
    Result(Box<Outcome>),
    Codex,
    Records,
}

/// 로비 메뉴
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Train,
    Daily,
    Codex,
    Records,
    Precheck,
    Quit,
}

pub const MENU: &[(MenuItem, &str, &str)] = &[
    (MenuItem::Train, "수련", "띠 지도"),
    (MenuItem::Daily, "오늘의 수련", "다음 판"),
    (MenuItem::Codex, "비급 도감", ""),
    (MenuItem::Records, "수련 기록", ""),
    (MenuItem::Precheck, "입문 의식", "사전 점검"),
    (MenuItem::Quit, "하산", "종료"),
];

/// 결과 화면의 한 줄
pub struct OutcomeRow {
    pub ask: String,
    pub sage: String,
    pub sage_strokes: u32,
    pub par: u32,
    pub strokes: Option<u32>,
}

pub struct Outcome {
    pub lesson_id: String,
    pub title: String,
    pub is_boss: bool,
    pub cleared: bool,
    pub stars: u8,
    pub score: u32,
    pub rows: Vec<OutcomeRow>,
    pub mistakes: u32,
    pub hints: u32,
    pub duration: Duration,
    pub new_best: bool,
    pub new_scroll: Option<String>,
    /// 승급했으면 새 띠 이름
    pub promoted: Option<String>,
    pub ki: Option<i32>,
}

/// 사부 — 트리거별 대사를 돌아가며 고른다
#[derive(Default)]
pub struct Sensei {
    turns: BTreeMap<&'static str, usize>,
}

impl Sensei {
    pub fn pick(&mut self, key: &'static str, lines: &[String], bindings: &Bindings) -> String {
        if lines.is_empty() {
            return String::new();
        }
        let n = self.turns.entry(key).or_default();
        let line = &lines[*n % lines.len()];
        *n += 1;
        bindings.render(line)
    }
}

pub struct App {
    pub screen: Screen,
    config: kmd_core::Config,
    /// 마지막으로 읽은 config.toml 의 수정 시각 — 바뀌었을 때만 배치를 다시 해석한다
    config_mtime: Option<std::time::SystemTime>,
    pub content: &'static Content,
    pub bindings: Bindings,
    pub report: Report,
    pub db: Option<Database>,
    pub progress: BTreeMap<String, DojoProgress>,
    pub scrolls: BTreeMap<String, DojoScroll>,
    pub standing: Standing,
    fixture: Option<Fixture>,
    pub lobby_sel: usize,
    pub map_belt: usize,
    pub map_sel: usize,
    pub codex_sel: usize,
    /// 라운드 일시정지 중이면 선택 항목
    pub pause: Option<usize>,
    pub sensei: Sensei,
    pub sensei_line: String,
    /// 화면 하단에 잠시 띄우는 알림
    pub notice: Option<(String, Instant)>,
    /// 맨 j/k 넌지시 권유는 세션당 한 번
    nudged: bool,
    /// 지난 입장 이후 배치가 바뀌었다
    pub keymap_changed: bool,
    pub quit: bool,
}

impl App {
    pub fn new(
        config: &kmd_core::Config,
        db: Option<Database>,
        start_lesson: Option<String>,
    ) -> Self {
        Self::with_report(config, db, start_lesson, precheck::run(config))
    }

    /// 사전 점검 결과를 받아 만든다 (테스트는 데몬 Ping 없이 결과를 주입한다)
    fn with_report(
        config: &kmd_core::Config,
        db: Option<Database>,
        start_lesson: Option<String>,
        report: Report,
    ) -> Self {
        let bindings = Bindings::from_config(config, report.trust);
        let content = content();
        let progress = db.as_ref().map(Database::dojo_progress).unwrap_or_default();
        let scrolls = db.as_ref().map(Database::dojo_scrolls).unwrap_or_default();
        let has = |a: Action| bindings.has(a);
        let standing = progress::standing(&content.curriculum, &progress, &has);

        let initiated = db.as_ref().and_then(|d| d.kv_get(KV_INITIATED)).is_some();
        let last_fp = db.as_ref().and_then(|d| d.kv_get(KV_KEYMAP_FP));
        let keymap_changed = last_fp
            .as_deref()
            .is_some_and(|fp| fp != bindings.fingerprint);
        if let Some(d) = &db {
            let _ = d.kv_set(KV_KEYMAP_FP, &bindings.fingerprint);
        }

        let mut app = Self {
            screen: if initiated {
                Screen::Lobby
            } else {
                Screen::Precheck
            },
            config: config.clone(),
            config_mtime: config_mtime(),
            content,
            bindings,
            report,
            db,
            progress,
            scrolls,
            standing,
            fixture: None,
            lobby_sel: 0,
            map_belt: 0,
            map_sel: 0,
            codex_sel: 0,
            pause: None,
            sensei: Sensei::default(),
            sensei_line: String::new(),
            notice: None,
            nudged: false,
            keymap_changed,
            quit: false,
        };
        app.sensei_line = app.say("lobby", |s| &s.lobby);
        if keymap_changed {
            app.notify("배치가 바뀌었다 — 안내를 네 배치로 다시 그렸다. 예전 기록에는 다른 배치 표식이 붙는다.");
        }
        if let Some(id) = start_lesson {
            if app.content.lesson(&id).is_some() {
                app.open_map_at(&id);
                app.screen = Screen::Intro(id);
            } else {
                app.notify(&format!("그런 과는 없다: {id}"));
            }
        }
        app
    }

    pub fn say(&mut self, key: &'static str, pick: fn(&SenseiLines) -> &Vec<String>) -> String {
        let lines = pick(&self.content.sensei);
        self.sensei.pick(key, lines, &self.bindings)
    }

    pub fn notify(&mut self, msg: &str) {
        self.notice = Some((msg.to_string(), Instant::now()));
    }

    fn has(&self) -> impl Fn(Action) -> bool + '_ {
        |a| self.bindings.has(a)
    }

    pub fn access(&self, belt_idx: usize, lesson: &Lesson) -> Access {
        self.standing.access(belt_idx, lesson, &self.has())
    }

    fn reload_progress(&mut self) {
        if let Some(db) = &self.db {
            self.progress = db.dojo_progress();
            self.scrolls = db.dojo_scrolls();
        }
        let has = |a: Action| self.bindings.has(a);
        self.standing = progress::standing(&self.content.curriculum, &self.progress, &has);
    }

    pub fn stars(&self, lesson_id: &str) -> u8 {
        self.progress.get(lesson_id).map_or(0, |p| p.best_stars)
    }

    /// 지도에서 보이는 띠 행 수 (커리큘럼 + 검은띠 + 단)
    pub fn map_rows(&self) -> usize {
        self.content.curriculum.belt.len() + 2
    }

    /// 지도 오른쪽 목록 — 과들 + 보스
    pub fn map_lessons(&self) -> Vec<&'static Lesson> {
        self.content
            .curriculum
            .belt
            .get(self.map_belt)
            .map(|b| b.lesson.iter().chain(b.boss.iter()).collect())
            .unwrap_or_default()
    }

    /// config.toml 이 바뀌었으면(또는 `force`) 다시 읽고 배치를 다시 해석한다 (docs/10 §6-5).
    /// 파일 감시자는 두지 않는다 — 지도·점검 화면에 들어갈 때만 mtime 을 본다.
    fn refresh_keymap(&mut self, force: bool) {
        let mtime = config_mtime();
        if !force && mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        if let Ok(config) = crate::cmd::load_config() {
            self.config = config;
        }
        self.report = precheck::run(&self.config);
        let bindings = Bindings::from_config(&self.config, self.report.trust);
        if bindings.fingerprint != self.bindings.fingerprint {
            self.keymap_changed = true;
            if let Some(d) = &self.db {
                let _ = d.kv_set(KV_KEYMAP_FP, &bindings.fingerprint);
            }
            self.bindings = bindings;
            self.notify("배치가 바뀌었다 — 안내를 네 배치로 다시 그렸다.");
        } else {
            self.bindings = bindings;
        }
        self.reload_progress();
    }

    fn open_map(&mut self) {
        self.refresh_keymap(false);
        self.map_belt = self
            .standing
            .current
            .min(self.content.curriculum.belt.len() - 1);
        let lessons = self.map_lessons();
        self.map_sel = lessons
            .iter()
            .position(|l| self.stars(&l.id) == 0 && l.is_ready())
            .unwrap_or(0);
        self.screen = Screen::Map;
    }

    fn open_map_at(&mut self, lesson_id: &str) {
        if let Some((belt, _, _)) = self.content.lesson(lesson_id) {
            self.map_belt = belt;
            self.map_sel = self
                .map_lessons()
                .iter()
                .position(|l| l.id == lesson_id)
                .unwrap_or(0);
        }
        self.screen = Screen::Map;
    }

    fn fixture(&mut self) -> &mut Fixture {
        self.fixture
            .get_or_insert_with(|| Fixture::new(self.content))
    }

    fn start_round(&mut self, lesson_id: &str) {
        let Some((belt, _, lesson)) = self.content.lesson(lesson_id) else {
            return;
        };
        match self.access(belt, lesson) {
            Access::Open => {}
            Access::Building => {
                self.sensei_line = self.say("building", |s| &s.building);
                return;
            }
            Access::Missing(miss) => {
                let names: Vec<_> = miss.iter().map(|a| a.label()).collect();
                self.notify(&format!(
                    "네 배치엔 {} 수가 없다 — 비급만 읽을 수 있다",
                    names.join(", ")
                ));
                return;
            }
            Access::LockedBelt | Access::LockedBoss => {
                self.sensei_line = self.say("locked", |s| &s.locked);
                return;
            }
        }
        let content = self.content;
        let round = Round::new(content, lesson, self.fixture());
        match round {
            Some(r) => {
                self.pause = None;
                self.sensei_line = self.bindings.render(&lesson.why);
                self.screen = Screen::Round(Box::new(r));
            }
            None => self.notify("이 과의 의뢰 세트를 찾지 못했다"),
        }
    }

    // ── 이벤트 ────────────────────────────────────────────────────────────────

    pub fn on_tick(&mut self, now: Instant) -> bool {
        let mut dirty = false;
        if let Some((_, at)) = &self.notice {
            if now.duration_since(*at) > Duration::from_secs(4) {
                self.notice = None;
                dirty = true;
            }
        }
        if self.pause.is_none() {
            if let Screen::Round(round) = &mut self.screen {
                if round.boss.is_some() {
                    dirty = true; // 남은 시간 표시
                }
                if round
                    .flash
                    .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_millis(150))
                {
                    round.flash = None;
                    dirty = true;
                }
                if round.tick(now) == RoundEvent::Failed {
                    self.finish_round(false);
                    dirty = true;
                }
            }
        }
        dirty
    }

    pub fn on_paste(&mut self, text: &str) {
        if self.pause.is_some() {
            return;
        }
        if let Screen::Round(_) = self.screen {
            let mut fixture = self
                .fixture
                .take()
                .unwrap_or_else(|| Fixture::new(self.content));
            if let Screen::Round(round) = &mut self.screen {
                round.on_paste(text, &mut fixture);
            }
            self.fixture = Some(fixture);
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match &self.screen {
            Screen::Round(_) => return self.round_key(key),
            _ if ctrl_c => {
                self.quit = true;
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Precheck => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                    if let Some(db) = &self.db {
                        let _ = db.kv_set(KV_INITIATED, "1");
                    }
                    self.screen = Screen::Lobby;
                }
            }
            Screen::Lobby => self.lobby_key(key),
            Screen::Map => self.map_key(key),
            Screen::Intro(ref id) => {
                let id = id.clone();
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') => self.start_round(&id),
                    KeyCode::Esc => self.open_map_at(&id),
                    _ => {}
                }
            }
            Screen::Result(ref out) => {
                let id = out.lesson_id.clone();
                match key.code {
                    KeyCode::Char('r') | KeyCode::Char('R') => self.start_round(&id),
                    KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') => {
                        self.open_map();
                        self.sensei_line = self.say("clear", |s| &s.clear);
                    }
                    _ => {}
                }
            }
            Screen::Codex => match self.nav(key) {
                Nav::Up => self.codex_sel = self.codex_sel.saturating_sub(1),
                Nav::Down => {
                    self.codex_sel = (self.codex_sel + 1).min(self.content.scrolls.scroll.len() - 1)
                }
                Nav::Back => self.screen = Screen::Lobby,
                _ => {}
            },
            Screen::Records => {
                if matches!(self.nav(key), Nav::Back | Nav::Enter) {
                    self.screen = Screen::Lobby;
                }
            }
            Screen::Round(_) => unreachable!(),
        }
    }

    /// 메뉴 공통 이동 해석 — 화살표(= nav 레이어 결과)가 정석, 맨 h/j/k/l 도 받는다
    fn nav(&mut self, key: KeyEvent) -> Nav {
        let raw = matches!(key.code, KeyCode::Char('j' | 'k' | 'h' | 'l'));
        if raw && !self.nudged {
            self.nudged = true;
            self.sensei_line = self.say("raw_jk", |s| &s.raw_jk);
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => Nav::Up,
            KeyCode::Down | KeyCode::Char('j') => Nav::Down,
            KeyCode::Left | KeyCode::Char('h') => Nav::Left,
            KeyCode::Right | KeyCode::Char('l') => Nav::Right,
            KeyCode::Enter | KeyCode::Char(' ') => Nav::Enter,
            KeyCode::Esc | KeyCode::Char('q') => Nav::Back,
            _ => Nav::None,
        }
    }

    fn lobby_key(&mut self, key: KeyEvent) {
        match self.nav(key) {
            Nav::Up => self.lobby_sel = self.lobby_sel.saturating_sub(1),
            Nav::Down => self.lobby_sel = (self.lobby_sel + 1).min(MENU.len() - 1),
            Nav::Back => self.quit = true,
            Nav::Enter => match MENU[self.lobby_sel].0 {
                MenuItem::Train => self.open_map(),
                MenuItem::Daily => self.notify("오늘의 수련은 다음 판에서 열린다 (검은띠 조건)."),
                MenuItem::Codex => self.screen = Screen::Codex,
                MenuItem::Records => self.screen = Screen::Records,
                MenuItem::Precheck => {
                    self.refresh_keymap(true);
                    self.screen = Screen::Precheck;
                }
                MenuItem::Quit => self.quit = true,
            },
            _ => {}
        }
    }

    fn map_key(&mut self, key: KeyEvent) {
        let belts = self.map_rows();
        match self.nav(key) {
            Nav::Left => {
                self.map_belt = self.map_belt.saturating_sub(1);
                self.map_sel = 0;
            }
            Nav::Right => {
                self.map_belt = (self.map_belt + 1).min(belts - 1);
                self.map_sel = 0;
            }
            Nav::Up => self.map_sel = self.map_sel.saturating_sub(1),
            Nav::Down => {
                let n = self.map_lessons().len();
                self.map_sel = (self.map_sel + 1).min(n.saturating_sub(1));
            }
            Nav::Enter => {
                if let Some(l) = self.map_lessons().get(self.map_sel) {
                    self.screen = Screen::Intro(l.id.clone());
                }
            }
            Nav::Back => self.screen = Screen::Lobby,
            Nav::None => {}
        }
    }

    fn round_key(&mut self, key: KeyEvent) {
        if let Some(sel) = self.pause {
            match key.code {
                KeyCode::Up => self.pause = Some(sel.saturating_sub(1)),
                KeyCode::Down => self.pause = Some((sel + 1).min(2)),
                KeyCode::Char('r') | KeyCode::Char('R') => self.restart_round(),
                KeyCode::Esc => self.abandon_round(),
                KeyCode::Enter => match sel {
                    0 => self.pause = None,
                    1 => self.restart_round(),
                    _ => self.abandon_round(),
                },
                _ => {}
            }
            return;
        }
        if key.code == KeyCode::Esc {
            self.pause = Some(0);
            return;
        }
        let mut fixture = self
            .fixture
            .take()
            .unwrap_or_else(|| Fixture::new(self.content));
        let event = match &mut self.screen {
            Screen::Round(round) => round.on_key(key, &mut fixture),
            _ => RoundEvent::None,
        };
        self.fixture = Some(fixture);
        self.after_round_event(event);
    }

    fn after_round_event(&mut self, event: RoundEvent) {
        let Screen::Round(round) = &self.screen else {
            return;
        };
        let streak = round.fail_streak;
        match event {
            RoundEvent::Mistake if streak >= 3 => {
                self.sensei_line = self.say("fail_streak", |s| &s.fail_streak);
            }
            RoundEvent::Mistake if streak == 1 && round.total_mistakes == 1 => {
                self.sensei_line = self.say("first_fail", |s| &s.first_fail);
            }
            RoundEvent::Cleared {
                sage_equal: true, ..
            } => {
                self.sensei_line = self.say("sage_equal", |s| &s.sage_equal);
            }
            RoundEvent::Cleared { over_par: true, .. } => {
                self.sensei_line = self.say("over_par", |s| &s.over_par);
            }
            RoundEvent::Finished => self.finish_round(true),
            RoundEvent::Failed => self.finish_round(false),
            _ => {}
        }
        if let Screen::Round(round) = &self.screen {
            if round.hangul_warn {
                self.sensei_line = self.say("hangul", |s| &s.hangul);
            }
        }
    }

    fn restart_round(&mut self) {
        if let Screen::Round(round) = &self.screen {
            let id = round.lesson_id.clone();
            self.start_round(&id);
        }
    }

    fn abandon_round(&mut self) {
        self.pause = None;
        if let Screen::Round(round) = &self.screen {
            let id = round.lesson_id.clone();
            self.open_map_at(&id);
        }
    }

    /// 라운드를 마치고 기록·해금·승급을 처리한 뒤 결과 화면으로 간다
    fn finish_round(&mut self, cleared: bool) {
        let Screen::Round(round) = std::mem::replace(&mut self.screen, Screen::Lobby) else {
            return;
        };
        let Some((_, _, lesson)) = self.content.lesson(&round.lesson_id) else {
            return;
        };
        let holes = round.holes();
        let (stars, score) = match (&round.boss, cleared) {
            (_, false) => (0, 0),
            (Some(b), true) => (
                score::boss_stars(b.ki),
                score::stroke_score(&holes, round.total_mistakes),
            ),
            (None, true) => (
                score::stroke_stars(&holes),
                score::stroke_score(&holes, round.total_mistakes),
            ),
        };
        let strokes: u32 = holes.iter().map(|h| h.strokes).sum();
        let par: u32 = holes.iter().map(|h| h.par).sum();
        let duration = round.started.elapsed();

        let before = self.standing.current;
        let mut new_best = false;
        let mut new_scroll = None;
        if let Some(db) = &self.db {
            let run = DojoRun {
                lesson: lesson.id.clone(),
                kind: lesson.kind.clone(),
                strokes: cleared.then_some(strokes),
                par: cleared.then_some(par),
                score,
                stars,
                max_combo: None,
                mistakes: round.total_mistakes,
                hints: round.hints,
                duration_ms: duration.as_millis() as u64,
                os: std::env::consts::OS.to_string(),
                terminal: self.report.terminal.clone(),
                keymap_fp: Some(self.bindings.fingerprint.clone()),
            };
            new_best = db.dojo_record_run(&run).unwrap_or(false);
            if stars > 0 {
                if let Some(s) = &lesson.scroll {
                    if db.dojo_unlock_scroll(s).unwrap_or(false) {
                        new_scroll = Some(s.clone());
                    }
                }
            }
        }
        self.reload_progress();
        let promoted = (self.standing.current > before)
            .then(|| {
                self.content
                    .curriculum
                    .belt
                    .get(before + 1)
                    .map(|b| b.name.clone())
            })
            .flatten();

        let rows = round
            .requests
            .iter()
            .enumerate()
            .map(|(i, r)| OutcomeRow {
                ask: r.ask.clone(),
                sage: round.sages[i].display(),
                sage_strokes: round.sages[i].strokes(),
                par: round.pars[i],
                strokes: round.cleared.get(i).map(|c| c.hole.strokes),
            })
            .collect();
        self.sensei_line = match (cleared, promoted.is_some()) {
            (true, true) => "승급이다. 띠를 단단히 매어라.".into(),
            (true, false) if stars == 3 => self.say("sage_equal", |s| &s.sage_equal),
            (true, false) => self.say("clear", |s| &s.clear),
            (false, _) => lesson.line.clone().unwrap_or_default(),
        };
        self.screen = Screen::Result(Box::new(Outcome {
            lesson_id: lesson.id.clone(),
            title: lesson.title.clone(),
            is_boss: lesson.is_boss(),
            cleared,
            stars,
            score,
            rows,
            mistakes: round.total_mistakes,
            hints: round.hints,
            duration,
            new_best,
            new_scroll,
            promoted,
            ki: round.boss.as_ref().map(|b| b.ki.max(0)),
        }));
    }
}

fn config_mtime() -> Option<std::time::SystemTime> {
    let path = kmd_core::Config::default_config_dir().join(kmd_core::CONFIG_FILENAME);
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,
    None,
}

/// 결과 행의 골프 기호
pub fn row_mark(row: &OutcomeRow) -> Option<Mark> {
    row.strokes.map(|s| Mark::of(s, row.par))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::binding::Trust;
    use crate::tui::dojo::precheck::Backend;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app() -> App {
        let report = Report {
            backend: Backend::Daemon,
            trust: Trust::Full,
            profile: "vim-nav".into(),
            terminal: Some("TestTerm".into()),
        };
        let db = Database::open_in_memory().ok();
        App::with_report(&kmd_core::Config::default(), db, None, report)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// 80×24 가상 터미널에 그려 텍스트로 돌려준다
    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::super::ui::render(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..h {
            let mut line = String::new();
            let mut skip = 0;
            for x in 0..w {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let sym = buf[(x, y)].symbol();
                line.push_str(sym);
                skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// 사부의 수대로 현재 의뢰를 푼다
    fn solve_current(app: &mut App) {
        let (query, downs) = match &app.screen {
            Screen::Round(r) => (r.sage().query.clone(), r.sage().downs),
            _ => panic!("라운드가 아니다"),
        };
        type_str(app, &query);
        for _ in 0..downs {
            press(app, KeyCode::Down);
        }
        press(app, KeyCode::Enter);
    }

    fn play_lesson(app: &mut App, id: &str) {
        app.screen = Screen::Intro(id.into());
        press(app, KeyCode::Enter);
        while matches!(app.screen, Screen::Round(_)) {
            solve_current(app);
        }
    }

    #[test]
    fn 첫_입장은_입문_의식이고_enter_로_입구에_간다() {
        let mut a = app();
        assert!(matches!(a.screen, Screen::Precheck));
        assert!(draw(&a, 80, 24).contains("사전 점검"));
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Lobby));
        let screen = draw(&a, 80, 24);
        assert!(screen.contains("k m d   d o j o"));
        assert!(
            screen.contains("CapsLock nav"),
            "로비에 사용자 배치가 보인다\n{screen}"
        );
    }

    #[test]
    fn 흰띠를_사부의_수로_완주하면_별_셋에_비급이_열리고_승급한다() {
        let mut a = app();
        for id in ["white.1", "white.2", "white.3", "white.4"] {
            play_lesson(&mut a, id);
            let Screen::Result(out) = &a.screen else {
                panic!("{id}: 결과 화면이 아니다")
            };
            assert!(out.cleared);
            assert_eq!(out.stars, 3, "{id}: 사부의 수면 ★★★");
            assert!(out.new_scroll.is_some(), "{id}: 첫 완주는 비급을 연다");
            assert!(draw(&a, 80, 24).contains("완  주"));
            press(&mut a, KeyCode::Enter);
            assert!(matches!(a.screen, Screen::Map));
        }
        assert_eq!(a.standing.current, 0, "관문 전에는 아직 흰띠");
        play_lesson(&mut a, "white.boss");
        let Screen::Result(out) = &a.screen else {
            panic!("관문 결과가 아니다")
        };
        assert!(out.cleared && out.is_boss);
        assert_eq!(out.promoted.as_deref(), Some("노란띠"));
        let screen = draw(&a, 80, 24);
        assert!(screen.contains("승  급"), "{screen}");
        assert_eq!(a.standing.current, 1);
        assert_eq!(a.scrolls.len(), 5);
    }

    #[test]
    fn 잠긴_관문과_공사_중인_과는_시작되지_않는다() {
        let mut a = app();
        a.screen = Screen::Intro("white.boss".into());
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Intro(_)));
        assert!(draw(&a, 80, 24).contains("관문이 열린다"));

        a.screen = Screen::Intro("yellow.1".into());
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Intro(_)));
        let screen = draw(&a, 80, 24);
        assert!(
            screen.contains("[Caps]"),
            "공사 중이어도 두 손 키보드는 배치대로 미리 보인다\n{screen}"
        );
        assert!(screen.contains("[H]"));
    }

    #[test]
    fn esc_는_일시정지이고_한_번_더_누르면_지도로_돌아간다() {
        let mut a = app();
        a.screen = Screen::Intro("white.1".into());
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Esc);
        assert_eq!(a.pause, Some(0));
        assert!(draw(&a, 80, 24).contains("숨 고르기"));
        press(&mut a, KeyCode::Esc);
        assert!(matches!(a.screen, Screen::Map));
        assert!(
            a.db.as_ref().unwrap().dojo_recent_runs(5).is_empty(),
            "그만둔 판은 기록되지 않는다"
        );
    }

    #[test]
    fn 작은_터미널은_안내만_보인다() {
        let a = app();
        assert!(draw(&a, 60, 20).contains("창을 키워라"));
    }

    #[test]
    fn 모든_화면이_80x24_와_큰_터미널에서_패닉_없이_그려진다() {
        let mut a = app();
        let mut screens: Vec<Screen> = vec![
            Screen::Precheck,
            Screen::Lobby,
            Screen::Map,
            Screen::Codex,
            Screen::Records,
        ];
        for b in &a.content.curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                screens.push(Screen::Intro(l.id.clone()));
            }
        }
        for s in screens {
            a.screen = s;
            for belt in 0..a.map_rows() {
                a.map_belt = belt;
                draw(&a, 80, 24);
            }
            draw(&a, 132, 40);
        }
        // 라운드·보스·결과
        play_lesson(&mut a, "white.1");
        draw(&a, 80, 24);
        a.standing.belts[0].done = a.standing.belts[0].required;
        a.screen = Screen::Intro("white.boss".into());
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "한글");
        draw(&a, 80, 24);
    }

    #[test]
    #[ignore]
    fn dump_screens() {
        let mut a = app();
        println!("{}", draw(&a, 80, 24));
        press(&mut a, KeyCode::Enter);
        println!("{}", draw(&a, 80, 24));
        press(&mut a, KeyCode::Enter);
        println!("{}", draw(&a, 80, 24));
        press(&mut a, KeyCode::Enter);
        println!("{}", draw(&a, 80, 24));
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "fi");
        println!("{}", draw(&a, 80, 24));
        type_str(&mut a, "r");
        press(&mut a, KeyCode::Enter);
        while matches!(a.screen, Screen::Round(_)) {
            solve_current(&mut a);
        }
        println!("{}", draw(&a, 80, 24));
        a.screen = Screen::Intro("yellow.1".into());
        println!("{}", draw(&a, 80, 24));
        a.screen = Screen::Codex;
        println!("{}", draw(&a, 80, 24));
    }
}
