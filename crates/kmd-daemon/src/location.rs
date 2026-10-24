//! 현재 위치 — `:길` 의 기본 출발지 (docs/20 §3.1).
//!
//! 런처는 단축키마다 새로 뜨므로 매번 위치를 새로 재면 1~3초가 걸린다. 상주하는
//! 데몬이 마지막 측정값을 **메모리에만** 들고 있다가 IPC 로 넘긴다 — 디스크에는
//! 남기지 않는다. 측정은 OS 위치 서비스(주변 Wi-Fi 기반, 수십 m)를 쓴다.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 측정값 하나
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lng: f64,
    /// 정확도 반경 (m)
    pub accuracy_m: f64,
}

/// 이보다 새 측정값은 다시 재지 않고 그대로 준다
const FRESH: Duration = Duration::from_secs(60);
/// 새로 재다 실패하면 이만큼 지난 값까지는 대신 준다
const STALE: Duration = Duration::from_secs(30 * 60);
/// 한 번 재는 데 기다리는 시간 — IPC 응답 제한(5초) 안
const MEASURE_TIMEOUT: Duration = Duration::from_secs(4);

static LAST: Mutex<Option<(Fix, Instant)>> = Mutex::new(None);

/// 현재 위치와 측정 후 지난 초. 측정은 블로킹이다 (IPC 클라이언트 스레드에서 부른다).
pub fn current() -> Result<(Fix, u64), String> {
    let cached = LAST.lock().ok().and_then(|g| *g);
    if let Some((fix, at)) = cached {
        if at.elapsed() < FRESH {
            return Ok((fix, at.elapsed().as_secs()));
        }
    }
    match platform::measure(MEASURE_TIMEOUT) {
        Ok(fix) => {
            if let Ok(mut g) = LAST.lock() {
                *g = Some((fix, Instant::now()));
            }
            Ok((fix, 0))
        }
        Err(e) => match cached {
            Some((fix, at)) if at.elapsed() < STALE => {
                tracing::debug!("위치 재측정 실패, 이전 값 사용: {e}");
                Ok((fix, at.elapsed().as_secs()))
            }
            _ => Err(e),
        },
    }
}

#[cfg(windows)]
mod platform {
    use super::Fix;
    use std::time::Duration;
    use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
    use windows::Foundation::TimeSpan;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    /// Windows 위치 API (`Windows.Devices.Geolocation`).
    ///
    /// 설정 > 개인정보 > 위치에서 "데스크톱 앱의 위치 접근 허용"이 꺼져 있으면
    /// 접근 거부가 난다 — 그 안내를 오류 문구에 담는다.
    pub fn measure(timeout: Duration) -> Result<Fix, String> {
        // 클라이언트 스레드마다 처음 한 번 — 이미 초기화됐으면 무해한 오류가 난다
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let denied = || {
            "Windows 위치 접근이 꺼져 있습니다 — 설정 > 개인정보 및 보안 > 위치에서 \
             '위치 서비스'와 '데스크톱 앱의 위치 접근 허용'을 켜세요"
                .to_string()
        };
        let status = Geolocator::RequestAccessAsync()
            .and_then(|op| op.join())
            .map_err(|e| format!("위치 권한 확인 실패: {e}"))?;
        if status != GeolocationAccessStatus::Allowed {
            return Err(denied());
        }
        let locator = Geolocator::new().map_err(|e| format!("위치 서비스 열기 실패: {e}"))?;
        let hundred_ns = |d: Duration| TimeSpan {
            Duration: (d.as_nanos() / 100) as i64,
        };
        let pos = locator
            .GetGeopositionAsyncWithAgeAndTimeout(
                hundred_ns(Duration::from_secs(60)),
                hundred_ns(timeout),
            )
            .and_then(|op| op.join())
            .map_err(|e| format!("현재 위치를 얻지 못했습니다: {e}"))?;
        let coord = pos.Coordinate().map_err(|e| e.to_string())?;
        let p = coord
            .Point()
            .and_then(|pt| pt.Position())
            .map_err(|e| e.to_string())?;
        Ok(Fix {
            lat: p.Latitude,
            lng: p.Longitude,
            accuracy_m: coord.Accuracy().unwrap_or(0.0),
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Fix;
    use std::time::{Duration, Instant};

    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
    use objc2_core_location::{
        kCLLocationAccuracyHundredMeters, CLAuthorizationStatus, CLLocationManager,
    };

    /// 이보다 오래된 CoreLocation 캐시는 새 측정을 기다린다
    const MAX_AGE_SECS: f64 = 120.0;

    /// CoreLocation — 델리게이트 없이 `location` 속성을 런 루프를 돌리며 살핀다.
    ///
    /// 처음엔 "위치 접근 허용" 창이 뜬다(권한은 데몬 실행 파일에 붙는다 — 문구는
    /// build.rs 가 넣는 Info.plist). 승인을 기다리는 동안 이번 요청은 시간 초과로
    /// 실패하고, 승인 뒤 요청부터 위치가 나온다.
    pub fn measure(timeout: Duration) -> Result<Fix, String> {
        let manager = unsafe { CLLocationManager::new() };
        let status = unsafe { manager.authorizationStatus() };
        if status == CLAuthorizationStatus::Denied || status == CLAuthorizationStatus::Restricted {
            return Err(
                "macOS 위치 접근이 거부돼 있습니다 — 시스템 설정 > 개인정보 보호 및 \
                        보안 > 위치 서비스에서 kmd-daemon 을 켜세요"
                    .to_string(),
            );
        }
        if status == CLAuthorizationStatus::NotDetermined {
            unsafe { manager.requestWhenInUseAuthorization() };
        }
        unsafe {
            manager.setDesiredAccuracy(kCLLocationAccuracyHundredMeters);
            manager.startUpdatingLocation();
        }
        let deadline = Instant::now() + timeout;
        let found = loop {
            if let Some(loc) = unsafe { manager.location() } {
                let age = -unsafe { loc.timestamp().timeIntervalSinceNow() };
                let acc = unsafe { loc.horizontalAccuracy() };
                if age < MAX_AGE_SECS && acc >= 0.0 {
                    let c = unsafe { loc.coordinate() };
                    break Some(Fix {
                        lat: c.latitude,
                        lng: c.longitude,
                        accuracy_m: acc,
                    });
                }
            }
            if Instant::now() >= deadline {
                break None;
            }
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.2, true);
        };
        unsafe { manager.stopUpdatingLocation() };
        found.ok_or_else(|| {
            if unsafe { manager.authorizationStatus() } == CLAuthorizationStatus::NotDetermined {
                "위치 접근 허용 창에서 승인한 뒤 다시 시도하세요".to_string()
            } else {
                "현재 위치를 얻지 못했습니다 (Wi-Fi 가 꺼져 있으면 위치를 못 잡습니다)".to_string()
            }
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::Fix;
    use std::time::Duration;

    pub fn measure(_timeout: Duration) -> Result<Fix, String> {
        Err("이 OS에서는 아직 현재 위치를 지원하지 않습니다".to_string())
    }
}
