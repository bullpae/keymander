//! `:m` / `:memo` — 메모 (docs/23).
//!
//! 메모 하나가 Markdown 파일 하나다. 폴더를 클라우드 드라이브에 두면 기기끼리 맞춰지고,
//! 하루 한 파일과 달리 두 기기가 같은 날 적어도 충돌 사본이 생기지 않는다.
//! 원본은 언제나 파일이고, `notes-index.json` 은 이 기기에서 다시 읽지 않으려는 캐시다 —
//! 지워져도 파일에서 다시 만든다.
//!
//! 판단은 여기, 화면은 UI 몫이다. 데스크톱과 TUI 는 [`NoteSession`] 을 하나씩 들고
//! 결과([`NoteSession::results`])를 그리고 Enter·단축키를 넘긴다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::index::{IndexItem, ItemKind, Source};

pub const NOTE_ALIASES: &[&str] = &[":memo", ":메모", ":m", ":ㅡ"];

/// Enter = 새 메모 저장 또는 수정 저장
pub const NOTE_COMMIT_MARKER: &str = "kmd:note:commit";
/// Enter = 이 메모 복사. 뒤에 메모 id
pub const NOTE_ITEM_MARKER: &str = "kmd:note:item:";
/// 안내 행
pub const NOTE_NOOP_MARKER: &str = "kmd:note:noop";

/// 목록에 보여 줄 메모 수
const RESULT_LIMIT: usize = 50;
const TRASH_DIR: &str = ".trash";
const CACHE_FILE: &str = "notes-index.json";

/// `[notes]` — 메모 설정.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct NotesConfig {
    /// 메모 폴더. 없으면 `<data_dir>/notes` (이 기기에만 저장)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
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
}

