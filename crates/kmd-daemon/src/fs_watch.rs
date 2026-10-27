//! 파일 변경 감시 — 본문 색인(docs/15 P2)과 파일 검색 색인(docs/22 F3)이 감시자 하나를 같이 쓴다.
//!
//! 검색 폴더를 재귀 감시하고:
//! - 본문 색인: 이벤트가 500ms 잠잠해지면 증분 sync (최소 간격 10초, 최대 지연 60초)
//! - 파일 검색 색인: 이벤트가 난 폴더만 다시 읽어 이름이 생기거나 지워졌으면 새 세대 (수 초).
//!   폴더 시각을 믿을 수 없는 exFAT·VeraCrypt 볼륨도 이벤트는 오므로 5분 주기를 기다리지 않는다.
//!   감시가 이벤트를 놓치면(버퍼 넘침) 폴더 시각 대조로 한 번 훑는다.
//!
//! **감시가 볼륨 분리를 막지 않게** — Windows 의 재귀 감시는 검색 폴더의 디렉터리 핸들을 열어 두어
//! VeraCrypt 마운트 해제가 "사용 중"으로 실패했다. VeraCrypt 는 해제 직전 모든 최상위 창에
//! `WM_DEVICECHANGE(DBT_DEVICEREMOVEPENDING)` 을 보내고 볼륨 잠금을 50ms 간격으로 30번 다시 시도한다
//! (VeraCrypt `UnmountVolumeBase`). 숨은 창으로 그 알림을 받아 그 드라이브의 감시를 바로 푼다.
//! macOS(FSEvents)·Linux(inotify) 감시는 마운트 해제를 막지 않는다.
//! Linux 는 본문 색인이 켜져 있을 때만 감시한다([`watch_paths`]).
//! 사라졌던 검색 폴더가 다시 나타나면(다시 마운트) 10초 안에 다시 감시한다.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kmd_core::Config;
use notify::{RecursiveMode, Watcher};

use crate::server::{load_config, lower_indexer_thread_priority, sync_content_index};

const CONTENT_DEBOUNCE: Duration = Duration::from_millis(500);
const CONTENT_MIN_INTERVAL: Duration = Duration::from_secs(10);
const CONTENT_MAX_DELAY: Duration = Duration::from_secs(60);
/// 파일 검색 색인 — 이벤트가 이만큼 잠잠하면 반영. 계속 이벤트가 와도 `FILE_MAX_DELAY` 마다는 반영
const FILE_DEBOUNCE: Duration = Duration::from_secs(1);
const FILE_MIN_INTERVAL: Duration = Duration::from_secs(3);
const FILE_MAX_DELAY: Duration = Duration::from_secs(15);
const CONFIG_RELOAD: Duration = Duration::from_secs(60);
/// 사라진 검색 폴더가 다시 나타났는지 확인하는 간격
const RECHECK: Duration = Duration::from_secs(10);
/// 볼륨 분리 알림 뒤 그 볼륨을 다시 감시하지 않는 시간 — 분리가 취소되면(다른 프로그램이 잡고 있음) 이 뒤에 다시 건다
#[cfg(windows)]
const RELEASE_HOLD: Duration = Duration::from_secs(15);

pub(crate) enum Msg {
    /// 바뀐 경로들
    Paths(Vec<PathBuf>),
    /// 감시가 이벤트를 놓쳤다 — 다시 훑어야 한다
    Rescan,
    /// 볼륨이 새로 붙었다
    #[cfg(windows)]
    VolumeArrived,
}

/// 감시 대상 검색 폴더 — 있는 것만 감시하고, 사라지면 풀고, 다시 나타나면 다시 건다
struct Roots {
    watcher: notify::RecommendedWatcher,
    desired: Vec<PathBuf>,
    watched: Vec<PathBuf>,
    #[cfg(windows)]
    held: Vec<(PathBuf, Instant)>,
}

impl Roots {
    fn set(&mut self, desired: Vec<PathBuf>) {
        for p in self.watched.drain(..) {
            let _ = self.watcher.unwatch(&p);
        }
        self.desired = desired;
        self.recheck();
        tracing::info!("파일 감시: 검색 폴더 {}개", self.watched.len());
    }

