//! 마우스 워커 — 키 홀드 상태를 연속 포인터 이동으로 변환 (플랫폼 공용)
//!
//! 엔진의 MouseEngage/MouseRelease 결정으로 방향 플래그를 세우고,
//! 전용 스레드가 125Hz(8ms) 틱으로 가속 커브를 적용한 상대 이동을
//! [`MouseSink`]에 넘긴다. 주입(SendInput/CGEvent)은 어댑터 몫이다.
//!
//! 가속: 시작 180px/s → 500ms에 걸쳐 1300px/s (QMK kinetic 모드와 유사).
//! 첫 몇 픽셀은 정밀 조준, 길게 누르면 화면을 가로지른다.
//! 이동이 완전히 멈추면 램프가 초기화된다.

use super::MouseBind;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// 틱 간격 — 125Hz
const TICK: Duration = Duration::from_millis(8);
/// 시작 속도 (px/s)
const MIN_SPEED: f32 = 180.0;
/// 최대 속도 (px/s)
const MAX_SPEED: f32 = 1300.0;
/// 최대 속도 도달 시간 (ms)
const RAMP_MS: f32 = 500.0;
/// 저속 정밀 모드 배율 (mouse:slow 홀드)
const SLOW_FACTOR: f32 = 0.25;
/// 휠 홀드 연사 속도 (노치/초) — kanata 프리셋 mwheel(50ms 간격)과 같은 체감.
/// 키다운 즉시 1노치는 pending_wheel이 별도 보장하므로 이 값은 홀드 연사에만 관여한다.
const WHEEL_NOTCHES_PER_SEC: f32 = 20.0;

/// 램프 경과 시간(ms)에 대한 이동 속도(px/s). slow 모드면 SLOW_FACTOR 적용.
/// 순수 함수로 분리해 스레드/벽시계 없이 단위 테스트한다 (타이밍 플레이크 방지).
fn ramp_speed(t_ms: f32, slow: bool) -> f32 {
    let speed = MIN_SPEED + (MAX_SPEED - MIN_SPEED) * (t_ms / RAMP_MS).min(1.0);
    if slow {
        speed * SLOW_FACTOR
    } else {
        speed
    }
}

/// 상대 이동/휠 주입 — 플랫폼 어댑터가 구현
pub trait MouseSink: Send + 'static {
    /// 포인터 상대 이동 (픽셀)
    fn move_rel(&mut self, dx: i32, dy: i32);
    /// 휠 스크롤 (+1 = 위로 1노치)
    fn wheel(&mut self, notches: i32);
}

#[derive(Default)]
struct MotionState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    wheel_up: bool,
    wheel_down: bool,
    slow: bool,
    /// 키다운 시점에 적립되는 즉시 발사 노치 (+위/-아래).
    /// 홀드 누적(초당 N노치)만으로는 짧은 탭이 1노치 문턱(1/N초)에 못 미쳐
    /// 이벤트가 아예 안 나가는 문제를 막는다 — 탭 1회 = 최소 1노치 보장.
    pending_wheel: i32,
    /// 워커 종료 신호
    shutdown: bool,
    /// 워커가 완전 정지 상태로 대기 중 — 이 동안은 어떤 이동/휠도 내보내지 않는다.
    /// 테스트가 "정지가 반영됐다"를 벽시계 sleep 없이 확인하는 데 쓴다.
    idle: bool,
}

impl MotionState {
    fn active(&self) -> bool {
        self.up || self.down || self.left || self.right || self.wheel_up || self.wheel_down
    }
}