impl NoteStore {
    pub fn new(dir: impl Into<PathBuf>, configured: bool) -> Self {
        Self {
            dir: dir.into(),
            configured,
            cache: None,
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
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
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
            crate::fsutil::write_atomic(&path, render(None, body).as_bytes(), true)
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

/// `:m` 뒤의 글
pub fn strip_command(query: &str) -> &str {
    for alias in NOTE_ALIASES {
        if let Some(rest) = query.strip_prefix(alias) {
            if rest.is_empty() || rest.starts_with(' ') {
                return rest.trim();
            }
        }
    }
    query.trim()
}

/// 선택한 행의 메모 id
pub fn selected_id(item: &IndexItem) -> Option<&str> {
    item.keywords.strip_prefix(NOTE_ITEM_MARKER)
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

/// 런처 한 창의 메모 상태
#[derive(Debug)]
pub struct NoteSession {
    store: NoteStore,
    loaded: Option<Result<Vec<Note>, String>>,
    editing: Option<String>,
    deleted: Option<String>,
}

impl NoteSession {
    pub fn new(store: NoteStore) -> Self {
        Self {
            store,
            loaded: None,
            editing: None,
            deleted: None,
        }
    }

    /// `:m` 을 벗어났다 — 수정을 취소하고, 다시 들어오면 파일을 다시 본다
    pub fn leave(&mut self) {
        self.editing = None;
        self.loaded = None;
    }

    fn notes(&mut self) -> &Result<Vec<Note>, String> {
        let store = &self.store;
        self.loaded.get_or_insert_with(|| store.load())
    }

    /// 결과 행. 맨 위는 언제나 저장 행이라, 비슷한 메모가 있어도 Enter 는 새 메모다.
    pub fn results(&mut self, query: &str, use_emoji: bool) -> Vec<IndexItem> {
        let text = strip_command(query).to_string();
        if self.editing.is_some() {
            return vec![if text.is_empty() {
                row(
                    "수정할 내용을 입력하세요",
                    "빈 메모는 저장하지 않습니다",
                    NOTE_NOOP_MARKER,
                    icon_info(use_emoji),
                )
            } else {
                row(
                    &format!("수정 저장: {text}"),
                    "Enter 저장 · 입력을 모두 지우면 취소",
                    NOTE_COMMIT_MARKER,
                    if use_emoji {
                        "\u{270F}\u{FE0F}"
                    } else {
                        "[EDIT]"
                    },
                )
            }];
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
        let now = Local::now();
        let configured = self.store.is_configured();
        match self.notes() {
            Ok(notes) => {
                items.extend(
                    search(notes, &text)
                        .into_iter()
                        .take(RESULT_LIMIT)
                        .map(|n| note_row(n, &now, use_emoji)),
                );
                if text.is_empty() {
                    let empty = notes.is_empty();
                    items.extend(hint_rows(empty, configured, use_emoji));
                }
            }
            Err(e) => items.push(row(
                "메모를 읽지 못했습니다",
                &e.clone(),
                NOTE_NOOP_MARKER,
                icon_info(use_emoji),
            )),
        }
        items
    }

    /// Enter. `target` 이 있으면 그 메모를 복사하고, 없으면 입력을 저장한다
    pub fn commit(&mut self, query: &str, target: Option<&str>) -> Result<Applied, String> {
        if let Some(id) = target {
            let body = self
                .notes()
                .as_ref()
                .ok()
                .and_then(|notes| notes.iter().find(|n| n.id == id))
                .map(|n| n.body.clone())
                .ok_or_else(|| "메모를 찾지 못했습니다".to_string())?;
            return Ok(Applied {
                message: "복사했습니다".to_string(),
                copy: Some(body),
                close: true,
                ..Applied::default()
            });
        }
        let text = strip_command(query);
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

    pub fn begin_edit(&mut self, id: &str) -> Result<EditStart, String> {
        let note = self
            .notes()
            .as_ref()
            .ok()
            .and_then(|notes| notes.iter().find(|n| n.id == id))
            .cloned()
            .ok_or_else(|| "메모를 찾지 못했습니다".to_string())?;
        if note.body.trim().contains('\n') {
            return Ok(EditStart::External(self.store.file_path(id)?));
        }
        self.editing = Some(note.id);
        Ok(EditStart::Inline(format!(":m {}", note.body.trim())))
    }

    pub fn delete(&mut self, id: &str) -> Result<String, String> {
        self.store.trash(id)?;
        self.deleted = Some(id.to_string());
        self.loaded = None;
        Ok(format!("메모를 지웠습니다 — {UNDO_KEY} 되돌리기"))
    }

    /// 방금 지운 메모를 되살린다. 지운 것이 없으면 None
    pub fn undo_delete(&mut self) -> Option<Result<String, String>> {
        let id = self.deleted.take()?;
        self.loaded = None;
        Some(
            self.store
                .restore(&id)
                .map(|()| "메모를 되살렸습니다".to_string()),
        )
    }
}

#[cfg(target_os = "macos")]
const UNDO_KEY: &str = "⌘Z";
#[cfg(not(target_os = "macos"))]
const UNDO_KEY: &str = "Ctrl+Z";

#[cfg(target_os = "macos")]
const KEY_HINT: &str = "Enter 복사 · ⌘E 수정 · ⌘D 삭제 · ⌘Z 되돌리기";
#[cfg(not(target_os = "macos"))]
const KEY_HINT: &str = "Enter 복사 · Ctrl+E 수정 · Ctrl+Del 삭제 · Ctrl+Z 되돌리기";

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
        assert!(s.undo_delete().is_none());
        s.delete(&id).unwrap();
        assert!(store.load().unwrap().is_empty());
        assert!(store
            .dir()
            .join(".trash")
            .join(format!("{id}.md"))
            .is_file());
        s.undo_delete().unwrap().unwrap();
        assert_eq!(store.load().unwrap()[0].body, "지울 것");
        assert!(s.undo_delete().is_none());
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
    fn 명령_글자_떼기() {
        assert_eq!(strip_command(":m 내용 #태그"), "내용 #태그");
        assert_eq!(strip_command(":ㅡ 내용"), "내용");
        assert_eq!(strip_command(":메모"), "");
        assert_eq!(strip_command(":memo  여러  칸"), "여러  칸");
    }
}
