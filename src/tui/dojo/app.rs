//! dojo 앱 상태와 화면 전환·키 처리

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use kmd_core::dojo::{self as store, DojoProgress, DojoRun, DojoScroll, DojoSeal, DojoStreak};
use kmd_core::Database;

use super::action::{Action, Os};
use super::binding::Bindings;
use super::calib::{Calib, CalibEvent, CalibRun, KEY_ACTIONS};
use super::content::{content, Content, Lesson, Request, SenseiLines, STAGE_KINDS};
use super::diagnose::{self, Diagnosis};
use super::gauntlet::{self, Gauntlet};
use super::golf::{Edge, GolfRound};
use super::ime::ImeRound;
use super::maze::MazeRound;
use super::mouse::{MouseRound, ARENA_ORIGIN};
use super::play::{OutcomeRow, Play, Rng, RoundEvent};
use super::precheck::{self, Report};
use super::progress::{self, Access, Standing};
use super::quiz::QuizRound;
use super::score::Mark;
use super::spar::Spar;
use super::summon::{Fixture, Round};

/// kv_store 키 — 입문 의식을 마쳤는가
const KV_INITIATED: &str = "dojo.initiated";
/// kv_store 키 — 마지막으로 본 배치 지문 (docs/10 §6-5)
const KV_KEYMAP_FP: &str = "dojo.keymap_fp";
/// kv_store 키 접두 — 터미널별 키 감응 보정 (docs/10 §6-2)
const KV_CALIB: &str = "dojo.calib.";

/// 도장 밖 실전을 열려면 필요한 실행 기록 수 (docs/10 §4-4)
const REAL_MIN: usize = 10;
/// 비급 도감에서 연 권별 시험의 과 id — 커리큘럼에 없다
const CODEX_QUIZ: &str = "codex";

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
    /// S19 배치 진단 — (진단, 스크롤)
    Diagnose(Box<Diagnosis>, usize),
    /// S17 결과 카드
    Card,
    /// 인장 — 코드 내보내기·가져오기 (docs/10 §8-4)
    Seals(Box<SealView>),
}

