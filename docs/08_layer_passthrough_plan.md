# 레이어 패스쓰루(VIA/QMK KC_TRNS 스타일) 도입 검토 및 단계별 전략

> **상태: P0–P3 구현 완료 (v0.9.3 릴리스) · P2(Windows)/P3(macOS) 실기기 검증 대기 ·
> P4 미완** — 기본 프리셋 passthrough(F3)·`:keymap` 표시(F5)는
> [17_feature_backlog.md](17_feature_backlog.md) 참조. 이 문서는 **설계 이력**이다 —
> 왜 이 구조를 골랐는지와 검토했던 대안이 남아 있다. 현재 동작의 정본은
> `crates/kmd-daemon/src/keybind/engine.rs` 와 CHANGELOG의 0.9.3–0.9.5 항목이다.
> 아래 "단계별 전략"의 미래형 서술은 당시 계획 시점의 표현이다.

작성: 2026-07-12 (v0.9.2 엔진 기준)

## 1. 문제 정의

현재 Windows에서 Alt를 레이어 트리거로 쓰면:

- 트리거 down은 **즉시 억제**된다 (`engine.rs` §3, tap-vs-hold 판정을 위해
  OS에 Alt를 보이지 않게 함).
- 레이어에 **매핑된 키**는 새 액션을 실행한다. ✅
- 레이어에 **매핑되지 않은 키**는 엔진 최하단에서 `PassThrough`되지만,
  OS 입장에서는 Alt가 눌린 적이 없으므로 **맨키(Tab, F4)만 전달**된다.
  → Alt+Tab 창 전환, Alt+F4 닫기 등 OS 기본 조합이 전부 죽는다.

이 때문에 지금은 `toggle_keymap` 콤보(키맵 임시 on/off)로 우회하고 있다.
VIA의 KC_TRNS처럼 "매핑 없는 키는 아래 레이어로 투과"가 되면
Alt+Tab은 그대로 살고, 토글 기능의 존재 이유가 크게 줄어든다.

## 2. 현재 구조 진단 — 도입 가능한가?

**결론: 가능하다. 엔진 쪽 기반은 좋고, 어려운 부분은 훅 어댑터의 "지속 주입"이다.**

유리한 점:

1. **결정 엔진이 순수 상태 머신으로 분리**되어 있다 (`keybind/engine.rs`,
   입력 = (VKey, down/up, tick) → 출력 = `KeyDecision`). 패스쓰루도 새
   Decision 변형 + 상태 추가로 표현 가능하고, 단위 테스트로 타이밍 시나리오를
   전부 시뮬레이션할 수 있다 (기존 테스트 21+개와 같은 방식).
2. **양 플랫폼이 동일 엔진을 공유** (0.8.0에서 macOS 통합 완료) — 로직을 한 번만
   만들면 된다.
3. **훅 콜백은 이미 주입 이벤트를 무시** (`LLKHF_INJECTED` skip) — 우리가 Alt를
   합성 주입해도 피드백 루프가 없다.
4. **훅 액션이 워커 스레드에서 실행** (0.7.0) — 주입 시퀀스가 길어져도
   LowLevelHooksTimeout 위험이 없다.

부족한 점 (개선 필요):

1. **지속(hold) 주입 개념이 없다.** 현재 `SendCombo`는 mod down → key →
   mod up을 한 번에 보내는 일회성이다. Alt+Tab 스위처 UI처럼 "Alt를 계속
   누르고 있는" 의미론을 만들려면 *트리거 down을 주입한 뒤 물리 트리거 up까지
   유지*하는 새로운 액션 형태가 필요하다.
2. **이벤트 순서 제어.** 물리 Tab을 그대로 통과시키고 Alt만 주입하면 큐 순서상
   Tab이 Alt보다 먼저 도달할 수 있다. 물리 키를 억제하고 (Alt down → Tab down)을
   묶어서 주입해야 순서가 보장된다. 이때 물리 Tab의 keyup도 추적해 주입
   Tab up으로 변환해야 한다.
