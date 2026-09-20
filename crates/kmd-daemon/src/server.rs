//! TCP IPC 서버 — 검색 엔진 메모리 상주 + 요청 처리

use crate::autostart;
use crate::keybind::{self, KeybindConfig};
use fs2::FileExt;
use kmd_core::ipc::{self, Request, Response, SearchHit};
use kmd_core::{Config, Index, SearchEngine};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const DAEMON_LOCK_FILE: &str = "daemon.lock";

/// OS가 프로세스 종료 시 자동으로 해제하는 데몬 단일 인스턴스 잠금.
///
/// 잠금 파일 자체는 지우지 않는다. 잠금을 쥔 채 파일을 지우면 새 프로세스가
/// 같은 경로에 다른 inode를 만들어 두 프로세스가 서로 다른 파일을 잠글 수 있다.
struct DaemonInstanceGuard {
    _file: File,
}

enum DaemonInstanceAction {
    Acquired(DaemonInstanceGuard),
    AlreadyRunning,
}

fn daemon_lock_path() -> PathBuf {
    ipc::runtime_dir().join(DAEMON_LOCK_FILE)
}

/// 파일 생성과 OS 배타 잠금을 한 단계로 묶어 동시 기동의 check-then-start
/// 레이스를 없앤다. 프로세스가 비정상 종료해도 파일 핸들이 닫히며 잠금은
/// 자동 해제되므로 남은 daemon.lock 파일은 다음 기동을 막지 않는다.
fn acquire_daemon_instance() -> color_eyre::Result<DaemonInstanceAction> {
    std::fs::create_dir_all(ipc::runtime_dir())?;
    let path = daemon_lock_path();
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            // 잠금 소유자만 진단용 PID를 갱신한다. 실패해도 잠금 자체는 유효하다.
            use std::io::{Seek, SeekFrom};
            let _ = file.set_len(0);
            let _ = file.seek(SeekFrom::Start(0));
            let _ = write!(file, "{}", std::process::id());
            Ok(DaemonInstanceAction::Acquired(DaemonInstanceGuard {
                _file: file,
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Ok(DaemonInstanceAction::AlreadyRunning)
        }
        Err(error) => Err(error.into()),
    }
}

/// 전체 인덱스를 공유 캐시(index.bin/index.json)에 저장
fn save_full_index_cache(index: &Index) {
    let data_dir = Config::default_data_dir();
    kmd_core::index::store::save_both(
        index,
        &data_dir.join(kmd_core::INDEX_CACHE_BIN_FILENAME),
        &data_dir.join(kmd_core::INDEX_CACHE_FILENAME),
    );
}

/// quick 인덱스(앱/PATH/시스템 명령)를 데스크톱 캐시에 저장
fn save_quick_index_cache(use_emoji: bool) {
    let desktop_dir = Config::default_data_dir().join("desktop");
    let index = Index::build_quick(use_emoji);
    kmd_core::index::store::save_both(
        &index,
        &desktop_dir.join(kmd_core::QUICK_INDEX_CACHE_BIN_FILENAME),
        &desktop_dir.join(kmd_core::QUICK_INDEX_CACHE_FILENAME),
    );
}

/// 설정된 리프레시 주기를 부팅 캐시의 최대 수명으로도 사용한다.
/// 0(off)은 기존 캐시를 만료시키지 않되, 캐시가 아예 없으면 한 번은
/// 백그라운드에서 전체 인덱스를 만든다.
fn index_cache_max_age(refresh_minutes: u64) -> Option<Duration> {
    (refresh_minutes != 0).then(|| Duration::from_secs(refresh_minutes.saturating_mul(60)))
}

/// 부팅 크리티컬 패스에서는 디스크 전체 스캔을 하지 않는다. 신선한 전체
/// 캐시가 있으면 즉시 쓰고, 없으면 PATH/시스템 명령만 포함하는 quick 인덱스로
/// IPC를 먼저 연 뒤 리프레셔가 전체 인덱스로 교체한다.
fn load_startup_index(config: &Config) -> (Index, bool) {
    let data_dir = Config::default_data_dir();
    let bin_path = data_dir.join(kmd_core::INDEX_CACHE_BIN_FILENAME);
    let json_path = data_dir.join(kmd_core::INDEX_CACHE_FILENAME);
    let config_modified = config
        .config_path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok()?.modified().ok());
    let newest_cache = [&bin_path, &json_path]
        .into_iter()
        .filter_map(|path| std::fs::metadata(path).ok()?.modified().ok())
        .max();
    let config_changed = config_modified
        .zip(newest_cache)
        .is_some_and(|(config_time, cache_time)| config_time > cache_time);
    let cached = if config_changed {
        tracing::info!("설정 파일이 인덱스 캐시보다 새로움 — 백그라운드 재빌드 예약");
        None
    } else {
        kmd_core::index::store::try_load_cached_with_max_age(
            &bin_path,
            &json_path,
            Index::current_version(),
            index_cache_max_age(config.launcher.index_refresh_minutes),
        )
    };

    match cached {
        Some(index) => {
            tracing::info!(
                "신선한 전체 인덱스 캐시로 즉시 시작 ({}개 항목)",
                index.len()
            );
            (index, false)
        }
        None => {
            let index = Index::build_quick(config.general.emoji_icons);
            tracing::info!(
                "전체 인덱스 캐시 없음/만료 — quick 인덱스로 시작 ({}개 항목)",
                index.len()
            );
            (index, true)
        }
    }
}

