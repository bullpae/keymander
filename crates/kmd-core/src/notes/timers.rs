//! 타이머·뽀모도로·리마인더 상태 파일 (docs/23 N3·N4).
//!
//! 런처·CLI 가 끝나는 시각을 적고, 데몬이 1초마다 읽어 울린다. 시간을 IPC 가 아니라 파일로 주고받아서
//! 데몬이 꺼져 있어도 타이머를 걸 수 있고, 다시 켜지거나 노트북이 깨어나면 늦게라도 울린다.
//! 런처와 데몬이 모두 고치므로 쓰기는 잠금 파일로 차례를 지킨다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

const LOCK_WAIT: Duration = Duration::from_secs(2);
/// 이보다 오래된 잠금은 죽은 프로세스가 남긴 것이다
const LOCK_STALE: Duration = Duration::from_secs(10);
/// 울린 리마인더 기록을 지우는 나이
const FIRED_KEEP_MS: i64 = 30 * 24 * 3600 * 1000;
/// 이만큼 늦게 끝난 것은 "늦었다"고 알린다
const LATE_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Timer,
    Focus,
    Break,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    pub id: String,
    pub kind: Kind,
    pub label: String,
    pub ends_ms: i64,
    /// 뽀모도로를 시작한 할 일
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    timers: Vec<Timer>,
    /// 이 기기에서 울린 리마인더 (`id|시각` → 울린 때). 기기별이라 파일에 적지 않는다 —
    /// 적으면 동기화로 다른 기기의 알림까지 막힌다
    fired: BTreeMap<String, i64>,
    focus_day: String,
    focus_count: u32,
}

/// 끝난 타이머를 어떻게 알릴지
#[derive(Debug, PartialEq)]
pub struct Fired {
    pub title: String,
    pub body: String,
    /// 이어서 걸 휴식 (초)
    pub start_break: Option<i64>,
    /// 집중 한 번을 세어야 한다
    pub count_focus: bool,
}

/// 끝난 타이머 → 알림. 휴식 시간이 통째로 지났으면(잠들어 있었음) 휴식을 걸지 않는다
pub fn plan_fired(timer: &Timer, now_ms: i64, break_secs: i64) -> Fired {
    let late_ms = now_ms - timer.ends_ms;
    let late = if late_ms >= LATE_MS {
        format!("{} 전에 끝났습니다", format_span(late_ms))
    } else {
        String::new()
    };
    match timer.kind {
        Kind::Timer => Fired {
            title: if timer.label.is_empty() {
                "타이머 끝".to_string()
            } else {
                format!("{} 끝", timer.label)
            },
            body: late,
            start_break: None,
            count_focus: false,
        },
        Kind::Focus => {
            let rest = late_ms < break_secs * 1000;
            Fired {
                title: if rest {
                    format!("집중 끝 — {} 쉬세요", format_span(break_secs * 1000))
                } else {
                    "집중 끝".to_string()
                },
                body: if late.is_empty() {
                    timer.label.clone()
                } else {
                    late
                },
                start_break: rest.then_some(break_secs),
                count_focus: true,
            }
        }
        Kind::Break => Fired {
            title: "휴식 끝".to_string(),
            body: if late.is_empty() {
                ":pomo 로 다시 집중".to_string()
            } else {
                late
            },
            start_break: None,
            count_focus: false,
        },
    }
}

/// `12분`, `1시간 3분`, `30초`
pub fn format_span(ms: i64) -> String {
    let secs = (ms.max(0) + 999) / 1000;
    if secs < 60 {
        return format!("{secs}초");
    }
    let mins = (secs + 59) / 60;
    if mins < 60 {
        return format!("{mins}분");
    }
    match mins % 60 {
        0 => format!("{}시간", mins / 60),
        m => format!("{}시간 {m}분", mins / 60),
    }
}

/// 상태 파일
#[derive(Debug, Clone)]
pub struct TimerStore {
    path: PathBuf,
}

impl TimerStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn read(path: &Path) -> State {
        // 타이머는 잃어도 다시 걸면 되는 값이라 깨진 파일 때문에 멈추지 않는다
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn update<T>(&self, f: impl FnOnce(&mut State) -> T) -> Result<T, String> {
        let _lock = FileLock::acquire(&self.path.with_extension("lock"))?;
        let mut state = Self::read(&self.path);
        let out = f(&mut state);
        let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
        crate::fsutil::write_atomic(&self.path, &bytes, true)
            .map_err(|e| format!("타이머 저장 실패: {e}"))?;
        Ok(out)
    }

