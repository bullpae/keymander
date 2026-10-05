# 기능 백로그 — 계획·누락 전수 조사

> **상태: 작업 목록** — 2026-10-05, v0.16.8 기준으로 `docs/*.md`에 적힌 모든
> "향후·예정·보류·미구현·P2~P4" 항목을 뽑아 **코드와 대조**한 결과다.
> `docs/11_refactor_backlog.md`가 "코드 모양"을 다루는 문서라면, 이 문서는
> **"약속했는데 없는 기능"**을 다룬다.
>
> 조사에서 가장 중요한 발견은 미구현 목록이 아니라 **문서가 거짓말을 하고 있던
> 6건**이다(§4). 계획서가 낡으면 다음 사람이 있는 걸 또 만들거나, 없는 걸
> 있다고 믿는다. 후자가 더 위험하다 — 실제로 `docs/03`의 플러그인 스펙은
> 배선이 0건인데 정본처럼 읽힌다.

## 1. 분류 기준

| 분류 | 뜻 |
|---|---|
| ⬜ 미구현 | 문서에 있고 코드에 없다 |
| 🔶 부분구현 | 일부 플랫폼/일부 UI만, 또는 코드는 있고 검증만 남음 |
| ✅ 이미완료 | 코드에 있는데 문서가 "향후"로 남아 있다 → §4 |
| 🧊 의도적보류 | 사유가 여전히 유효하다 → §3. **다시 꺼내지 말 것** |

## 2. 우선순위 — 사용자 체감 순

착수 순서다. 위에서부터 한다.

### 2.1 키맵 — 매일 손에 닿는 것

| # | 항목 | 출처 | 상태 | 왜 지금인가 |
|---|---|---|---|---|
| F1 | **레이어 로컬 Shift** — `BindAction`에 수정자 홀드 추가 | `docs/16:399` | ⬜ (**채택 결정됨**) | 백로그에서 유일하게 "채택"까지 끝난 항목인데 미착수다. 현재 `BindAction`은 SendKey/SendCombo/Macro/Launch/Mouse/ClipPaste뿐 — 레이어 안에서 Shift를 눌러 둘 방법이 없어 CapsLock↔LShift 충돌을 매일 겪는다 |
| F2 | **`.`/`/` 홀드 게이트 (~120ms)** | `docs/16:395` | ⬜ | Backspace/Delete는 연타 1위 키인데 `docs/16`의 오발사 지도에 🔴로 남은 두 칸이 그대로다. 게이트는 엔진에 타이머 하나 |
| F3 | **기본 프리셋 `unmapped = "passthrough"` 적용 결정** | `docs/08:160` | ⬜ (결정 미확정) | 엔진 코드는 이미 있고 `dist/config.keymap.{macos,windows}.toml`에서 **주석만 벗기면** 되는데 2개월 넘게 멈춰 있다. Alt 조합이 레이어에 먹히는 문제가 이것 하나로 해소된다 |
| F4 | **R1 — TG(n)/OSL(n)/TT(n) 레이어 액션** | `docs/08:171` | ⬜ | VIA/QMK를 쓰던 사람이 제일 먼저 찾는 것. 엔진 상태 1~2개 추가로 끝나 비용/효용비가 이 목록에서 가장 좋다 |
| F5 | **`:keymap` 화면에 레이어별 passthrough 상태 표시** | `docs/08:159` | ⬜ | 설정을 켰는지 **UI로 확인할 방법이 없다**(로그뿐). F3을 내보내면 바로 필요해진다 |
| F6 | **R2 — 레이어 스택 + KC_TRNS 투과** | `docs/08:177` | ⬜ | 체감은 F4보다 늦게 오지만, F4(TG) 도입 즉시 다층 동시 활성이 **실제로 발생**하므로 F4 직후가 적기다 |
| F7 | kanata 프리셋 **실행 검증** (CI config 검사 모드) | `docs/16:398` | ⬜ | 문법 오류가 사용자 머신에서야 드러나는 유일한 미검증 산출물. `.github/workflows/`에 `kanata` 0건 |
| F8 | HHKB `tap=Ctrl / hold=layer` | `docs/13:95` | ⬜ | `keybind/mod.rs:524`의 `tap_action`이 단일 키만 받아 수정자 tap-hold가 안 된다. F1과 같은 자리를 건드리므로 묶으면 싸다 |

### 2.2 인프라 — 다른 작업을 막고 있는 것

| # | 항목 | 출처 | 상태 | 왜 지금인가 |
|---|---|---|---|---|
| F9 | **`KMD_CONFIG_DIR` / `KMD_DATA_DIR`** | `docs/06:419` | ⬜ | 문서에 **공개된 환경변수가 동작하지 않는다**. 게다가 Windows E2E 편입(`docs/14:43`)이 여기에 막혀 Tier 1 E2E가 여전히 unix 전용이다. 하나 고치면 둘이 풀린다 |
| F10 | export/import (설정+DB 아카이브) | `docs/01:327` | ⬜ | 기기 이전·재설치 때 config·히스토리·인덱스를 손으로 옮겨야 한다. F9의 디렉터리 추상화가 선행되면 훨씬 싸다 |
| F11 | Tier 2를 Windows CI에 올리기 | `docs/14:77` | ⬜ | F9 의존 |