/// 마우스 모션 워커 — 버튼은 다루지 않는다 (버튼 down/up은 어댑터가
/// 키 이벤트 시점에 직접 주입해야 드래그 지연이 없다)
pub struct MouseWorker {
    state: Arc<(Mutex<MotionState>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MouseWorker {
    pub fn start<S: MouseSink>(mut sink: S) -> Self {
        let state = Arc::new((Mutex::new(MotionState::default()), Condvar::new()));
        let shared = state.clone();

        let thread = std::thread::spawn(move || {
            let (lock, cv) = &*shared;
            let mut ramp_start: Option<Instant> = None;
            // 픽셀 미만 이동량 누적 (틱당 소수 픽셀을 버리지 않는다)
            let mut acc_x = 0f32;
            let mut acc_y = 0f32;
            let mut acc_wheel = 0f32;

            loop {
                let (u, d, l, r, wu, wd, slow, pending_wheel) = {
                    let mut guard = match lock.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    // pending_wheel이 남아 있으면 홀드가 이미 풀렸어도 발사해야 한다
                    while !guard.active() && guard.pending_wheel == 0 && !guard.shutdown {
                        // 완전 정지 — 램프/누적 초기화 후 대기
                        ramp_start = None;
                        acc_x = 0.0;
                        acc_y = 0.0;
                        acc_wheel = 0.0;
                        guard.idle = true;
                        guard = match cv.wait(guard) {
                            Ok(g) => g,
                            Err(_) => return,
                        };
                        guard.idle = false;
                    }
                    if guard.shutdown {
                        return;
                    }
                    (
                        guard.up,
                        guard.down,
                        guard.left,
                        guard.right,
                        guard.wheel_up,
                        guard.wheel_down,
                        guard.slow,
                        std::mem::take(&mut guard.pending_wheel),
                    )
                };

                // ── 이동 속도: 시간 기반 가속 램프 ──
                let now = Instant::now();
                let start = *ramp_start.get_or_insert(now);
                let t_ms = now.duration_since(start).as_secs_f32() * 1000.0;
                let speed = ramp_speed(t_ms, slow);

                let mut vx = (r as i8 - l as i8) as f32;
                let mut vy = (d as i8 - u as i8) as f32;
                if vx != 0.0 && vy != 0.0 {
                    // 대각선 정규화 — 축 이동과 같은 체감 속도
                    let inv = std::f32::consts::FRAC_1_SQRT_2;
                    vx *= inv;
                    vy *= inv;
                }

                let dt = TICK.as_secs_f32();
                acc_x += vx * speed * dt;
                acc_y += vy * speed * dt;
                let dx = acc_x as i32;
                let dy = acc_y as i32;
                acc_x -= dx as f32;
                acc_y -= dy as f32;
                if dx != 0 || dy != 0 {
                    sink.move_rel(dx, dy);
                }

                // ── 휠: 키다운 즉시 노치 + 홀드 고정 속도 누적 ──
                let wv = (wu as i8 - wd as i8) as f32;
                if wv != 0.0 {
                    acc_wheel += wv * WHEEL_NOTCHES_PER_SEC * dt;
                }
                let mut notches = acc_wheel as i32;
                acc_wheel -= notches as f32;
                notches += pending_wheel;
                if notches != 0 {
                    sink.wheel(notches);
                }

                std::thread::sleep(TICK);
            }
        });

        Self {
            state,
            thread: Some(thread),
        }
    }

    /// 이동/휠/저속 바인딩 시작. 버튼 바인딩은 무시한다 (어댑터 직접 처리).
    pub fn engage(&self, bind: MouseBind) {
        self.set(bind, true);
    }

    /// 이동/휠/저속 바인딩 정지
    pub fn release(&self, bind: MouseBind) {
        self.set(bind, false);
    }

    /// 모든 모션 정지 (레이어 트리거 해제, keymap 토글, 종료 시)
    pub fn stop_all(&self) {
        let (lock, cv) = &*self.state;
        if let Ok(mut guard) = lock.lock() {
            *guard = MotionState {
                shutdown: guard.shutdown,
                idle: guard.idle,
                ..MotionState::default()
            };
            cv.notify_one();
        }
    }

    /// 워커가 정지 대기에 들어갈 때까지 기다린다 (테스트용). 시간 안에 못 들어가면 false.
    #[cfg(test)]
    fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.state.0.lock().map(|g| g.idle).unwrap_or(false) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    fn set(&self, bind: MouseBind, on: bool) {
        let (lock, cv) = &*self.state;
        if let Ok(mut guard) = lock.lock() {
            match bind {
                MouseBind::MoveUp => guard.up = on,
                MouseBind::MoveDown => guard.down = on,
                MouseBind::MoveLeft => guard.left = on,
                MouseBind::MoveRight => guard.right = on,
                MouseBind::WheelUp => {
                    if on && !guard.wheel_up {
                        guard.pending_wheel += 1; // 키다운 즉시 1노치 (탭 보장)
                    }
                    guard.wheel_up = on;
                }
                MouseBind::WheelDown => {
                    if on && !guard.wheel_down {
                        guard.pending_wheel -= 1;
                    }
                    guard.wheel_down = on;
                }
                MouseBind::Slow => guard.slow = on,
                MouseBind::BtnLeft | MouseBind::BtnRight | MouseBind::BtnMiddle => return,
            }
            cv.notify_one();
        }
    }
}

impl Drop for MouseWorker {
    fn drop(&mut self) {
        let (lock, cv) = &*self.state;
        if let Ok(mut guard) = lock.lock() {
            guard.shutdown = true;
            cv.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct RecordingSink(mpsc::Sender<(i32, i32)>);
    impl MouseSink for RecordingSink {
        fn move_rel(&mut self, dx: i32, dy: i32) {
            let _ = self.0.send((dx, dy));
        }
        fn wheel(&mut self, _notches: i32) {}
    }

    struct WheelSink(mpsc::Sender<i32>);
    impl MouseSink for WheelSink {
        fn move_rel(&mut self, _dx: i32, _dy: i32) {}
        fn wheel(&mut self, notches: i32) {
            let _ = self.0.send(notches);
        }
    }

    #[test]
    fn wheel_tap_emits_immediate_notch() {
        let (tx, rx) = mpsc::channel();
        let worker = MouseWorker::start(WheelSink(tx));

        // 홀드 누적 문턱(1/WHEEL_NOTCHES_PER_SEC초)보다 짧은 탭이라도
        // pending_wheel 덕에 최소 1노치가 보장돼야 한다
        worker.engage(MouseBind::WheelUp);
        worker.release(MouseBind::WheelUp);
        let n = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("탭 즉시 노치 발생");
        assert!(n >= 1, "위 방향 노치 기대: {n}");

        worker.engage(MouseBind::WheelDown);
        worker.release(MouseBind::WheelDown);
        let n = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("탭 즉시 노치 발생");
        assert!(n <= -1, "아래 방향 노치 기대: {n}");
    }

    #[test]
    fn engage_moves_release_stops() {
        let (tx, rx) = mpsc::channel();
        let worker = MouseWorker::start(RecordingSink(tx));

        worker.engage(MouseBind::MoveRight);
        // 수 틱 안에 오른쪽 이동이 나와야 한다
        let (dx, dy) = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("이동 발생");
        assert!(dx > 0, "오른쪽 이동 기대: dx={dx}");
        assert_eq!(dy, 0);

        worker.release(MouseBind::MoveRight);
        // 워커가 정지 대기에 들어간 뒤에는 이동이 나올 수 없다 — 그 전 잔여 틱만 비운다.
        // (예전엔 50ms sleep 후 비우기라 CI 부하로 틱이 늦으면 플레이크가 났다)
        assert!(worker.wait_idle(Duration::from_secs(5)), "정지 대기 진입");
        while rx.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(30));
        assert!(rx.try_recv().is_err(), "정지 후 이동 없음");
    }

    #[test]
    fn stop_all_clears_motion() {
        let (tx, rx) = mpsc::channel();
        let worker = MouseWorker::start(RecordingSink(tx));
        worker.engage(MouseBind::MoveLeft);
        worker.engage(MouseBind::MoveUp);
        rx.recv_timeout(Duration::from_millis(500))
            .expect("이동 발생");

        worker.stop_all();
        assert!(worker.wait_idle(Duration::from_secs(5)), "정지 대기 진입");
        while rx.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(30));
        assert!(rx.try_recv().is_err(), "stop_all 후 이동 없음");
    }

    // slow 모드 속도 검증은 순수 함수 ramp_speed로 한다. 워커 스레드 + 벽시계로
    // 재면 CI 부하에 따라 틱 간격이 달라져 플레이크가 난다(실제로 두 번 깨졌다) —
    // 속도 로직 자체는 시간의 결정적 함수이므로 스레드 없이 직접 검증한다.
    #[test]
    fn slow_mode_reduces_step() {
        for &t in &[0.0f32, 100.0, 250.0, 500.0, 1000.0] {
            let normal = ramp_speed(t, false);
            let slow = ramp_speed(t, true);
            assert!(normal >= MIN_SPEED, "일반 속도는 최소 속도 이상 (t={t})");
            assert!(
                (slow - normal * SLOW_FACTOR).abs() < 0.01,
                "저속 = 일반 × {SLOW_FACTOR} (t={t}: slow={slow}, normal={normal})"
            );
            assert!(slow < normal, "저속은 항상 일반보다 느리다 (t={t})");
        }
    }

    #[test]
    fn ramp_가속은_min에서_max까지_단조증가_후_고정() {
        assert!(
            (ramp_speed(0.0, false) - MIN_SPEED).abs() < 0.01,
            "t=0은 MIN"
        );
        assert!(
            ramp_speed(250.0, false) > ramp_speed(0.0, false),
            "중간은 더 빠름"
        );
        assert!(
            (ramp_speed(RAMP_MS, false) - MAX_SPEED).abs() < 0.01,
            "램프 끝=MAX"
        );
        assert!(
            (ramp_speed(RAMP_MS * 10.0, false) - MAX_SPEED).abs() < 0.01,
            "램프 이후는 MAX에 고정(clamp)"
        );
    }
}