    /// 감시 상태를 실제 폴더와 맞춘다. 다시 감시하게 된 폴더가 있으면 true (다시 마운트 등)
    fn recheck(&mut self) -> bool {
        let watcher = &mut self.watcher;
        self.watched.retain(|p| {
            let alive = p.is_dir();
            if !alive {
                let _ = watcher.unwatch(p);
                tracing::info!("감시 해제 (폴더 없음): {}", p.display());
            }
            alive
        });
        #[cfg(windows)]
        self.held
            .retain(|(_, since)| since.elapsed() < RELEASE_HOLD);
        let mut armed = false;
        for p in &self.desired {
            if self.watched.contains(p) || !p.is_dir() {
                continue;
            }
            #[cfg(windows)]
            if self.held.iter().any(|(h, _)| h == p) {
                continue;
            }
            match self.watcher.watch(p, RecursiveMode::Recursive) {
                Ok(()) => {
                    self.watched.push(p.clone());
                    armed = true;
                }
                Err(e) => tracing::warn!("경로 감시 실패 {}: {e}", p.display()),
            }
        }
        armed
    }

    /// 분리될 드라이브 위의 감시를 바로 푼다 — 디렉터리 핸들이 열려 있으면 분리가 실패한다
    #[cfg(windows)]
    fn release_drives(&mut self, mask: u32) {
        let watcher = &mut self.watcher;
        let held = &mut self.held;
        self.watched.retain(|p| {
            let on_drive = drive_bit(p).is_some_and(|bit| mask & bit != 0);
            if on_drive {
                let _ = watcher.unwatch(p);
                held.push((p.clone(), Instant::now()));
                tracing::info!("볼륨 분리 알림 — 감시 해제: {}", p.display());
            }
            !on_drive
        });
    }
}