#[cfg(target_os = "windows")]
fn lower_indexer_thread_priority() {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };

    // 전체 파일 스캔이 키 훅/포그라운드 앱과 CPU를 다투지 않게 한다.
    // 실패해도 기능은 유지되므로 경고만 남긴다.
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) } == 0 {
        tracing::warn!("Windows 인덱서 스레드 우선순위 하향 실패");
    }
}

#[cfg(target_os = "macos")]
fn lower_indexer_thread_priority() {
    use std::os::raw::{c_int, c_uint};

    // QOS_CLASS_UTILITY(0x11) — 인덱싱/워처 스레드를 시스템이 E-코어·저순위로
    // 스케줄하게 한다. 이 프로세스는 키 이벤트 탭을 쥐고 있으므로(키보드 먹통
    // 사고 이력) 백그라운드 스캔이 기본 QoS로 도는 것을 피한다.
    const QOS_CLASS_UTILITY: c_uint = 0x11;
    extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: c_uint, relative_priority: c_int) -> c_int;
    }
    if unsafe { pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0) } != 0 {
        tracing::warn!("macOS 인덱서 스레드 QoS 하향 실패");
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn lower_indexer_thread_priority() {}

/// 전체 인덱스 재생성 직렬화 락.
///
/// 재생성 경로는 둘이다 — 주기 리프레셔와 IPC `RebuildIndex`. 둘이 겹치면 같은
/// 트리를 두 번 스캔하고 같은 캐시 파일에 경쟁적으로 쓴다(임시 파일명이 PID
/// 기준이라 같은 데몬 안의 두 스레드는 구분되지 않았다). 이 락으로 한 번에
/// 하나만 돌게 한다.
static INDEX_REBUILD_LOCK: Mutex<()> = Mutex::new(());

/// 재생성 락을 잡는다. 오염된 락(이전 재생성이 패닉)도 계속 쓴다 —
/// 인덱스 재생성은 실패해도 다음 주기에 다시 하면 되는 작업이라,
/// 락 오염 때문에 영구히 멈추는 쪽이 더 나쁘다.
fn lock_index_rebuild() -> std::sync::MutexGuard<'static, ()> {
    INDEX_REBUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn rebuild_full_index(engine: &Arc<Mutex<SearchEngine>>, config: &Config) {
    let _rebuild_guard = lock_index_rebuild();
    let started = Instant::now();
    let index = Index::build(&config.launcher, config.general.emoji_icons);
    let count = index.items.len();
    save_full_index_cache(&index);
    if let Ok(mut e) = engine.lock() {
        e.set_kind_weights(config.launcher.kind_weights.clone());
        e.load(index.items);
    } else {
        tracing::warn!("인덱스 교체 실패: 검색 엔진 잠금 오염");
    }
    save_quick_index_cache(config.general.emoji_icons);
    tracing::info!(
        "백그라운드 인덱스 리프레시 완료 ({count}개 항목, {}ms)",
        started.elapsed().as_millis()
    );

    sync_content_index(config);
}

/// 문서 본문 인덱스 증분 갱신 (docs/15 P1) — 파일명 인덱스 리프레시 직후,
/// 그리고 파일 변경 감시(spawn_content_watcher)에서도 호출된다.
/// 실패해도 파일명 검색에는 영향 없다 (경고만 남기고 계속).
///
/// 여러 경로(리프레셔/IPC/워처)가 동시에 부를 수 있어 겹침은 스킵한다 —
/// sync는 증분·멱등이라 다음 호출이 어차피 따라잡는다.
fn sync_content_index(config: &Config) {
    use std::sync::atomic::AtomicBool;
    static RUNNING: AtomicBool = AtomicBool::new(false);

    // 비활성이어도 sync()는 호출한다 — sync가 기존 인덱스를 purge해
    // "꺼도 과거 본문이 남는" 반쪽 비활성화를 막는다 (빈 인덱스면 수 ms no-op).
    if RUNNING.swap(true, Ordering::SeqCst) {
        tracing::debug!("본문 인덱스 sync 겹침 — 스킵");
        return;
    }
    // RAII 해제 — 아래 어느 경로로 반환돼도 플래그가 풀린다
    struct Reset<'a>(&'a AtomicBool);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset(&RUNNING);

    let started = Instant::now();
    let db_path = Config::default_data_dir().join(kmd_core::DB_FILENAME);
    match kmd_core::Database::open(&db_path) {
        Ok(db) => match kmd_core::content_index::sync(&db, &config.launcher) {
            Ok(s) => tracing::info!(
                "본문 인덱스 동기화 완료 (스캔 {} · 갱신 {} · 스킵 {} · 제거 {} · 실패 {}, {}ms)",
                s.scanned,
                s.indexed,
                s.skipped,
                s.removed,
                s.failed,
                started.elapsed().as_millis()
            ),
            Err(e) => tracing::warn!("본문 인덱스 동기화 실패: {e}"),
        },
        Err(e) => tracing::warn!("본문 인덱스 DB 열기 실패: {e}"),
    }
}

