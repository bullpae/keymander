//! `:m` / `:memo` — 메모 (docs/23).
//!
//! 메모 하나가 Markdown 파일 하나다. 폴더를 클라우드 드라이브에 두면 기기끼리 맞춰지고,
//! 하루 한 파일과 달리 두 기기가 같은 날 적어도 충돌 사본이 생기지 않는다.
//! 원본은 언제나 파일이고, `notes-index.json` 은 이 기기에서 다시 읽지 않으려는 캐시다 —
//! 지워져도 파일에서 다시 만든다.
//!
//! 할 일은 머리말에 `type: todo` 가 붙은 메모이고, 리마인더는 거기에 `due` 가 붙은 것이다.
//! 타이머·뽀모도로는 파일로 남기지 않는다([`timers`]).
//!
//! 판단은 여기, 화면은 UI 몫이다. 데스크톱과 TUI 는 [`NoteSession`] 을 하나씩 들고
//! 결과([`NoteSession::results`])를 그리고 Enter·단축키를 넘긴다.

pub mod timers;
pub mod when;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::index::{IndexItem, ItemKind, Source};
use crate::query_prefix::{prefix_of, QueryPrefix};
use timers::{format_span, Kind, Timer, TimerStore};

pub const NOTE_ALIASES: &[&str] = &[":memo", ":메모", ":m", ":ㅡ"];
pub const TODO_ALIASES: &[&str] = &[":todo", ":할일", ":td", ":ㅅㅇ"];
pub const TIMER_ALIASES: &[&str] = &[":timer", ":타이머"];
pub const POMO_ALIASES: &[&str] = &[":pomo", ":뽀모도로", ":뽀모"];

/// Enter = 새 메모 저장 또는 수정 저장
pub const NOTE_COMMIT_MARKER: &str = "kmd:note:commit";
/// Enter = 이 메모 복사. 뒤에 메모 id
pub const NOTE_ITEM_MARKER: &str = "kmd:note:item:";
/// 안내 행
pub const NOTE_NOOP_MARKER: &str = "kmd:note:noop";
/// 돌아가는 타이머 행. 뒤에 타이머 id
pub const NOTE_TIMER_MARKER: &str = "kmd:note:timer:";

/// `kmd-desktop --memo` — 런처를 `:m ` 상태로 연다. 레이어 키가 부른다 (N1.1)
pub const MEMO_ARG: &str = "--memo";
/// `kmd-desktop --alert <제목> <내용>` — 알림 창. 데몬이 부른다 (N3)
pub const ALERT_ARG: &str = "--alert";
/// `--memo` 로 열었을 때의 입력
pub const MEMO_SEED: &str = ":m ";

/// 목록에 보여 줄 메모 수
const RESULT_LIMIT: usize = 50;
const TRASH_DIR: &str = ".trash";
const CACHE_FILE: &str = "notes-index.json";

/// `[notes]` — 메모 설정.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct NotesConfig {
    /// 메모 폴더. 없으면 `<data_dir>/notes` (이 기기에만 저장)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
    /// 뽀모도로 집중 시간 (분)
    pub focus_minutes: u32,
    /// 뽀모도로 휴식 시간 (분)
    pub break_minutes: u32,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            dir: None,
            focus_minutes: 25,
            break_minutes: 5,
        }
    }
}

/// 메모 하나
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// 확장자 뺀 파일 이름. `20261106-143012-a1b2`
    pub id: String,
    pub body: String,
    /// 머리말(`---` 줄까지 그대로). 수정해도 지키도록 들고 있는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front_matter: Option<String>,
    /// 만든 시각(ms). 이름에서 읽고, 다른 앱이 만든 파일이면 수정 시각
    pub created_ms: i64,
}

impl Note {
    /// 첫 줄. Markdown 제목 표시는 뗀다
    pub fn title(&self) -> &str {
        let line = self
            .body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        line.trim_start_matches('#').trim_start()
    }

    /// 머리말 값
    pub fn meta(&self, key: &str) -> Option<&str> {
        meta_get(self.front_matter.as_deref()?, key)
    }

    pub fn is_todo(&self) -> bool {
        self.meta("type") == Some("todo")
    }

    pub fn is_done(&self) -> bool {
        self.meta("done") == Some("true")
    }

    pub fn due(&self) -> Option<NaiveDateTime> {
        when::parse_stored(self.meta("due")?)
    }

    pub fn pomodoros(&self) -> u32 {
        self.meta("pomodoros")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn extra_lines(&self) -> usize {
        self.body.trim().lines().count().saturating_sub(1)
    }

    fn matches(&self, tokens: &[String]) -> bool {
        let hay = crate::textenc::nfc(&self.body).to_lowercase();
        let last = tokens.len().saturating_sub(1);
        tokens
            .iter()
            .enumerate()
            .all(|(i, t)| token_matches(&hay, t, i == last))
    }
}

/// 파일 검색과 같은 규칙 — 초성, 한영 오타, 치는 중인 마지막 음절
fn token_matches(hay: &str, token: &str, is_last: bool) -> bool {
    use crate::hangul;
    if hay.contains(token) {
        return true;
    }
    if hangul::is_choseong_query(token) {
        return hangul::choseong_string(hay).contains(token);
    }
    if hangul::qwerty_to_hangul(token).is_some_and(|h| hay.contains(&h)) {
        return true;
    }
    is_last
        && hangul::split_partial(token)
            .is_some_and(|p| hangul::partial_matches(hay, &p).next().is_some())
}

/// 질의에 맞는 메모. 비었으면 전부
pub fn search<'a>(notes: &'a [Note], query: &str) -> Vec<&'a Note> {
    let normalized = crate::textenc::nfc(query).to_lowercase();
    let tokens: Vec<String> = normalized.split_whitespace().map(str::to_string).collect();
    notes.iter().filter(|n| n.matches(&tokens)).collect()
}

/// 파일 내용 → (머리말, 본문). 닫는 `---` 가 없으면 전부 본문이다
fn parse(raw: &str) -> (Option<String>, String) {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    let mut rest = raw;
    let mut len = 0;
    let mut first = true;
    while let Some(end) = rest.find('\n') {
        let line = rest[..end].trim_end_matches('\r');
        len += end + 1;
        rest = &rest[end + 1..];
        if first {
            if line != "---" {
                break;
            }
            first = false;
        } else if line == "---" {
            return (
                Some(raw[..len].to_string()),
                raw[len..].trim_matches(['\r', '\n']).to_string(),
            );
        }
    }
    (None, raw.trim_matches(['\r', '\n']).to_string())
}

/// 머리말 안의 `key: value`
fn meta_get<'a>(front_matter: &'a str, key: &str) -> Option<&'a str> {
    front_matter.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// 머리말 값을 바꾼다. 모르는 줄은 그대로 둔다. `None` 은 지우기
fn meta_set(front_matter: Option<&str>, changes: &[(&str, Option<String>)]) -> String {
    let raw = front_matter.unwrap_or("---\n---\n");
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = raw.lines().collect();
    let inner = &lines[1..lines.len().saturating_sub(1).max(1)];
    let mut out: Vec<String> = inner
        .iter()
        .filter(|l| {
            let key = l.split_once(':').map(|(k, _)| k.trim());
            !changes.iter().any(|(k, _)| key == Some(*k))
        })
        .map(|l| l.to_string())
        .collect();
    for (k, v) in changes {
        if let Some(v) = v {
            out.push(format!("{k}: {v}"));
        }
    }
    let mut text = format!("---{eol}");
    for line in out {
        text.push_str(&line);
        text.push_str(eol);
    }
    text.push_str("---");
    text.push_str(eol);
    text
}

fn render(front_matter: Option<&str>, body: &str) -> String {
    format!("{}{}\n", front_matter.unwrap_or(""), body.trim())
}

fn created_from_id(id: &str) -> Option<i64> {
    let stamp = id.get(..15)?;
    let naive = NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()?;
    Some(
        Local
            .from_local_datetime(&naive)
            .earliest()?
            .timestamp_millis(),
    )
}