### 2.3 기능 확장

| # | 항목 | 출처 | 상태 | 비고 |
|---|---|---|---|---|
| F12 | **클립보드 v2 — 출처 앱 표시** | `docs/12:156` | ⬜ | 히스토리 UI를 이미 쓰는데 "이 텍스트가 어디서 왔는지"를 못 보여준다. v2 묶음 중 이것만 떼서 먼저 해도 된다 |
| F13 | 클립보드 v2 — 레지스터(a–z), 이미지 | `docs/12:156` | ⬜ | |
| F14 | `ExtensionAction::PasteToPrevious` / `Paste`·`Inject` | `docs/12:131`, `docs/11` R3-4 | ⬜ | `plugin/mod.rs:32-41`은 Display/CopyToClipboard/OpenUrl/Noop 4개뿐. 리팩토링 R3-4와 **같은 작업** |
| F15 | 테마 — TUI가 `general.theme`를 읽게 하기 | `docs/05` | ⬜ | §4-10 참조. 설정 항목은 있는데 **TUI가 읽지 않는다** — 사용자가 바꿔도 아무 일이 없다. 이건 기능 누락이 아니라 **거짓 설정**이라 F-급으로 올릴 가치가 있다 |
| F16 | TOML 외부 테마 파일 + 테마 핫 리로드 | `docs/05:87`, `:157`, `docs/01:326` | ⬜ | F15가 선행 |
| F17 | 터미널 색상 지원 감지 (256/16색 다운그레이드) | `docs/05:140` | ⬜ | 현재 Rgb 직접 사용 — 저색상 터미널에서 깨진다 |
| F18 | `kmd dojo` 트레이너 (M1~M4) | `docs/10` 전체 | ⬜ | 체감은 크지만 규모가 이 목록에서 가장 크고(M1~M4), 마우스 레이어 실기기 검증이 선행 조건. 한 릴리스를 통째로 써야 한다 |
| F19 | bookmarks UI (별표/핀) | `docs/04:84` | ⬜ | `db.rs:210,221`에 API만 있고 **호출처 0건** — 死코드 상태 |
| F20 | `kmd plugin install/remove/update` | `docs/03:190` | ⬜ | §3의 Script Plugin 배선 결정에 종속 |
| F21 | nav `V` = `;` 프리필 런처 실행 | `docs/12:114` | ⬜ | "(선택)" 항목. `keymap.rs:1363`은 V 부재를 **검사하는 가드**라 넣으려면 가드부터 고쳐야 한다 |
| F22 | docx(quick-xml)·pdf 본문 추출 | `docs/15:107` | ⬜ | 순수 Rust 크레이트 한정. 사이드카 금지 원칙 유지 |

### 2.4 검증만 남은 것 (코드 완료)

사용자 실기기 확인 대기. 새로 만들 것은 없다.

| 항목 | 출처 |
|---|---|
| P2/P3 레이어 어댑터 실기기 검증 | `docs/08:122,142` |
| 클립보드 P3 GUI 상호작용 확인 | `docs/12:153` |
| R1-1 Windows 훅 자동복구 실사용 검증 | `docs/11` §1 |
| R1-3 진짜 콜드 스타트 실측 | `docs/11` §1 |

## 3. 의도적 보류 — 다시 꺼내지 말 것

사유가 **여전히 유효함을 2026-10-05에 재확인**했다. 꺼내려면 사유를 먼저 깨야 한다.

| 항목 | 보류 사유 (재확인) |
|---|---|
| `deferred-ideas.md` 7건 전부 — Intent Router, Flow Command, Plugin Marketplace, AI Summarize, Snippet Manager, Window Manager, Bookmark | "핵심 영역 이탈" 또는 "기존 기능으로 충족". 전부 유효 |
| **Script Plugin 전체 스펙** (`docs/03:75-183`) | 사용자 기반이 없는데 프로세스 격리·타임아웃 유지비만 남는다. **단 §4-9의 문서 수정은 필수** — 지금은 동작하는 기능처럼 읽힌다 |
| 플러그인 중앙 레지스트리/마켓플레이스 | 위에 종속 |
| Nerd Font 글리프 (`docs/05:205`) | 폰트 설치 요구가 **무설치 원칙과 상충** |
| Tier 3 GUI 픽셀 자동화 (`docs/14:80`) | 유지비 > 효용 |
| Lindera 형태소 색인 (`docs/15:27`, `docs/06:145`) | 사전 ~23MB. **한국어 재현율 실측 전에 손대면 안 된다** |
| ripgrep 실시간 grep (`docs/15:108`) | 기각됨 — 대형 트리 지연·랭킹 부재 |
| AUR / COPR / homebrew-core (`docs/07:259`) | 별 75+ 등 외부 요건 의존 |
| `N`/`M` ↔ 단어이동 자리 교환 (`docs/16:397`) | 재학습 비용 > 이득 |
| 키 단위 Shift 정책 `shift_mappings` (`docs/16:400`) | **조건부** — LAlt 트리거 복귀를 검토할 때만 필요 |
| 트리거 결정 CapsLock vs LAlt (`docs/16:12`) | 실사용 검증 대기. CapsLock 기본 유지 |
| R4 홈로우 모드 (`docs/08:190`) | 타이밍 판정 튜닝이 본체 — R1~R3 이후 |
| 코드 첫 키 빠른 릴리스 up 추적 (`docs/08:131`) | 빈도 낮고 자가 회복됨 |
| macOS/Linux LLM 오토파일럿 (`docs/09:19`) | URL/클립보드 폴백이 동작하고, 플랫폼 창 API 격차가 본질 |
| dojo v2 hard mode (`docs/10:133`) | v1조차 없다 |
| `ipc::Response::status_lines` presenter 분리 | 새 소비자(JSON/영문 출력)가 생길 때 |