/// 파일 변경 감시 스레드 — 본문 인덱스 실시간 증분 sync (docs/15 P2).
///
/// search_paths를 재귀 감시하고, 이벤트가 500ms 잠잠해지면 증분 sync를
/// 돌린다 (무변경 sync는 수 ms — mtime+size 스킵 덕). 안전장치:
/// - 최소 간격 10초 — 이벤트 폭풍에도 sync 스래시 방지
/// - 최대 지연 60초 — 연속 이벤트(로그 파일 등)로 인한 기아 방지
/// - 60초마다 config 재로드 — search_paths 변경 시 재감시, 비활성화 반영
/// - 이벤트 유실은 기존 6시간 주기 리프레셔가 보정 (docs/15의 이중 안전망)
fn spawn_content_watcher(shutdown: Arc<AtomicBool>) {
    use notify::{RecursiveMode, Watcher};

    const DEBOUNCE: Duration = Duration::from_millis(500);
    const MIN_INTERVAL: Duration = Duration::from_secs(10);
    const MAX_DELAY: Duration = Duration::from_secs(60);
    const CONFIG_RELOAD: Duration = Duration::from_secs(60);

    std::thread::spawn(move || {
        lower_indexer_thread_priority();

        let mut config = load_config();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<_>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("본문 감시자 생성 실패 (주기 리프레시로만 동작): {e}");
                return;
            }
        };

        let mut watched: Vec<std::path::PathBuf> = Vec::new();
        let rewatch = |watcher: &mut notify::RecommendedWatcher,
                       watched: &mut Vec<std::path::PathBuf>,
                       cfg: &Config| {
            for p in watched.iter() {
                let _ = watcher.unwatch(p);
            }
            watched.clear();
            if !cfg.launcher.content_search.enabled {
                return;
            }
            for p in &cfg.launcher.search_paths {
                if !p.is_dir() {
                    continue;
                }
                match watcher.watch(p, RecursiveMode::Recursive) {
                    Ok(()) => watched.push(p.clone()),
                    Err(e) => tracing::warn!("경로 감시 실패 {}: {e}", p.display()),
                }
            }
            tracing::info!("본문 인덱스 파일 감시 시작 ({}개 경로)", watched.len());
        };
        rewatch(&mut watcher, &mut watched, &config);

        let mut dirty = false;
        let mut last_event = Instant::now();
        let mut last_sync = Instant::now();
        let mut last_config_load = Instant::now();

        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(()) => {
                    dirty = true;
                    last_event = Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }

            if last_config_load.elapsed() >= CONFIG_RELOAD {
                last_config_load = Instant::now();
                let fresh = load_config();
                if fresh.launcher.search_paths != config.launcher.search_paths
                    || fresh.launcher.content_search.enabled
                        != config.launcher.content_search.enabled
                {
                    config = fresh;
                    rewatch(&mut watcher, &mut watched, &config);
                } else {
                    config = fresh;
                }
            }

            let quiet = last_event.elapsed() >= DEBOUNCE;
            let starved = last_sync.elapsed() >= MAX_DELAY;
            if dirty && (quiet || starved) && last_sync.elapsed() >= MIN_INTERVAL {
                dirty = false;
                last_sync = Instant::now();
                sync_content_index(&config);
            }
        }
    });
}