3. **코드(chord) 모드 중 레이어 매핑 충돌.** Alt 주입이 걸린 상태에서 레이어
   매핑 키를 누르면 합성 액션이 Alt+액션으로 오염된다. 정책 결정 필요 (§4 P1).
4. **설정 스키마**에 레이어별 미매핑 키 동작 옵션이 없다.

## 3. 목표 동작 (설계)

```
[Alt down]           → 억제, 레이어 활성 (현행 유지)
[매핑 키 down]        → 레이어 액션 실행 (현행 유지)
[미매핑 키 down]      → "코드 모드" 진입:
                        Alt down 주입 → 해당 키 down 주입 (물리 이벤트는 억제)
                        이후 이 키의 물리 up → 주입 up으로 변환
[코드 모드 중 추가 키] → 그대로 주입 통과 (Alt 유지 중이므로 Alt+연속키 동작,
                        예: Alt 홀드 + Tab Tab Tab)
[Alt up]             → 코드 모드였으면 Alt up 주입 (스위처 확정 등),
                        아니면 현행 tap/hold 판정
[순수 Alt tap]        → 현행 유지 (tap_action or 무시). OS 메뉴 포커스는
                        지금처럼 발생하지 않음 — 회귀 없음
```

핵심 불변식:

- 코드 모드에 들어간 홀드에서는 **tap_action을 발동하지 않는다** (Alt+Tab 후
  Alt를 뗐는데 Escape가 나가면 안 됨) — `layer_key_used`와 같은 방식.
- 패스쓰루는 **트리거가 수정자 키일 때만** 의미가 있다 (Alt/Ctrl/Win/Shift).
  CapsLock 같은 비수정자 트리거는 지금처럼 맨키 통과가 자연스럽다.
- 데몬 종료/키맵 off 시 주입된 트리거가 stuck되지 않아야 한다
  (기존 `held_modifiers()` 해제 경로에 주입 상태 포함).

설정안 (레이어 단위, 기본값은 현행 유지로 하위 호환):

```toml
[[keymap.layers]]
trigger = "LAlt"
unmapped = "passthrough"   # passthrough | plain(현행) | block
```

추후 필요하면 VIA처럼 키 단위 `transparent = ["Tab", "F4"]` /
`blocked = [...]` 목록으로 세분화한다.

## 4. 단계별 전략

### P0 — 설계 확정 + 엔진 테스트 선행 (반나절)
- `KeyDecision`에 `EngageChord { trigger }` / 코드 모드 상태 추가 설계
- 정책 결정 1건: **코드 모드 중 레이어 매핑 키 처리**
  - A안(단순, 권장): 코드 모드 진입 후엔 홀드가 끝날 때까지 전부 OS 조합으로
    통과. "Alt+Tab을 시작했으면 그 홀드는 OS 것"— 멘탈 모델이 명확
  - B안(정교): 매핑 키가 오면 Alt up 주입 → 액션 실행 → Alt down 재주입.
    스위처 UI가 닫히는 등 부작용이 커서 비권장
- 실패 시나리오를 단위 테스트로 먼저 작성 (tap 미발동, 연속 Tab, 이중 진입,
  tick wraparound, keymap 토글 중 코드 모드 등)

#### P0 결정 기록 (2026-07-12 확정)

| 결정 | 내용 |
|---|---|
| 코드 모드 정책 | **A안 채택** — 진입 후 홀드 종료까지 매핑 키 포함 전부 OS 조합으로 통과 |
| Decision 명세 | `EngageChord { trigger, key }` (트리거 down + key down 순서 주입, 물리 억제) / `ReleaseChord { trigger, deferred_action }` (트리거 up 주입 + 지연 Launch 전달) |
| 코드 모드 중 후속 키 | 첫 키만 `EngageChord`로 묶음 주입해 순서 보장. 이후 키는 OS에 트리거가 이미 눌려 있으므로 물리 이벤트 그대로 `PassThrough` (up 포함) |
| 설정 스키마 | 레이어별 `unmapped = "plain"(기본) \| "passthrough" \| "block"` — 기본값이 현행이므로 완전 하위 호환 |
| 패스쓰루 성립 조건 | 트리거가 수정자 키일 때만 코드 진입. 비수정자 트리거는 plain으로 폴백 |
| tap 억제 | 코드 진입 = `layer_key_used` 설정 → tap_action 미발동. Block의 억제된 키 down도 동일 |
| keymap 토글 중 코드 | 토글이 코드 모드를 끊을 때 `ReleaseChord` 반환 — 주입 트리거 stuck 방지 |
| 어댑터 미구현 구간 | P2/P3 전까지 어댑터 스텁은 plain과 동일하게 동작 (EngageChord→통과, ReleaseChord→억제+지연 액션 실행). 설정을 미리 켜도 안전 |

