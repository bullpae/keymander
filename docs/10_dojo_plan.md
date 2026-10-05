# kmd dojo — 인터랙티브 키맵 트레이너 구현 계획

> **상태: 미구현 계획** (2026-10-05 재확인 — v0.16.8, `dojo` 관련 코드 0건:
> 서브커맨드·`src/tui/dojo/`·`dojo_runs` 마이그레이션 전부 없음) ·
> 착수 우선순위는 [17_feature_backlog.md](17_feature_backlog.md) F18 —
> 규모가 가장 크고(M1~M4) 마우스 레이어 실기기 검증이 선행 조건이다 ·
> 관련: README "Missions" 섹션, [08_layer_passthrough_plan.md](08_layer_passthrough_plan.md)

## 1. 목표와 컨셉

README의 미션 시나리오는 "무엇을, 왜"를 가르치지만, 근육 기억은 반복과
피드백에서만 생긴다. `kmd dojo`는 vimtutor의 단계적 코스에 타이핑 게임의
즉각 피드백(시간 측정, 점수, 콤보)을 결합한 **TUI 내 인터랙티브 트레이너**다.

- 목표 1 — **습득**: vim-nav / tap-hold / 마우스 레이어를 게임으로 반복해
  일주일 안에 몸에 붙게 한다.
- 목표 2 — **검증**: dojo를 통과했다는 것은 실제 키맵(kanata 또는 데몬)이
  그 기기에서 올바르게 동작한다는 뜻이다. 설치 직후 셀프 진단 도구를 겸한다.
- 목표 3 — **재미로 인한 전파**: 점수·별점·최고기록은 공유하고 싶어지는
  결과물이다 (스크린샷 한 장으로 "나 이거 3성 깼다").

## 2. UX 흐름

```
$ kmd dojo                # 레벨 선택 화면 (진행도·별점·최고기록 표시)
$ kmd dojo --level 3      # 바로 Lv3 시작
TUI/Desktop에서 :dojo     # 터미널로 TUI dojo 실행
```