    /// 지금 걸린 타이머, 먼저 끝나는 것부터
    pub fn list(&self) -> Vec<Timer> {
        let mut timers = Self::read(&self.path).timers;
        timers.sort_by_key(|t| t.ends_ms);
        timers
    }

    pub fn add(
        &self,
        kind: Kind,
        label: &str,
        secs: i64,
        note: Option<&str>,
        now_ms: i64,
    ) -> Result<Timer, String> {
        let mut suffix = [0u8; 4];
        getrandom::fill(&mut suffix).map_err(|e| format!("id 생성 실패: {e}"))?;
        let timer = Timer {
            id: format!("{:08x}", u32::from_le_bytes(suffix)),
            kind,
            label: label.to_string(),
            ends_ms: now_ms + secs * 1000,
            note: note.map(str::to_string),
        };
        let stored = timer.clone();
        self.update(move |s| s.timers.push(stored))?;
        Ok(timer)
    }

    /// 끈 타이머를 돌려준다
    pub fn cancel(&self, id: &str) -> Result<Option<Timer>, String> {
        self.update(|s| {
            let i = s.timers.iter().position(|t| t.id == id)?;
            Some(s.timers.remove(i))
        })
    }

    pub fn restore(&self, timer: Timer) -> Result<(), String> {
        self.update(move |s| s.timers.push(timer))
    }

    /// 끝난 타이머를 꺼낸다. 끝난 것이 없으면 파일을 쓰지 않는다 (1초마다 부른다)
    pub fn take_due(&self, now_ms: i64) -> Result<Vec<Timer>, String> {
        if !Self::read(&self.path)
            .timers
            .iter()
            .any(|t| t.ends_ms <= now_ms)
        {
            return Ok(Vec::new());
        }
        self.update(|s| {
            let (due, keep) = std::mem::take(&mut s.timers)
                .into_iter()
                .partition(|t| t.ends_ms <= now_ms);
            s.timers = keep;
            due
        })
    }

    /// 아직 안 울린 리마인더만 골라 울렸다고 적는다
    pub fn claim_reminders(&self, keys: &[String], now_ms: i64) -> Result<Vec<String>, String> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        self.update(|s| {
            s.fired.retain(|_, at| now_ms - *at < FIRED_KEEP_MS);
            keys.iter()
                .filter(|k| s.fired.insert((*k).clone(), now_ms).is_none())
                .cloned()
                .collect()
        })
    }

    /// 오늘 집중 횟수를 하나 올린다
    pub fn count_focus(&self, day: &str) -> Result<u32, String> {
        self.update(|s| {
            if s.focus_day != day {
                s.focus_day = day.to_string();
                s.focus_count = 0;
            }
            s.focus_count += 1;
            s.focus_count
        })
    }

    pub fn focus_today(&self, day: &str) -> u32 {
        let s = Self::read(&self.path);
        if s.focus_day == day {
            s.focus_count
        } else {
            0
        }
    }
}

/// 만들어 두는 동안만 잡히는 잠금. 프로세스가 죽어 남으면 나이로 걷어 낸다
struct FileLock(PathBuf);

