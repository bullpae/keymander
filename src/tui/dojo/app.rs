//! dojo 앱 상태와 화면 전환·키 처리

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use kmd_core::dojo::{DojoProgress, DojoRun, DojoScroll};
use kmd_core::Database;

use super::action::{Action, Os};
use super::binding::Bindings;
use super::calib::{Calib, CalibEvent, CalibRun, KEY_ACTIONS};
use super::content::{content, Content, Lesson, SenseiLines};
use super::golf::{Edge, GolfRound};
use super::maze::MazeRound;
use super::mouse::{MouseRound, ARENA_ORIGIN};
use super::play::{OutcomeRow, Play, RoundEvent};
use super::precheck::{self, Report};
use super::progress::{self, Access, Standing};
use super::score::Mark;
use super::spar::Spar;
use super::summon::{Fixture, Round};

/// kv_store 키 — 입문 의식을 마쳤는가
const KV_INITIATED: &str = "dojo.initiated";
/// kv_store 키 — 마지막으로 본 배치 지문 (docs/10 §6-5)
const KV_KEYMAP_FP: &str = "dojo.keymap_fp";
/// kv_store 키 접두 — 터미널별 키 감응 보정 (docs/10 §6-2)
const KV_CALIB: &str = "dojo.calib.";

/// 실수 직후 화면이 흔들리는 시간
pub const SHAKE: Duration = Duration::from_millis(140);
/// 움직이는 화면의 프레임 간격 — 30fps 면 충분하고 CPU 는 1% 안팎이다
const FRAME: Duration = Duration::from_millis(33);
/// 쉬는 동안의 대기 — 키 훅 데몬이 도는 중에 CPU 를 태우지 않는다 (사고 수칙)
const IDLE: Duration = Duration::from_millis(250);

pub enum Screen {
    Precheck,
    Calib(Box<CalibRun>),
    Lobby,
    Map,
    Intro(String),
    Round(Box<Play>),
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
    (MenuItem::Precheck, "입문 의식", "점검·감응"),
    (MenuItem::Quit, "하산", "종료"),
];

pub struct Outcome {
    pub lesson_id: String,
    pub title: String,
    pub kind: String,
    pub is_boss: bool,
    pub cleared: bool,
    pub stars: u8,
    pub score: u32,
    pub rows: Vec<OutcomeRow>,
    pub stats: Vec<(String, String)>,
    pub mistakes: u32,
    pub hints: u32,
    pub duration: Duration,
    pub new_best: bool,
    pub new_scroll: Option<String>,
    /// 승급했으면 새 띠 이름
    pub promoted: Option<String>,
    pub ki: Option<i32>,
    /// 결과 화면이 열린 순간 (별이 하나씩 켜지는 연출)
    pub shown_at: Instant,
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
    /// 이 터미널의 키 감응 보정
    pub calib: Calib,
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
    /// 자유 수련 — 띠·관문 잠금 없이 아무 과나 (`kmd dojo --free`)
    pub free: bool,
    /// 터미널 크기 — 마우스 좌표를 무대 좌표로 바꿀 때 쓴다
    pub term_size: (u16, u16),
    /// 마지막 실수 순간 (화면 흔들림)
    pub shake: Option<Instant>,
    pub quit: bool,
}

impl App {
    pub fn new(
        config: &kmd_core::Config,
        db: Option<Database>,
        start_lesson: Option<String>,
        free: bool,
    ) -> Self {
        let mut app = Self::with_report(config, db, start_lesson, precheck::run(config));
        app.free = free;
        if free {
            app.notify("자유 수련 — 띠와 관문 잠금 없이 아무 과나 들어간다. 기록은 남는다.");
        }
        app
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
        let calib = load_calib(db.as_ref(), report.terminal.as_deref());
        let has = |a: Action| bindings.has(a) && calib.observable(a);
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
            calib,
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
            free: false,
            term_size: (80, 24),
            shake: None,
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

    /// 이 배치에 있고 이 터미널에서 관측되는 수인가
    fn has(&self) -> impl Fn(Action) -> bool + '_ {
        |a| self.bindings.has(a) && self.calib.observable(a)
    }

