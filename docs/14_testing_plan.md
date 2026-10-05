# 테스트 전략 — E2E 자동화 계획 + 테스트 정리 계획

> 상태: 2026-08-11 수립. **Tier 1·Tier 2 모두 구현 완료** (Tier 2는 2026-08-11).
> 남은 것은 Tier 2의 Windows CI 편입(`KMD_DATA_DIR` 선행 필요)과
> Tier 3(보류). → [17_feature_backlog.md](17_feature_backlog.md) F9·F11

## 현황 (2026-10-05 갱신)

**규모 수치는 적지 않는다** — 손으로 적으면 며칠 만에 낡는다.
현재 값은 `./scripts/code-metrics.sh`.

| 층 | 수단 | 규모 | CI |
|---|---|---|---|
| 단위 (로직) | `#[test]` 인라인 | 전 크레이트 | ✅ 3-OS 매트릭스 |
| 엔진 시뮬레이션 | engine.rs 키 시퀀스 테스트 | 탭홀드·더블탭·chord·리피트 | ✅ |
| 통합 | kmd-core `tests/audit_no_window.rs` | — | ✅ |
| 데몬 프로세스 E2E | **Tier 1** | `tests/e2e_daemon.rs` | ✅ unix (Windows는 F9 대기) |
| 키 주입 진짜 E2E | **Tier 2** — `kmd daemon e2e` (아래 §Tier 2) | — | 🔶 로컬·릴리스 게이트. CI 편입은 F11 |
| GUI (iced) | 앱 모델 단위 테스트만 | — | ❌ 🧊 Tier 3 보류(유지비 > 효용) |

교훈(2026-08-10 키보드 먹통 사고): 엔진 단위 테스트는 **OS와의 상호작용**
(Ctrl+Space 홀드가 입력 소스 피커를 여는 것 같은)을 잡지 못한다. 그 빈틈을
메우는 것이 E2E 계획의 목적이다.

## Part A — E2E 자동화

### Tier 1: 데몬 프로세스/IPC E2E (CI, unix) — ✅ 구현됨

`crates/kmd-daemon/tests/e2e_daemon.rs`. 빌드된 실제 `kmd-daemon` 바이너리를
`CARGO_BIN_EXE_kmd-daemon`으로 spawn해 검증한다:

- 기동 → 포트 파일 생성 → Status 응답(레이어 로드, config 오류 없음)
- **토큰 인증 거부** (잘못된 토큰 → 요청 거부) — 보안 회귀 방지
- 중복 기동 거부 (kmd.lock 단일 인스턴스)
- Shutdown → 프로세스 정상 종료

격리: 자식 프로세스의 `HOME`/`XDG_*`를 tempdir로 돌려 config·data·런타임
파일(포트/락)이 실제 사용자 환경과 완전히 분리된다. 이 때문에 **unix 전용**
— Windows의 `dirs`는 KnownFolder API를 쓰므로 env로 격리가 안 된다
(Tier 2에서 `KMD_DATA_DIR` 오버라이드를 추가해 Windows도 편입).

안전장치: 테스트 config는 트리거를 `LAlt`로 명시한다 — 기본값(CapsLock)이면
데몬이 hidutil 재맵(시스템 전역 상태)을 건드린다. `config_error == None`
어서션이 이 안전장치가 무효화되는 것(파싱 실패 → 기본 config 폴백)을 잡는다.

게이트: `KMD_E2E=1`일 때만 실행 (미설정 시 조용히 skip). CI test 잡이 설정하고,
로컬은 `KMD_E2E=1 cargo test -p kmd-daemon --test e2e_daemon`.

**첫 실행 성과**: 중복 기동 시나리오가 실제 버그를 잡았다 — kmd-daemon
바이너리 자체에 단일 인스턴스 가드가 없어(CLI만 체크) launchd·직접 실행
경로로 데몬 2개가 동시에 뜰 수 있었다(이벤트 탭 이중 설치 + 포트 파일
덮어쓰기). server::run() 진입부에 Ping 생존 판정 가드를 추가해 해결.

### Tier 2: 키 주입 진짜 E2E — `kmd daemon e2e` ✅ 구현됨 (2026-08-11)

`crates/kmd-daemon/src/keybind/selftest.rs` + IPC `KeybindSelfTest`. 데몬이
자기 자신을 검증한다: listen-only 탭(tail)으로 최종 이벤트 스트림을 캡처하며,
마커(MAGIC_USER_DATA) 없는 합성 키를 주입해 활성 탭이 물리 입력처럼 처리하게
한다. 시나리오: A. 트리거 홀드+매핑 키→기대 출력(원본 키 누출 감지 포함)
B. 탭=한영→Ctrl+Space 정확히 1회 C. 연타 디바운스→추가 1회만
(B+C 토글 합계 2회 = 입력 소스 원상 복귀). 실기기 통과 확인.

- 접근성이 부여된 머신 전용 — **릴리스 전 로컬 1커맨드 게이트**로 운용.
  GitHub macOS 러너는 SIP 때문에 접근성 부여 불가 → CI 편입 불가.
- 판정은 이벤트 개수/순서 기반, 대기는 폴링+데드라인 (벽시계 실측 금지).
- 실행 중 1~2초 합성 키가 실제 입력 경로에 흐르므로 타이핑 금지 (CLI가 안내).