fn format_time(ms: i64, now: &DateTime<Local>) -> String {
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    if t.year() == now.year() {
        t.format("%m/%d %H:%M").to_string()
    } else {
        t.format("%Y/%m/%d %H:%M").to_string()
    }
}

fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains(['/', '\\']) || id.contains("..") {
        return Err(format!("잘못된 메모 id: {id}"));
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    dir: PathBuf,
    entries: HashMap<String, CacheEntry>,
}

#[derive(Serialize, Deserialize, PartialEq)]
struct CacheEntry {
    mtime_ms: i64,
    size: u64,
    note: Note,
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 메모 폴더
#[derive(Debug, Clone)]
pub struct NoteStore {
    dir: PathBuf,
    /// 사용자가 정한 폴더인가 — 이런 폴더가 없으면 드라이브가 빠졌거나 경로가 틀린 것이라
    /// 만들지 않고 알린다. 만들면 메모가 기기마다 갈라진다.
    configured: bool,
    cache: Option<PathBuf>,
    focus_secs: i64,
    break_secs: i64,
}

impl NoteStore {
    pub fn new(dir: impl Into<PathBuf>, configured: bool) -> Self {
        Self {
            dir: dir.into(),
            configured,
            cache: None,
            focus_secs: 25 * 60,
            break_secs: 5 * 60,
        }
    }

    pub fn from_config(cfg: &NotesConfig) -> Self {
        let data = crate::Config::default_data_dir();
        let configured = cfg
            .dir
            .as_ref()
            .and_then(|p| crate::keystats::expand_sync_path(&p.to_string_lossy()).ok());
        Self {
            configured: configured.is_some(),
            dir: configured.unwrap_or_else(|| data.join("notes")),
            cache: Some(data.join(CACHE_FILE)),
            focus_secs: i64::from(cfg.focus_minutes.max(1)) * 60,
            break_secs: i64::from(cfg.break_minutes.max(1)) * 60,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 타이머 상태 파일 — 기기별이라 메모 폴더(동기화)가 아니라 데이터 폴더에 둔다
    pub fn timers(&self) -> TimerStore {
        TimerStore::new(match &self.cache {
            Some(cache) => cache.with_file_name("timers.json"),
            None => self.dir.join(".timers.json"),
        })
    }

    pub fn break_secs(&self) -> i64 {
        self.break_secs
    }

    pub fn focus_secs(&self) -> i64 {
        self.focus_secs
    }

    /// 공유 폴더를 따로 정했는가
    pub fn is_configured(&self) -> bool {
        self.configured
    }

    pub fn file_path(&self, id: &str) -> Result<PathBuf, String> {
        check_id(id)?;
        Ok(self.dir.join(format!("{id}.md")))
    }

    /// 전체 메모, 최근 것부터. 바뀐 파일만 읽는다
    pub fn load(&self) -> Result<Vec<Note>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !self.configured => {
                return Ok(Vec::new())
            }
            Err(e) => return Err(format!("{} 을 읽지 못했습니다: {e}", self.dir.display())),
        };
        let mut old = self.read_cache();
        let mut fresh: HashMap<String, CacheEntry> = HashMap::new();
        let mut changed = false;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name
                .strip_suffix(".md")
                .or_else(|| name.strip_suffix(".MD"))
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() || crate::fsutil::is_hidden_entry(&entry) {
                continue;
            }
            let (mtime, size) = (mtime_ms(&meta), meta.len());
            if let Some(hit) = old
                .remove(&id)
                .filter(|c| c.mtime_ms == mtime && c.size == size)
            {
                fresh.insert(id, hit);
                continue;
            }
            // 동기화 중인 파일은 잠시 못 읽을 수 있다 — 다음에 다시 읽는다
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let raw = crate::textenc::decode_utf8_or_cp949(bytes).text;
            let (front_matter, body) = parse(&raw);
            let created_ms = created_from_id(&id).unwrap_or(mtime);
            changed = true;
            let note = Note {
                id: id.clone(),
                body,
                front_matter,
                created_ms,
            };
            fresh.insert(
                id,
                CacheEntry {
                    mtime_ms: mtime,
                    size,
                    note,
                },
            );
        }
        if changed || !old.is_empty() {
            self.write_cache(&fresh);
        }
        let mut notes: Vec<Note> = fresh.into_values().map(|c| c.note).collect();
        notes.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then(b.id.cmp(&a.id)));
        Ok(notes)
    }

    fn read_cache(&self) -> HashMap<String, CacheEntry> {
        let Some(path) = &self.cache else {
            return HashMap::new();
        };
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Cache>(&b).ok())
            .filter(|c| c.dir == self.dir)
            .map(|c| c.entries)
            .unwrap_or_default()
    }

    /// 캐시는 없어도 그만이라 실패해도 알리지 않는다
    fn write_cache(&self, entries: &HashMap<String, CacheEntry>) {
        let Some(path) = &self.cache else {
            return;
        };
        #[derive(Serialize)]
        struct Out<'a> {
            dir: &'a Path,
            entries: &'a HashMap<String, CacheEntry>,
        }
        let out = Out {
            dir: &self.dir,
            entries,
        };
        if let Ok(bytes) = serde_json::to_vec(&out) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = crate::fsutil::write_atomic(path, &bytes, true);
        }
    }

    fn ensure_dir(&self) -> Result<(), String> {
        if self.configured && !self.dir.is_dir() {
            return Err(format!(
                "메모 폴더가 없습니다: {} — 드라이브가 연결됐는지 확인하세요",
                self.dir.display()
            ));
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("{} 을 만들지 못했습니다: {e}", self.dir.display()))
    }

    /// 새 메모. 돌려주는 값은 id
    pub fn add(&self, body: &str, now: DateTime<Local>) -> Result<String, String> {
        self.add_with_front_matter(body, None, now)
    }

    /// 새 할 일
    pub fn add_todo(
        &self,
        body: &str,
        due: Option<NaiveDateTime>,
        now: DateTime<Local>,
    ) -> Result<String, String> {
        let front_matter = meta_set(
            None,
            &[
                ("type", Some("todo".to_string())),
                ("done", Some("false".to_string())),
                ("due", due.map(when::format_stored)),
            ],
        );
        self.add_with_front_matter(body, Some(&front_matter), now)
    }

    fn add_with_front_matter(
        &self,
        body: &str,
        front_matter: Option<&str>,
        now: DateTime<Local>,
    ) -> Result<String, String> {
        if body.trim().is_empty() {
            return Err("빈 메모는 저장하지 않습니다".to_string());
        }
        self.ensure_dir()?;
        let stamp = now.format("%Y%m%d-%H%M%S");
        for _ in 0..8 {
            let mut suffix = [0u8; 2];
            getrandom::fill(&mut suffix).map_err(|e| format!("id 생성 실패: {e}"))?;
            let id = format!("{stamp}-{:02x}{:02x}", suffix[0], suffix[1]);
            let path = self.file_path(&id)?;
            if path.exists() {
                continue;
            }
            crate::fsutil::write_atomic(&path, render(front_matter, body).as_bytes(), true)
                .map_err(|e| format!("메모 저장 실패: {e}"))?;
            return Ok(id);
        }
        Err("메모 이름을 정하지 못했습니다".to_string())
    }

    pub fn add_now(&self, body: &str) -> Result<String, String> {
        self.add(body, Local::now())
    }

    /// 본문만 바꾼다 (머리말은 지킨다). 그새 다른 기기에서 지워졌으면 같은 이름으로 되살린다
    pub fn update(&self, id: &str, body: &str) -> Result<(), String> {
        if body.trim().is_empty() {
            return Err("빈 메모는 저장하지 않습니다".to_string());
        }
        self.ensure_dir()?;
        let path = self.file_path(id)?;
        let front_matter = std::fs::read(&path)
            .ok()
            .map(|b| crate::textenc::decode_utf8_or_cp949(b).text)
            .and_then(|raw| parse(&raw).0);
        crate::fsutil::write_atomic(
            &path,
            render(front_matter.as_deref(), body).as_bytes(),
            true,
        )
        .map_err(|e| format!("메모 저장 실패: {e}"))
    }

    /// 머리말 값만 바꾼다
    pub fn update_meta(&self, id: &str, changes: &[(&str, Option<String>)]) -> Result<(), String> {
        let path = self.file_path(id)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("메모를 읽지 못했습니다: {e}"))?;
        let raw = crate::textenc::decode_utf8_or_cp949(bytes).text;
        let (front_matter, body) = parse(&raw);
        let front_matter = meta_set(front_matter.as_deref(), changes);
        crate::fsutil::write_atomic(&path, render(Some(&front_matter), &body).as_bytes(), true)
            .map_err(|e| format!("메모 저장 실패: {e}"))
    }

    /// 뽀모도로 한 번을 그 할 일에 더한다
    pub fn add_pomodoro(&self, id: &str) -> Result<(), String> {
        let count = self
            .load()?
            .iter()
            .find(|n| n.id == id)
            .map_or(0, Note::pomodoros);
        self.update_meta(id, &[("pomodoros", Some((count + 1).to_string()))])
    }

    /// 휴지통(`.trash/`)으로 옮긴다 — 지운 것이 곧장 모든 기기에서 사라지므로 되살릴 길을 남긴다
    pub fn trash(&self, id: &str) -> Result<(), String> {
        let path = self.file_path(id)?;
        let trash = self.dir.join(TRASH_DIR);
        std::fs::create_dir_all(&trash).map_err(|e| format!("휴지통을 만들지 못했습니다: {e}"))?;
        std::fs::rename(&path, trash.join(format!("{id}.md")))
            .map_err(|e| format!("메모 삭제 실패: {e}"))
    }

    pub fn restore(&self, id: &str) -> Result<(), String> {
        let path = self.file_path(id)?;
        std::fs::rename(self.dir.join(TRASH_DIR).join(format!("{id}.md")), path)
            .map_err(|e| format!("메모 복원 실패: {e}"))
    }

    /// 메모를 다른 폴더로 옮긴다 (폴더를 바꿀 때). 같은 이름이 이미 있으면 그대로 둔다.
    /// 돌려주는 값은 (옮긴 수, 건너뛴 수)
    pub fn move_all_to(&self, target: &Path) -> Result<(usize, usize), String> {
        std::fs::create_dir_all(target)
            .map_err(|e| format!("{} 을 만들지 못했습니다: {e}", target.display()))?;
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
            Err(e) => return Err(format!("{} 을 읽지 못했습니다: {e}", self.dir.display())),
        };
        let (mut moved, mut skipped) = (0, 0);
        for entry in entries.flatten() {
            let from = entry.path();
            let is_note = from
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if !is_note || !from.is_file() {
                continue;
            }
            let to = target.join(entry.file_name());
            if to.exists() {
                skipped += 1;
                continue;
            }
            // 드라이브가 다르면 rename 이 안 된다
            let done = std::fs::rename(&from, &to).is_ok()
                || (std::fs::copy(&from, &to).is_ok() && std::fs::remove_file(&from).is_ok());
            if !done {
                return Err(format!("{} 을 옮기지 못했습니다", from.display()));
            }
            moved += 1;
        }
        Ok((moved, skipped))
    }
}