/// 감시 목록을 잠그고 `f` — 잠근 쪽이 패닉했어도 목록은 그대로 쓸 수 있다
fn with_roots<T>(roots: &Mutex<Roots>, f: impl FnOnce(&mut Roots) -> T) -> T {
    f(&mut roots.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `E:\MyOffice` → E: 의 비트. UNC 등 드라이브 문자가 없으면 None
#[cfg(any(windows, test))]
fn drive_bit(p: &Path) -> Option<u32> {
    let s = p.as_os_str().to_str()?;
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let mut chars = s.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    (letter.is_ascii_uppercase() && chars.next() == Some(':'))
        .then(|| 1u32 << (letter as u32 - 'A' as u32))
}

/// 이벤트 경로 → 다시 읽을 폴더 (그 경로가 든 폴더). 색인 자신의 쓰기는 거른다
fn dirty_dir(path: &Path, own: &[PathBuf]) -> Option<PathBuf> {
    if own.iter().any(|d| path.starts_with(d)) {
        return None;
    }
    Some(path.parent().unwrap_or(path).to_path_buf())
}

/// 감시할 검색 폴더. Linux 는 파일 검색만으로는 감시하지 않는다 — inotify 재귀 감시는 하위 폴더마다
/// 감시를 하나씩 걸어 한도(옛 커널 8,192)에 걸리고, ext4 는 폴더 시각을 믿을 수 있어 열 때 대조로 충분하다.
fn watch_paths(cfg: &Config) -> Vec<PathBuf> {
    let for_files = cfg.launcher.file_search.enabled() && !cfg!(target_os = "linux");
    if cfg.launcher.content_search.enabled || for_files {
        cfg.launcher.effective_search_paths()
    } else {
        Vec::new()
    }
}

pub(crate) fn spawn(shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        lower_indexer_thread_priority();

        let (tx, rx) = mpsc::channel::<Msg>();
        let event_tx = tx.clone();
        let watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else {
                    return;
                };
                if event.need_rescan() {
                    let _ = event_tx.send(Msg::Rescan);
                } else if !matches!(event.kind, notify::EventKind::Access(_)) {
                    let _ = event_tx.send(Msg::Paths(event.paths));
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("파일 감시자 생성 실패 (주기 리프레시로만 동작): {e}");
                    return;
                }
            };
        let mut config = load_config();
        // 볼륨 분리 알림은 알림 창 스레드가 직접 푼다 — 이 루프가 본문 sync·전체 대조로 몇 초 바쁜 동안
        // 기다리면 VeraCrypt 의 잠금 재시도(약 1.5초)가 끝나 해제가 실패한다
        let roots = Arc::new(Mutex::new(Roots {
            watcher,
            desired: Vec::new(),
            watched: Vec::new(),
            #[cfg(windows)]
            held: Vec::new(),
        }));
        with_roots(&roots, |r| r.set(watch_paths(&config)));
        #[cfg(windows)]
        volume_events::spawn(roots.clone(), tx.clone());
        drop(tx);
        let own = kmd_core::filesearch::data_dirs();

        let mut content_dirty = false;
        let mut content_event = Instant::now();
        let mut content_sync = Instant::now();
        let mut file_dirty: HashSet<PathBuf> = HashSet::new();
        let mut file_rescan = false;
        let mut file_first_event = Instant::now();
        let mut file_event = Instant::now();
        let mut file_apply = Instant::now();
        let mut last_config_load = Instant::now();
        let mut last_recheck = Instant::now();

        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            let mut on_event = |dirty: &mut HashSet<PathBuf>, rescan: bool| {
                let now = Instant::now();
                content_dirty = true;
                content_event = now;
                if dirty.is_empty() && !file_rescan {
                    file_first_event = now;
                }
                file_event = now;
                file_rescan |= rescan;
            };
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Msg::Paths(paths)) => {
                    let dirs: Vec<PathBuf> =
                        paths.iter().filter_map(|p| dirty_dir(p, &own)).collect();
                    if !dirs.is_empty() {
                        on_event(&mut file_dirty, false);
                        file_dirty.extend(dirs);
                    }
                }
                Ok(Msg::Rescan) => on_event(&mut file_dirty, true),
                #[cfg(windows)]
                Ok(Msg::VolumeArrived) => {
                    if with_roots(&roots, Roots::recheck) {
                        on_event(&mut file_dirty, true);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }

            if last_config_load.elapsed() >= CONFIG_RELOAD {
                last_config_load = Instant::now();
                let fresh = load_config();
                let rewatch = watch_paths(&fresh) != watch_paths(&config);
                config = fresh;
                if rewatch {
                    with_roots(&roots, |r| r.set(watch_paths(&config)));
                }
            }
            if last_recheck.elapsed() >= RECHECK {
                last_recheck = Instant::now();
                // 다시 마운트된 검색 폴더 — 꺼져 있던 동안의 변경은 이벤트가 없으니 훑는다
                if with_roots(&roots, Roots::recheck) {
                    file_rescan = true;
                    file_first_event = Instant::now();
                }
            }

            if config.launcher.content_search.enabled {
                let quiet = content_event.elapsed() >= CONTENT_DEBOUNCE;
                let starved = content_sync.elapsed() >= CONTENT_MAX_DELAY;
                if content_dirty
                    && (quiet || starved)
                    && content_sync.elapsed() >= CONTENT_MIN_INTERVAL
                {
                    content_dirty = false;
                    content_sync = Instant::now();
                    sync_content_index(&config);
                }
            } else {
                content_dirty = false;
            }

            if !config.launcher.file_search.enabled() {
                file_dirty.clear();
                file_rescan = false;
                continue;
            }
            let pending = file_rescan || !file_dirty.is_empty();
            let quiet = file_event.elapsed() >= FILE_DEBOUNCE;
            let starved = file_first_event.elapsed() >= FILE_MAX_DELAY;
            if pending && (quiet || starved) && file_apply.elapsed() >= FILE_MIN_INTERVAL {
                file_apply = Instant::now();
                let started = Instant::now();
                let result = if std::mem::take(&mut file_rescan) {
                    file_dirty.clear();
                    kmd_core::filesearch::refresh(&config.launcher)
                } else {
                    let dirs: Vec<PathBuf> = file_dirty.drain().collect();
                    kmd_core::filesearch::apply_dirty(&config.launcher, &dirs)
                };
                match result {
                    Ok(Some(s)) => tracing::info!(
                        "파일 색인 감시 반영: 폴더 {}개 다시 읽음, +{} −{} ({}ms)",
                        s.checked_dirs,
                        s.added,
                        s.removed,
                        started.elapsed().as_millis()
                    ),
                    Ok(None) => {}
                    Err(e) => tracing::warn!("파일 색인 감시 반영 실패: {e}"),
                }
            }
        }
    });
}

