//! 타이머·뽀모도로·리마인더를 울린다 (docs/23 N3·N4).
//!
//! 런처는 금방 닫히므로 시간은 항상 떠 있는 데몬이 잰다. 끝나는 시각은 파일(`kmd_core::notes::timers`)에
//! 있으므로 데몬을 다시 시작해도, 노트북이 잠들었다 깨도 늦게라도 울린다.
//! 알림은 `kmd-desktop --alert` 창이다 — OS 알림은 방해 금지 모드에 막힌다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use kmd_core::notes::timers::{plan_fired, Kind, Timer, ALERT_REMINDER};
use kmd_core::notes::{due_reminders, NoteStore};

use crate::server::{load_config, lower_indexer_thread_priority};

const TICK: Duration = Duration::from_secs(1);
const REMINDER_SCAN: Duration = Duration::from_secs(30);
/// 리마인더가 한꺼번에 이보다 많으면 창 하나로 묶는다
const MAX_REMINDER_WINDOWS: usize = 3;

pub(crate) fn spawn(shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        lower_indexer_thread_priority();
        let mut store = store();
        let mut last_scan = Instant::now() - REMINDER_SCAN;
        while !shutdown.load(Ordering::Relaxed) {
            std::thread::sleep(TICK);
            let now = Local::now();
            match store.timers().take_due(now.timestamp_millis()) {
                Ok(due) => {
                    for timer in due {
                        fire(&store, &timer);
                    }
                }
                Err(e) => tracing::warn!("타이머 확인 실패: {e}"),
            }
            if last_scan.elapsed() >= REMINDER_SCAN {
                last_scan = Instant::now();
                store = self::store(); // 폴더·시간 설정 변경 반영
                scan_reminders(&store);
            }
        }
    });
}

fn store() -> NoteStore {
    NoteStore::from_config(&load_config().notes)
}

fn fire(store: &NoteStore, timer: &Timer) {
    let now = Local::now();
    let now_ms = now.timestamp_millis();
    let fired = plan_fired(timer, now_ms, store.break_secs());
    tracing::info!("타이머 끝: {}", fired.title);
    show_alert(&fired.title, &fired.body, timer.kind.alert());
    let timers = store.timers();
    if fired.count_focus {
        if let Err(e) = timers.count_focus(&now.format("%Y-%m-%d").to_string()) {
            tracing::warn!("집중 횟수 저장 실패: {e}");
        }
        if let Some(id) = &timer.note {
            if let Err(e) = store.add_pomodoro(id) {
                tracing::warn!("할 일에 뽀모도로 기록 실패: {e}");
            }
        }
    }
    if let Some(secs) = fired.start_break {
        if let Err(e) = timers.add(Kind::Break, "", secs, None, now_ms) {
            tracing::warn!("휴식 타이머 실패: {e}");
        }
    }
}

fn scan_reminders(store: &NoteStore) {
    // 메모 폴더가 없으면(드라이브 빠짐) 조용히 넘어간다 — 다시 연결되면 울린다
    let Ok(notes) = store.load() else {
        return;
    };
    let now = Local::now();
    let due = due_reminders(&notes, now.naive_local());
    let keys: Vec<String> = due.iter().map(|(k, _)| k.clone()).collect();
    let claimed = match store
        .timers()
        .claim_reminders(&keys, now.timestamp_millis())
    {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::warn!("리마인더 기록 실패: {e}");
            return;
        }
    };
    let titles: Vec<&str> = due
        .iter()
        .filter(|(k, _)| claimed.contains(k))
        .map(|(_, title)| title.as_str())
        .collect();
    if titles.len() > MAX_REMINDER_WINDOWS {
        show_alert(
            &format!("할 일 {}개", titles.len()),
            &titles.join(" · "),
            ALERT_REMINDER,
        );
    } else {
        for title in titles {
            show_alert(title, "할 일 시간이 됐습니다", ALERT_REMINDER);
        }
    }
}

fn show_alert(title: &str, body: &str, kind: &str) {
    use kmd_core::process::HideConsole;
    let exe = crate::keybind::resolve_launch_cmd("kmd-desktop");
    if let Err(e) = std::process::Command::new(&exe)
        .arg(kmd_core::notes::ALERT_ARG)
        .arg(title)
        .arg(body)
        .arg(kind)
        .hide_console()
        .spawn()
    {
        tracing::error!("알림 창 실행 실패: {exe} — {e}");
    }
}