/// `:m` `:td` `:timer` `:pomo` 뒤의 글
pub fn strip_command(query: &str) -> &str {
    for aliases in [NOTE_ALIASES, TODO_ALIASES, TIMER_ALIASES, POMO_ALIASES] {
        for alias in aliases {
            if let Some(rest) = query.strip_prefix(alias) {
                if rest.is_empty() || rest.starts_with(' ') {
                    return rest.trim();
                }
            }
        }
    }
    query.trim()
}

/// 선택한 행의 메모 id
pub fn selected_id(item: &IndexItem) -> Option<&str> {
    item.keywords.strip_prefix(NOTE_ITEM_MARKER)
}

/// 이 글들이 다루는 명령인가
pub fn is_notes_query(query: &str) -> bool {
    matches!(
        prefix_of(query),
        QueryPrefix::Notes | QueryPrefix::Todo | QueryPrefix::Timer | QueryPrefix::Pomodoro
    )
}

/// 울릴 때가 된 리마인더 (`id|시각`, 제목). 하루 넘게 지난 것은 울리지 않는다 —
/// 새 기기나 오래 꺼 둔 기기에서 지난 일이 한꺼번에 뜨면 안 된다. 그런 일은 목록에서 "기한 지남"으로 보인다.
pub fn due_reminders(notes: &[Note], now: NaiveDateTime) -> Vec<(String, String)> {
    notes
        .iter()
        .filter(|n| n.is_todo() && !n.is_done())
        .filter_map(|n| {
            let due = n.due()?;
            (due <= now && now - due < Duration::hours(24)).then(|| {
                (
                    format!("{}|{}", n.id, when::format_stored(due)),
                    n.title().to_string(),
                )
            })
        })
        .collect()
}

/// Enter 를 눌러 일어난 일 — UI 가 적용한다
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub message: String,
    /// 클립보드로 복사할 글
    pub copy: Option<String>,
    pub next_query: Option<String>,
    /// 런처 창이면 닫는다. 생각을 적자마자 하던 일로 돌아가는 것이 이 기능의 핵심이다
    pub close: bool,
}

/// 수정 시작 결과
#[derive(Debug, PartialEq)]
pub enum EditStart {
    /// 입력창이 이 글로 바뀐다
    Inline(String),
    /// 여러 줄이라 입력창(한 줄)에 못 담는다 — 이 파일을 편집기로 연다
    External(PathBuf),
}

/// Ctrl+Z 로 되돌릴 일
#[derive(Debug)]
enum Undo {
    Deleted(String),
    Toggled(String),
    Cancelled(Timer),
}

/// 런처 한 창의 메모 상태
#[derive(Debug)]
pub struct NoteSession {
    store: NoteStore,
    timers: TimerStore,
    loaded: Option<Result<Vec<Note>, String>>,
    editing: Option<String>,
    undo: Option<Undo>,
}

impl NoteSession {
    pub fn new(store: NoteStore) -> Self {
        Self {
            timers: store.timers(),
            store,
            loaded: None,
            editing: None,
            undo: None,
        }
    }

    /// 이 명령들을 벗어났다 — 수정을 취소하고, 다시 들어오면 파일을 다시 본다
    pub fn leave(&mut self) {
        self.editing = None;
        self.loaded = None;
    }

    fn notes(&mut self) -> &Result<Vec<Note>, String> {
        let store = &self.store;
        self.loaded.get_or_insert_with(|| store.load())
    }

    fn note(&mut self, id: &str) -> Result<Note, String> {
        self.notes()
            .as_ref()
            .ok()
            .and_then(|notes| notes.iter().find(|n| n.id == id))
            .cloned()
            .ok_or_else(|| "메모를 찾지 못했습니다".to_string())
    }

    /// 결과 행. 맨 위는 언제나 저장 행이라, 비슷한 메모가 있어도 Enter 는 새 메모다.
    pub fn results(&mut self, query: &str, use_emoji: bool) -> Vec<IndexItem> {
        let text = strip_command(query).to_string();
        let now = Local::now();
        match prefix_of(query) {
            QueryPrefix::Todo => self.todo_rows(&text, &now, use_emoji),
            QueryPrefix::Timer => self.timer_rows(&text, &now, use_emoji),
            QueryPrefix::Pomodoro => self.pomo_rows(&text, &now, use_emoji),
            _ => self.note_rows(&text, &now, use_emoji),
        }
    }

    fn edit_row(&self, text: &str, sub: &str, use_emoji: bool) -> IndexItem {
        if text.is_empty() {
            row(
                "수정할 내용을 입력하세요",
                "빈 메모는 저장하지 않습니다",
                NOTE_NOOP_MARKER,
                icon_info(use_emoji),
            )
        } else {
            row(
                &format!("수정 저장: {text}"),
                sub,
                NOTE_COMMIT_MARKER,
                if use_emoji {
                    "\u{270F}\u{FE0F}"
                } else {
                    "[EDIT]"
                },
            )
        }
    }