/// 백그라운드 인덱스 리프레셔 스레드.
///
/// `launcher.index_refresh_minutes` 주기(기본 6시간, 0=off)로 전체/quick
/// 인덱스를 재빌드해 공유 캐시를 원자적으로 갱신하고 데몬 검색 엔진도
/// 교체한다. kmd-desktop은 언제 떠도 신선한 캐시를 즉시 로드하므로
/// 실행 시점(alt+space)에 인덱싱 비용을 지불하지 않는다.
fn spawn_index_refresher(
    engine: Arc<Mutex<SearchEngine>>,
    shutdown: Arc<AtomicBool>,
    rebuild_immediately: bool,
) {
    std::thread::spawn(move || {
        lower_indexer_thread_priority();
        let boot_cfg = load_config();
        tracing::info!(
            "인덱스 리프레셔 시작 (주기 {}분, 0=off)",
            boot_cfg.launcher.index_refresh_minutes
        );

        if rebuild_immediately {
            // 첫 설치/만료 직후에도 IPC와 훅이 안정화될 시간을 먼저 준다.
            // Windows에서는 이 스레드 우선순위도 낮춰 포그라운드 키 입력을
            // 인덱싱보다 우선 처리한다.
            for _ in 0..20 {
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            rebuild_full_index(&engine, &boot_cfg);
        }

        let mut elapsed_secs: u64 = 0;
        let mut interval_min = boot_cfg.launcher.index_refresh_minutes;
        loop {
            if shutdown.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            elapsed_secs += 1;

            // 1분마다 설정 재로드 — 주기 변경/비활성화를 재시작 없이 반영
            if elapsed_secs.is_multiple_of(60) {
                interval_min = load_config().launcher.index_refresh_minutes;
            }
            if interval_min == 0 || elapsed_secs < interval_min * 60 {
                continue;
            }
            elapsed_secs = 0;

            let cfg = load_config();
            rebuild_full_index(&engine, &cfg);
        }
    });
}

/// 데몬 메인 루프
pub fn run() -> color_eyre::Result<()> {
    // Ping 선확인은 두 프로세스가 동시에 통과할 수 있다. OS 배타 잠금을 먼저
    // 획득해 check→초기화 전체를 직렬화하고 프로세스 수명 동안 보유한다.
    let _instance_guard = match acquire_daemon_instance()? {
        DaemonInstanceAction::Acquired(guard) => guard,
        DaemonInstanceAction::AlreadyRunning => {
            println!("이미 실행 중인 데몬이 있어 종료합니다.");
            return Ok(());
        }
    };

    // 잠금 도입 전 버전의 살아 있는 데몬과도 공존하지 않는다. 잠금 소유권을
    // 얻은 뒤 Ping이 실패하면 남은 포트/PID는 죽은 데몬의 stale runtime이므로
    // 소유자만 정리한다.
    if let Ok(ipc::Response::Pong) = ipc::send_request_result(&ipc::Request::Ping) {
        println!("이미 실행 중인 데몬이 있어 종료합니다.");
        return Ok(());
    }
    ipc::cleanup_stale_files();

    let started_at = Instant::now();
    let shutdown = Arc::new(AtomicBool::new(false));
    // Shutdown 요청 → 채널로 메인 스레드를 즉시 깨운다 (200ms 폴링 제거)
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // 설정 로드 (성공/폴백 로그는 load_config 내부에서)
    let config = load_config();

    // 키 바인딩 엔진 시작
    let mut kb_backend = keybind::create_backend();
    let kb_preset = resolve_keybind_preset(&config);
    // start()는 훅 스레드를 띄우기만 하므로 Ok(())가 "훅 설치 성공"을 뜻하지
    // 않는다. 실제 설치 결과는 keybind::hook_error()로 확인한다.
    match kb_backend.start(kb_preset) {
        Ok(()) => tracing::info!("키 바인딩 엔진 기동 (훅 설치 결과는 이후 로그 참조)"),
        Err(e) => {
            keybind::set_hook_error(Some(format!("키 바인딩 시작 실패: {e}")));
            tracing::warn!("키 바인딩 시작 실패: {e}");
        }
    }

    // 전체 파일 스캔은 키 훅이 살아 있는 부팅 크리티컬 패스에서 제거한다.
    // 신선한 캐시 또는 quick 인덱스로 먼저 IPC를 연다.
    let (index, rebuild_immediately) = load_startup_index(&config);
    let item_count = index.items.len();
    tracing::info!("{item_count}개 항목으로 검색 엔진 초기화");

    let engine = Arc::new(Mutex::new({
        let mut e = SearchEngine::new();
        e.set_kind_weights(config.launcher.kind_weights.clone());
        e.load(index.items);
        e
    }));

    // 클립보드 히스토리 감시 (opt-in) — clip:N 붙여넣기의 데이터 소스
    crate::clipboard::spawn_watcher(
        config.clipboard.history_enabled,
        config.clipboard.history_size,
    );

    // TCP 서버 시작 (OS가 빈 포트 자동 할당)
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    tracing::info!("IPC 서버 시작: 127.0.0.1:{port}");

    // 인증 토큰 생성 후 포트/PID 파일 기록 (daemon.port: line1=port, line2=token)
    let token = Arc::new(ipc::generate_token());
    write_runtime_files(port, &token)?;

    println!(
        "kmd-daemon 실행 중 (port={port}, pid={})",
        std::process::id()
    );

    // IPC 준비를 외부에 알린 다음에만 전체 인덱싱을 시작한다. 캐시가 신선하면
    // 주기 리프레시 전까지 아무 스캔도 하지 않는다.
    spawn_index_refresher(engine.clone(), shutdown.clone(), rebuild_immediately);
    spawn_content_watcher(shutdown.clone());

    // accept 루프를 별도 스레드에서 blocking 모드로 실행 — 50ms 바쁜 대기 제거.
    // 종료 시 `listener`를 닫으면 accept()가 에러를 반환하며 루프가 자연스럽게 종료된다.
    let accept_shutdown = shutdown.clone();
    let accept_token = token.clone();
    let accept_tx = shutdown_tx.clone();
    let accept_handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            if accept_shutdown.load(Ordering::Relaxed) {
                break;
            }
            match stream {
                Ok(stream) => {
                    let engine = engine.clone();
                    let conn_tx = accept_tx.clone();
                    let conn_token = accept_token.clone();
                    std::thread::spawn(move || {
                        if let Err(e) =
                            handle_client(stream, &engine, &conn_tx, started_at, &conn_token)
                        {
                            tracing::warn!("클라이언트 처리 에러: {e}");
                        }
                    });
                }
                Err(e) => {
                    if accept_shutdown.load(Ordering::Relaxed) {
                        break; // 정상 종료 신호
                    }
                    tracing::warn!("accept 에러: {e}");
                }
            }
        }
    });

    // 종료 신호가 올 때까지 블로킹 대기 (Shutdown 요청 시 send로 깨어남)
    let _ = shutdown_rx.recv();
    shutdown.store(true, Ordering::Relaxed);

    // accept 스레드는 accept()에서 블로킹 중이므로 self-connect로 깨운다.
    // 기존에는 클라이언트(kmd daemon stop)의 폴링 connect가 우연히 깨워줄
    // 때만 종료가 완료됐다 — 다른 경로의 Shutdown 요청 시 영구 대기했음.
    let _ = TcpStream::connect(("127.0.0.1", port));

    let _ = accept_handle.join();

    // 키 바인딩 종료
    if let Err(e) = kb_backend.stop() {
        tracing::warn!("키 바인딩 종료 실패: {e}");
    }

    cleanup_runtime_files();
    tracing::info!("데몬 종료");
    Ok(())
}