## 4. 문서가 거짓말을 하고 있던 것 — 즉시 수정

코드 작업이 아니라 **문서 수정**이다. 그래서 가장 싸고, 방치 비용은 가장 크다.

| # | 어디 | 무엇이 틀렸나 | 실제 |
|---|---|---|---|
| D1 | `docs/06_config_reference.md:303` | 클립보드 `history_enabled` 기본 off 사유를 "Concealed 마크 제외가 **아직 없어**(추후 지원)"로 적었다 | **구현돼 있다** — `clipboard.rs:544`(macOS `ConcealedType`), `:635`(Windows `ExcludeClipboardContentFromMonitorProcessing`), `:198,219` 감시 루프 적용. Linux만 stub(`:821`). **off 유지 사유 자체가 틀렸으니 기본값을 재검토해야 한다** |
| D2 | `docs/07_distribution.md:18,75,90` · `docs/11:65` · `README.md:249` | winget 🔶 "모더레이터 승인 대기" / "Pending initial registration" | **머지 완료** — `CHANGELOG.md:118` (v0.16.2, 2026-09-10) |
| D3 | `docs/01_prd.md:317` | "향후: RegisterHotKey / CGEventTap / XGrabKey" | 구현됨 — `keybind/macos.rs`·`windows.rs` + `server.rs:859` 콤보 핫키 등록 |
| D4 | `docs/01_prd.md:291,329` + 간트 | FTS 본문 검색을 Phase 6 Future로 | v0.15.0 완료 — `content_index.rs` |
| D5 | `docs/04_database_schema.md:122,131-133` | "Migration 002: (향후 예정)", 버전 이력 1까지 | `db.rs:313,325,350` — **user_version 3**까지 구현(content_files 등) |
| D6 | `docs/14_testing_plan.md:15-16` | 현황표의 "키 주입 진짜 E2E = ❌" | **같은 문서 §Tier 2**가 "✅ 구현됨(2026-08-11)" — 문서 내부 모순 |
| D7 | `docs/README.md:40-44` | `12_clipboard_plan`·`14_testing_plan`을 "계획"으로 분류 | 12는 P1~P3 완료, 14는 Tier 1·2 완료 → **"설계 이력"으로 이동** |
| D8 | `docs/10_dojo_plan.md:3` | "2026-08-08 확인 — 현재 v0.12.0" | 2026-10-05 / v0.16.8 재확인 (상태는 여전히 미구현) |
| D9 | `docs/03_plugin_spec.md:75` | §3 Script Plugin이 **정본 스펙처럼** 기술됨 | `plugin/protocol.rs`에 타입만 정의, **`protocol::` 참조 0건**. `loader.rs`는 `discover_plugins`/`default_plugin_dir` 2개 함수뿐. → "스펙만 존재, 미배선" 배너 필수 |
| D10 | `docs/05_theming.md:128-138` + §3 전체 | "예정 테마 7종(catppuccin×2/nord/tokyo-night/gruvbox/solarized)" | 실제 내장은 **데스크톱 5종**(`kmd-desktop/src/theme.rs:66-166` midnight/obsidian/snow/rose_pine/nord). 문서 목록과 전혀 다르다. 또한 §3이 "TUI 테마"를 전제하는데 **TUI는 `general.theme`를 읽지 않는다**(F15) |
| D11 | `docs/01_prd.md:328` | 공식 플러그인 kmd-todo/memo/clipboard | clipboard만 **네이티브로** 구현(`;`/`:clip`), 플러그인 형태 아님. todo/memo는 0건 |

## 5. 다음 릴리스 묶음 제안

`main`에 들어갔으나 아직 릴리스되지 않은 것 + 위 목록의 저비용 항목.

- (커밋됨) 폴더 제안 신호 ② — 실행 이력 기반 (`6a59437`)
- **D1~D11 문서 수정** — 전부 합쳐 한 커밋. 코드 위험 0
- **F3** 기본 프리셋 passthrough — 주석 해제 + 테스트
- **F9** `KMD_CONFIG_DIR`/`KMD_DATA_DIR` — F10·F11을 푸는 선행 작업

F1·F2·F4는 엔진을 건드리므로 **실기기 검증이 붙는 별도 릴리스**가 맞다.