    fn note_rows(&mut self, text: &str, now: &DateTime<Local>, use_emoji: bool) -> Vec<IndexItem> {
        if self.editing.is_some() {
            return vec![self.edit_row(text, "Enter 저장 · 입력을 모두 지우면 취소", use_emoji)];
        }
        let mut items = Vec::new();
        if !text.is_empty() {
            items.push(row(
                &format!("새 메모: {text}"),
                "Enter 저장 · ↓ 기존 메모 고르기",
                NOTE_COMMIT_MARKER,
                if use_emoji { "\u{2795}" } else { "[+]" },
            ));
        }
        let configured = self.store.is_configured();
        match self.notes() {
            Ok(notes) => {
                items.extend(search(notes, text).into_iter().take(RESULT_LIMIT).map(|n| {
                    if n.is_todo() {
                        todo_row(n, now, use_emoji)
                    } else {
                        note_row(n, now, use_emoji)
                    }
                }));
                if text.is_empty() {
                    items.extend(hint_rows(notes.is_empty(), configured, use_emoji));
                }
            }
            Err(e) => items.push(read_error_row(e, use_emoji)),
        }
        items
    }

    fn todo_rows(&mut self, text: &str, now: &DateTime<Local>, use_emoji: bool) -> Vec<IndexItem> {
        let parsed = when::split_due(text, now.naive_local());
        let due_note = match (&parsed.due, parsed.unreadable) {
            (Some(d), _) => format!(" · {}", format_due(*d, now)),
            (None, true) => " · 시각을 읽지 못해 글 그대로 저장합니다".to_string(),
            _ => String::new(),
        };
        if self.editing.is_some() {
            let sub = format!("Enter 저장{due_note} · 입력을 모두 지우면 취소");
            return vec![self.edit_row(&parsed.title, &sub, use_emoji)];
        }
        let mut items = Vec::new();
        if !parsed.title.is_empty() {
            items.push(row(
                &format!("할 일 추가: {}", parsed.title),
                &format!("Enter 저장{due_note} · ↓ 기존 일 고르기"),
                NOTE_COMMIT_MARKER,
                if use_emoji { "\u{2795}" } else { "[+]" },
            ));
        }
        let show_done = matches!(text, "done" | "완료");
        match self.notes() {
            Ok(notes) => {
                let list = if show_done {
                    let mut done: Vec<&Note> = notes
                        .iter()
                        .filter(|n| n.is_todo() && n.is_done())
                        .collect();
                    done.truncate(RESULT_LIMIT);
                    done
                } else {
                    sorted_todos(search(notes, text), now.naive_local())
                };
                let empty = list.is_empty();
                items.extend(
                    list.into_iter()
                        .take(RESULT_LIMIT)
                        .map(|n| todo_row(n, now, use_emoji)),
                );
                if text.is_empty() {
                    items.push(if empty {
                        row(
                            "할 일이 없습니다",
                            "`:td 보고서 쓰기 @내일 9시` 를 치고 Enter",
                            NOTE_NOOP_MARKER,
                            icon_info(use_emoji),
                        )
                    } else {
                        row(
                            TODO_KEY_HINT,
                            "완료한 일 보기: :td done",
                            NOTE_NOOP_MARKER,
                            icon_info(use_emoji),
                        )
                    });
                }
            }
            Err(e) => items.push(read_error_row(e, use_emoji)),
        }
        items
    }

    fn timer_list_rows(&self, now: &DateTime<Local>, use_emoji: bool) -> Vec<IndexItem> {
        let now_ms = now.timestamp_millis();
        self.timers
            .list()
            .iter()
            .map(|t| {
                let name = match (t.kind, t.label.is_empty()) {
                    (Kind::Focus, true) => "집중".to_string(),
                    (Kind::Focus, false) => format!("집중: {}", t.label),
                    (Kind::Break, _) => "휴식".to_string(),
                    (Kind::Timer, true) => "타이머".to_string(),
                    (Kind::Timer, false) => t.label.clone(),
                };
                let ends = Local
                    .timestamp_millis_opt(t.ends_ms)
                    .single()
                    .map(|e| e.format("%H:%M").to_string())
                    .unwrap_or_default();
                row(
                    &name,
                    &format!(
                        "{} 남음 · {ends} 끝 · {CANCEL_KEY} 끄기",
                        format_span(t.ends_ms - now_ms)
                    ),
                    &format!("{NOTE_TIMER_MARKER}{}", t.id),
                    if use_emoji { "\u{23F3}" } else { "[RUN]" },
                )
            })
            .collect()
    }

    fn timer_rows(&mut self, text: &str, now: &DateTime<Local>, use_emoji: bool) -> Vec<IndexItem> {
        let mut items = Vec::new();
        if !text.is_empty() {
            items.push(match when::parse_timer(text) {
                Some((secs, label)) => {
                    let ends = *now + Duration::seconds(secs);
                    let name = if label.is_empty() {
                        "타이머"
                    } else {
                        &label
                    };
                    row(
                        &format!("{} — {}", name, format_span(secs * 1000)),
                        &format!("Enter 시작 · {} 에 끝", ends.format("%H:%M")),
                        NOTE_COMMIT_MARKER,
                        if use_emoji {
                            "\u{23F1}\u{FE0F}"
                        } else {
                            "[TIME]"
                        },
                    )
                }
                None => row(
                    "시간을 읽지 못했습니다",
                    "예: :timer 10분 라면, :timer 1시간 30분",
                    NOTE_NOOP_MARKER,
                    icon_info(use_emoji),
                ),
            });
        }
        items.extend(self.timer_list_rows(now, use_emoji));
        if text.is_empty() && items.is_empty() {
            items.push(row(
                "돌아가는 타이머가 없습니다",
                "`:timer 10분 라면` 을 치고 Enter · 데몬이 실행 중일 때 울립니다",
                NOTE_NOOP_MARKER,
                icon_info(use_emoji),
            ));
        }
        items
    }

    fn pomo_rows(&mut self, text: &str, now: &DateTime<Local>, use_emoji: bool) -> Vec<IndexItem> {
        let focus_mins = self.store.focus_secs() / 60;
        let break_mins = self.store.break_secs() / 60;
        let count = self.timers.focus_today(&day_key(now));
        let mut items = vec![row(
            &if text.is_empty() {
                format!("집중 {focus_mins}분 시작")
            } else {
                format!("집중 {focus_mins}분 시작: {text}")
            },
            &format!("Enter 시작 · 끝나면 {break_mins}분 휴식 · 오늘 {count}번 집중"),
            NOTE_COMMIT_MARKER,
            if use_emoji { "\u{1F345}" } else { "[POMO]" },
        )];
        items.extend(self.timer_list_rows(now, use_emoji));
        items
    }

    /// Enter. 메모 행은 복사, 할 일 행은 완료 표시. 저장 행이면 입력을 저장·시작한다
    pub fn commit(&mut self, query: &str, target: Option<&str>) -> Result<Applied, String> {
        let prefix = prefix_of(query);
        if let Some(id) = target {
            if prefix == QueryPrefix::Todo {
                return self.toggle_done(id);
            }
            let body = self.note(id)?.body;
            return Ok(Applied {
                message: "복사했습니다".to_string(),
                copy: Some(body),
                close: true,
                ..Applied::default()
            });
        }
        let text = strip_command(query);
        match prefix {
            QueryPrefix::Todo => self.commit_todo(text),
            QueryPrefix::Timer => self.commit_timer(text),
            QueryPrefix::Pomodoro => self.start_focus(text, None),
            _ => self.commit_note(text),
        }
    }

    fn commit_note(&mut self, text: &str) -> Result<Applied, String> {
        self.loaded = None;
        if let Some(id) = self.editing.clone() {
            self.store.update(&id, text)?;
            self.editing = None;
            return Ok(Applied {
                message: "메모를 고쳤습니다".to_string(),
                next_query: Some(":m".to_string()),
                ..Applied::default()
            });
        }
        self.store.add(text, Local::now())?;
        Ok(Applied {
            message: "메모를 저장했습니다".to_string(),
            next_query: Some(":m ".to_string()),
            close: true,
            ..Applied::default()
        })
    }