### P1 — 엔진 구현 (1일)
- `EngineState`에 `chord_engaged: bool` + 미매핑 키 up 추적(`chorded_keys`)
- 순수 로직이므로 플랫폼 코드 없이 테스트 완결
- `unmapped` 설정 파싱 (`config.rs` + `06_config_reference.md` 갱신)

### P2 — Windows 어댑터 (1~2일, 실기기 검증 필수) — 구현됨, 실기기 검증 대기
- ✅ 워커 작업 큐를 `WorkerJob`(Action | ChordEngage | ChordRelease)으로 확장
- ✅ Engage: 트리거 down + 키 down을 **한 번의 SendInput 호출**(INPUT 2개)로
  원자 주입 — 사이에 다른 입력이 끼어들 수 없어 순서 보장
- ✅ Release: 물리 트리거 up 억제 → 주입 트리거 up, 지연 Launch는 그 뒤 실행
- ✅ stuck-Alt 방지: keymap 토글(ReleaseChord) + backend stop() 시 잔여
  트리거 up 주입
- 알려진 엣지: 코드 첫 키를 수 ms 내에 떼면 물리 up(통과)이 워커의 주입보다
  먼저 도달할 수 있다 — 키가 논리적으로 눌린 채 남지만 같은 키를 한 번
  누르면 회복된다. 실사용 빈도 낮음, 검증에서 문제 시 up 추적 추가 예정
- 실기기 검증 체크리스트 (unmapped = "passthrough" 설정 후):
  1. Alt 홀드 + Tab 반복 → 스위처가 열린 채 순회, Alt 놓으면 확정
  2. Alt+F4 → 활성 창 닫힘
  3. Alt 짧은 탭 → 여전히 Escape (tap_action 회귀 없음)
  4. Alt 홀드 + H/J/K/L → 코드 진입 **전이면** 방향키 (레이어 매핑 유지)
  5. Alt+Tab 후 같은 홀드에서 H → Alt+H로 OS에 전달 (A안 확인)
  6. 한/영 전환(Shift+Space, RShift 더블탭) 및 한글 조합 간섭 없음
  7. toggle_keymap 콤보를 코드 모드 중 눌러도 Alt가 stuck되지 않음
  8. kmd daemon stop을 코드 모드 중 실행해도 Alt가 stuck되지 않음

### P3 — macOS 어댑터 (1일) — 구현됨, 실기기 검증 대기
- ✅ `send_chord_engage`: 트리거 down → 키 down 주입 (MAGIC_USER_DATA로
  탭 재진입 차단은 기존 인프라 재사용). 주입된 트리거가 OS 수정자 상태에
  유지되므로 이후 통과되는 물리 키가 트리거 조합으로 인식된다
- ✅ ReleaseChord: 물리 트리거 up(flagsChanged) 억제 → 주입 up 대체.
  keyDown 경로(토글이 코드를 끊는 경우)에도 동일 처리
- ✅ stuck 방지: 기존 stop()의 held_modifiers 해제 루프가 코드 트리거를
  자연히 포함한다 (코드 모드 중에는 트리거가 항상 물리 홀드 상태)