fn handle_client(
    stream: TcpStream,
    engine: &Arc<Mutex<SearchEngine>>,
    shutdown_tx: &mpsc::Sender<()>,
    started_at: Instant,
    token: &str,
) -> color_eyre::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;

    // 요청 크기 상한 — 개행 없는 대용량 스트림으로 인한 메모리 소진 방지
    const MAX_REQUEST_BYTES: u64 = 64 * 1024;
    let mut reader = BufReader::new(std::io::Read::take(&stream, MAX_REQUEST_BYTES));
    let mut writer = &stream;

    // line 1: 인증 토큰. 불일치 시 응답 없이 연결 종료.
    let mut token_line = String::new();
    reader.read_line(&mut token_line)?;
    if !ipc::constant_time_eq(token_line.trim().as_bytes(), token.as_bytes()) {
        tracing::warn!("IPC 인증 실패 — 연결 종료");
        return Ok(());
    }

    // line 2: JSON 요청
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let request = ipc::decode_request(&line)?;
    let response = process_request(request, engine, shutdown_tx, started_at);

    let encoded = ipc::encode_response(&response)?;
    writer.write_all(encoded.as_bytes())?;
    writer.flush()?;

    Ok(())
}

fn process_request(
    request: Request,
    engine: &Arc<Mutex<SearchEngine>>,
    shutdown_tx: &mpsc::Sender<()>,
    started_at: Instant,
) -> Response {
    match request {
        Request::Ping => Response::Pong,

        Request::Status => {
            let uptime = started_at.elapsed().as_secs();
            let items = engine.lock().map(|e| e.len()).unwrap_or(0);
            Response::Status {
                uptime_secs: uptime,
                index_items: items,
                pid: std::process::id(),
                keymap_layers: KEYMAP_SUMMARY.lock().map(|g| g.clone()).unwrap_or_default(),
                config_error: CONFIG_LOAD_ERROR.lock().map(|g| g.clone()).unwrap_or(None),
                keybind_error: keybind::hook_error(),
                hook_health: keybind::hook_health_summary(),
            }
        }

        Request::AutostartStatus => Response::AutostartStatus {
            installed: autostart::is_installed(),
        },

        Request::AutostartEnable => match autostart::install() {
            Ok(detail) => Response::Ok {
                message: format!("자동 시작 등록 완료 ({detail})"),
            },
            Err(e) => Response::Error {
                message: format!("자동 시작 등록 실패: {e}"),
            },
        },

        Request::AutostartDisable => match autostart::uninstall() {
            Ok(()) => Response::Ok {
                message: "자동 시작 해제 완료".into(),
            },
            Err(e) => Response::Error {
                message: format!("자동 시작 해제 실패: {e}"),
            },
        },

        Request::Search { query, limit } => {
            let results = match engine.lock() {
                Ok(mut e) => {
                    let (_mode, hits) = e.search(&query, limit);
                    hits
                }
                Err(_) => {
                    return Response::Error {
                        message: "엔진 잠금 실패".into(),
                    }
                }
            };

            let items: Vec<SearchHit> = results
                .into_iter()
                .map(|r| SearchHit {
                    name: r.item.name,
                    path: r.item.path,
                    kind: r.item.kind.to_string(),
                    icon: r.item.icon,
                    score: r.score,
                })
                .collect();

            Response::SearchResults { items }
        }

        Request::RebuildIndex => {
            // 주기 리프레셔와 같은 락 — 둘이 겹쳐 중복 스캔·캐시 경쟁을 만들지 않게.
            let _rebuild_guard = lock_index_rebuild();
            let config = load_config();
            let index = Index::build(&config.launcher, config.general.emoji_icons);
            let count = index.items.len();

            // 공유 캐시도 함께 갱신 — kmd-desktop이 다음 실행에서 바로 반영
            save_full_index_cache(&index);
            save_quick_index_cache(config.general.emoji_icons);

            match engine.lock() {
                Ok(mut e) => {
                    e.set_kind_weights(config.launcher.kind_weights.clone());
                    e.load(index.items);
                }
                Err(_) => {
                    return Response::Error {
                        message: "엔진 잠금 실패".into(),
                    }
                }
            }

            sync_content_index(&config);

            Response::Ok {
                message: format!("인덱스 리빌드 완료 ({count}개 항목)"),
            }
        }

        Request::Shutdown => {
            let _ = shutdown_tx.send(());
            Response::Ok {
                message: "데몬을 종료합니다.".into(),
            }
        }

        Request::LlmAutopilot { jobs } => {
            let n = jobs.len();
            crate::autopilot::run_autopilot(jobs);
            Response::Ok {
                message: format!("LLM 오토파일럿 시작 ({n}개)"),
            }
        }

        Request::LlmFollowup { prompt } => {
            if !crate::autopilot::has_session() {
                return Response::Error {
                    message: "이어서 질문할 LLM 창이 없습니다. 먼저 @gpt/@llm 등으로 여세요."
                        .into(),
                };
            }
            crate::autopilot::run_followup(prompt);
            Response::Ok {
                message: "이어서 질문 전달 중".into(),
            }
        }

        Request::InjectPaste => {
            keybind::inject_paste();
            Response::Ok {
                message: "붙여넣기 주입".into(),
            }
        }

        Request::ClipPaste { slot, to_previous } => {
            // 워커가 아니라 여기서 직접 실행 — macOS는 RESTORE_DELAY(300ms),
            // Windows는 텍스트 길이에 비례한 주입 시간만큼 블로킹하나 이 커넥션
            // 스레드에 한정되며 IPC 응답도 그 뒤에 나간다.
            // 빈 슬롯/스냅샷·주입 실패는 성공으로 위장하지 않고 Error로 알린다.
            match crate::clipboard::paste_slot_checked(slot, to_previous) {
                Ok(()) => Response::Ok {
                    message: format!(
                        "클립보드 슬롯 {slot} 붙여넣기 (히스토리 {}건)",
                        crate::clipboard::len()
                    ),
                },
                Err(message) => Response::Error { message },
            }
        }

        Request::ClipPasteItem { id, to_previous } => {
            match crate::clipboard::paste_item(id, to_previous) {
                Ok(()) => Response::Ok {
                    message: "클립보드 항목 붙여넣기".into(),
                },
                Err(message) => Response::Error { message },
            }
        }

        Request::ClipCopyItem { id } => match crate::clipboard::copy_item(id) {
            Ok(()) => Response::Ok {
                message: "클립보드 항목 복사".into(),
            },
            Err(message) => Response::Error { message },
        },

        Request::ClipHistory { query, limit } => Response::ClipHistory {
            hits: crate::clipboard::search(&query, limit),
        },

        Request::ClipCaptureForeground => {
            crate::clipboard::capture_foreground_app();
            Response::Ok {
                message: "전경 앱 캡처".into(),
            }
        }

        Request::KeybindSelfTest => {
            #[cfg(target_os = "macos")]
            {
                let preset = resolve_keybind_preset(&load_config());
                // launchd 아래에서는 stderr가 유실되므로 패닉을 삼켜 IPC 응답으로
                // 돌려준다 — 핸들러 스레드 패닉 = 클라이언트엔 원인 불명 연결 리셋.
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    keybind::selftest::run(&preset)
                })) {
                    Ok(Ok(report)) => Response::Ok { message: report },
                    Ok(Err(e)) => Response::Error { message: e },
                    Err(panic) => {
                        let msg = panic
                            .downcast_ref::<String>()
                            .map(String::as_str)
                            .or_else(|| panic.downcast_ref::<&str>().copied())
                            .unwrap_or("(비문자열 패닉)");
                        Response::Error {
                            message: format!("셀프테스트 패닉: {msg}"),
                        }
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            Response::Error {
                message: "키 주입 셀프테스트는 현재 macOS 전용입니다".into(),
            }
        }
    }
}