    fn commit_todo(&mut self, text: &str) -> Result<Applied, String> {
        self.loaded = None;
        let now = Local::now();
        let parsed = when::split_due(text, now.naive_local());
        if let Some(id) = self.editing.clone() {
            self.store.update(&id, &parsed.title)?;
            self.store
                .update_meta(&id, &[("due", parsed.due.map(when::format_stored))])?;
            self.editing = None;
            return Ok(Applied {
                message: "할 일을 고쳤습니다".to_string(),
                next_query: Some(":td".to_string()),
                ..Applied::default()
            });
        }
        self.store.add_todo(&parsed.title, parsed.due, now)?;
        Ok(Applied {
            message: match parsed.due {
                Some(d) => format!("할 일을 저장했습니다 — {}", format_due(d, &now)),
                None => "할 일을 저장했습니다".to_string(),
            },
            next_query: Some(":td ".to_string()),
            close: true,
            ..Applied::default()
        })
    }

    fn toggle_done(&mut self, id: &str) -> Result<Applied, String> {
        let note = self.note(id)?;
        if !note.is_todo() {
            return Err("할 일이 아닙니다".to_string());
        }
        self.set_done(id, !note.is_done())?;
        self.undo = Some(Undo::Toggled(id.to_string()));
        Ok(Applied {
            message: if note.is_done() {
                "다시 할 일로 돌렸습니다".to_string()
            } else {
                format!("완료했습니다 — {UNDO_KEY} 되돌리기")
            },
            ..Applied::default()
        })
    }

    fn set_done(&mut self, id: &str, done: bool) -> Result<(), String> {
        self.loaded = None;
        self.store
            .update_meta(id, &[("done", Some(done.to_string()))])
    }

    fn commit_timer(&mut self, text: &str) -> Result<Applied, String> {
        let (secs, label) = when::parse_timer(text)
            .ok_or_else(|| "시간을 읽지 못했습니다 — 예: :timer 10분 라면".to_string())?;
        let now_ms = Local::now().timestamp_millis();
        self.timers.add(Kind::Timer, &label, secs, None, now_ms)?;
        Ok(Applied {
            message: format!("{} 타이머를 걸었습니다", format_span(secs * 1000)),
            next_query: Some(":timer ".to_string()),
            close: true,
            ..Applied::default()
        })
    }

    /// 뽀모도로 시작. 집중은 한 번에 하나
    fn start_focus(&mut self, label: &str, note: Option<&str>) -> Result<Applied, String> {
        if self.timers.list().iter().any(|t| t.kind == Kind::Focus) {
            return Err(format!(
                "이미 집중 중입니다 — {CANCEL_KEY} 로 끈 뒤 다시 시작하세요"
            ));
        }
        let secs = self.store.focus_secs();
        let now_ms = Local::now().timestamp_millis();
        self.timers.add(Kind::Focus, label, secs, note, now_ms)?;
        Ok(Applied {
            message: format!("{} 집중을 시작했습니다", format_span(secs * 1000)),
            next_query: Some(":pomo".to_string()),
            close: true,
            ..Applied::default()
        })
    }

    /// 고른 할 일로 뽀모도로를 시작한다. 할 일 행이 아니면 None
    pub fn start_focus_item(&mut self, item: &IndexItem) -> Option<Result<Applied, String>> {
        let id = selected_id(item)?.to_string();
        let note = self.note(&id).ok()?;
        note.is_todo()
            .then(|| self.start_focus(note.title(), Some(&id)))
    }

    pub fn begin_edit(&mut self, id: &str) -> Result<EditStart, String> {
        let note = self.note(id)?;
        if note.body.trim().contains('\n') {
            return Ok(EditStart::External(self.store.file_path(id)?));
        }
        let seed = if note.is_todo() {
            match note.due() {
                Some(d) => format!(":td {} @{}", note.body.trim(), d.format("%Y-%m-%d %H:%M")),
                None => format!(":td {}", note.body.trim()),
            }
        } else {
            format!(":m {}", note.body.trim())
        };
        self.editing = Some(note.id);
        Ok(EditStart::Inline(seed))
    }

    pub fn delete(&mut self, id: &str) -> Result<String, String> {
        self.store.trash(id)?;
        self.undo = Some(Undo::Deleted(id.to_string()));
        self.loaded = None;
        Ok(format!("메모를 지웠습니다 — {UNDO_KEY} 되돌리기"))
    }

    /// 고른 행을 지운다(메모) 또는 끈다(타이머). 그런 행이 아니면 None
    pub fn delete_item(&mut self, item: &IndexItem) -> Option<Result<String, String>> {
        if let Some(id) = selected_id(item) {
            return Some(self.delete(id));
        }
        let id = item.keywords.strip_prefix(NOTE_TIMER_MARKER)?;
        Some(self.timers.cancel(id).and_then(|timer| match timer {
            Some(t) => {
                self.undo = Some(Undo::Cancelled(t));
                Ok(format!("타이머를 껐습니다 — {UNDO_KEY} 되돌리기"))
            }
            None => Err("이미 끝난 타이머입니다".to_string()),
        }))
    }

    /// 방금 한 일(삭제·완료·타이머 끄기)을 되돌린다. 되돌릴 것이 없으면 None
    pub fn undo_last(&mut self) -> Option<Result<String, String>> {
        let undo = self.undo.take()?;
        self.loaded = None;
        Some(match undo {
            Undo::Deleted(id) => self
                .store
                .restore(&id)
                .map(|()| "메모를 되살렸습니다".to_string()),
            Undo::Toggled(id) => {
                let done = self.note(&id).map(|n| n.is_done());
                done.and_then(|d| self.set_done(&id, !d))
                    .map(|()| "되돌렸습니다".to_string())
            }
            Undo::Cancelled(t) => self
                .timers
                .restore(t)
                .map(|()| "타이머를 되살렸습니다".to_string()),
        })
    }

    /// 다른 곳에서 받은 글을 메모로 저장한다 (클립보드 항목)
    pub fn save_text(&mut self, text: &str) -> Result<String, String> {
        self.store.add_now(text)?;
        self.loaded = None;
        Ok("메모로 저장했습니다".to_string())
    }
}

fn day_key(now: &DateTime<Local>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// 안 끝낸 일을 볼 순서로 — 기한 지난 것·오늘, 그 뒤 기한 순, 기한 없는 일은 최근 것부터 맨 뒤
fn sorted_todos(found: Vec<&Note>, now: NaiveDateTime) -> Vec<&Note> {
    let mut list: Vec<&Note> = found
        .into_iter()
        .filter(|n| n.is_todo() && !n.is_done())
        .collect();
    list.sort_by_key(|n| {
        (
            n.due().is_none(),
            n.due().unwrap_or(now),
            std::cmp::Reverse(n.created_ms),
        )
    });
    list
}

fn format_due(due: NaiveDateTime, now: &DateTime<Local>) -> String {
    let today = now.date_naive();
    let date = due.date();
    let time = due.format("%H:%M");
    if date == today {
        format!("오늘 {time}")
    } else if date == today + Duration::days(1) {
        format!("내일 {time}")
    } else if date.year() == today.year() {
        format!("{} {time}", due.format("%m/%d"))
    } else {
        format!("{} {time}", due.format("%Y/%m/%d"))
    }
}

#[cfg(target_os = "macos")]
const UNDO_KEY: &str = "⌘Z";
#[cfg(not(target_os = "macos"))]
const UNDO_KEY: &str = "Ctrl+Z";

#[cfg(target_os = "macos")]
const CANCEL_KEY: &str = "⌘D";
#[cfg(not(target_os = "macos"))]
const CANCEL_KEY: &str = "Ctrl+Del";

#[cfg(target_os = "macos")]
const KEY_HINT: &str = "Enter 복사 · ⌘E 수정 · ⌘D 삭제 · ⌘Z 되돌리기";
#[cfg(not(target_os = "macos"))]
const KEY_HINT: &str = "Enter 복사 · Ctrl+E 수정 · Ctrl+Del 삭제 · Ctrl+Z 되돌리기";

#[cfg(target_os = "macos")]
const TODO_KEY_HINT: &str = "Enter 완료 · ⌘E 수정 · ⌘D 삭제 · ⌘P 뽀모도로 · ⌘Z 되돌리기";
#[cfg(not(target_os = "macos"))]
const TODO_KEY_HINT: &str =
    "Enter 완료 · Ctrl+E 수정 · Ctrl+Del 삭제 · Ctrl+P 뽀모도로 · Ctrl+Z 되돌리기";

fn icon_info(use_emoji: bool) -> &'static str {
    if use_emoji {
        "\u{1F4A1}"
    } else {
        "[i]"
    }
}