/// 볼륨 분리·연결 알림 (Windows) — `WM_DEVICECHANGE` 는 최상위 창에만 방송되므로 숨은 창을 하나 둔다.
/// 메시지 전용 창(HWND_MESSAGE)은 방송을 받지 못한다.
#[cfg(windows)]
mod volume_events {
    use super::{with_roots, Msg, Roots};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, DBT_DEVICEARRIVAL, DBT_DEVICEQUERYREMOVE, DBT_DEVICEREMOVEPENDING,
        DBT_DEVTYP_VOLUME, DEV_BROADCAST_HDR, DEV_BROADCAST_VOLUME, MSG, WM_DEVICECHANGE,
        WNDCLASSW, WS_OVERLAPPED,
    };

    struct Shared {
        roots: Arc<Mutex<Roots>>,
        tx: Mutex<Sender<Msg>>,
    }

    static SHARED: OnceLock<Shared> = OnceLock::new();

    pub(super) fn spawn(roots: Arc<Mutex<Roots>>, tx: Sender<Msg>) {
        let shared = Shared {
            roots,
            tx: Mutex::new(tx),
        };
        if SHARED.set(shared).is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            let class: Vec<u16> = "kmd-volume-events\0".encode_utf16().collect();
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&wc) == 0 {
                tracing::warn!(
                    "볼륨 알림 창 등록 실패 — VeraCrypt 해제 시 감시가 해제를 막을 수 있음"
                );
                return;
            }
            // 보이지 않는 최상위 창 (WS_VISIBLE 없음)
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                tracing::warn!(
                    "볼륨 알림 창 생성 실패 — VeraCrypt 해제 시 감시가 해제를 막을 수 있음"
                );
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    fn on_removing(mask: u32) {
        if let Some(shared) = SHARED.get() {
            with_roots(&shared.roots, |r| r.release_drives(mask));
        }
    }

    fn on_arrived() {
        if let Some(shared) = SHARED.get() {
            if let Ok(tx) = shared.tx.lock() {
                let _ = tx.send(Msg::VolumeArrived);
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_DEVICECHANGE {
            let event = wparam as u32;
            // 방송한 쪽(VeraCrypt)은 100ms 안에 답을 기다린다 — 감시 해제는 핸들 닫기 요청만 보내 바로 끝난다
            if lparam != 0 {
                // SAFETY: WM_DEVICECHANGE 의 lparam 은 DEV_BROADCAST_HDR 로 시작하는 구조체이고,
                // 장치 종류가 볼륨이면 DEV_BROADCAST_VOLUME 이다 (Win32 문서)
                let hdr = unsafe { &*(lparam as *const DEV_BROADCAST_HDR) };
                if hdr.dbch_devicetype == DBT_DEVTYP_VOLUME {
                    let vol = unsafe { &*(lparam as *const DEV_BROADCAST_VOLUME) };
                    match event {
                        DBT_DEVICEREMOVEPENDING | DBT_DEVICEQUERYREMOVE => {
                            on_removing(vol.dbcv_unitmask)
                        }
                        DBT_DEVICEARRIVAL => on_arrived(),
                        _ => {}
                    }
                }
            }
            return 1; // TRUE — 분리를 거부하지 않는다
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 드라이브_문자() {
        assert_eq!(drive_bit(Path::new(r"E:\MyOffice")), Some(1 << 4));
        assert_eq!(drive_bit(Path::new(r"o:\")), Some(1 << 14));
        assert_eq!(drive_bit(Path::new(r"\\?\E:\MyOffice")), Some(1 << 4));
        assert_eq!(drive_bit(Path::new(r"\\server\share")), None);
        assert_eq!(drive_bit(Path::new("/Users/me")), None);
    }

    #[test]
    fn 이벤트_경로는_든_폴더로_색인_자신의_쓰기는_거름() {
        let own = vec![PathBuf::from("/data/kmd")];
        assert_eq!(
            dirty_dir(Path::new("/docs/2026/보고서.hwp"), &own),
            Some(PathBuf::from("/docs/2026"))
        );
        assert_eq!(
            dirty_dir(Path::new("/data/kmd/filesearch/files.3.snap"), &own),
            None
        );
    }
}