- 실기기 검증 체크리스트 (LAlt 트리거 + unmapped = "passthrough"):
  1. Alt 홀드 + 미매핑 글자(T 등) → Option 특수문자(†) 입력 (OS 조합 복원)
  2. Alt 홀드 + H/J/K/L → 코드 진입 전이면 방향키 유지
  3. Alt 짧은 탭 → Escape 유지
  4. 코드 진입 후 같은 홀드의 H → Option+H로 전달 (A안)
  5. 한글 IME 조합 중 간섭 없음 (45ms 지연 노하우 관련 회귀 확인)
  6. 코드 모드 중 daemon stop → Option stuck 없음

### P4 — UX 정리 + 토글 은퇴 결정 (반나절)
- `:keymap` 화면에 레이어별 passthrough 상태 표시
- 기본 프리셋(`dist/config.*`)에 `unmapped = "passthrough"` 적용 여부 결정
- **toggle_keymap은 P4까지 유지** 후 제거가 아닌 *유지* 권장:
  패스쓰루가 Alt 조합 문제는 해소하지만, 토글은 "게임/원격데스크톱 등에서
  키맵 전체를 끄고 싶다"는 별개 용도가 남는다. 문서에서 용도를 재정의하는
  것으로 충분하다.

## 5. 후속 로드맵 — VIA 레이어 개념 완성과 그 너머

패스쓰루(P0~P4) 이후, VIA/QMK 대비 남는 격차를 메우는 순서.
자세한 대응표는 이 문서의 배경이 된 2026-07-12 검토 참조.

### R1 — TG / OSL / TT (저비용 고효율, 패스쓰루 P1과 같은 사이클에 얹기 좋음)
- `TG(n)` 레이어 고정 토글, `OSL(n)` 원샷(다음 1키만), `TT(n)` 홀드=모멘터리·N탭=고정
- 셋 다 엔진 상태 1~2개 추가 수준. 설정: 레이어에 `toggle_key`, `one_shot = true` 등
- 여기까지 하면 "VIA 사용자가 이질감 없이 쓰는 수준"

### R2 — 레이어 스택 + 투과 순서 (개념적 완성)
- 단일 `active_layer: Option<usize>` → 활성 레이어 스택으로 리팩토링
- 키 조회를 스택 상단부터 내려가며 투과(KC_TRNS 의미론의 완전형)
- 중규모 리팩토링이므로 독립 단계로. R1의 TG가 생기면 다층 동시 활성이
  실제로 발생하므로 그 직후가 적기

### R3 — 앱별 레이어 (VIA가 원리적으로 불가능한 차별화 영역)
- 포커스된 앱에 따라 레이어 매핑 전환 (예: VSCode용 Alt 레이어 ≠ 브라우저용)
- 플랫폼별 포커스 추적(Win32 이벤트 훅 / NSWorkspace) 기반 — 독립 트랙
- 런처(kmd)와의 시너지: `:keymap` UI에서 앱별 프로필 관리

### R4 — 홈로우 모드 (가장 마지막)
- 기능이 아니라 타이밍 판정이 본체 — QMK/kanata의 판정 모드
  (hold-on-other-press, permissive hold 등)를 벤치마킹해 설계
- 빠른 타이핑 롤오버 오판정과의 싸움이므로 실사용 튜닝 기간 필요

키별 tapping term, 3탭+ 탭댄스 확장은 필요해질 때 R1~R2에 끼워 넣는다.

## 6. 리스크

| 리스크 | 완화 |
|---|---|
| 주입 Alt가 stuck (크래시/타이밍) | stop/토글/패닉 경로에서 일괄 해제, 워치독 테스트 |
| 이벤트 순서 역전 (Tab이 Alt보다 먼저) | 물리 키 억제 후 묶음 주입으로 순서 보장 |
| 보안 SW의 SendInput 탐지 | 기존에도 SendCombo로 주입 중 — 신규 리스크 아님 |
| IME 조합 간섭 | macOS 45ms 지연 등 기존 노하우 재사용, P2/P3 실기기 검증 |
| Alt 단독 tap 의미 변화 | 없음 — tap/hold 로직 그대로, 코드 모드만 추가 |