fn row(name: &str, path: &str, marker: &str, icon: &str) -> IndexItem {
    IndexItem {
        name: name.to_string(),
        path: path.to_string(),
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon: icon.to_string(),
        keywords: marker.to_string(),
        icon_path: None,
    }
}

fn read_error_row(error: &str, use_emoji: bool) -> IndexItem {
    row(
        "메모를 읽지 못했습니다",
        error,
        NOTE_NOOP_MARKER,
        icon_info(use_emoji),
    )
}

fn note_row(note: &Note, now: &DateTime<Local>, use_emoji: bool) -> IndexItem {
    let mut sub = format_time(note.created_ms, now);
    let extra = note.extra_lines();
    if extra > 0 {
        sub.push_str(&format!(" · {extra}줄 더"));
    }
    row(
        note.title(),
        &sub,
        &format!("{NOTE_ITEM_MARKER}{}", note.id),
        if use_emoji { "\u{1F4DD}" } else { "[MEMO]" },
    )
}

fn todo_row(note: &Note, now: &DateTime<Local>, use_emoji: bool) -> IndexItem {
    let mut parts = Vec::new();
    if let Some(due) = note.due() {
        parts.push(format_due(due, now));
        if !note.is_done() && due < now.naive_local() {
            parts.push("기한 지남".to_string());
        }
    }
    let pomodoros = note.pomodoros();
    if pomodoros > 0 {
        parts.push(format!("🍅{pomodoros}"));
    }
    if parts.is_empty() {
        parts.push(format_time(note.created_ms, now));
    }
    let icon = match (note.is_done(), use_emoji) {
        (true, true) => "\u{2705}",
        (false, true) => "\u{2B1C}",
        (true, false) => "[x]",
        (false, false) => "[ ]",
    };
    row(
        note.title(),
        &parts.join(" · "),
        &format!("{NOTE_ITEM_MARKER}{}", note.id),
        icon,
    )
}