/// config.toml 로드 실패 메시지 — `kmd daemon status`로 노출한다.
/// TOML 문법 오류(테이블 중복 정의 등)가 조용히 기본값 폴백되면 사용자는
/// 설정이 반영된 줄 알게 되므로, 에러를 상태 조회에서 바로 볼 수 있어야 한다.
static CONFIG_LOAD_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn load_config() -> Config {
    let config_dir = Config::default_config_dir();
    match Config::load(&config_dir) {
        Ok(config) => {
            if let Ok(mut g) = CONFIG_LOAD_ERROR.lock() {
                *g = None;
            }
            tracing::info!("설정 로드 완료: {}", config_dir.display());
            config
        }
        Err(e) => {
            // 데몬은 계속 떠 있어야 하므로 기본값으로 폴백하되 에러를 명확히 남긴다
            let msg = format!("{} 로드 실패: {e}", config_dir.display());
            tracing::error!("{msg} — 기본 설정으로 동작합니다");
            if let Ok(mut g) = CONFIG_LOAD_ERROR.lock() {
                *g = Some(msg);
            }
            Config::default()
        }
    }
}

/// 실행 중인 키맵 레이어 요약 — `kmd daemon status`로 노출해 원격 진단에 쓴다
/// (어떤 config가 실제 엔진에 적용됐는지 로그 없이 확인 가능).
static KEYMAP_SUMMARY: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn keymap_layer_summaries(kb: &KeybindConfig) -> Vec<String> {
    kb.layers
        .iter()
        .map(|l| {
            format!(
                "{}: {:?} 홀드 · unmapped={:?} · 매핑 {}개 · 더블탭 {}개",
                l.name,
                l.trigger,
                l.unmapped,
                l.mappings.len(),
                l.double_tap_mappings.len()
            )
        })
        .collect()
}