    pub fn access(&self, belt_idx: usize, lesson: &Lesson) -> Access {
        match self.standing.access(belt_idx, lesson, &self.has()) {
            Access::LockedBelt | Access::LockedBoss if self.free => Access::Open,
            a => a,
        }
    }

    fn reload_progress(&mut self) {
        if let Some(db) = &self.db {
            self.progress = db.dojo_progress();
            self.scrolls = db.dojo_scrolls();
        }
        let has = |a: Action| self.bindings.has(a) && self.calib.observable(a);
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

    /// 배치에서 그 수를 누르면 도착하는 이벤트들. 없거나 관측 불가면 `None`.
    pub fn edge_for(&self, a: Action) -> Option<Edge> {
        if !self.bindings.has(a) || !self.calib.observable(a) {
            return None;
        }
        Some(match self.bindings.double_partner(a) {
            Some(first) => Edge {
                action: a,
                seq: vec![first, a],
            },
            None => Edge::single(a),
        })
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
                    "네 배치(또는 이 터미널)엔 {} 수가 없다 — 비급만 읽을 수 있다",
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
        let now = Instant::now();
        let play = match lesson.kind.as_str() {
            "summon" => Round::new(content, lesson, self.fixture()).map(Play::Summon),
            "golf" => {
                let unobservable: Vec<Action> = KEY_ACTIONS
                    .iter()
                    .copied()
                    .filter(|a| self.bindings.has(*a) && !self.calib.observable(*a))
                    .collect();
                GolfRound::new(content, lesson, &|a| self.edge_for(a), unobservable, now)
                    .map(Play::Golf)
            }
            "spar" => Some(Play::Spar(Spar::new(lesson, now))),
            "maze" => MazeRound::new(content, lesson, now).map(Play::Maze),
            "mouse" => MouseRound::new(lesson, now).map(Play::Mouse),
            _ => None,
        };
        match play {
            Some(p) => {
                self.pause = None;
                self.shake = None;
                self.sensei_line = self.bindings.render(&lesson.why);
                if lesson.kind == "golf" && !self.calib.done {
                    self.notify("키 감응 시험 전이다 — 수가 안 먹으면 입문 의식에서 C 로 이 터미널을 보정하라");
                }
                self.screen = Screen::Round(Box::new(p));
            }
            None => self.notify("이 과의 수련 자료를 찾지 못했다"),
        }
    }

    fn start_calib(&mut self) {
        let steps: Vec<Action> = KEY_ACTIONS
            .iter()
            .copied()
            .filter(|a| self.bindings.has(*a))
            .collect();
        let doubles = steps
            .iter()
            .filter_map(|a| self.bindings.double_partner(*a).map(|p| (*a, p)))
            .collect();
        self.screen = Screen::Calib(Box::new(CalibRun::new(steps, doubles, Instant::now())));
    }

    fn finish_calib(&mut self, run: &CalibRun) {
        self.calib = run.finish();
        if let (Some(db), Some(term)) = (&self.db, &self.report.terminal) {
            let _ = db.kv_set(&format!("{KV_CALIB}{term}"), &self.calib.to_json());
        }
        self.reload_progress();
    }

    /// 다음 루프까지 기다릴 시간 — 움직이는 화면만 프레임으로 돈다
    pub fn frame_interval(&self, now: Instant) -> Duration {
        let moving = match &self.screen {
            Screen::Round(p) => self.pause.is_none() && p.animating(now),
            Screen::Calib(run) => run.current().is_some(),
            Screen::Result(out) => now.duration_since(out.shown_at) < Duration::from_millis(1200),
            _ => false,
        };
        if moving || self.shake.is_some() {
            FRAME
        } else {
            IDLE
        }
    }

    /// 마우스 캡처를 켤 화면인가
    pub fn wants_mouse(&self) -> bool {
        matches!(&self.screen, Screen::Round(p) if p.wants_mouse())
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
        if self.shake.is_some_and(|t| now.duration_since(t) > SHAKE) {
            self.shake = None;
            dirty = true;
        }
        match &mut self.screen {
            Screen::Calib(run) => {
                dirty = true; // 남은 시간 막대
                if run.tick(now) == CalibEvent::Done {
                    let run = std::mem::replace(
                        run,
                        Box::new(CalibRun::new(Vec::new(), BTreeMap::new(), now)),
                    );
                    self.finish_calib(&run);
                    self.screen = Screen::Calib(run);
                }
            }
            Screen::Result(out) => {
                dirty |= now.duration_since(out.shown_at) < Duration::from_millis(1300);
            }
            Screen::Round(play) if self.pause.is_none() => {
                if play.animating(now) {
                    dirty = true;
                }
                if play
                    .flash()
                    .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_millis(150))
                {
                    play.clear_flash();
                    dirty = true;
                }
                let ev = play.tick(now);
                if ev != RoundEvent::None {
                    dirty = true;
                    self.after_round_event(ev);
                }
            }
            _ => {}
        }
        dirty
    }