impl FileLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("타이머 폴더 실패: {e}"))?;
        }
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => return Ok(Self(path.to_path_buf())),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|age| age > LOCK_STALE);
                    if stale {
                        let _ = std::fs::remove_file(path);
                    } else if started.elapsed() > LOCK_WAIT {
                        return Err("타이머 파일이 잠겨 있습니다".to_string());
                    } else {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(e) => return Err(format!("타이머 잠금 실패: {e}")),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, TimerStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TimerStore::new(dir.path().join("timers.json"));
        (dir, store)
    }

    #[test]
    fn 걸고_끄고_되살린다() {
        let (_d, s) = store();
        let a = s.add(Kind::Timer, "라면", 600, None, 1000).unwrap();
        let b = s.add(Kind::Timer, "차", 60, None, 1000).unwrap();
        assert_eq!(a.ends_ms, 601_000);
        let list = s.list();
        assert_eq!(list[0].id, b.id, "먼저 끝나는 것부터");
        let off = s.cancel(&a.id).unwrap().unwrap();
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.cancel(&a.id).unwrap(), None);
        s.restore(off).unwrap();
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn 끝난_것만_꺼내고_두_번_울리지_않는다() {
        let (_d, s) = store();
        s.add(Kind::Timer, "짧음", 10, None, 0).unwrap();
        s.add(Kind::Timer, "김", 100, None, 0).unwrap();
        assert!(s.take_due(5_000).unwrap().is_empty());
        let due = s.take_due(10_000).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].label, "짧음");
        assert!(s.take_due(10_000).unwrap().is_empty());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn 아무것도_안_끝났으면_파일을_쓰지_않는다() {
        let (d, s) = store();
        assert!(s.take_due(0).unwrap().is_empty());
        assert!(!d.path().join("timers.json").exists());
    }

    #[test]
    fn 리마인더는_기기에서_한_번만() {
        let (_d, s) = store();
        let keys = vec!["a|1".to_string(), "b|1".to_string()];
        assert_eq!(s.claim_reminders(&keys, 100).unwrap(), keys);
        assert!(s.claim_reminders(&keys, 200).unwrap().is_empty());
        let moved = vec!["a|2".to_string()];
        assert_eq!(
            s.claim_reminders(&moved, 300).unwrap(),
            moved,
            "시각을 바꾸면 다시"
        );
        // 오래된 기록은 지워진다
        let later = FIRED_KEEP_MS + 1000;
        assert_eq!(s.claim_reminders(&keys, later).unwrap(), keys);
    }

    #[test]
    fn 집중_횟수는_날마다_새로() {
        let (_d, s) = store();
        assert_eq!(s.focus_today("2026-11-06"), 0);
        assert_eq!(s.count_focus("2026-11-06").unwrap(), 1);
        assert_eq!(s.count_focus("2026-11-06").unwrap(), 2);
        assert_eq!(s.focus_today("2026-11-06"), 2);
        assert_eq!(s.focus_today("2026-11-07"), 0);
        assert_eq!(s.count_focus("2026-11-07").unwrap(), 1);
    }

    #[test]
    fn 동시에_써도_잃지_않는다() {
        let (_d, s) = store();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                std::thread::spawn(move || s.add(Kind::Timer, &i.to_string(), 60, None, 0).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(s.list().len(), 8);
    }

    #[test]
    fn 남은_잠금은_나이로_걷어_낸다() {
        let (d, s) = store();
        let lock = d.path().join("timers.lock");
        std::fs::write(&lock, "").unwrap();
        filetime::set_file_mtime(&lock, filetime::FileTime::from_unix_time(1000, 0)).unwrap();
        s.add(Kind::Timer, "", 60, None, 0).unwrap();
        assert!(!lock.exists());
    }

    fn timer(kind: Kind, label: &str, ends_ms: i64) -> Timer {
        Timer {
            id: "x".into(),
            kind,
            label: label.into(),
            ends_ms,
            note: None,
        }
    }

    #[test]
    fn 끝났을_때_알림() {
        let f = plan_fired(&timer(Kind::Timer, "라면", 1000), 1000, 300);
        assert_eq!((f.title.as_str(), f.body.as_str()), ("라면 끝", ""));
        let f = plan_fired(&timer(Kind::Timer, "", 0), 12 * 60_000, 300);
        assert_eq!(f.title, "타이머 끝");
        assert_eq!(f.body, "12분 전에 끝났습니다");

        let f = plan_fired(&timer(Kind::Focus, "보고서", 0), 500, 300);
        assert_eq!(f.title, "집중 끝 — 5분 쉬세요");
        assert_eq!(f.body, "보고서");
        assert_eq!(f.start_break, Some(300));
        assert!(f.count_focus);

        let f = plan_fired(&timer(Kind::Focus, "", 0), 3_600_000, 300);
        assert_eq!(f.start_break, None, "잠든 사이 휴식 시간이 지났다");
        assert!(f.count_focus);
        assert_eq!(f.body, "1시간 전에 끝났습니다");

        let f = plan_fired(&timer(Kind::Break, "", 0), 0, 300);
        assert_eq!(f.title, "휴식 끝");
        assert_eq!(f.start_break, None);
    }

    #[test]
    fn 시간_표시() {
        assert_eq!(format_span(30_000), "30초");
        assert_eq!(format_span(60_000), "1분");
        assert_eq!(format_span(61_000), "2분");
        assert_eq!(format_span(3_600_000), "1시간");
        assert_eq!(format_span(3_780_000), "1시간 3분");
    }
}