레벨 선택 → 라운드 시작 전 3초 카운트다운 → 화면에 타겟 제시("문서 아이콘까지
포인터를 옮겨 클릭하세요") → 판정·즉각 피드백(✓ 반응시간 표시 / ✗ 힌트) →
라운드 종료 시 점수·별점·최고기록 갱신.

각 라운드 시작 전에 **왜 이 배치인가** 한 줄을 보여준다 (README 미션 카드의
설계 근거를 그대로 재사용). 외우는 게 아니라 납득하고 습득하는 것이 원칙.

## 3. 레벨 설계

| 레벨 | 이름 | 훈련 대상 | 판정 방식 |
|------|------|-----------|-----------|
| Lv1 | Summon | 런처 검색·실행 (`fire`→Enter, `@gh`, `.pdf`) | TUI 내 모의 검색 (kmd-core 검색 엔진 그대로 사용, 실행은 시뮬레이션) |
| Lv2 | Navigate | LAlt 홀드 + HJKL/N/M/I/O | 매핑 **결과** 키 도착 판정 (Left/PageDown/Home …) |
| Lv3 | Mod-Tap | CapsLock 탭/홀드 (Caps+C/V/S, 탭=CapsLock) | Ctrl+문자 도착 판정 + 탭/홀드 구분 라운드 |
| Lv4 | Pointer | RAlt 홀드 + ESDF 이동, Space 클릭, LShift 정밀 | 터미널 마우스 캡처 — 셀 좌표 타겟 박스 안 클릭 판정 |
| Lv5 | Gauntlet | Lv1~4 혼합 타임어택 | 위 판정 전부, 랜덤 순서 |

레벨별 별점: ★ 완주 / ★★ 제한시간 내 / ★★★ 무실수 + 상위 반응속도.

## 4. 판정 아키텍처 — 핵심 설계 결정

**dojo는 물리 키가 아니라 "매핑 결과"를 관찰한다.** 키맵(kanata/데몬)이 켜진
상태에서 사용자가 `LAlt+H`를 누르면 터미널에는 **Left 화살표가 도착**한다.
crossterm이 `KeyCode::Left`를 받으면 판정 성공 — 이것이 정확히 우리가 원하는
것이다. 물리 입력 후킹이 필요 없고(권한 문제 없음), 동시에 실제 키맵이
동작하는지까지 검증된다.

- **Lv2**: `KeyCode::Left/Right/Up/Down/PageUp/PageDown/Home/End` 도착 판정.
- **Lv3**: `KeyCode::Char('c')` + `KeyModifiers::CONTROL` 등. raw mode에서는
  Ctrl+C가 SIGINT가 아니라 키 이벤트로 도착하고(ISIG off), Ctrl+S도 흐름
  제어에 먹히지 않는다(IXON off) — crossterm raw mode가 이미 보장.
  단, **dojo 화면에서는 종료 키를 `Ctrl+C` → `Esc` 2회(또는 `q`)로 변경**해야
  Lv3 판정과 충돌하지 않는다. CapsLock "탭" 판정은 터미널로 CapsLock 자체가
  전달되지 않으므로, 탭 라운드는 "Caps 탭 후 `a`를 눌러 대문자 `A`가
  입력되는지"로 우회 판정한다.
- **Lv4**: crossterm `EnableMouseCapture` → `MouseEvent{kind: Down(Left),
  column, row}`. 타겟 박스(예: 6×3 셀)를 랜덤 위치에 렌더하고 좌표 포함 여부
  판정. 드래그 라운드는 `Down → Drag → Up` 시퀀스의 시작/끝 좌표로 판정.
  포인터가 터미널 창 위에 있어야 이벤트가 오므로, 라운드 시작 시 "포인터를
  창 안 시작 지점으로" 안내 타겟을 먼저 띄운다.

### 정직 시스템의 한계와 후속 강화

터미널 판정으로는 사용자가 진짜 화살표 키를 눌러도 통과된다. v1은 이를
**정직 시스템**으로 수용한다(연습 도구이지 시험이 아니다). 후속(v2)으로
데몬이 떠 있을 때 물리 키 이벤트 스트림(IPC)을 구독해 "정말 LAlt+H였는지"
엄격 판정하는 **hard mode**를 붙일 수 있다 — 데몬 keybind 엔진에 이벤트
브로드캐스트 채널만 추가하면 된다.

## 5. 사전 조건 감지

dojo 진입 시:

1. `kmd keymap status` / 데몬 상태를 조회해 키맵 활성 여부 확인.
   비활성이면 Lv2~4 잠금 + "지금 켜기" 안내 (`kmd keymap start` 실행 제안).
2. IME 상태 안내: 한글 입력 모드면 ESDF가 ㄷㄴㅇㄹ로 들어온다.
   Lv1 판정에서 한글 자모가 감지되면 "영문 모드로 전환하세요" 힌트를 즉시
   표시 (기존 `reset_ime_on_launch` 로직 재사용 검토).
3. 터미널 capability: 마우스 캡처 불가 터미널(일부 원격/멀티플렉서 환경)이면
   Lv4를 잠그고 사유 표시.

## 6. 점수·진행 저장

kmd-core SQLite에 테이블 추가 (DB 마이그레이션 1건):

```sql
CREATE TABLE dojo_runs (
  id INTEGER PRIMARY KEY,
  level INTEGER NOT NULL,
  score INTEGER NOT NULL,        -- 반응시간·정확도 합산
  stars INTEGER NOT NULL,        -- 0~3
  max_combo INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL,
  mistakes INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
```

- 레벨 선택 화면에 레벨별 최고기록·별점·최근 7일 스트릭 표시.
- README의 "3일 연속 승급 규칙"을 스트릭 UI로 구현 (마우스 0회는 측정
  불가하므로 dojo 완주 스트릭으로 대체).

## 7. 구현 구조

- **메인 crate (kmd) — `src/tui/dojo/`**: 레벨 선택·라운드 화면·판정 로직
  전부 TUI 모듈. 기존 ratatui 앱 상태 머신에 `Screen::Dojo` 분기 추가.
  - `mod.rs` — 레벨 정의·화면 전환
  - `rounds.rs` — 라운드 생성기 (타겟 랜덤화, 시퀀스)
  - `judge.rs` — 키/마우스 이벤트 → 판정 (순수 함수, 단위 테스트 대상)
  - `score.rs` — 점수·별점·콤보 계산 (순수 함수)
- **kmd-core**: `dojo_runs` 마이그레이션 + 기록 조회/저장 API.
- **CLI**: `kmd dojo [--level N]` 서브커맨드.
- **TUI/Desktop `:dojo`**: TUI는 화면 전환, Desktop은 터미널로 `kmd dojo`
  실행 (Desktop 안에 게임을 넣지 않는다 — 창 하나, 코드 한 벌).

판정·점수 로직을 순수 함수로 분리해 이벤트 시퀀스 → 판정 결과를 엔진
테스트처럼 단위 테스트한다 (keybind engine 테스트 스타일 재사용).

## 8. 마일스톤

| 단계 | 내용 | 규모(추정) |
|------|------|------|
| M1 | dojo 골격: 서브커맨드, 레벨 선택 화면, Lv1(런처 모의 검색), 점수 저장 | 중 — 신규 UI지만 검색은 kmd-core 재사용 |
| M2 | Lv2(vim-nav) + Lv3(tap-hold): 결과 키 판정, 종료 키 변경, 사전 조건 감지 | 소~중 — 판정은 crossterm 이벤트 매칭 |
| M3 | Lv4(마우스): 마우스 캡처, 타겟 박스, 드래그 라운드, capability 감지 | 중 |
| M4 | Lv5(종합) + 별점·스트릭·기록 화면 다듬기, README 미션 → dojo 연결 | 소 |
| v2(후속) | hard mode: 데몬 물리 키 이벤트 IPC 구독 엄격 판정 | 중 — 데몬에 이벤트 채널 추가 |

M1~M2까지만 나가도 "설치 → 미션 → dojo 연습"의 습득 루프가 완성된다.
M3(마우스)는 마우스 레이어 실기기 검증이 끝난 뒤 진행하는 것이 안전하다.

## 9. 리스크·오픈 퀘스천

- **터미널 편차**: Windows Terminal / iTerm2 / alacritty의 마우스 캡처·수정자
  키 전달이 다를 수 있다 → M2에서 3터미널 스모크 테스트를 마일스톤 완료
  조건에 포함.
- **CapsLock 탭 판정 우회**(대문자 `A` 확인)는 Shift로도 통과 가능 — 정직
  시스템 원칙상 수용, 화면에 "Caps 탭으로" 명시.
- **kanata cmd 레이어와의 충돌**: vim-nav의 `Alt+Space`(kmd-desktop 실행)가
  dojo 도중 눌리면 창이 뜬다 — 라운드 타겟에서 Alt+Space는 제외.
- **점수 인플레이션 방지**: 반응시간 기준값은 M1에서 자체 플레이 데이터로
  보정한 뒤 상수로 고정.