**구현이 발견한 버그·교훈 3건**:
1. **주입 위치는 HID**: `CGEventPost(kCGSessionEventTap)`으로 세션 위치에
   꽂은 이벤트는 같은 위치의 head 탭(데몬 활성 탭)에 전달되지 않는다 —
   리맵이 안 걸리고 원본이 누출됐다. `kCGHIDEventTap`(최상류) 주입으로 해결.
2. **클라이언트 IPC 타임아웃 5초 고정**: 수 초 걸리는 요청이 불가능했다 →
   `send_request_with_timeout()` 추가 (e2e는 60초).
3. **살아있는 데몬 유령화 버그(수정)**: 클라이언트가 Io 오류(요청 중 연결
   끊김 포함)마다 포트 파일을 지워, 데몬이 살아 있어도 재발견 불가가 됐다.
   재연결 프로브로 생사를 가른 뒤에만 정리하도록 수정.
4. 핸들러 스레드 패닉은 launchd 아래서 stderr가 유실돼 원인불명 연결 리셋으로
   보인다 — 셀프테스트는 catch_unwind로 패닉을 IPC 응답에 실어 보낸다.

추가 실험(후순위): Windows 러너는 대화형 데스크톱이 있어 SendInput+LL 훅이
동작하는 편 — Tier 2를 windows CI에 올릴 수 있는지 1회 검증해 볼 가치.

### Tier 3: GUI (kmd-desktop) — 보류

iced는 접근성 트리/드라이버가 없어 표준 GUI 자동화가 불가. 현실적 노선:

- update 로직을 모델 단위 테스트로 계속 확대 (지금 방식 유지)
- 픽셀/스크린샷 자동화는 도입하지 않는다 — 유지비 > 효용 (테마·폰트·DPI churn)
- 실기기 확인이 필요한 부분(런처 GUI 상호작용)은 docs 체크리스트 유지

## Part B — 불필요 테스트 정리

전체 실행 시간이 ~4초라 **런타임 비용은 없다. 비용은 유지보수(churn)다** —
정리 기준도 거기에 맞춘다.

### 삭제 기준 (하나라도 해당하면 후보)

1. **구현 미러링**: 코드의 매핑 테이블/상수를 그대로 복사해 비교하는 테스트 —
   데이터 추가마다 같은 내용을 두 번 쓰게 만들고, 실수를 잡지 못한다
   (테이블 오타가 테스트에도 복사되므로).
2. **중복 커버리지**: 같은 경로를 다른 이름으로 다시 도는 테스트 — 대표 1개 +
   경계 사례만 남긴다.
3. **죽은 기능**: 제거·비활성화된 기능의 테스트.
4. **벽시계 의존**: sleep/Instant 실측에 기대는 판정 — 삭제가 아니라 순수 함수
   재작성 대상 (CI 플레이크 원천).

### 유지 기준 (삭제 금지)

- 엔진 시퀀스 테스트 전체 (engine.rs 53개) — 이 프로젝트의 핵심 자산
- 사고·버그의 회귀 테스트 (오토리피트, chord 누출, stuck key, 한영 스냅백류)
- 파서/조합 로직 (hangul 조합, query prefix, transform, web URL 빌드)
- 보안 어서션 (토큰 인증, 파일 권한, audit_no_window)

### 감사 결과 (2026-08-11, 1차)

| 영역 | 개수 | 판정 |
|---|---|---|
| engine.rs 53 · server.rs 5 · mouse/윈도우 훅 | 유지 | 핵심 + 진행 중 브랜치 영역(동결) |
| kmd-core hangul 19 · web 22 · transform 9 · search 17 등 | 유지 | 행동 테스트, 값어치 있음 |
| brand_icons.rs 20 · system_icons.rs 5 | **유지 (감사 완료)** | 미러링이 아니라 `detect_service()`의 prefix/browse/URL 폴백·우선순위(gemini>google 등) 행동 테스트로 판명 |
| builtin_emoji.rs 10 | **유지 (감사 완료)** | db 로드+검색 행동 테스트 |
| mouse.rs sleep 2건 | 허용 | "정지 후 무이벤트" 검증의 안정화 대기 — 어서션은 이벤트 기반 |
| kmd-core 3.67초 (최다 소요) | 수용 | 전체 ~4초라 최적화 실익 없음 |

**2026-08-11 감사 결론: 삭제 대상 없음.** 의심 후보가 전부 행동 테스트로
판명됐고 벽시계 실측 판정도 남아 있지 않다 — 현 테스트 자산은 건강하다.
정리 기준(위 1~4)은 앞으로 신규 테스트 리뷰 기준으로 사용한다.

**동결 영역**: 진행 중 4개 브랜치가 건드리는 파일(macos.rs, windows.rs,
clipboard.rs, engine.rs, server.rs, ipc.rs, config.rs, app.rs, tui/app.rs)의
테스트는 브랜치 합류 전까지 정리하지 않는다 — 충돌 비용 > 정리 이득.

### 프로세스

- 삭제는 커밋 메시지에 기준 번호(위 1~4)와 근거를 남긴다.
- 새 버그 수정에는 회귀 테스트를 함께 넣는 관행 유지 (이번 한영 직렬화처럼
  E2E가 필요한 것은 Tier 2 시나리오 목록에 추가).