fn hint_rows(empty: bool, configured: bool, use_emoji: bool) -> Vec<IndexItem> {
    let mut rows = Vec::new();
    if empty {
        rows.push(row(
            "아직 메모가 없습니다",
            "`:m 내용` 을 치고 Enter",
            NOTE_NOOP_MARKER,
            icon_info(use_emoji),
        ));
    } else {
        rows.push(row(
            KEY_HINT,
            "#태그도 글자 그대로 찾습니다",
            NOTE_NOOP_MARKER,
            icon_info(use_emoji),
        ));
    }
    if !configured {
        rows.push(row(
            "이 기기에만 저장됩니다",
            "여러 기기에서 보려면: kmd memo dir <공유 폴더>",
            NOTE_NOOP_MARKER,
            icon_info(use_emoji),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, NoteStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = NoteStore::new(dir.path().join("notes"), false);
        (dir, store)
    }

    fn note(body: &str) -> Note {
        Note {
            id: "x".into(),
            body: body.into(),
            front_matter: None,
            created_ms: 0,
        }
    }

    fn found(body: &str, query: &str) -> bool {
        let notes = [note(body)];
        !search(&notes, query).is_empty()
    }

    #[test]
    fn 찾기는_파일_검색과_같은_규칙() {
        assert!(found("회의록 양식 바꾸기", "양식"));
        assert!(found("회의록 양식 바꾸기", "양식 회의"), "단어 순서 무관");
        assert!(
            !found("회의록 양식 바꾸기", "양식 보고"),
            "모든 단어가 있어야"
        );
        assert!(found("회의록 양식", "ㅎㅇㄹ"), "초성");
        assert!(found("회의록 양식", "ghldml"), "한영 오타");
        assert!(found("보고서 쓰기", "보곳"), "치는 중인 음절");
        assert!(found("Meeting Notes", "meeting"), "대소문자");
        assert!(found("할 일 #업무", "#업무"), "태그는 글자 그대로");
    }

    #[test]
    fn 추가하면_파일_하나가_되고_다시_읽힌다() {
        let (_d, store) = store();
        let now = Local.with_ymd_and_hms(2026, 11, 6, 14, 30, 12).unwrap();
        let id = store.add("  떠오른 생각 \n", now).unwrap();
        assert!(id.starts_with("20261106-143012-"));
        let raw = std::fs::read_to_string(store.file_path(&id).unwrap()).unwrap();
        assert_eq!(raw, "떠오른 생각\n");
        let notes = store.load().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "떠오른 생각");
        assert_eq!(notes[0].created_ms, now.timestamp_millis());
        assert!(store.add("   ", now).is_err());
    }

    #[test]
    fn 최근_것부터_다른_앱이_만든_파일도_읽는다() {
        let (_d, store) = store();
        store
            .add("옛것", Local.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap())
            .unwrap();
        store
            .add("새것", Local.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap())
            .unwrap();
        std::fs::write(store.dir().join("Obsidian 노트.md"), "# 제목\n내용\n").unwrap();
        std::fs::write(store.dir().join("그림.png"), "x").unwrap();
        let notes = store.load().unwrap();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].title(), "제목", "수정 시각(지금)이 가장 최근");
        assert_eq!(notes[0].extra_lines(), 1);
        assert_eq!(notes[1].body, "새것");
        assert_eq!(notes[2].body, "옛것");
    }

    #[test]
    fn 수정해도_머리말은_지킨다() {
        let (_d, store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        let path = store.dir().join("a.md");
        std::fs::write(&path, "---\r\ntype: todo\r\n---\r\n옛 내용\r\n").unwrap();
        assert_eq!(store.load().unwrap()[0].body, "옛 내용");
        store.update("a", "새 내용").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "---\r\ntype: todo\r\n---\r\n새 내용\n"
        );
    }

    #[test]
    fn 머리말이_닫히지_않으면_본문이다() {
        assert_eq!(parse("---\n내용\n"), (None, "---\n내용".to_string()));
        assert_eq!(parse("\u{FEFF}내용\n"), (None, "내용".to_string()));
        assert_eq!(
            parse("---\na: 1\n---\n"),
            (Some("---\na: 1\n---\n".to_string()), String::new())
        );
    }

    #[test]
    fn 삭제는_휴지통으로_되돌릴_수_있다() {
        let (_d, store) = store();
        let id = store.add("지울 것", Local::now()).unwrap();
        let mut s = NoteSession::new(store.clone());
        assert!(s.undo_last().is_none());
        s.delete(&id).unwrap();
        assert!(store.load().unwrap().is_empty());
        assert!(store
            .dir()
            .join(".trash")
            .join(format!("{id}.md"))
            .is_file());
        s.undo_last().unwrap().unwrap();
        assert_eq!(store.load().unwrap()[0].body, "지울 것");
        assert!(s.undo_last().is_none());
    }

    #[test]
    fn 캐시가_있어도_바뀐_파일은_다시_읽고_지운_파일은_뺀다() {
        let d = tempfile::tempdir().unwrap();
        let mut store = NoteStore::new(d.path().join("notes"), false);
        store.cache = Some(d.path().join(CACHE_FILE));
        let a = store.add("처음", Local::now()).unwrap();
        let b = store.add("둘째", Local::now()).unwrap();
        assert_eq!(store.load().unwrap().len(), 2);
        assert!(d.path().join(CACHE_FILE).is_file());

        // 캐시를 쓰는지 — 내용이 달라도 시각·크기가 같으면 다시 읽지 않는다
        let path = store.file_path(&a).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        std::fs::write(&path, "다름\n".as_bytes()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), meta.len());
        filetime::set_file_mtime(&path, mtime).unwrap();
        let body =
            |notes: &[Note], id: &str| notes.iter().find(|n| n.id == id).unwrap().body.clone();
        assert_eq!(body(&store.load().unwrap(), &a), "처음");

        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(1000, 0)).unwrap();
        assert_eq!(body(&store.load().unwrap(), &a), "다름");

        std::fs::remove_file(store.file_path(&b).unwrap()).unwrap();
        assert_eq!(store.load().unwrap().len(), 1);
    }

    #[test]
    fn 정해_둔_폴더가_없으면_만들지_않고_알린다() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("빠진 드라이브");
        let store = NoteStore::new(&gone, true);
        assert!(store.load().is_err());
        assert!(store.add("메모", Local::now()).is_err());
        assert!(!gone.exists());
        // 기본 폴더는 처음 저장할 때 만든다
        let (_d, local) = self::store();
        assert!(local.load().unwrap().is_empty());
    }

    #[test]
    fn 이상한_id_는_폴더_밖으로_못_나간다() {
        let (_d, store) = store();
        for id in ["../x", "a/b", r"a\b", ""] {
            assert!(store.file_path(id).is_err(), "{id}");
            assert!(store.trash(id).is_err(), "{id}");
        }
    }

    #[test]
    fn 폴더를_옮기면_메모가_따라간다() {
        let (d, store) = store();
        let id = store.add("옮길 것", Local::now()).unwrap();
        let target = d.path().join("공유");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join(format!("{id}.md")), "이미 있음").unwrap();
        let other = store.add("다른 것", Local::now()).unwrap();
        let (moved, skipped) = store.move_all_to(&target).unwrap();
        assert_eq!((moved, skipped), (1, 1));
        assert!(target.join(format!("{other}.md")).is_file());
        assert!(
            store.file_path(&id).unwrap().is_file(),
            "겹치면 원본을 둔다"
        );
    }

    #[test]
    fn 비슷한_메모가_있어도_맨_위는_새_메모_저장() {
        let (_d, store) = store();
        store.add("회의록 양식", Local::now()).unwrap();
        let mut s = NoteSession::new(store.clone());
        let rows = s.results(":m 회의", false);
        assert_eq!(rows[0].keywords, NOTE_COMMIT_MARKER);
        assert_eq!(rows[0].name, "새 메모: 회의");
        assert!(selected_id(&rows[1]).is_some());
        assert_eq!(rows.len(), 2);

        let a = s.commit(":m 회의", None).unwrap();
        assert!(a.close);
        assert_eq!(store.load().unwrap().len(), 2);
        assert!(s.commit(":m ", None).is_err(), "빈 입력");
    }

    #[test]
    fn 빈_질의는_최근_메모와_안내() {
        let (_d, store) = store();
        let mut s = NoteSession::new(store.clone());
        let rows = s.results(":m", false);
        assert_eq!(rows[0].name, "아직 메모가 없습니다");
        assert!(rows.iter().all(|r| r.keywords == NOTE_NOOP_MARKER));
        assert!(rows.iter().any(|r| r.name == "이 기기에만 저장됩니다"));

        store.add("첫 메모", Local::now()).unwrap();
        s.leave();
        let rows = s.results(":memo", false);
        assert_eq!(rows[0].name, "첫 메모");
        assert_eq!(rows[1].name, KEY_HINT);
    }

    #[test]
    fn 고른_메모는_복사하고_한_줄_메모는_입력창에서_고친다() {
        let (_d, store) = store();
        let id = store.add("# 고칠 메모", Local::now()).unwrap();
        let mut s = NoteSession::new(store.clone());
        let a = s.commit(":m", Some(&id)).unwrap();
        assert_eq!(a.copy.as_deref(), Some("# 고칠 메모"));
        assert!(a.close);
        assert!(s.commit(":m", Some("없는것")).is_err());

        assert_eq!(
            s.begin_edit(&id).unwrap(),
            EditStart::Inline(":m # 고칠 메모".into())
        );
        let rows = s.results(":m 고친 메모", false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].keywords, NOTE_COMMIT_MARKER);
        assert!(s.results(":m", false)[0].keywords == NOTE_NOOP_MARKER);
        assert!(s.commit(":m ", None).is_err(), "빈 수정은 거절");

        let a = s.commit(":m 고친 메모", None).unwrap();
        assert!(!a.close, "고친 뒤에는 목록으로");
        assert_eq!(a.next_query.as_deref(), Some(":m"));
        let notes = store.load().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id);
        assert_eq!(notes[0].body, "고친 메모");
    }

    #[test]
    fn 수정_중_명령을_벗어나면_취소() {
        let (_d, store) = store();
        let id = store.add("그대로", Local::now()).unwrap();
        let mut s = NoteSession::new(store.clone());
        s.begin_edit(&id).unwrap();
        s.leave();
        s.commit(":m 새 메모", None).unwrap();
        let notes = store.load().unwrap();
        assert_eq!(notes.len(), 2, "수정이 아니라 새 메모");
    }

    #[test]
    fn 여러_줄_메모는_편집기로() {
        let (_d, store) = store();
        let id = store.add("첫 줄\n둘째 줄", Local::now()).unwrap();
        let mut s = NoteSession::new(store.clone());
        assert_eq!(
            s.begin_edit(&id).unwrap(),
            EditStart::External(store.file_path(&id).unwrap())
        );
        assert!(s.results(":m", false)[0].path.contains("1줄 더"));
    }

    #[test]
    fn 읽지_못하면_알리고_저장도_실패를_돌려준다() {
        let d = tempfile::tempdir().unwrap();
        let mut s = NoteSession::new(NoteStore::new(d.path().join("없음"), true));
        let rows = s.results(":m 내용", false);
        assert_eq!(rows[0].keywords, NOTE_COMMIT_MARKER);
        assert_eq!(rows[1].name, "메모를 읽지 못했습니다");
        assert!(s.commit(":m 내용", None).is_err());
    }

    #[test]
    fn 머리말_값_읽고_바꾸기() {
        let fm = meta_set(None, &[("type", Some("todo".into())), ("due", None)]);
        assert_eq!(fm, "---\ntype: todo\n---\n");
        assert_eq!(meta_get(&fm, "type"), Some("todo"));
        assert_eq!(meta_get(&fm, "due"), None);
        let keep = "---\r\nauthor: 나\r\ndone: false\r\n---\r\n";
        assert_eq!(
            meta_set(Some(keep), &[("done", Some("true".into()))]),
            "---\r\nauthor: 나\r\ndone: true\r\n---\r\n"
        );
        assert_eq!(
            meta_set(Some("---\na: 1\nb: 2\n---\n"), &[("a", None)]),
            "---\nb: 2\n---\n"
        );
    }

    #[test]
    fn 할_일은_파일에_머리말로_남는다() {
        let (_d, store) = store();
        let mut s = NoteSession::new(store.clone());
        let a = s
            .commit(":td 보고서 쓰기 @2030-01-02 09:00 #업무", None)
            .unwrap();
        assert!(a.close);
        let notes = store.load().unwrap();
        let raw = std::fs::read_to_string(store.file_path(&notes[0].id).unwrap()).unwrap();
        assert_eq!(
            raw,
            "---\ntype: todo\ndone: false\ndue: 2030-01-02T09:00\n---\n보고서 쓰기 #업무\n"
        );
        assert!(notes[0].is_todo() && !notes[0].is_done());
        assert_eq!(notes[0].title(), "보고서 쓰기 #업무");
    }

    #[test]
    fn 할_일_완료와_되돌리기() {
        let (_d, store) = store();
        let mut s = NoteSession::new(store.clone());
        s.commit(":td 끝낼 일", None).unwrap();
        s.leave();
        let rows = s.results(":td", false);
        assert_eq!(rows[0].name, "끝낼 일");
        assert_eq!(rows[0].icon, "[ ]");
        let id = selected_id(&rows[0]).unwrap().to_string();

        let a = s.commit(":td", Some(&id)).unwrap();
        assert!(!a.close, "완료해도 목록에 머문다");
        assert!(store.load().unwrap()[0].is_done());
        assert!(
            s.results(":td", false).iter().all(|r| r.name != "끝낼 일"),
            "완료한 일은 기본 목록에서 빠진다"
        );
        assert!(s
            .results(":td done", false)
            .iter()
            .any(|r| r.name == "끝낼 일"));

        s.undo_last().unwrap().unwrap();
        assert!(!store.load().unwrap()[0].is_done());
    }

    #[test]
    fn 메모_목록에서_할_일을_고르면_복사() {
        let (_d, store) = store();
        let mut s = NoteSession::new(store.clone());
        s.commit(":td 복사될 일", None).unwrap();
        s.leave();
        let rows = s.results(":m", false);
        let id = selected_id(&rows[0]).unwrap().to_string();
        assert_eq!(
            s.commit(":m", Some(&id)).unwrap().copy.as_deref(),
            Some("복사될 일")
        );
        assert!(!store.load().unwrap()[0].is_done());
    }

    #[test]
    fn 할_일_순서는_기한_순_기한_없는_일은_뒤() {
        let (_d, store) = store();
        let now = Local.with_ymd_and_hms(2026, 11, 6, 14, 0, 0).unwrap();
        let at = |s: &str| when::parse_stored(s);
        store.add_todo("기한 없음", None, now).unwrap();
        store.add_todo("나중", at("2026-11-20T09:00"), now).unwrap();
        store
            .add_todo("지난 일", at("2026-11-01T09:00"), now)
            .unwrap();
        store.add_todo("내일", at("2026-11-07T09:00"), now).unwrap();
        store.add("그냥 메모", now).unwrap();
        let notes = store.load().unwrap();
        let sorted = sorted_todos(search(&notes, ""), now.naive_local());
        let titles: Vec<&str> = sorted.iter().map(|n| n.title()).collect();
        assert_eq!(titles, ["지난 일", "내일", "나중", "기한 없음"]);

        let late = todo_row(sorted[0], &now, false);
        assert_eq!(late.path, "11/01 09:00 · 기한 지남");
        assert_eq!(todo_row(sorted[1], &now, false).path, "내일 09:00");
    }

    #[test]
    fn 할_일을_고치면_시각도_바뀌고_뽀모도로는_남는다() {
        let (_d, store) = store();
        let now = Local::now();
        let due = when::parse_stored("2030-01-02T09:00");
        let id = store.add_todo("옛 일", due, now).unwrap();
        store.add_pomodoro(&id).unwrap();
        store.add_pomodoro(&id).unwrap();
        let mut s = NoteSession::new(store.clone());
        assert_eq!(
            s.begin_edit(&id).unwrap(),
            EditStart::Inline(":td 옛 일 @2030-01-02 09:00".into())
        );
        assert_eq!(
            s.results(":td 새 일 @2031-05-06 10:00", false)[0].name,
            "수정 저장: 새 일"
        );
        s.commit(":td 새 일 @2031-05-06 10:00", None).unwrap();
        let n = &store.load().unwrap()[0];
        assert_eq!(n.body, "새 일");
        assert_eq!(n.meta("due"), Some("2031-05-06T10:00"));
        assert_eq!(n.pomodoros(), 2);
        assert!(n.is_todo());
    }

    #[test]
    fn 리마인더는_기한이_지난_하루_안의_안_끝낸_일만() {
        let (_d, store) = store();
        let now = Local.with_ymd_and_hms(2026, 11, 6, 14, 0, 0).unwrap();
        let at = |s: &str| when::parse_stored(s);
        let a = store.add_todo("방금", at("2026-11-06T13:59"), now).unwrap();
        store
            .add_todo("어제 전", at("2026-11-05T13:00"), now)
            .unwrap();
        store.add_todo("나중", at("2026-11-06T15:00"), now).unwrap();
        store.add_todo("기한 없음", None, now).unwrap();
        let done = store.add_todo("끝남", at("2026-11-06T13:00"), now).unwrap();
        store
            .update_meta(&done, &[("done", Some("true".into()))])
            .unwrap();
        let due = due_reminders(&store.load().unwrap(), now.naive_local());
        assert_eq!(
            due,
            vec![(format!("{a}|2026-11-06T13:59"), "방금".to_string())]
        );
    }

    #[test]
    fn 타이머_시작과_끄기와_되돌리기() {
        let (_d, store) = store();
        let timers = store.timers();
        let mut s = NoteSession::new(store);
        assert_eq!(s.results(":timer", false)[0].keywords, NOTE_NOOP_MARKER);
        let rows = s.results(":timer 10분 라면", false);
        assert_eq!(rows[0].name, "라면 — 10분");
        assert_eq!(rows[0].keywords, NOTE_COMMIT_MARKER);
        assert_eq!(
            s.results(":timer 라면", false)[0].keywords,
            NOTE_NOOP_MARKER
        );
        assert!(s.commit(":timer 라면", None).is_err());

        assert!(s.commit(":timer 10분 라면", None).unwrap().close);
        assert_eq!(timers.list()[0].label, "라면");

        let rows = s.results(":timer", false);
        assert!(rows[0].path.contains("10분 남음"), "{}", rows[0].path);
        assert!(rows[0].keywords.starts_with(NOTE_TIMER_MARKER));
        s.delete_item(&rows[0]).unwrap().unwrap();
        assert!(timers.list().is_empty());
        assert!(s.delete_item(&rows[0]).unwrap().is_err(), "이미 끈 타이머");
        s.undo_last().unwrap().unwrap();
        assert_eq!(timers.list().len(), 1);
    }

    #[test]
    fn 뽀모도로는_한_번에_하나_할_일에서도_시작한다() {
        let (_d, store) = store();
        let timers = store.timers();
        let id = store.add_todo("보고서", None, Local::now()).unwrap();
        let memo = store.add("메모", Local::now()).unwrap();
        let mut s = NoteSession::new(store);
        let rows = s.results(":pomo", false);
        assert!(rows[0].name.contains("집중 25분"));
        assert!(rows[0].path.contains("오늘 0번"));

        let todo = s
            .results(":td", false)
            .into_iter()
            .find(|r| selected_id(r) == Some(id.as_str()))
            .unwrap();
        s.start_focus_item(&todo).unwrap().unwrap();
        let running = timers.list();
        assert_eq!(running[0].kind, Kind::Focus);
        assert_eq!(running[0].label, "보고서");
        assert_eq!(running[0].note.as_deref(), Some(id.as_str()));
        assert!(s.commit(":pomo", None).is_err(), "이미 집중 중");

        let memo_row = s
            .results(":m", false)
            .into_iter()
            .find(|r| selected_id(r) == Some(memo.as_str()))
            .unwrap();
        assert!(
            s.start_focus_item(&memo_row).is_none(),
            "메모에서는 시작하지 않는다"
        );
    }

    #[test]
    fn 받은_글을_메모로_저장() {
        let (_d, store) = store();
        let mut s = NoteSession::new(store.clone());
        s.save_text("첫 줄\n둘째 줄").unwrap();
        assert_eq!(store.load().unwrap()[0].extra_lines(), 1);
    }

    #[test]
    fn 명령_글자_떼기() {
        assert_eq!(strip_command(":m 내용 #태그"), "내용 #태그");
        assert_eq!(strip_command(":ㅡ 내용"), "내용");
        assert_eq!(strip_command(":메모"), "");
        assert_eq!(strip_command(":memo  여러  칸"), "여러  칸");
        assert_eq!(strip_command(":td 할 일 @내일"), "할 일 @내일");
        assert_eq!(strip_command(":timer 10분"), "10분");
        assert_eq!(strip_command(":pomo"), "");
    }
}