fn resolve_keybind_preset(config: &Config) -> KeybindConfig {
    // 프리셋 기본값·사용자 병합·한/영 폴백은 전부 kmd-core의
    // effective_keymap(단일 소스)에서 수행된다 — 치트시트와 항상 동일 결과.
    let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
    let mut kb = KeybindConfig::from_config(&effective).unwrap_or_else(KeybindConfig::empty);

    // global_hotkey → kmd-desktop 실행 콤보 (프로필과 무관하게 항상 등록)
    let hotkey = &config.keybindings.global_hotkey;
    if !hotkey.is_empty() {
        if let Some(trigger) = keybind::parse_combo_trigger(hotkey) {
            kb.combos
                .push((trigger, keybind::BindAction::Launch("kmd-desktop".into())));
            tracing::info!("글로벌 핫키 등록: {hotkey} → kmd-desktop 실행");
        }
    }

    let toggle_keymap = config.keybindings.toggle_keymap.trim();
    if !toggle_keymap.is_empty() {
        if let Some(trigger) = keybind::parse_combo_trigger(toggle_keymap) {
            kb.toggle_keymap = Some(trigger);
            tracing::info!("keymap toggle hotkey registered: {toggle_keymap}");
        } else {
            tracing::warn!("keymap toggle hotkey parse failed: {toggle_keymap}");
        }
    }

    let summaries = keymap_layer_summaries(&kb);
    for s in &summaries {
        tracing::info!("레이어 {s}");
    }
    if let Ok(mut g) = KEYMAP_SUMMARY.lock() {
        *g = summaries;
    }

    kb
}

fn write_runtime_files(port: u16, token: &str) -> color_eyre::Result<()> {
    // 런타임 파일은 포터블 모드와 무관하게 OS 표준 사용자 디렉터리에 둔다
    // (토큰 노출 방지 — ipc::runtime_dir 문서 참조)
    std::fs::create_dir_all(ipc::runtime_dir())?;

    // daemon.port: line1=port, line2=token (동일 사용자만 읽도록 Unix 0600).
    // 쓰기 후 chmod 하면 그 사이에 다른 사용자가 읽을 수 있으므로
    // 처음부터 0600으로 생성한다. 기존 파일은 권한이 남을 수 있어 먼저 제거.
    let port_path = ipc::port_file_path();
    let _ = std::fs::remove_file(&port_path);
    {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&port_path)?;
        write!(f, "{port}\n{token}\n")?;
    }

    std::fs::write(ipc::pid_file_path(), std::process::id().to_string())?;
    Ok(())
}