/// 인장 화면 상태
#[derive(Debug, Default)]
pub struct SealView {
    /// 가져오기 — 붙여 넣는 중인 코드
    pub input: Option<String>,
    /// 이 OS 인장의 내보내기 코드
    pub code: Option<String>,
    /// 마지막 결과 한 줄 (성공?, 내용)
    pub msg: Option<(bool, String)>,
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
    (MenuItem::Daily, "오늘의 수련", "2분"),
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
    /// 도장 밖 실전 판이었다 (R 로 다시 하면 실전으로)
    pub real: bool,
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
    pub streak: DojoStreak,
    pub seals: Vec<DojoSeal>,
    /// 라운드에서 도착한 수 — 판이 끝나면 `dojo_keys` 에 더한다 (결과 카드의 많이 쓴 수)
    key_counts: BTreeMap<String, u32>,
    fixture: Option<Fixture>,
    /// 도장 밖 실전 — 내 실제 인덱스 (처음 쓸 때 읽는다)
    real_fixture: Option<Fixture>,
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
            streak: DojoStreak::default(),
            seals: Vec::new(),
            key_counts: BTreeMap::new(),
            fixture: None,
            real_fixture: None,
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
        app.reload_progress();
        app.sensei_line = app.say("lobby", |s| &s.lobby);
        if keymap_changed && initiated {
            // S19 — 배치 지문이 바뀐 뒤 첫 입장
            app.open_diagnose();
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
            self.streak = db.dojo_streak();
            self.seals = db.dojo_seals();
        }
        self.apply_conditions();
        if self.earn_seal() {
            self.apply_conditions();
        }
    }

    /// 기록이 아니라 조건으로 끝나는 과 — 연속일(검은띠)·인장 수(단). 저장하지 않고
    /// 매번 계산해 최고 기록 자리에 끼워 넣는다(진실 소스 하나).
    fn apply_conditions(&mut self) {
        for b in &self.content.curriculum.belt {
            for l in &b.lesson {
                let met = match (l.days, l.seals) {
                    (Some(d), _) => self.streak.best >= d,
                    (_, Some(n)) => self.seals.len() as u32 >= n,
                    _ => continue,
                };
                if met {
                    let p = self.progress.entry(l.id.clone()).or_default();
                    p.best_stars = 3;
                } else {
                    self.progress.remove(&l.id);
                }
            }
        }
        let has = |a: Action| self.bindings.has(a) && self.calib.observable(a);
        self.standing = progress::standing(&self.content.curriculum, &self.progress, &has);
    }

    /// 검은띠를 땄으면 이 OS 의 인장을 찍는다. 새로 찍었으면 `true`.
    fn earn_seal(&mut self) -> bool {
        let os = std::env::consts::OS;
        let Some(black) = self
            .content
            .curriculum
            .belt
            .iter()
            .position(|b| b.id == "black")
        else {
            return false;
        };
        if !self.standing.belts[black].completed || self.seals.iter().any(|s| s.os == os) {
            return false;
        }
        let Some(db) = &self.db else {
            return false;
        };
        if db.dojo_add_seal(os, now_secs(), "local").unwrap_or(false) {
            self.seals = db.dojo_seals();
            self.notify(&format!(
                "◆ {} 인장이 찍혔다 — 단(段)의 인장 화면에서 코드로 다른 기기에 옮긴다",
                os_name(os)
            ));
            return true;
        }
        false
    }

    /// 로컬 날짜 — 오늘의 수련 씨앗
    pub fn today(&self) -> String {
        self.db
            .as_ref()
            .map_or_else(|| "today".into(), Database::dojo_today)
    }

    /// 도장 밖 손버릇 (docs/19) — 최근 7일 레이어 수 횟수와 맨 화살표 비율.
    /// 통계를 켜지 않았거나 모인 것이 없으면 `None`.
    pub fn real_usage(&self) -> Option<(u64, Option<u64>)> {
        let (rows, active) = self.db.as_ref()?.key_stats_since(7);
        let km = kmd_core::keymap::effective_keymap(&self.config.launcher.keymap);
        let r = kmd_core::keystats::KeyReport::build(&rows, active, 7, &km);
        (!r.is_empty()).then(|| (r.total_moves(), r.raw_pct()))
    }

    /// 오늘의 수련 단계 미리보기 (입문 화면)
    pub fn daily_preview(&self) -> Vec<Lesson> {
        gauntlet::daily_stages(self.content, &|l| self.lesson_open(l), &self.today())
    }

    /// 지도에서 들어갈 수 있는 과인가 (자유 수련이 아니어도) — 오늘의 수련 재료
    fn lesson_open(&self, l: &Lesson) -> bool {
        let Some((belt, _, _)) = self.content.lesson(&l.id) else {
            return false;
        };
        !l.is_boss() && self.standing.access(belt, l, &self.has()) == Access::Open
    }

    pub fn stars(&self, lesson_id: &str) -> u8 {
        self.progress.get(lesson_id).map_or(0, |p| p.best_stars)
    }

    /// 지도에서 보이는 띠 행 수
    pub fn map_rows(&self) -> usize {
        self.content.curriculum.belt.len()
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
            self.notify(
                "배치가 바뀌었다 — 안내를 네 배치로 다시 그렸다. 입문 의식에서 D 로 진단을 본다.",
            );
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
        self.start_round_with(lesson_id, false);
    }

    /// `unlocked`: 띠 잠금을 건너뛴다 (입구의 오늘의 수련)
    fn start_round_with(&mut self, lesson_id: &str, unlocked: bool) {
        let Some((belt, _, lesson)) = self.content.lesson(lesson_id) else {
            return;
        };
        match self.access(belt, lesson) {
            Access::Open => {}
            Access::LockedBelt | Access::LockedBoss if unlocked => {}
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
        if lesson.kind == "seal" {
            self.open_seals();
            return;
        }
        let now = Instant::now();
        match self.build_play(lesson, now) {
            Some(p) => {
                self.pause = None;
                self.shake = None;
                self.sensei_line = self.bindings.render(&lesson.why);
                match p.inner() {
                    Play::Ime(_) => self.notify(
                        "OS 한글 입력기(두벌식)를 켜 두어라 — 판정은 도착한 글자로 한다",
                    ),
                    Play::Golf(_) if !self.calib.done => self.notify(
                        "키 감응 시험 전이다 — 수가 안 먹으면 입문 의식에서 C 로 이 터미널을 보정하라",
                    ),
                    _ => {}
                }
                if lesson.kind == "daily" && self.streak.today_done {
                    self.notify(
                        "오늘은 이미 마쳤다 — 이 판은 연습이다. 연속일은 내일 다시 이어라.",
                    );
                }
                self.screen = Screen::Round(Box::new(p));
            }
            None => self.notify("이 과의 수련 자료를 찾지 못했다"),
        }
    }

    /// 과 하나의 라운드를 만든다 — 이어 붙이기의 단계도 같은 길로 만든다
    fn build_play(&mut self, lesson: &Lesson, now: Instant) -> Option<Play> {
        let content = self.content;
        match lesson.kind.as_str() {
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
            "spar" => {
                // 카드에는 이 배치·터미널에서 이벤트 하나로 도착하는 수만 — 2탭(줄 처음)은
                // 첫 탭이 다른 수로 먼저 도착해 오답이 된다
                let mut l = lesson.clone();
                l.moves.retain(|m| {
                    Action::from_id(m)
                        .and_then(|a| self.edge_for(a))
                        .is_some_and(|e| e.seq.len() == 1)
                });
                Some(Play::Spar(Spar::new(&l, now)))
            }
            "maze" => MazeRound::new(content, lesson, now).map(Play::Maze),
            "mouse" => MouseRound::new(lesson, now).map(Play::Mouse),
            "ime" => ImeRound::new(content, lesson, now).map(Play::Ime),
            "quiz" => {
                let ids = self.quiz_scrolls(lesson.count.unwrap_or(10) as usize);
                let seed = format!("{}.{}", lesson.id, now_secs());
                QuizRound::new(content, &lesson.id, &ids, &seed, false, now).map(Play::Quiz)
            }
            "mixed" => {
                let stages = lesson
                    .stage
                    .iter()
                    .filter_map(|st| {
                        let (_, _, src) = content.lesson(&st.from)?;
                        STAGE_KINDS
                            .contains(&src.kind.as_str())
                            .then(|| src.staged(st))
                    })
                    .collect();
                self.gauntlet(lesson, stages, now)
            }
            "daily" => {
                let day = self.today();
                let stages = gauntlet::daily_stages(content, &|l| self.lesson_open(l), &day);
                self.gauntlet(lesson, stages, now)
            }
            _ => None,
        }
    }

    /// 이어 붙이기 — 첫 단계를 만들 수 있을 때까지 앞에서부터 시도한다
    fn gauntlet(&mut self, lesson: &Lesson, mut stages: Vec<Lesson>, now: Instant) -> Option<Play> {
        while !stages.is_empty() {
            if let Some(first) = self.build_play(&stages[0].clone(), now) {
                let g = Gauntlet::new(lesson, stages, first, now);
                return Some(Play::Mixed(Box::new(g)));
            }
            stages.remove(0);
        }
        None
    }

    /// 다음 단계를 넣는다 — 만들 수 없는 단계는 건너뛴다
    fn advance_gauntlet(&mut self) {
        loop {
            let next = match &self.screen {
                Screen::Round(p) => match p.as_ref() {
                    Play::Mixed(g) if g.pending_next => g.next_stage().cloned(),
                    _ => return,
                },
                _ => return,
            };
            let Some(lesson) = next else {
                return;
            };
            let now = Instant::now();
            let built = self.build_play(&lesson, now);
            let line = format!("{} — {}", lesson.title, self.bindings.render(&lesson.why));
            let Screen::Round(p) = &mut self.screen else {
                return;
            };
            let Play::Mixed(g) = p.as_mut() else {
                return;
            };
            match built {
                Some(play) => {
                    g.push(play, now);
                    self.sensei_line = line;
                    return;
                }
                None => {
                    if g.skip() == RoundEvent::Finished {
                        self.finish_round(true);
                        return;
                    }
                }
            }
        }
    }

    /// 종합 시험 문항 — 연 비급 중 숙련이 낮은 것부터, 모자라면 아직 못 연 권으로 채운다
    fn quiz_scrolls(&self, n: usize) -> Vec<String> {
        let mut rng = Rng::seeded(&format!("quiz.{}", now_secs()));
        let mut open: Vec<(u8, u64, String)> = self
            .content
            .scrolls
            .scroll
            .iter()
            .filter_map(|s| {
                self.scrolls
                    .get(&s.id)
                    .map(|st| (st.mastery, rng.next(), s.id.clone()))
            })
            .collect();
        open.sort();
        let mut ids: Vec<String> = open.into_iter().map(|(_, _, id)| id).collect();
        for s in &self.content.scrolls.scroll {
            if ids.len() >= n {
                break;
            }
            if !ids.contains(&s.id) {
                ids.push(s.id.clone());
            }
        }
        ids.truncate(n);
        ids
    }

    /// 비급 도감에서 그 권의 시험 (S16)
    fn start_codex_quiz(&mut self, scroll: &str) {
        if !self.scrolls.contains_key(scroll) {
            self.notify("아직 열지 않은 비급이다 — 과를 넘어 먼저 열어라");
            return;
        }
        let seed = format!("codex.{scroll}.{}", now_secs());
        let now = Instant::now();
        if let Some(q) = QuizRound::new(
            self.content,
            CODEX_QUIZ,
            &[scroll.to_string()],
            &seed,
            true,
            now,
        ) {
            self.pause = None;
            self.sensei_line = "외운 자는 잊고, 납득한 자는 쓴다.".into();
            self.screen = Screen::Round(Box::new(Play::Quiz(q)));
        }
    }

    /// 도장 밖 실전 — 내 인덱스 위에서, 내 실행 기록으로 의뢰를 낸다 (docs/10 §4-4)
    fn start_real(&mut self, lesson_id: &str) {
        let Some((_, _, lesson)) = self.content.lesson(lesson_id) else {
            return;
        };
        let history: Vec<_> = self
            .db
            .as_ref()
            .map(|d| d.query_history(80))
            .unwrap_or_default()
            .into_iter()
            .filter(|h| matches!(h.item_type.as_str(), "app" | "file" | "dir"))
            .collect();
        if history.len() < REAL_MIN {
            self.notify(&format!(
                "실행 기록이 {}건이다 — {REAL_MIN}건이 쌓이면 도장 밖 실전이 열린다",
                history.len()
            ));
            return;
        }
        let config = self.config.clone();
        let fixture = self
            .real_fixture
            .get_or_insert_with(|| Fixture::from_engine(crate::cmd::create_search_engine(&config)));
        let mut rng = Rng::seeded(&format!("real.{}", now_secs()));
        let mut picks: Vec<Request> = Vec::new();
        for h in history {
            // 기록의 표시 이름이 인덱스에 그대로 있어야 의뢰가 된다
            let name = h
                .display
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&h.display)
                .to_string();
            if name.is_empty() || fixture.rank_of(&name, &name).is_none() {
                continue;
            }
            let kind = match h.item_type.as_str() {
                "app" => "앱",
                "dir" => "폴더",
                _ => "파일",
            };
            picks.push(Request {
                ask: format!(
                    "자주 여는 {kind} 「{name}」 를 불러라 (실행 {}회)",
                    h.frequency
                ),
                target: Some(name),
                accept: Vec::new(),
            });
            if picks.len() >= 15 {
                break;
            }
        }
        // 자주 쓰는 15개 중 다섯을 섞어서
        for i in (1..picks.len()).rev() {
            picks.swap(i, rng.below(i as u64 + 1) as usize);
        }
        picks.truncate(5);
        match Round::real(lesson, picks, fixture) {
            Some(r) => {
                self.pause = None;
                self.sensei_line = "도장 밖이다. 연습 목록이 아니라 네 컴퓨터의 수만 개 항목 위에서 불러라.".into();
                self.screen = Screen::Round(Box::new(Play::Summon(r)));
            }
            None => self.notify("인덱스에서 실행 기록의 항목을 찾지 못했다 — kmd index 로 인덱스를 새로 만들어 보라"),
        }
    }

    fn open_seals(&mut self) {
        let os = std::env::consts::OS;
        let code = self.seals.iter().find(|s| s.os == os).map(|s| {
            store::seal_encode(&store::SealCode {
                os: s.os.clone(),
                belt: "black".into(),
                at: s.earned_at,
            })
        });
        self.screen = Screen::Seals(Box::new(SealView {
            code,
            ..Default::default()
        }));
    }

    /// 인장 코드를 가져온다
    fn import_seal(&mut self, code: &str) -> (bool, String) {
        let seal = match store::seal_decode(code) {
            Ok(s) => s,
            Err(e) => return (false, e),
        };
        let Some(db) = &self.db else {
            return (false, "기록 DB 를 열 수 없어 인장을 남기지 못한다".into());
        };
        match db.dojo_add_seal(&seal.os, seal.at, "imported") {
            Ok(true) => {
                self.reload_progress();
                (
                    true,
                    format!(
                        "◆ {} 인장을 들였다 — 인장 {}/3",
                        os_name(&seal.os),
                        self.seals.len()
                    ),
                )
            }
            Ok(false) => (true, format!("{} 인장은 이미 있다", os_name(&seal.os))),
            Err(e) => (false, format!("저장 실패: {e}")),
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
        if let Screen::Seals(v) = &mut self.screen {
            if let Some(input) = &mut v.input {
                input.push_str(text.trim());
            }
            return;
        }
        if self.pause.is_some() {
            return;
        }
        let mut fixture = self.fixture.take();
        let mut real = self.real_fixture.take();
        let content = self.content;
        if let Screen::Round(play) = &mut self.screen {
            if let Play::Summon(round) = play.inner_mut() {
                let fx = if round.real {
                    real.as_mut()
                } else {
                    Some(fixture.get_or_insert_with(|| Fixture::new(content)))
                };
                if let Some(fx) = fx {
                    round.on_paste(text, fx);
                }
            }
        }
        self.fixture = fixture;
        self.real_fixture = real;
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
            Screen::Round(play) => {
                let ev = match play.inner_mut() {
                    Play::Mouse(r) => r.on_mouse(ev.kind, x, y, now),
                    Play::Ime(r) => r.on_mouse(ev.kind, x, y, now),
                    _ => return false,
                };
                match play.as_mut() {
                    Play::Mixed(g) => g.absorb(ev, now),
                    _ => ev,
                }
            }
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
                KeyCode::Char('d') | KeyCode::Char('D') => self.open_diagnose(),
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
                let summon = self
                    .content
                    .lesson(&id)
                    .is_some_and(|(_, _, l)| l.kind == "summon");
                match key.code {
                    KeyCode::Enter | KeyCode::Char(' ') => self.start_round(&id),
                    KeyCode::Char('r') | KeyCode::Char('R') if summon => self.start_real(&id),
                    KeyCode::Esc => self.open_map_at(&id),
                    _ => {}
                }
            }
            Screen::Result(ref out) => {
                let (id, real) = (out.lesson_id.clone(), out.real);
                match key.code {
                    KeyCode::Char('r') | KeyCode::Char('R') if real => self.start_real(&id),
                    KeyCode::Char('r') | KeyCode::Char('R') => self.start_round(&id),
                    KeyCode::Char('c') | KeyCode::Char('C') => self.screen = Screen::Card,
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
                Nav::Enter => {
                    let id = self.content.scrolls.scroll[self.codex_sel].id.clone();
                    self.start_codex_quiz(&id);
                }
                Nav::Back => self.screen = Screen::Lobby,
                _ => {}
            },
            Screen::Records => {
                if matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
                    self.screen = Screen::Card;
                } else if matches!(self.nav(key), Nav::Back | Nav::Enter) {
                    self.screen = Screen::Lobby;
                }
            }
            Screen::Card => match key.code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    let text = super::card::text(self);
                    match kmd_core::clipboard::set_text(&text) {
                        Ok(()) => self.notify("결과 카드를 클립보드에 담았다 — 붙여 넣어 자랑하라"),
                        Err(e) => self.notify(&format!("클립보드에 담지 못했다: {e}")),
                    }
                }
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') | KeyCode::Char('q') => {
                    self.screen = Screen::Lobby;
                }
                _ => {}
            },
            Screen::Seals(_) => self.seals_key(key),
            Screen::Diagnose(ref d, scroll) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    let s = scroll.saturating_sub(1);
                    self.set_diag_scroll(s);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let s = (scroll + 1).min(d.rows.len().saturating_sub(1));
                    self.set_diag_scroll(s);
                }
                // 권장: 입문 의식을 다시 — 바뀐 배치로 감응 시험
                KeyCode::Enter => self.screen = Screen::Precheck,
                KeyCode::Char(' ') | KeyCode::Esc => self.screen = Screen::Lobby,
                _ => {}
            },
            Screen::Round(_) | Screen::Calib(_) => unreachable!(),
        }
    }

    fn seals_key(&mut self, key: KeyEvent) {
        let Screen::Seals(v) = &mut self.screen else {
            return;
        };
        if let Some(input) = &mut v.input {
            match key.code {
                KeyCode::Esc => v.input = None,
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    let code = input.clone();
                    let msg = self.import_seal(&code);
                    let ok = msg.0;
                    if let Screen::Seals(v) = &mut self.screen {
                        v.msg = Some(msg);
                        if ok {
                            v.input = None;
                        }
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(c);
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('i') | KeyCode::Char('I') => {
                v.input = Some(String::new());
                v.msg = None;
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                let msg = match &v.code {
                    Some(code) => match kmd_core::clipboard::set_text(code) {
                        Ok(()) => (true, "인장 코드를 클립보드에 담았다".to_string()),
                        Err(e) => (false, format!("클립보드에 담지 못했다: {e}")),
                    },
                    None => (
                        false,
                        "이 OS 의 인장이 아직 없다 — 검은띠를 먼저 따라".to_string(),
                    ),
                };
                v.msg = Some(msg);
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('q') => {
                self.open_map_at("dan.1");
            }
            _ => {}
        }
    }

    fn open_diagnose(&mut self) {
        let d = diagnose::from_config(&self.config, &self.bindings, self.content);
        self.screen = Screen::Diagnose(Box::new(d), 0);
    }

    fn set_diag_scroll(&mut self, to: usize) {
        if let Screen::Diagnose(_, s) = &mut self.screen {
            *s = to;
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
                MenuItem::Daily => self.start_round_with("black.1", true),
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
        if let Some(a) = action {
            *self.key_counts.entry(a.id().to_string()).or_default() += 1;
        }
        // 선택을 모르는 모드에서 Shift 는 그냥 이동이다
        let bare = action.map(|a| a.unshifted().unwrap_or(a));
        let mut fixture = self.fixture.take();
        let mut real = self.real_fixture.take();
        let content = self.content;
        let Screen::Round(play) = &mut self.screen else {
            self.fixture = fixture;
            self.real_fixture = real;
            return;
        };
        let ev = match play.inner_mut() {
            Play::Summon(r) => {
                let fx = if r.real {
                    real.as_mut()
                } else {
                    Some(fixture.get_or_insert_with(|| Fixture::new(content)))
                };
                fx.map_or(RoundEvent::None, |fx| r.on_key(key, fx))
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
            Play::Spar(r) => bare.map_or(RoundEvent::None, |a| r.on_action(a, now)),
            Play::Maze(r) => match key.code {
                KeyCode::Tab => {
                    r.hint();
                    RoundEvent::None
                }
                _ => bare.map_or(RoundEvent::None, |a| r.on_action(a, now)),
            },
            Play::Mouse(r) => r.on_key(&key, now),
            Play::Ime(r) => r.on_key(&key, now),
            Play::Quiz(r) => r.on_key(&key, now),
            Play::Mixed(_) => RoundEvent::None,
        };
        let ev = match play.as_mut() {
            Play::Mixed(g) => g.absorb(ev, now),
            _ => ev,
        };
        self.fixture = fixture;
        self.real_fixture = real;
        self.after_round_event(ev);
    }

    fn after_round_event(&mut self, event: RoundEvent) {
        let Screen::Round(play) = &self.screen else {
            return;
        };
        let streak = play.fail_streak();
        let mistakes = play.total_mistakes();
        let combo = match play.inner() {
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
            if let Play::Summon(round) = play.inner() {
                if round.hangul_warn {
                    self.sensei_line = self.say("hangul", |s| &s.hangul);
                }
            }
        }
        self.advance_gauntlet();
    }

    fn restart_round(&mut self) {
        if let Screen::Round(play) = &self.screen {
            let id = play.lesson_id().to_string();
            match play.inner() {
                Play::Summon(r) if r.real => self.start_real(&id),
                Play::Quiz(q) if q.codex => {
                    let scroll = q.question().scroll.clone();
                    self.start_codex_quiz(&scroll);
                }
                _ => self.start_round_with(&id, id == "black.1"),
            }
        }
    }

    fn abandon_round(&mut self) {
        self.pause = None;
        self.flush_keys();
        if let Screen::Round(play) = &self.screen {
            let id = play.lesson_id().to_string();
            if id == CODEX_QUIZ {
                self.screen = Screen::Codex;
            } else {
                self.open_map_at(&id);
            }
        }
    }

    /// 판에서 쓴 수를 더한다 (결과 카드)
    fn flush_keys(&mut self) {
        let counts = std::mem::take(&mut self.key_counts);
        if let (Some(db), false) = (&self.db, counts.is_empty()) {
            let _ = db.dojo_add_keys(&counts);
        }
    }

    /// 라운드를 마치고 기록·해금·승급을 처리한 뒤 결과 화면으로 간다
    fn finish_round(&mut self, cleared: bool) {
        let Screen::Round(play) = std::mem::replace(&mut self.screen, Screen::Lobby) else {
            return;
        };
        self.flush_keys();
        // 비급 도감의 권별 시험 — 기록 없이 숙련만
        if let Play::Quiz(q) = play.as_ref() {
            if q.codex {
                let q = q.question();
                let ok = matches!(play.as_ref(), Play::Quiz(r) if !r.correct.is_empty());
                let mastery = match (&self.db, ok) {
                    (Some(db), true) => db.dojo_master_scroll(&q.scroll).ok().flatten(),
                    _ => None,
                };
                self.reload_progress();
                self.notify(&match mastery {
                    Some(m) => format!("卷{} 숙련 {}", q.scroll, "★".repeat(m as usize)),
                    None if ok => format!("卷{} 정답", q.scroll),
                    None => format!("卷{} 오답 — 본문을 다시 읽어라", q.scroll),
                });
                self.screen = Screen::Codex;
                return;
            }
        }
        let Some((_, _, lesson)) = self.content.lesson(play.lesson_id()) else {
            return;
        };
        let mut sum = play.summary(cleared);
        let duration = play.started().elapsed();
        let real = matches!(play.inner(), Play::Summon(r) if r.real);
        let mut streak = None;

        let before = self.standing.reach;
        let mut new_best = false;
        let mut new_scroll = None;
        if let Some(db) = &self.db {
            let run = DojoRun {
                // 도장 밖 실전은 인덱스가 사람마다 달라 과의 최고 기록에 섞지 않는다
                lesson: if real {
                    "real.summon".into()
                } else {
                    lesson.id.clone()
                },
                kind: if real {
                    "real".into()
                } else {
                    lesson.kind.clone()
                },
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
            new_best = db.dojo_record_run(&run).unwrap_or(false) && !real;
            if sum.stars > 0 && !real {
                if let Some(s) = &lesson.scroll {
                    if db.dojo_unlock_scroll(s).unwrap_or(false) {
                        new_scroll = Some(s.clone());
                    }
                }
            }
            if let (Play::Quiz(q), true) = (play.as_ref(), cleared) {
                for s in &q.correct {
                    let _ = db.dojo_master_scroll(s);
                }
            }
            if lesson.kind == "daily" && cleared && sum.stars > 0 {
                let _ = db.dojo_complete_daily(sum.score);
                streak = Some(db.dojo_streak().days);
            }
        }
        self.reload_progress();
        let promoted = (self.standing.reach > before).then(|| {
            self.content
                .curriculum
                .belt
                .get(self.standing.reach)
                .map_or_else(|| "모든 띠를 넘었다 — 삼국 통일".into(), |b| b.name.clone())
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
        if let Some(n) = streak {
            sum.stats
                .push(("연속 수련".into(), format!("{n}일 (검은띠 조건 3일)")));
            self.sensei_line = format!("연속 {n}일. 띠는 매는 것이 아니라 매일 다시 매는 것이다.");
        }
        self.screen = Screen::Result(Box::new(Outcome {
            lesson_id: lesson.id.clone(),
            title: if real {
                "도장 밖 실전".into()
            } else {
                lesson.title.clone()
            },
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
            real,
            ki: sum.ki,
            shown_at: Instant::now(),
        }));
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// OS id → 표시 이름
pub fn os_name(os: &str) -> &'static str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        _ => "?",
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
    use crate::tui::dojo::card;
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
        // 선택 수 = 같은 이동 + Shift
        let (a, shift) = match a.unshifted() {
            Some(base) => (base, KeyModifiers::SHIFT),
            None => (a, KeyModifiers::NONE),
        };
        match a {
            Action::WordLeft => key(app, KeyCode::Left, KeyModifiers::ALT | shift),
            Action::WordRight => key(app, KeyCode::Right, KeyModifiers::ALT | shift),
            _ => key(app, arrow(a), shift),
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
            Screen::Round(p) => match p.inner() {
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
                if l.kind == "seal" {
                    assert!(matches!(a.screen, Screen::Seals(_)), "{}: 인장 화면", l.id);
                    draw(&a, 80, 24);
                    press(&mut a, KeyCode::Esc);
                    continue;
                }
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
        for s in [Screen::Card, Screen::Seals(Box::default())] {
            a.screen = s;
            draw(&a, 80, 24);
            draw(&a, 132, 40);
        }
    }

    /// 지금 라운드(이어 붙이기면 지금 단계)를 한 걸음 푼다
    fn solve_step(a: &mut App, now: &mut Instant) {
        use crate::tui::dojo::ime::Phase as P;
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (sx, sy) = (10u16, 3u16);
        let mouse = |a: &mut App, kind, x: u16, y: u16| {
            a.on_mouse(MouseEvent {
                kind,
                column: sx + ARENA_ORIGIN.0 + x,
                row: sy + ARENA_ORIGIN.1 + y,
                modifiers: KeyModifiers::NONE,
            });
        };
        let Screen::Round(p) = &a.screen else { return };
        match p.inner() {
            Play::Summon(_) => solve_current(a),
            Play::Golf(r) => {
                let (path, edges) = (r.sage().moves.clone(), r.allowed[r.idx].clone());
                for m in path {
                    let e = edges.iter().find(|e| e.action == m).unwrap();
                    for s in &e.seq {
                        send(a, *s);
                    }
                }
                press(a, KeyCode::Enter);
            }
            Play::Spar(s) => {
                let want = s.prompt().unwrap();
                send(a, want);
            }
            Play::Quiz(q) => {
                let ans = q.question().answer;
                press(a, KeyCode::Char(char::from(b'1' + ans as u8)));
                press(a, KeyCode::Enter);
            }
            Play::Ime(r) => match r.phase {
                P::Type => {
                    let line = r.line().to_string();
                    type_str(a, &line);
                }
                P::Click => {
                    let t = r.target.unwrap();
                    mouse(a, MouseEventKind::Down(MouseButton::Left), t.x + 1, t.y);
                }
                P::LineDone(_) => {
                    *now += Duration::from_secs(1);
                    a.on_tick(*now);
                }
            },
            Play::Mouse(r) => {
                let t = r.target().unwrap();
                let (rect, button) = (t.rect, t.button);
                let (x, y) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
                mouse(a, MouseEventKind::Moved, x, y);
                if let Some(b) = button {
                    mouse(a, MouseEventKind::Down(b), x, y);
                    mouse(a, MouseEventKind::Up(b), x, y);
                }
            }
            Play::Maze(r) => {
                for m in r.sages[r.idx].clone() {
                    press(a, arrow(m));
                }
                *now += Duration::from_secs(2);
                a.on_tick(*now);
            }
            Play::Mixed(_) => unreachable!(),
        }
    }

    fn play_out(a: &mut App, id: &str) {
        a.on_resize(100, 30);
        a.screen = Screen::Intro(id.into());
        press(a, KeyCode::Enter);
        let mut now = Instant::now();
        let mut guard = 0;
        while matches!(a.screen, Screen::Round(_)) {
            solve_step(a, &mut now);
            guard += 1;
            assert!(guard < 500, "{id}: 끝나지 않는다");
        }
    }

    #[test]
    fn 하루치_실전은_다섯_단계를_이어_완주한다() {
        let mut a = app();
        a.free = true;
        a.on_resize(100, 30);
        a.screen = Screen::Intro("brown.3".into());
        assert!(
            draw(&a, 80, 24).contains("아침 — 의뢰 소환"),
            "입문에 단계 목록"
        );
        press(&mut a, KeyCode::Enter);
        let text = draw(&a, 100, 30);
        assert!(text.contains("◈"), "머리줄에 단계 표지\n{text}");
        let mut now = Instant::now();
        let mut kinds = Vec::new();
        let mut guard = 0;
        while let Screen::Round(p) = &a.screen {
            let k = std::mem::discriminant(p.inner());
            if kinds.last() != Some(&k) {
                kinds.push(k);
            }
            solve_step(&mut a, &mut now);
            guard += 1;
            assert!(guard < 300, "끝나지 않는다\n{}", draw(&a, 100, 30));
        }
        assert!(kinds.len() >= 4, "단계마다 다른 라운드");
        let Screen::Result(out) = &a.screen else {
            panic!("결과가 아니다")
        };
        assert!(out.cleared);
        assert!(out.stars >= 2, "{}", out.stars);
        assert_eq!(out.stats.len(), 5, "{:?}", out.stats);
        assert!(draw(&a, 80, 24).contains("저녁 — 문단 손질"));
    }

    #[test]
    fn 혼합_대련과_종합_시험을_넘는다() {
        let mut a = app();
        a.free = true;
        play_out(&mut a, "brown.1");
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert!(out.cleared && out.stars >= 2);

        // 비급 둘을 열어 둔다 — 시험은 연 권부터 낸다
        for s in ["七", "九"] {
            a.db.as_ref().unwrap().dojo_unlock_scroll(s).unwrap();
        }
        a.reload_progress();
        a.screen = Screen::Intro("brown.2".into());
        press(&mut a, KeyCode::Enter);
        let Screen::Round(p) = &a.screen else {
            panic!()
        };
        let Play::Quiz(q) = p.as_ref() else { panic!() };
        assert_eq!(q.questions.len(), 10);
        let first: Vec<_> = q.questions[..2].iter().map(|q| q.scroll.clone()).collect();
        assert!(first.contains(&"七".to_string()) && first.contains(&"九".to_string()));
        assert!(draw(&a, 80, 24).contains("비급 시험"));
        play_out(&mut a, "brown.2");
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert_eq!(out.stars, 3);
        assert_eq!(a.scrolls["七"].mastery, 1, "맞힌 권은 숙련이 오른다");
    }

    #[test]
    fn 관문_이탈마는_게이지를_달고_시작한다() {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Intro("brown.boss".into());
        press(&mut a, KeyCode::Enter);
        let Screen::Round(p) = &a.screen else {
            panic!()
        };
        assert!(p.boss().is_some());
        let text = draw(&a, 80, 24);
        assert!(text.contains("마") && text.contains("기"), "{text}");
        assert!(text.contains("第一關"), "첫 단계 표지\n{text}");
    }

    #[test]
    fn 오늘의_수련은_입구에서_바로_들어가_연속일을_남긴다() {
        let mut a = app();
        a.screen = Screen::Lobby;
        a.lobby_sel = MENU.iter().position(|m| m.0 == MenuItem::Daily).unwrap();
        press(&mut a, KeyCode::Enter);
        assert!(
            matches!(&a.screen, Screen::Round(p) if matches!(p.as_ref(), Play::Mixed(_))),
            "흰띠여도 오늘의 수련은 열린다"
        );
        let mut now = Instant::now();
        let mut guard = 0;
        while matches!(a.screen, Screen::Round(_)) {
            solve_step(&mut a, &mut now);
            guard += 1;
            assert!(guard < 300, "끝나지 않는다\n{}", draw(&a, 100, 30));
        }
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert!(out.cleared);
        assert!(
            out.stats.iter().any(|(k, _)| k == "연속 수련"),
            "{:?}",
            out.stats
        );
        assert_eq!(a.streak.days, 1);
        assert!(a.streak.today_done);
        assert!(draw(&a, 80, 24).contains("연속 수련"));
        a.screen = Screen::Lobby;
        assert!(draw(&a, 80, 24).contains("✓ 연속 1일"));
    }

    #[test]
    fn 사흘을_이으면_검은띠와_인장이다() {
        let mut a = app();
        a.free = true;
        // 갈색띠까지 마친 기록 — 조건 과(검은띠·단)는 빼고
        let db = a.db.take().unwrap();
        for b in &a.content.curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                if l.days.is_some() || l.seals.is_some() || b.id == "dan" {
                    continue;
                }
                db.dojo_record_run(&DojoRun {
                    lesson: l.id.clone(),
                    kind: l.kind.clone(),
                    stars: 1,
                    os: "macos".into(),
                    ..Default::default()
                })
                .unwrap();
            }
        }
        a.db = Some(db);
        a.reload_progress();
        let black = a
            .content
            .curriculum
            .belt
            .iter()
            .position(|b| b.id == "black")
            .unwrap();
        assert_eq!(a.standing.current, black, "연속일 전에는 검은띠 수련 중");
        assert!(a.seals.is_empty());

        a.streak = DojoStreak {
            days: 3,
            today_done: true,
            best: 3,
            total: 3,
        };
        a.apply_conditions();
        assert!(a.earn_seal());
        assert!(a.standing.belts[black].completed);
        assert_eq!(a.seals.len(), 1);
        assert_eq!(a.seals[0].os, std::env::consts::OS);
        assert!(!a.earn_seal(), "인장은 한 번만");

        // 다른 기기의 인장 코드를 붙여 넣는다
        let other = if std::env::consts::OS == "linux" {
            "windows"
        } else {
            "linux"
        };
        let code = store::seal_encode(&store::SealCode {
            os: other.into(),
            belt: "black".into(),
            at: 1,
        });
        a.screen = Screen::Intro("dan.1".into());
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Seals(_)));
        press(&mut a, KeyCode::Char('i'));
        a.on_paste(&code);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.seals.len(), 2);
        let text = draw(&a, 80, 24);
        assert!(text.contains("인장을 들였다"), "{text}");
        // 들이면서 DB 의 연속일(0)로 다시 계산됐다 — 시험용 연속일을 되살린다
        a.streak.best = 3;
        a.apply_conditions();
        a.screen = Screen::Card;
        let text = draw(&a, 80, 24);
        assert!(text.contains("검은띠") && text.contains("◆"), "{text}");
    }

    #[test]
    fn 비급_도감에서_권별_시험을_본다() {
        let mut a = app();
        a.db.as_ref().unwrap().dojo_unlock_scroll("一").unwrap();
        a.reload_progress();
        a.screen = Screen::Codex;
        a.codex_sel = 0;
        press(&mut a, KeyCode::Enter);
        let Screen::Round(p) = &a.screen else {
            panic!("시험이 열려야")
        };
        let Play::Quiz(q) = p.as_ref() else { panic!() };
        let wrong = (q.question().answer + 1) % q.question().choices.len();
        press(&mut a, KeyCode::Char(char::from(b'1' + wrong as u8)));
        let text = draw(&a, 80, 24);
        assert!(
            text.contains("다시 펼쳐"),
            "오답이면 그 권을 펼친다\n{text}"
        );
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Codex));
        assert_eq!(a.scrolls["一"].mastery, 0);
        press(&mut a, KeyCode::Enter);
        let mut now = Instant::now();
        solve_step(&mut a, &mut now);
        assert!(matches!(a.screen, Screen::Codex));
        assert_eq!(a.scrolls["一"].mastery, 1);
        assert!(
            a.db.as_ref().unwrap().dojo_recent_runs(5).is_empty(),
            "권별 시험은 기록하지 않는다"
        );
        // 잠긴 권은 시험을 열지 않는다
        a.codex_sel = 5;
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Codex));
    }

    #[test]
    fn 쓴_수는_결과_카드에_쌓인다() {
        let mut a = app();
        a.free = true;
        play_out(&mut a, "orange.1");
        let keys = a.db.as_ref().unwrap().dojo_keys();
        assert!(!keys.is_empty());
        assert!(keys.iter().any(|(k, _)| k == "WordRight"), "{keys:?}");
        let text = card::text(&a);
        assert!(text.contains("많이 쓴 수") && text.contains("█"), "{text}");
        assert!(!text.contains("실전 7일"), "손버릇 통계가 없으면 줄도 없다");
        a.db.as_ref()
            .unwrap()
            .key_stats_add(&[("move:nav:H".into(), 30), ("raw:Left".into(), 10)])
            .unwrap();
        let text = card::text(&a);
        assert!(
            text.contains("실전 7일  레이어 수 30번 · 맨 화살표 25%"),
            "{text}"
        );
        for l in text.lines() {
            assert_eq!(
                unicode_width::UnicodeWidthStr::width(l),
                card::WIDTH + 4,
                "카드 줄 폭이 고르다: {l}"
            );
        }
    }

    #[test]
    fn 실행_기록이_적으면_도장_밖_실전은_닫혀_있다() {
        let mut a = app();
        a.screen = Screen::Intro("white.1".into());
        assert!(draw(&a, 80, 24).contains("R 도장 밖 실전"));
        press(&mut a, KeyCode::Char('r'));
        assert!(matches!(a.screen, Screen::Intro(_)));
        assert!(a.notice.as_ref().unwrap().0.contains("실행 기록"));
    }

    #[test]
    fn 골프를_사부의_수로_치면_별_셋이다() {
        for id in ["orange.1", "orange.4", "green.1", "green.2", "green.boss"] {
            golf_by_sage(id);
        }
    }

    fn golf_by_sage(id: &str) {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Intro(id.into());
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
            panic!("{id}: 결과가 아니다")
        };
        assert!(out.cleared, "{id}");
        assert_eq!(out.stars, 3, "{id}");
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

    /// 한영 리듬 — 글자를 따라 치고, 줄 사이 과녁은 과녁장 좌표로 클릭한다
    #[test]
    fn 톡과_쥠은_글자와_클릭으로_끝난다() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut a = app();
        a.free = true;
        a.on_resize(100, 30);
        a.screen = Screen::Intro("blue.2".into());
        press(&mut a, KeyCode::Enter);
        assert!(a.wants_mouse(), "줄 사이 과녁이 있으니 마우스 캡처");
        let (sx, sy) = (10u16, 3u16);
        let mut now = Instant::now();
        while let Screen::Round(p) = &a.screen {
            let Play::Ime(r) = p.as_ref() else { panic!() };
            match r.phase {
                crate::tui::dojo::ime::Phase::Type => {
                    let line = r.line().to_string();
                    assert!(draw(&a, 100, 30).contains('^'), "톡 자리 표시");
                    type_str(&mut a, &line);
                }
                crate::tui::dojo::ime::Phase::Click => {
                    let t = r.target.unwrap();
                    a.on_mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: sx + ARENA_ORIGIN.0 + t.x + 1,
                        row: sy + ARENA_ORIGIN.1 + t.y,
                        modifiers: KeyModifiers::NONE,
                    });
                }
                crate::tui::dojo::ime::Phase::LineDone(_) => {
                    now += Duration::from_secs(1);
                    a.on_tick(now);
                }
            }
        }
        let Screen::Result(out) = &a.screen else {
            panic!()
        };
        assert_eq!(out.stars, 3);
    }

    #[test]
    fn 배치가_바뀐_뒤_첫_입장은_배치_진단이다() {
        let report = Report {
            backend: Backend::Daemon,
            trust: Trust::Full,
            profile: "vim-nav".into(),
            terminal: Some("TestTerm".into()),
        };
        let db = Database::open_in_memory().unwrap();
        db.kv_set(KV_INITIATED, "1").unwrap();
        db.kv_set(KV_KEYMAP_FP, "옛-지문").unwrap();
        let mut a = App::with_report(&kmd_core::Config::default(), Some(db), None, report);
        assert!(matches!(a.screen, Screen::Diagnose(..)));
        let text = draw(&a, 80, 24);
        assert!(
            text.contains("배치가 바뀌었다") && text.contains("사부의 진단"),
            "{text}"
        );
        press(&mut a, KeyCode::Down);
        press(&mut a, KeyCode::Char(' '));
        assert!(matches!(a.screen, Screen::Lobby));
        // 입문 의식에서 D 로 다시
        a.screen = Screen::Precheck;
        press(&mut a, KeyCode::Char('d'));
        assert!(matches!(a.screen, Screen::Diagnose(..)));
        press(&mut a, KeyCode::Enter);
        assert!(matches!(a.screen, Screen::Precheck));
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

    #[test]
    #[ignore]
    fn dump_m5() {
        let mut a = app();
        a.free = true;
        a.screen = Screen::Lobby;
        println!("{}", draw(&a, 80, 24));
        for b in 6..9 {
            a.map_belt = b;
            a.map_sel = 0;
            a.screen = Screen::Map;
            println!("{}", draw(&a, 80, 24));
        }
        for id in [
            "brown.1",
            "brown.2",
            "brown.3",
            "brown.boss",
            "black.1",
            "dan.1",
            "dan.boss",
        ] {
            a.screen = Screen::Intro(id.into());
            println!("{}", draw(&a, 80, 24));
            press(&mut a, KeyCode::Enter);
            println!("{}", draw(&a, 80, 24));
            if matches!(a.screen, Screen::Round(_)) {
                let later = Instant::now() + Duration::from_millis(2000);
                a.on_tick(later);
                println!("{}", draw(&a, 80, 24));
                press(&mut a, KeyCode::Esc);
                press(&mut a, KeyCode::Esc);
            } else {
                press(&mut a, KeyCode::Esc);
            }
        }
        play_out(&mut a, "brown.3");
        println!("{}", draw(&a, 80, 24));
        a.screen = Screen::Card;
        println!("{}", draw(&a, 80, 24));
    }

    #[test]
    #[ignore]
    fn dump_m3() {
        let mut a = app();
        a.free = true;
        a.open_diagnose();
        println!("{}", draw(&a, 80, 24));
        for id in ["green.1", "green.3", "green.boss", "blue.1", "blue.boss"] {
            a.screen = Screen::Intro(id.into());
            println!("{}", draw(&a, 80, 24));
            press(&mut a, KeyCode::Enter);
            println!("{}", draw(&a, 80, 24));
            match id {
                "green.1" => {
                    send(&mut a, Action::WordRight);
                    send(&mut a, Action::Right);
                    send(&mut a, Action::SelRight);
                    send(&mut a, Action::SelRight);
                }
                "green.3" => {
                    send(&mut a, Action::WordRight);
                    send(&mut a, Action::SelWordRight);
                    send(&mut a, Action::Backspace);
                }
                "blue.1" => type_str(&mut a, "kmd fh"),
                "blue.boss" => type_str(&mut a, "did"),
                _ => {}
            }
            println!("{}", draw(&a, 80, 24));
            press(&mut a, KeyCode::Esc);
            press(&mut a, KeyCode::Esc);
        }
    }
}