    pub fn on_paste(&mut self, text: &str) {
        if self.pause.is_some() {
            return;
        }
        if let Screen::Round(play) = &self.screen {
            if !matches!(**play, Play::Summon(_)) {
                return;
            }
            let mut fixture = self
                .fixture
                .take()
                .unwrap_or_else(|| Fixture::new(self.content));
            if let Screen::Round(play) = &mut self.screen {
                if let Play::Summon(round) = play.as_mut() {
                    round.on_paste(text, &mut fixture);
                }
            }
            self.fixture = Some(fixture);
        }
    }

    pub fn on_resize(&mut self, w: u16, h: u16) {
        self.term_size = (w, h);
    }

    /// 마우스 이벤트 — 터미널 좌표를 과녁장 좌표로 바꿔 넘긴다
    pub fn on_mouse(&mut self, ev: MouseEvent) -> bool {
        if self.pause.is_some() {
            return false;
        }
        let (w, h) = self.term_size;
        let sx = w.saturating_sub(super::ui::STAGE_W) / 2;
        let sy = h.saturating_sub(super::ui::STAGE_H) / 2;
        let x = i32::from(ev.column) - i32::from(sx + ARENA_ORIGIN.0);
        let y = i32::from(ev.row) - i32::from(sy + ARENA_ORIGIN.1);
        let now = Instant::now();
        let out = match &mut self.screen {
            Screen::Round(play) => match play.as_mut() {
                Play::Mouse(r) => r.on_mouse(ev.kind, x, y, now),
                _ => return false,
            },
            _ => return false,
        };
        self.after_round_event(out);
        true
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match &self.screen {
            Screen::Round(_) => return self.round_key(key),
            Screen::Calib(_) => return self.calib_key(key),
            _ if ctrl_c => {
                self.quit = true;
                return;
            }
            _ => {}
        }
        match self.screen {
            Screen::Precheck => match key.code {
                KeyCode::Char('c') | KeyCode::Char('C') => self.start_calib(),
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') => {
                    if let Some(db) = &self.db {
                        let _ = db.kv_set(KV_INITIATED, "1");
                    }
                    self.screen = Screen::Lobby;
                }
                _ => {}
            },
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
            Screen::Round(_) | Screen::Calib(_) => unreachable!(),
        }
    }

    fn calib_key(&mut self, key: KeyEvent) {
        let now = Instant::now();
        let Screen::Calib(run) = &mut self.screen else {
            return;
        };
        if run.current().is_none() {
            // 결과 화면 — 아무 키나 점검으로
            if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                self.screen = Screen::Precheck;
            }
            return;
        }
        let ev = match key.code {
            KeyCode::Esc => {
                self.notify("감응 시험을 멈췄다 — 보정은 바뀌지 않았다");
                self.screen = Screen::Precheck;
                return;
            }
            KeyCode::Tab => run.skip(now),
            _ => run.on_key(&key, now),
        };
        if ev == CalibEvent::Done {
            let Screen::Calib(run) = std::mem::replace(&mut self.screen, Screen::Precheck) else {
                return;
            };
            self.finish_calib(&run);
            self.screen = Screen::Calib(run);
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
        let now = Instant::now();
        let action = self.calib.translate(&key, Os::current());
        let mut fixture = self.fixture.take();
        let Screen::Round(play) = &mut self.screen else {
            return;
        };
        let ev = match play.as_mut() {
            Play::Summon(r) => {
                let fx = fixture.get_or_insert_with(|| Fixture::new(self.content));
                r.on_key(key, fx)
            }
            Play::Golf(r) => match key.code {
                KeyCode::Tab => {
                    r.hint();
                    RoundEvent::None
                }
                KeyCode::Enter => r.next(),
                KeyCode::Char(' ') => {
                    r.replay(now);
                    RoundEvent::None
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    r.retry(now);
                    RoundEvent::None
                }
                _ => action.map_or(RoundEvent::None, |a| r.on_action(a, now)),
            },
            Play::Spar(r) => action.map_or(RoundEvent::None, |a| r.on_action(a, now)),
            Play::Maze(r) => match key.code {
                KeyCode::Tab => {
                    r.hint();
                    RoundEvent::None
                }
                _ => action.map_or(RoundEvent::None, |a| r.on_action(a, now)),
            },
            Play::Mouse(r) => r.on_key(&key, now),
        };
        self.fixture = fixture;
        self.after_round_event(ev);
    }

    fn after_round_event(&mut self, event: RoundEvent) {
        let Screen::Round(play) = &self.screen else {
            return;
        };
        let streak = play.fail_streak();
        let mistakes = play.total_mistakes();
        let combo = match play.as_ref() {
            Play::Spar(s) => s.combo,
            _ => 0,
        };
        let boss_line = self
            .content
            .lesson(play.lesson_id())
            .and_then(|(_, _, l)| l.line.clone());
        let now = Instant::now();
        if matches!(event, RoundEvent::Mistake | RoundEvent::Failed) {
            self.shake = Some(now);
        }
        match event {
            RoundEvent::Mistake if play.boss().is_some() && streak == 2 => {
                if let Some(line) = boss_line {
                    self.sensei_line = format!("(보스) {line}");
                }
            }
            RoundEvent::Mistake if streak >= 3 => {
                self.sensei_line = self.say("fail_streak", |s| &s.fail_streak);
            }
            RoundEvent::Mistake if streak == 1 && mistakes == 1 => {
                self.sensei_line = self.say("first_fail", |s| &s.first_fail);
            }
            RoundEvent::Hit if combo > 0 && combo % 10 == 0 => {
                let line = self.say("combo", |s| &s.combo);
                self.sensei_line = format!("×{combo} — {line}");
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
        if let Screen::Round(play) = &self.screen {
            if let Play::Summon(round) = play.as_ref() {
                if round.hangul_warn {
                    self.sensei_line = self.say("hangul", |s| &s.hangul);
                }
            }
        }
    }

    fn restart_round(&mut self) {
        if let Screen::Round(play) = &self.screen {
            let id = play.lesson_id().to_string();
            self.start_round(&id);
        }
    }

    fn abandon_round(&mut self) {
        self.pause = None;
        if let Screen::Round(play) = &self.screen {
            let id = play.lesson_id().to_string();
            self.open_map_at(&id);
        }
    }

    /// 라운드를 마치고 기록·해금·승급을 처리한 뒤 결과 화면으로 간다
    fn finish_round(&mut self, cleared: bool) {
        let Screen::Round(play) = std::mem::replace(&mut self.screen, Screen::Lobby) else {
            return;
        };
        let Some((_, _, lesson)) = self.content.lesson(play.lesson_id()) else {
            return;
        };
        let sum = play.summary(cleared);
        let duration = play.started().elapsed();

        let before = self.standing.reach;
        let mut new_best = false;
        let mut new_scroll = None;
        if let Some(db) = &self.db {
            let run = DojoRun {
                lesson: lesson.id.clone(),
                kind: lesson.kind.clone(),
                strokes: sum.strokes,
                par: sum.par,
                score: sum.score,
                stars: sum.stars,
                max_combo: sum.max_combo,
                mistakes: sum.mistakes,
                hints: sum.hints,
                duration_ms: duration.as_millis() as u64,
                os: std::env::consts::OS.to_string(),
                terminal: self.report.terminal.clone(),
                keymap_fp: Some(self.bindings.fingerprint.clone()),
            };
            new_best = db.dojo_record_run(&run).unwrap_or(false);
            if sum.stars > 0 {
                if let Some(s) = &lesson.scroll {
                    if db.dojo_unlock_scroll(s).unwrap_or(false) {
                        new_scroll = Some(s.clone());
                    }
                }
            }
        }
        self.reload_progress();
        let promoted = (self.standing.reach > before).then(|| {
            self.content
                .curriculum
                .belt
                .get(self.standing.reach)
                .map_or_else(
                    || "모든 수련장 — 다음 띠는 공사 중".into(),
                    |b| b.name.clone(),
                )
        });

        self.sensei_line = match (cleared, promoted.is_some()) {
            (true, true) => "승급이다. 띠를 단단히 매어라.".into(),
            (true, false) if sum.stars == 3 => self.say("sage_equal", |s| &s.sage_equal),
            (true, false) => self.say("clear", |s| &s.clear),
            (false, _) => lesson
                .line
                .clone()
                .unwrap_or_else(|| "다시 하라. 몸이 기억할 때까지.".into()),
        };
        self.screen = Screen::Result(Box::new(Outcome {
            lesson_id: lesson.id.clone(),
            title: lesson.title.clone(),
            kind: lesson.kind.clone(),
            is_boss: lesson.is_boss(),
            cleared,
            stars: sum.stars,
            score: sum.score,
            rows: sum.rows,
            stats: sum.stats,
            mistakes: sum.mistakes,
            hints: sum.hints,
            duration,
            new_best,
            new_scroll,
            promoted,
            ki: sum.ki,
            shown_at: Instant::now(),
        }));
    }
}

fn config_mtime() -> Option<std::time::SystemTime> {
    let path = kmd_core::Config::default_config_dir().join(kmd_core::CONFIG_FILENAME);
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// 저장된 보정이 있으면 그것, 없으면 터미널별 기본값
fn load_calib(db: Option<&Database>, terminal: Option<&str>) -> Calib {
    terminal
        .and_then(|t| db?.kv_get(&format!("{KV_CALIB}{t}")))
        .and_then(|j| Calib::from_json(&j))
        .unwrap_or_else(|| Calib::defaults_for(terminal, Os::current()))
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

    fn key(app: &mut App, code: KeyCode, m: KeyModifiers) {
        app.on_key(KeyEvent::new(code, m));
    }

    fn arrow(a: Action) -> KeyCode {
        match a {
            Action::Left => KeyCode::Left,
            Action::Right => KeyCode::Right,
            Action::Up => KeyCode::Up,
            Action::Down => KeyCode::Down,
            Action::PageUp => KeyCode::PageUp,
            Action::PageDown => KeyCode::PageDown,
            Action::Backspace => KeyCode::Backspace,
            Action::Delete => KeyCode::Delete,
            Action::LineStart => KeyCode::Home,
            Action::LineEnd => KeyCode::End,
            _ => unreachable!(),
        }
    }

    /// 수 하나를 macOS 데몬이 보내는 꼴로 도착시킨다
    fn send(app: &mut App, a: Action) {
        match a {
            Action::WordLeft => key(app, KeyCode::Left, KeyModifiers::ALT),
            Action::WordRight => key(app, KeyCode::Right, KeyModifiers::ALT),
            _ => press(app, arrow(a)),
        }
    }

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
            Screen::Round(p) => match p.as_ref() {
                Play::Summon(r) => (r.sage().query.clone(), r.sage().downs),
                _ => panic!("소환술이 아니다"),
            },
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
        assert!(matches!(a.screen, Screen::Intro(_)), "앞 띠를 넘어야 한다");
        let screen = draw(&a, 80, 24);
        assert!(
            screen.contains("[Caps]"),
            "잠겨 있어도 두 손 키보드는 배치대로 미리 보인다\n{screen}"
        );
        assert!(screen.contains("[H]"));

        a.screen = Screen::Intro("green.2".into());
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Intro(_)), "공사 중");
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
        a.free = true;
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
        a.screen = Screen::Intro("white.boss".into());
        press(&mut a, KeyCode::Enter);
        type_str(&mut a, "한글");
        draw(&a, 80, 24);
        // 모든 구현된 과의 라운드 첫 화면
        for b in &a.content.curriculum.belt {
            for l in b
                .lesson
                .iter()
                .chain(b.boss.iter())
                .filter(|l| l.is_ready())
            {
                a.screen = Screen::Intro(l.id.clone());
                press(&mut a, KeyCode::Enter);
                assert!(matches!(a.screen, Screen::Round(_)), "{}: 시작 안 됨", l.id);
                draw(&a, 80, 24);
                draw(&a, 132, 40);
                press(&mut a, KeyCode::Esc);
                draw(&a, 80, 24);
                press(&mut a, KeyCode::Esc);
            }
        }
        a.start_calib();
        draw(&a, 80, 24);
    }

    #[test]
    fn 골프를_사부의_수로_치면_별_셋이다() {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Intro("orange.1".into());
        press(&mut a, KeyCode::Enter);
        while let Screen::Round(p) = &a.screen {
            let (path, edges) = match p.as_ref() {
                Play::Golf(r) => (r.sage().moves.clone(), r.allowed[r.idx].clone()),
                _ => panic!("골프가 아니다"),
            };
            for m in path {
                let e = edges.iter().find(|e| e.action == m).unwrap();
                for s in &e.seq {
                    send(&mut a, *s);
                }
            }
            assert!(draw(&a, 80, 24).contains("Enter 다음"), "홀 아웃 오버레이");
            press(&mut a, KeyCode::Char(' '));
            press(&mut a, KeyCode::Enter);
        }
        let Screen::Result(out) = &a.screen else {
            panic!("결과가 아니다")
        };
        assert!(out.cleared);
        assert_eq!(out.stars, 3);
        assert!(draw(&a, 80, 24).contains("스코어카드"));
    }

    #[test]
    fn 줄_처음은_두_이벤트로_도착하고_두_타다() {
        let a = app();
        let e = a.edge_for(Action::LineStart).unwrap();
        assert_eq!(e.seq.len(), 2, "{e:?}");
        assert_eq!(e.seq[1], Action::LineStart);
    }

    #[test]
    fn 대련_카드는_결과_키로_받아친다() {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Intro("yellow.1".into());
        press(&mut a, KeyCode::Enter);
        while let Screen::Round(p) = &a.screen {
            let want = match p.as_ref() {
                Play::Spar(s) => s.prompt().unwrap(),
                _ => panic!(),
            };
            assert!(draw(&a, 80, 24).contains("대련"));
            press(&mut a, arrow(want));
        }
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert!(out.cleared);
        assert!(out.stars >= 2);
        assert!(draw(&a, 80, 24).contains("평균 반응"));
    }

    #[test]
    fn 미로는_사부의_길로_탈출한다() {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Intro("yellow.2".into());
        press(&mut a, KeyCode::Enter);
        let mut now = Instant::now();
        while let Screen::Round(p) = &a.screen {
            let path = match p.as_ref() {
                Play::Maze(r) => r.sages[r.idx].clone(),
                _ => panic!(),
            };
            for m in path {
                press(&mut a, arrow(m));
            }
            draw(&a, 80, 24);
            now += Duration::from_secs(2);
            a.on_tick(now);
        }
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert_eq!(out.stars, 3);
    }

    #[test]
    fn 마우스는_과녁장_좌표로_판정한다() {
        use crossterm::event::{MouseEvent, MouseEventKind};
        let mut a = app();
        a.free = true;
        a.on_resize(100, 30);
        a.screen = Screen::Intro("purple.1".into());
        press(&mut a, KeyCode::Enter);
        assert!(a.wants_mouse());
        let (sx, sy) = (10u16, 3u16);
        while let Screen::Round(p) = &a.screen {
            let rect = match p.as_ref() {
                Play::Mouse(r) => r.target().unwrap().rect,
                _ => panic!(),
            };
            a.on_mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: sx + ARENA_ORIGIN.0 + rect.x + rect.w / 2,
                row: sy + ARENA_ORIGIN.1 + rect.y + rect.h / 2,
                modifiers: KeyModifiers::NONE,
            });
        }
        assert!(matches!(a.screen, Screen::Result(_)));
        assert!(!a.wants_mouse());
    }

    #[test]
    fn 감응_시험은_보정을_저장하고_다음_입장에도_쓴다() {
        let mut a = app();
        a.start_calib();
        let steps = match &a.screen {
            Screen::Calib(run) => run.steps.clone(),
            _ => panic!(),
        };
        for s in steps {
            match s {
                // Terminal.app 처럼 ESC b 로 도착
                Action::WordLeft => key(&mut a, KeyCode::Char('b'), KeyModifiers::ALT),
                Action::WordRight => key(&mut a, KeyCode::Char('f'), KeyModifiers::ALT),
                // 줄 처음/끝은 두 번째 탭이 도착하지 않는다 — 건너뛴다
                Action::LineStart | Action::LineEnd => press(&mut a, KeyCode::Tab),
                _ => press(&mut a, arrow(s)),
            }
        }
        assert!(draw(&a, 80, 24).contains("감응 시험 끝"));
        assert!(!a.calib.observable(Action::LineStart));
        assert!(
            a.edge_for(Action::LineStart).is_none(),
            "관측 불가 수는 Par 에서 빠진다"
        );
        let saved =
            a.db.as_ref()
                .unwrap()
                .kv_get("dojo.calib.TestTerm")
                .unwrap();
        assert!(saved.contains("LineStart"));
        // 주황띠 2과(줄 처음과 끝)는 이 터미널에서 미보유가 된다
        a.free = true;
        a.screen = Screen::Intro("orange.2".into());
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Intro(_)));
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
        a.free = true;
        for id in [
            "yellow.1",
            "yellow.2",
            "yellow.3",
            "yellow.boss",
            "orange.1",
            "orange.3",
            "orange.4",
            "orange.boss",
            "purple.1",
            "purple.2",
            "purple.3",
            "purple.4",
            "purple.5",
            "purple.boss",
        ] {
            a.screen = Screen::Intro(id.into());
            println!("{}", draw(&a, 80, 24));
            press(&mut a, KeyCode::Enter);
            let later = Instant::now() + Duration::from_millis(4200);
            a.on_tick(later);
            println!("{}", draw(&a, 80, 24));
            press(&mut a, KeyCode::Esc);
            press(&mut a, KeyCode::Esc);
        }
        a.start_calib();
        println!("{}", draw(&a, 80, 24));
        a.screen = Screen::Codex;
        println!("{}", draw(&a, 80, 24));
    }
}