fn cleanup_runtime_files() {
    ipc::cleanup_stale_files();
}

#[cfg(test)]
mod tests {
    use super::*;

    // 프리셋·병합 시맨틱 테스트는 단일 소스인 kmd-core::keymap
    // (effective_keymap)의 테스트로 이동했다. 여기서는 daemon 고유 경로인
    // "effective → KeybindConfig 변환 + 프로필 비활성"만 확인한다.

    fn resolve_with_profile(profile: &str) -> KeybindConfig {
        let mut config = Config::default();
        config.launcher.keymap.active_profile = profile.to_string();
        config.keybindings.global_hotkey = String::new();
        config.keybindings.toggle_keymap = String::new();
        resolve_keybind_preset(&config)
    }

    #[test]
    fn index_cache_max_age는_refresh_off면_무기한() {
        assert_eq!(index_cache_max_age(0), None);
        assert_eq!(index_cache_max_age(6), Some(Duration::from_secs(360)));
    }

    #[test]
    fn resolve_vim_nav_기본_레이어() {
        let kb = resolve_with_profile("vim-nav");
        assert_eq!(kb.layers.len(), 2, "nav + mouse 레이어");
        assert!(
            kb.layers
                .iter()
                .any(|l| l.trigger == keybind::VKey::CapsLock),
            "0.13.0부터 nav 기본 트리거 = CapsLock"
        );
        let mouse = kb
            .layers
            .iter()
            .find(|l| l.trigger == keybind::VKey::RAlt)
            .expect("RAlt 마우스 레이어");
        assert!(
            mouse.trigger_aliases.contains(&keybind::VKey::Hangul),
            "한국어 배열(RAlt=한/영) 별칭"
        );
        assert!(!kb.combos.is_empty(), "한/영 Shift+Space 콤보 포함");
        #[cfg(target_os = "windows")]
        assert!(
            !kb.tap_holds
                .iter()
                .any(|t| t.key == keybind::VKey::CapsLock),
            "CapsLock이 기본 레이어 트리거(0.13.0)이므로 모드탭은 미주입"
        );
    }

    #[test]
    fn resolve_확장자_붙은_프로필도_vim_nav() {
        // 과거 치트시트는 완전 일치("vim-nav")만 프리셋으로 인식해
        // "vim-nav.kbd"에서 daemon과 표시가 어긋났다 — 회귀 방지
        let kb = resolve_with_profile("vim-nav.kbd");
        assert_eq!(kb.layers.len(), 2);
    }

    #[test]
    fn resolve_none_프로필은_키맵_비활성() {
        let kb = resolve_with_profile("none");
        assert!(kb.layers.is_empty());
        assert!(kb.remaps.is_empty());
        assert!(kb.combos.is_empty(), "비활성 시 한/영 콤보도 없음");
    }

    #[test]
    fn resolve_global_hotkey는_프로필과_무관() {
        let mut config = Config::default();
        config.launcher.keymap.active_profile = "none".to_string();
        config.keybindings.global_hotkey = "alt+space".to_string();
        config.keybindings.toggle_keymap = String::new();
        let kb = resolve_keybind_preset(&config);
        assert_eq!(kb.combos.len(), 1, "none 프로필에서도 글로벌 핫키는 등록");
    }

    #[test]
    fn 만료된_클립보드_id는_성공으로_위장하지_않는다() {
        // u64::MAX는 발급될 수 없는 ID — 항목 조회가 클립보드 접근 전에
        // 실패하므로 테스트가 실제 클립보드를 건드리지 않는다.
        let engine = Arc::new(Mutex::new(SearchEngine::new()));
        let (tx, _rx) = mpsc::channel();
        let response = process_request(
            Request::ClipPasteItem {
                id: u64::MAX,
                to_previous: false,
            },
            &engine,
            &tx,
            Instant::now(),
        );
        assert!(matches!(response, Response::Error { .. }));

        let response = process_request(
            Request::ClipCopyItem { id: u64::MAX },
            &engine,
            &tx,
            Instant::now(),
        );
        assert!(matches!(response, Response::Error { .. }));
    }

    #[test]
    fn resolve_launch_cmd_경로순회_방지() {
        let resolved = keybind::resolve_launch_cmd("../../../etc/passwd");
        assert!(
            !resolved.contains(".."),
            "경로 순회 문자가 제거되어야 함: {resolved}"
        );
    }
}
