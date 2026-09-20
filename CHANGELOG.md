# Changelog

All notable changes to keymander are documented here.

## [Unreleased]

## [0.16.6] — 2026-09-21

### Fixed
- **설정을 저장할 때 다른 곳에서 바꾼 값이 되돌아가던 문제.** 설정 창을 열어 둔
  채 다른 창이나 명령줄에서 설정을 바꾸면, 나중에 저장할 때 그 변경이 예전
  값으로 덮여 사라졌다. 이제 저장 직전에 파일을 다시 읽어 **내가 바꾼 항목만**
  반영한다.
- **빠른 명령이 실패해도 창이 닫히던 문제** (데스크톱). 실행이나 복사가
  실패하면 창을 유지하고 이유를 알린다.
- 검색 폴더 추가가 저장에 실패해도 추가된 것처럼 보이던 문제.
- TUI가 화면 전환 직전에 오류를 만나면 터미널이 망가진 채 남던 경우.

### Changed
- **릴리스가 테스트 통과를 요구한다.** 이전에는 테스트가 실패해도 배포가
  진행될 수 있었다. 이제 태그가 가리키는 커밋에서 Windows·macOS·Linux 테스트가
  모두 통과해야 빌드·게시가 시작된다.

## [0.16.5] — 2026-09-20

### Fixed
- **실패한 명령이 성공처럼 보이던 문제.** 빠른 명령(Quick Action)이 실패해도
  결과가 비어 있으면 "(no output)"으로 표시됐다. 이제 종료 코드와 오류 메시지를
  그대로 알린다.
- **복사가 안 됐는데 "Copied"라고 표시되던 문제** (TUI 계산기·이모지). 클립보드를
  쓸 수 없는 환경에서 복사된 줄 알고 붙여넣다 낭패를 보는 상황을 막는다.
- **데스크톱에서 실행이 실패해도 창이 닫히던 문제.** 터미널 실행이나 복사가
  실패하면 창을 유지하고 알린다.
- **설정 저장이 실패해도 저장된 것처럼 보이던 문제** (TUI). 저장에 성공한
  뒤에만 설정을 반영하고, 실패하면 편집 내용을 그대로 둔다.
- **여러 창에서 동시에 설정을 저장할 때 설정 파일이 섞일 수 있던 문제.**
  임시 파일 이름을 프로세스·스레드별로 분리했다.
- **TUI가 오류로 종료되면 터미널이 망가진 채 남던 문제** (입력이 안 보이거나
  프롬프트가 사라짐). 어떤 경로로 끝나도 터미널을 원래대로 되돌린다.
- 인덱스 재생성이 동시에 두 번 돌 수 있던 문제 — 같은 폴더를 중복 스캔하고
  캐시 파일을 두고 경쟁했다.

### Changed
- **빌드 재현성** — `Cargo.lock`을 저장소에 포함하고 빌드에 `--locked`를 적용해,
  같은 버전 태그가 항상 같은 의존성으로 빌드되게 했다.

## [0.16.4] — 2026-09-13

### Fixed
- **`:t`(맞춤법·번역)가 타이핑 도중 브라우저를 열던 문제.** 검색 단계에서 URL을
  열고 있어 `:t spell 안녕`을 입력하면 글자마다 탭이 열렸다. 이제 실행 항목을
  하나 보여주고 **Enter를 눌렀을 때만** 연다 — 무엇을 몇 개 서비스로 보낼지
  확인하고 실행할 수 있다.
- **데스크톱 런처에서 이모지를 골라도 복사되지 않던 문제.** Enter를 누르면
  창만 닫히고 클립보드는 그대로였다.
- `:prompt add`/`remove`가 저장에 실패해도 "저장됨"이라고 표시하던 문제.

### Added
- **주소를 입력하면 열 수 있는 항목이 뜬다** (데스크톱) — `example.com`처럼
  URL을 치면 맨 위에 열기 항목이 서고, 파일 검색 결과는 그대로 아래에 남는다.
- **런처를 열면 최근 실행한 항목이 보인다** (데스크톱) — 빈 화면 대신 바로
  고를 수 있는 목록이 뜬다.
- 데스크톱 상태 알림 — 설정 저장 실패 같은 알림을 상태바에 표시한다(다음
  검색에서 사라짐). 이전에는 실패해도 로그에만 남아 사용자가 알 수 없었다.

## [0.16.3] — 2026-09-13

### Added
- **본문 검색 연산자** (`?` 런처 / `kmd grep`) — `"정확한 구문"`, `-제외어`,
  `ext:md`, `path:docs`. 본문 조건은 FTS5 MATCH로, 파일 필터는 경로 LIKE로
  나뉜다. 필터만 있는 질의(`ext:rs`)는 전체를 훑는 대신 빈 결과를 준다.

### Changed
- **데스크톱 설정 핸들러 분해** — `:set` 목록 생성(246줄)과 액션 처리(274줄)를
  각각 23줄·106줄로. LLM·멀티웹·맞춤법·번역 4군이 라벨만 다르고 동작이 같아
  한 표로 합쳤다(중복 약 190줄 제거). "프로바이더를 전부 끄면 기본값 복구"
  규칙도 이제 한 곳이다.
- **TUI 설정 반영 경로 단일화** — config 파생 필드 12개를 저장 시 손으로
  재대입하던 것을 한 함수로. 새 설정을 추가할 때 반영을 빠뜨릴 자리가 없어졌다.
- **인덱스 캐시 절차 통합** — CLI·데스크톱이 각자 적던 "캐시 로드 → 없으면
  빌드 → 저장"을 코어 한 곳으로.

### Fixed
- `scripts/code-metrics.sh`가 긴 함수의 소속 파일을 잘못 보고하고 길이를 파일
  경계 너머로 누적하던 버그. 측정 도구가 틀리면 다음 판단도 틀어진다.

## [0.16.2] — 2026-09-10

### Changed
- **저장소 개명 `keymander-cli` → `keymander`.** winget 최초 등록이 머지되어
  진행. git 리모트·릴리스 URL·winget InstallerUrl은 GitHub 리다이렉트로 계속
  동작하지만, **GitHub Pages는 리다이렉트되지 않으므로 apt/yum 사용자는 저장소
  주소를 직접 갱신**해야 한다:
  ```bash
  # apt
  sudo sed -i 's#/keymander-cli/#/keymander/#' /etc/apt/sources.list.d/keymander.list
  sudo apt update
  # yum/dnf
  sudo curl -fsSL -o /etc/yum.repos.d/keymander.repo https://bullpae.github.io/keymander/keymander.repo
  ```

### Fixed
- **한/영(Shift+Space) 전환이 Shift를 떼야만 되던 문제** (macOS) — 전환은 네이티브
  단축키 Ctrl+Space 합성 주입으로 처리하는데, 물리 Shift가 눌린 채 주입하면
  OS가 Ctrl+Shift+Space(다른 단축키)로 해석해 전환이 안 됐다. 기존 코드는 Shift가
  물리적으로 떨어질 때까지 최대 500ms 대기해 이를 피했으나, 그 탓에 "Shift를
  떼야만 전환"되고 오래 쥐면 타임아웃 후 실패하는 부작용이 있었다. 이제 대기 대신
  주입 직전 flagsChanged로 Shift 플래그만 지운다 — Space를 누르는 즉시 전환된다.

## [0.16.1] — 2026-08-30

### Changed
- **앱 아이콘 전면 교체 — "키캡 베이더"** — 헬멧=정면 키캡(사다리꼴 윗면 +
  상·하 모서리 연결선), 얼굴=키보드 아이콘 어휘(기울어진 눈 키 2개·삼각 벤트 키
  코·키+스페이스바 그릴 입)인 오리지널 캐릭터. keyboard + commander라는 이름의
  기획 의도를 도상으로 옮겼다. `assets/icon.svg`(구 버전은 » 글리프가 U+FFFD로
  손상돼 있었다)·`icon.png`(1024px)·`icon.ico`(256/48/32/16) 일괄 재생성.
  눈 색은 다크 배경 `#56D2FF`, 라이트 배경 단독 글리프는 `#0A7FB5` 규정.

## [0.16.0] — 2026-08-28

### Added
- **소스 인코딩 검사 (R2-4)** — `scripts/check-encoding.sh` + CI 잡.
  UTF-8 유효성(CP949 혼입)과 BOM 규칙을 본다. **`.ps1`은 BOM이 필수**이고
  (PS 5.1이 없으면 ANSI로 읽어 한글을 깨뜨린다) 나머지는 금지라는 점이 핵심이다.
  도입하자마자 `scripts/assemble-config.ps1`에 BOM이 빠져 있던 것을 잡아 고쳤다.
  검사 대상이 0개면 통과가 아니라 실패로 본다 — 조용한 통과가 가장 위험하다.
- `scripts/code-metrics.sh` — 파일 규모·긴 함수·테스트 수 스냅샷.

### Changed
- **설정 키를 단일 레지스트리로 통합 (R2-5)** — `config_registry!` 매크로가
  **한 줄 선언에서 `get_value`/`set_value` 양쪽을 함께 생성**한다. 단순 키
  27개(숫자·불리언·문자열)에서 get/set 비대칭이 **원천적으로 불가능**해졌고,
  두 match에 각각 손으로 적혀 있던 중복 arm 54개가 사라졌다. 리스트 정규화나
  읽기 전용 의사 키(`_data_path` 등) 16개는 여전히 명시적 arm이다.
  같은 키가 레지스트리와 명시적 arm 양쪽에 있으면(뒤엣것이 죽은 코드가 된다)
  테스트가 잡는다.
- **TUI가 키 입력마다 config.toml을 읽던 것을 캐시로 (R2-7)** —
  `:keys`/`:keymap`/`:prompt`/`?` 핸들러가 매 키 입력마다 `load_config()`로
  디스크를 읽고 있었다(데스크톱은 캐시된 `runtime_config`를 쓴다).
  `AppState.config` 캐시를 두고, 설정을 바꾸는 경로(설정 모달 저장, 폴더 제안
  추가, 키맵 액션, 템플릿 add/remove)는 캐시를 직접 고쳐 어긋나지 않게 했다.
  `load_config()` 호출 **11곳 → 1곳**(시작 시 1회).

- **`process_key` 451줄 단일 함수를 진입점 43줄 + 12단계로 분해** — 섹션
  우선순위가 곧 이 엔진의 의미라, 분해 **전에** 40스텝 시나리오의 결정
  시퀀스 전체를 채취해 `process_key_동작_스냅샷` characterization 테스트로
  박아 두고 작업했다. 각 스텝은 `Option<KeyDecision>`을 반환하고 `None`이면
  다음으로 넘어가, 진입점만 읽으면 우선순위가 보인다. 동작 변경 없음.
- **프리픽스 파싱을 `kmd_core::query_prefix`로 통합** — 데스크톱과 TUI가 각자
  벗겨내던 별칭(`:emoji`/`:e`, `!`/`>`, `:keymap`/`:km`)을 한 곳에서 관리한다.
  나머지 핸들러는 실제로 갈라진 동작(오류 표시 방식·설정 출처·점수 부여)이라
  의도적으로 통합하지 않았다.
- 테스트 확충 — `src/` 7개 → 18개(프리픽스 디스패치 테이블, 선택 상태 전이,
  드릴다운 종료), core에 프리픽스 파싱 테스트 5개, 엔진에 동작 스냅샷 1개.

- `docs/11_refactor_backlog.md` 갱신 — 낡은 수치를 **적지 않기로** 하고
  `scripts/code-metrics.sh`를 추가했다(손으로 적은 줄 수는 며칠 만에 낡는다).
  완료·부분완료 항목을 반영하고, 이번에 발견한 R2-5(설정 키 4중 열거)·
  R2-6(라우팅 이중화)·R2-7(TUI가 검색마다 config를 디스크에서 읽음)을 추가.
  착수 순서를 현재 상태에 맞게 다시 매기고 보류 항목의 이유를 남겼다.

- 설정 항목 왕복 테스트를 강화 — 위젯 종류에 맞는 **다른 값**을 넣고 되읽어
  실제 반영을 확인하고, 숫자·불리언 위젯은 잘못된 값을 거부하는지도 본다.
  기존 테스트는 키 존재만 보장했다.
- `kmd-core`에 `toml_edit`를 명시 의존성으로 추가 (`toml`의 전이 의존이었으나
  직접 사용하게 됨).

- `daemon status` 출력 서식을 `kmd_core::ipc::Response::status_lines` 한 곳으로
  통합 — `kmd daemon status`와 `kmd-daemon status`가 각자 30줄을 destructure해
  출력하고 있었다. `hook_health` 추가 때 한쪽만 고쳐 컴파일 에러가 났던 지점이고,
  다음엔 **한쪽에만 표시되는 조용한 불일치**가 될 수 있었다.
- clippy 경고 2건 정리 (`needless_return`, `unnecessary_cast`) — 경고 0건 복귀.

### Fixed
- **잘못된 설정값이 "저장됨"으로 보이면서 조용히 버려지던 문제** —
  `Config::set_value`의 파싱 헬퍼가 실패 시 기존 값을 그대로 두고 `Ok(())`를
  반환했다. 그래서 `kmd config set general.render_fps abc`가 **성공 메시지와
  종료코드 0**을 내면서 값은 그대로였고, TUI 설정 화면은 결과를 `let _ =`로
  버린 뒤 무조건 "변경됨(dirty)"으로 표시했다. 이제 `ConfigError::InvalidValue`로
  전파하고, TUI는 실패 시 dirty를 올리지 않으며 도움말 줄에 사유를 표시한다.
  (Codex 교차 리뷰 2026-08-27에서 발견 — 직전 커밋의 설정 키 왕복 테스트는
  "현재 값 되쓰기"라 항상 파싱에 성공해 이 결함을 통과시켰다.)
- **`kmd config set`이 config.toml의 주석을 전부 날리고 저장이 비원자적이던 문제** —
  `Config::save`가 구조체를 통째로 직렬화해 `fs::write`로 덮어썼다. 손으로 단
  주석이 한 번에 사라졌고, 저장 도중 프로세스가 죽거나 디스크가 차면 **반쪽 파일**이
  남을 수 있었다. `toml_edit`로 기존 문서의 값만 갈아끼워 주석·키 순서를 보존하고,
  임시 파일 → flush/sync → rename 으로 원자적으로 교체한다. 기존 파일이 깨져
  파싱되지 않으면 병합을 포기하고 새로 쓴다(저장 실패보다 낫다).

- **TUI 설정의 "Everything path"가 아무것도 저장하지 않던 버그** — 설정 화면은
  `launcher.everything_path` 키를 노출하는데 `Config::get_value`/`set_value`
  양쪽 모두 해당 분기가 없었다. 읽기는 항상 `None`이라 **값이 있어도 화면은
  빈칸**이었고, 쓰기는 `UnknownKey` 오류가 나는데 호출부가 `let _ =`로 삼켜
  **UI상으로는 저장된 것처럼 보이면서 실제로는 버려졌다.** Everything이 Windows
  전용이라 발견이 늦었다.
  - 재발 방지: 모든 `SettingItem.key`가 `get_value`/`set_value`를 왕복하는지
    검사하는 테스트 추가 (`src/tui/settings/items.rs`). 설정 키는 네 곳
    (구조체 · get · set · 설정 화면)에 손으로 열거되는데 이를 잇는 컴파일 타임
    장치가 없어, 같은 종류의 누락이 언제든 재발할 수 있었다.
- **kanata `.kbd` 프리셋이 엔진 기본값과 두 세대 어긋나 있던 문제** —
  `kmd keymap init vim-nav`가 설치하는 프리셋에 트리거가 아직 `alt`였고,
  2026-08-12에 **레이어 불변식으로 제거한** `y`(줄 복사) `p`(붙여넣기)
  `u`(줄 삭제) `,`(단어 삭제)가 그대로 남아 있었다 — 프리셋을 쓰면 금지한
  오발사 위험이 되살아났다. 마우스 배치도 구 버전(`c`/`g` 클릭, `r`/`v` 휠)이었다.
  트리거 CapsLock · 위험 액션 제거 · `W`/`R` 클릭 · `T`/`G` 휠로 정렬했다.
  - 재발 방지: 열 정렬(defsrc ↔ 각 deflayer) · 금지 액션 부재 · 트리거 일치를
    검사하는 테스트 3개 추가. kanata 바이너리 없이 잡을 수 있는 것만 검사한다.
  - ⚠️ 이 프리셋은 **kanata 실행으로 검증하지 못했다**(로컬 미설치). 데몬 내장
    엔진 경로에는 영향이 없다.

- **`unmapped = "passthrough"` 설명이 낡아 있던 것 수정** —
  `dist/config.keymap.{windows,macos}.toml`이 *"어댑터 구현 전까지는 plain과
  동일하게 동작"*이라고 안내했지만 어댑터는 v0.9.3에 들어갔다. 이 주석을 믿고
  `passthrough`를 빼면 `LAlt` 트리거에서 `Ctrl+Alt+key`가 깨진다. Shift가
  투과 대상에서 제외된다는 점도 함께 명시.

### Documentation
- `docs/16` 모순 정리 — 열려 있는 과제 #4(kanata 드리프트)가 직전 커밋에서
  해결됐는데 "미착수"로 남아 있던 것을 완료로 바꾸고 실행 검증 미완 단서를 명시.
  결정 기록의 날짜 역순을 바로잡고, 08-26 행의 "`Alt+key`·`Alt+Shift+key`는
  구조적으로 못 살린다"가 다음 날 정정됐음을 그 자리에 표시.

- `docs/16` §5 정정 — `Alt+key` 충돌 범위는 **nav 매핑 11개 키뿐**이고 미매핑
  키는 `passthrough`로 보존된다(초판은 통째로 깨진다고 과장). `Alt+Shift+key`도
  "구조적 불가"가 아니라 **설계 선택**임을 AutoHotkey 대조로 명시 — AHK의 수정자
  핫키는 정확 일치 매칭이라 등록 안 한 조합이 통과하고, AHK의 커스텀 조합
  (`CapsLock & h::`)은 와일드카드 기본이라 우리 레이어와 같게 동작한다.
- `docs/16` §5-1b 신설 — 리본 KeyTip은 **순차 방식**이라 `LAlt` 트리거에서는
  `Alt+H` 하나가 아니라 **`Alt` 탭 자체가 죽어 체계 전체**가 막힌다.
  완화책은 `tap_action = "LAlt"`.
- 트리거 결정은 **실사용 검증 대기로 보류** 상태임을 문서 상단과 결정 기록에 명시.

- **`docs/16` §5 트리거 재검토 추가** — "`LAlt`로 되돌리는 게 낫지 않나"에 대한
  결론을 코드·문헌 근거와 함께 확정. `Ctrl+Alt+key`는 `unmapped="passthrough"`로
  **이미 보존되지만**(4-pre2 + `EngageChord`, v0.9.3), `Alt+key`(리본 니모닉)와
  `Alt+Shift+key`(HWP 자간좁히기)는 구조적으로 못 살린다 — 후자는 가드가
  Shift를 일부러 제외하기 때문. CapsLock 유지 + **레이어 로컬 Shift**(왼손 키
  홀드 = Shift)로 §3-2의 동시 입력 충돌만 떼어내는 쪽을 채택.
- `docs/16` §2·§3·§4에 문헌 대조 추가 — 키 위치별 노력 비용 모델(Engram,
  IJHCI 2026), "인접 + 같은 손가락"이 대체 오류의 조건이라는 오류 분류 연구,
  자동화 패턴 변경 시의 선행간섭(JMB 2020). 원문 대조 한계는 §5-5에 명시.

## [0.15.3] — 2026-08-23

### Changed
- **nav 레이어 단어/줄 이동 더블탭을 `U`/`I` → `I`/`O`로 되돌림** (v0.14.0에서
  바꾼 것을 복원). `CapsLock+I` = 이전 단어 / 2탭 = 줄 처음,
  `CapsLock+O` = 다음 단어 / 2탭 = 줄 끝. `U`는 다시 빈 키가 된다.

  위 행은 손바닥에서 멀어지는 **신전(펴기)** 동작이라 긴 손가락이 유리하고,
  적합도는 **중지 ≥ 약지 > 검지** 순이다. v0.14.0은 "검지+중지가 최강
  손가락"이라는 이유로 U/I를 골랐지만, 검지가 강한 건 *누르는 힘*이지
  *뻗는 거리*가 아니다 — 위 행의 병목은 힘이 아니라 도달성이다.

  부하 분산도 나빴다. 오른손 검지는 이미 `H`(←)/`J`(↓)/`N`(PgUp)/`M`(PgDn)을
  맡는데 `U`까지 얹으면 최빈 기능 5개가 한 손가락에 몰리고,
  `U`↔`H`/`J`/`N`/`M`이 전부 같은 손가락의 2행 왕복(same-finger bigram)이
  된다. `I`(중지)/`O`(약지)면 검지 4 · 중지 2 · 약지 2로 흩어진다.

  U/I가 노렸던 "O/P 오타 안전지대"는 `P`만으로도 대부분 유지된다 —
  `ㅖ`(Shift+P)는 흔하지만 `ㅒ`(Shift+O)는 드물고, `O` 오발사는 단어 이동이라
  레이어 불변식이 허용하는 무해 등급이다.

  > **주의**: 라이브 config에 `[launcher.keymap.layers.nav.double_taps.U]`
  > 블록이 남아 있으면 병합 규칙상 기본값 위에 덮여 `U`가 계속 살아 있다.
  > 해당 블록을 `.I`/`.O`로 바꿔야 한다 (`dist/config.keymap.*.toml` 참고).

### Documentation
- **`docs/16_keymap_ergonomics.md` 신설** — 키맵 전체 지도(nav·mouse·전역 +
  두벌식 Shift 지도), 손가락 배치 판단 기준, CapsLock↔LShift 인접의 구조적
  비용과 오발사 등급표, 열려 있는 과제, 배치 결정 기록. 지금까지 코드 주석과
  CHANGELOG에 흩어져 있던 근거를 한곳에 모았다.
- `docs/README.md` 색인 정정 — 13(v0.13.0부터 기본)·15(v0.15.0)를 "계획"에서
  "설계 이력"으로 옮기고, 누락돼 있던 14를 추가.

## [0.15.2] — 2026-08-20

v0.15.0 본문 검색의 전수 코드 리뷰(보안·정확성·성능·UX 4관점)에서 나온
개선 사항 일괄 반영.

### Fixed
- **본문 검색 비활성화가 반쪽만 동작하던 문제** — `content_search.enabled =
  false`로 꺼도 기존 인덱스가 남아 `?` 검색에 과거 본문이 계속 노출됐다.
  이제 끄면 다음 sync에서 인덱스를 전부 비우고, 런처도 비활성 안내만
  보여준다 (프라이버시 목적의 off 존중).
- **폴더 제안이 추가 직후에도 계속 뜨던 문제** — 제안 캐시(10분 TTL)가
  Enter로 방금 추가한 폴더를 계속 반환했다. 캐시 반환 시에도 현재
  search_paths와 겹치는 항목을 걸러낸다.
- **max_files 상한 도달 시 인덱스 오삭제** — 스캔이 상한에서 잘리면
  미도달 파일이 "사라짐"으로 오판돼 통째로 삭제·재인덱싱 churn이 생겼다.
  절단된 sync는 삭제 단계를 건너뛴다. 외장 디스크 언마운트 등으로
  search_path 루트가 일시 소실된 경우에도 하위 인덱스를 보존한다.
- **config 저장이 비원자적이던 문제** — 데몬의 주기적 재로드가 쓰다 만
  config.toml을 읽고 기본값으로 폴백할 수 있었다. tmp+rename 원자 쓰기로 교체.

### Changed
- **시크릿성 파일 기본 제외** — 파일명에 secret/credential/password/passwd가
  들어가면 본문 인덱싱에서 제외한다 (`content_search.exclude_names`로 대체
  가능). `env` 확장자도 기본 목록에서 제거 — `prod.env` 같은 시크릿 파일
  본문이 인덱스 DB에 평문 복제되지 않게. 필요하면 `extensions`에 다시
  추가하면 된다.
- **macOS 인덱서 스레드 QoS 하향** — 본문 인덱싱·파일 감시 스레드를
  QOS_CLASS_UTILITY로 강등 (Windows는 기존에 있었고 macOS는 no-op이었다).
  큰 폴더 첫 인덱싱이 포그라운드·키 입력과 CPU를 다투지 않는다.
- stat→read 사이에 파일이 커진 경우(TOCTOU) 크기 상한을 우회하지 못하게
  읽기 시점에도 상한을 검사한다.

## [0.15.1] — 2026-08-19

### Fixed
- **마우스 레이어 휠(T/G)이 짧은 탭에 무반응이던 문제** — 휠이 홀드 누적
  (초당 6노치)만으로 동작해 약 167ms 미만 탭은 이벤트가 아예 나가지
  않았다. 키다운 즉시 1노치를 보장(pending 적립)하고, 홀드 연사 속도를
  kanata 백엔드 프리셋과 같은 초당 20노치로 올렸다.

## [0.15.0] — 2026-08-17

### Added
- **문서 본문 검색 (docs/15 P1+P3)** — 파일 이름이 아니라 내용으로 찾는다.
  런처(데스크톱·TUI)에서 **`?질의`**(또는 `:grep`)로 검색 — 결과는
  "파일명 — «매치» 스니펫"이고 Enter로 파일이 열린다. CLI는
  `kmd grep <질의>`. 데몬이 인덱스 리프레시 주기마다 자동으로 증분
  갱신한다 (`kmd index --rebuild`도 함께 갱신). SQLite FTS5(bundled, 의존성
  추가 없음) + bm25 랭킹 + 매치 하이라이트 스니펫. 대상은 플레인 텍스트·
  소스코드 확장자(설정으로 대체 가능), 파일당 1MB 상한, UTF-8→EUC-KR 폴백,
  바이너리 NUL 스니핑 배제. 증분 판정은 mtime+size — 무변경 재실행은 수 ms.
  설정: `[launcher.content_search]` (enabled/max_file_kb/extensions/max_files).
  형태소 색인(P4)·검색 연산자는 docs/15 로드맵 참조.
- **본문 인덱스 실시간 갱신 (docs/15 P2)** — 데몬이 search_paths의 파일
  변경을 감시(notify)해 500ms 디바운스 후 증분 sync. 이벤트 폭풍(최소 간격
  10초)·연속 이벤트 기아(최대 지연 60초)·이벤트 유실(6시간 주기 리프레셔
  보정)까지 방어. 저장 직후의 문서가 곧바로 `?` 검색에 잡힌다.
- **"자주 변하는 폴더" 제안 (docs/15 P2)** — 검색 범위 밖에서 최근 2주
  활동이 활발한 폴더를 찾아 제안한다. 런처에서 `?`만 입력하면 하단에 제안
  행이 뜨고 **Enter로 즉시 search_paths에 추가**(config 저장), CLI는
  `kmd index --suggest`. 자동 추가는 하지 않는다 — 인덱스 범위는 사용자 결정.
  설계는 [Docufinder](https://github.com/chrisryugj/Docufinder) 분석에서
  채택(BSL 라이선스라 코드 재사용 없이 전부 자체 구현).

## [0.14.0] — 2026-08-12

### Changed
- **vim-nav 레이어 재설계 — "오발사해도 무해한 액션만" 불변식** (실사용
  피드백). CapsLock과 LShift가 같은 새끼손가락 인접 키라 한글 Shift 조합
  오타(예: ㅖ=Shift+P)가 붙여넣기·줄 삭제 같은 비싼 오발사로 이어졌다.
  - nav 레이어에서 P(붙여넣기)/Y(줄 복사)/U(줄 삭제)/,(단어 삭제) 기본 매핑
    제거. ','는 최빈 삭제 키 '.' 옆의 완충 빈칸으로 의도적으로 비워 둔다.
  - 단어/줄 이동 더블탭을 I/O(중지+약지)에서 **U/I**(검지+중지, J/K 세로열
    정렬)로 이동 — 최빈 기능에 최강 손가락.
  - **한/영 전환을 RAlt 짧은 탭으로 단일화** (물리 한영키 자리, macOS도
    오른쪽 Option 동일 위치). CapsLock 탭 한영은 폐지(탭 = 무동작 — 실수
    탭이 조용히 입력 소스를 바꾸던 문제), RShift 더블탭 기본도 제거.
    Shift+Space 콤보는 유지.
  - 마우스 레이어(RAlt 홀드) 클릭·휠을 공간 대응 배치로: **W/R = 좌/우클릭**
    (이동 클러스터 왼쪽/오른쪽 = 버튼 좌/우), **T/G = 휠 ↑/↓** (위 키=위로).
    구 배치(Space·C·G 클릭, R/V 휠)의 C 우클릭·G 중클릭·R/V 휠은 폐기,
    Space 좌클릭은 드래그용으로 유지(W는 S(←이동)와 같은 약지라 드래그 불가).

## [0.13.3] — 2026-08-11

### Added
- **`kmd daemon e2e` — 키 주입 셀프테스트 (macOS)** — 접근성이 부여된
  실기기에서 마커 없는 합성 키를 HID 위치에 주입해, 데몬의 실제 이벤트 탭
  경로로 레이어 홀드 매핑·탭=한영(Ctrl+Space 1회)·연타 디바운스(키보드 먹통
  사고 재발 방지 장치)를 검증한다 (docs/14 Tier 2). 릴리스 전 로컬 게이트
  용도. 실행 중 1~2초 타이핑 금지, 한영 토글은 짝수 회로 원상 복귀된다.
- `ipc::send_request_with_timeout()` — 수 초 이상 걸리는 데몬 요청용 응답
  대기 지정판 (기존 5초 고정이 위 셀프테스트를 불가능하게 했다).

### Fixed
- **살아있는 데몬 유령화 (CLI)** — 요청 처리 중 연결이 끊기면(핸들러 오류
  등) 클라이언트가 "데몬 종료"로 오판하고 포트 파일을 지워, 데몬이 살아
  있어도 CLI에서 재발견이 불가능해졌다. 재연결 프로브로 생사를 확인한 뒤에만
  정리한다.

## [0.13.2] — 2026-08-11

### Added
- **데몬 프로세스 E2E 테스트 (CI, unix)** — 실제 kmd-daemon 바이너리를 격리
  환경(HOME/XDG 리다이렉트)에서 spawn해 기동→토큰 인증 거부→상태→중복 기동
  거부→정상 종료를 검증한다 (`crates/kmd-daemon/tests/e2e_daemon.rs`).
  테스트 전략 전체(키 주입 E2E 로드맵, 테스트 정리 기준)는 docs/14.

### Changed
- **Windows 클립보드 붙여넣기를 무변형 Unicode 주입으로 전환** — 붙여넣기
  과정에서 시스템 클립보드를 교체·복원하지 않아 이미지·파일·Office 벡터·
  커스텀 포맷과 사용자의 동시 복사를 잃을 가능성을 구조적으로 제거했다.
  입력 큐 폭주와 물리 입력 인터리브를 막기 위해 한 번의 `SendInput`으로 최대
  UTF-16 4096유닛까지만 보내며, 터미널에서 의도치 않은 명령 실행을 막기 위해
  여러 줄 텍스트는 명확한 오류로 거부한다. `VK_PACKET`을 무시하는 raw-input
  앱에서는 직접 붙여넣기가 지원되지 않을 수 있다.
- **데몬 시작 인덱싱을 입력 크리티컬 경로에서 분리** — 신선한 캐시 또는 quick
  인덱스로 IPC와 키 훅을 먼저 준비한 뒤 전체 파일 인덱스를 백그라운드에서
  만든다. Windows에서는 인덱서 스레드 우선순위도 낮추고, 설정 변경 시 오래된
  캐시를 즉시 무효화한다.
- **클립보드 런처 응답성과 동작 명확화** — 히스토리 검색과 붙여넣기/복사 IPC를
  UI 스레드 밖에서 실행하고, 늦게 도착한 검색 결과는 세대 번호로 폐기한다.
  Cmd(macOS)/Ctrl(Windows)+Enter는 선택 항목을 붙여넣지 않고 복사만 하며,
  성공할 때만 런처를 닫고 실패 원인은 화면에 남긴다.

### Fixed
- **Windows 키 입력 지연과 훅 안정성** — 저수준 키보드 훅 콜백에서 느린 액션을
  워커로 분리해 key-up 지연을 막고, 합성 키·마우스 입력의 순서와 부분 실패를
  검증한다. 해제 실패는 성공할 때까지 제한 재시도하며, 훅 재설치 실패에는
  지수 backoff와 세대 재검증을 적용해 stuck modifier와 재시도 폭주를 방지한다.
  사용자 매크로는 합성 입력 64개로 제한해 훅 콜백 장기 점유를 막는다.
- **Windows 클립보드 감시 비용·경합** — 250ms마다 클립보드를 여는 대신
  `GetClipboardSequenceNumber`가 바뀔 때만 읽는다. 일시 점유 상태는 다음 틱에
  재시도하고, 민감정보 제외 마커를 존중하며, 안정 ID로 검색 후 슬롯 이동에도
  사용자가 고른 항목을 정확히 실행한다.
- **데몬 이중 기동 가드** — kmd-daemon 바이너리를 launchd나 직접 실행으로
  두 번 띄우면 둘 다 살아남아 이벤트 탭이 이중 설치되고(키 이벤트 이중 처리
  위험) 포트 파일을 덮어썼다. 프로세스 수명 동안 유지하는 OS 파일 잠금으로
  동시 기동 경쟁을 원자적으로 막고, 기존 버전과의 호환을 위해 Ping 확인도
  유지한다. 8개 프로세스 동시 기동 E2E로 단일 생존을 검증한다.
- **클립보드 대상·복구 안전성** — 데스크톱 런처 실행에만 이전 전경 앱을
  캡처하고, 대상 활성화 실패 또는 대기 중 포커스 변경 시 현재 앱에 잘못
  주입하지 않고 중단한다. Windows `SendInput`의 부분 실패 때 보내지 못한
  key-up/Unicode-up/마우스 button-up을 추적해 후속 입력 전에 재시도하고,
  주입 실패 시 남아 있는 마우스 이동도 즉시 멈춘다. TUI에서는 지원하지 않는
  클립보드 도움말 항목을 숨긴다.

## [0.13.1] — 2026-08-10

### Added
- **`kmd daemon restart`** — 정상 종료 후 재기동을 한 명령으로. 접근성 권한을
  다시 부여한 뒤 훅을 되살리는 표준 절차이며, launchd/systemd 밖에서 떠 있던
  데몬(stray)도 함께 정리된다.

### Changed
- **macOS 데몬 기동 launchd 일원화** — 자동실행(`kmd daemon install`)이 등록된
  환경에서 `kmd daemon start`/`restart`는 직접 spawn 대신 `launchctl
  bootout→bootstrap` 경유로 기동한다. 터미널에서 직접 띄우면 TCC 책임
  프로세스(responsible process)가 터미널로 귀속되어, 손쉬운 사용을 허용해도
  AXIsProcessTrusted=false로 훅 설치가 실패하던 함정 해소. `install`/
  `uninstall`도 구식 `launchctl load`/`unload` 대신 `bootstrap`/`bootout`을
  써서 이전 등록이 다른 바이너리 경로를 가리켜도 확실히 재바인딩된다.

### Fixed
- **한/영 연타 시 시스템 전역 키보드 먹통 (macOS, 치명)** — 한영 토글은
  Ctrl+Space(입력 소스 전환 단축키) 합성 주입인데, 토글이 직렬화 없이 겹치면
  두 주입의 Ctrl down/up이 인터리브되어 OS가 "Ctrl 홀드 + Space 연타"로
  해석한다. 그러면 입력 소스 피커(TextInputMenuAgent)가 열리고, 이 피커가
  key focus를 훔친 채 멈추면(wedge) 재부팅 전까지 모든 키 입력이 막힌다
  (마우스 포인터만 생존 — 2026-08-10 실사고, 통합 로그로 역추적).
  토글에 in-flight 가드 + 주입 완료 후 300ms 디바운스를 넣어 겹침 자체를
  차단했다. 연타 시 두 번째 탭이 무시되는 것은 의도된 동작이다.
  만에 하나 유사 증상 재발 시 재부팅 없는 응급 복구: 시스템 설정 → 손쉬운
  사용 → 키보드 → 보조 키보드를 마우스로 켠 뒤 터미널에
  `killall TextInputMenuAgent`.

## [0.13.0] — 2026-08-10

### Changed
- **vim-nav 기본 트리거 LAlt → CapsLock, 탭 = 한/영** — modifier 트리거는
  해당 modifier의 앱 단축키(HWP `Alt+Shift+N` 등)를 구조적으로 가리므로,
  non-modifier인 CapsLock을 기본으로 승격. 짧게 탭 = 한/영 전환(기존
  Escape 탭은 물리 Esc와 중복 + 오탭 부작용으로 교체). macOS는 데몬 실행 중
  CapsLock 대문자 잠금이 비활성(hidutil 재맵, 종료 시 자동 복구).
  Windows의 HHKB식 CapsLock 모드탭(탭=Caps/홀드=Ctrl)은 트리거를 LAlt로
  되돌린 경우에만 주입된다. 되돌리기: config에서 `trigger = "LAlt"` +
  `tap_action = "Escape"` 두 줄 (docs/13). 기존 사용자의 명시적 config는
  영향 없음.

### Added
- **CapsLock 레이어 트리거 (실험, opt-in)** — nav 레이어 트리거를 LAlt 대신
  CapsLock으로 쓸 수 있다 (`trigger = "CapsLock"`). macOS는 데몬이 시작 시
  hidutil로 CapsLock→F19를 재맵해 홀드 감지 불가 문제를 우회하고, 종료 시
  원복한다. Windows/Linux는 재맵 없이 그대로 동작. 트리거가 non-modifier가
  되면서 HWP `Alt+Shift+N` 등 modifier 단축키 가림이 사라진다. 탭 액션은
  `tap_action = "Hangul"` 권장 — 짧게 탭 = 한/영 전환, 홀드 = 레이어
  (macOS 순정 caps 한영과 동일 UX). 상세는 docs/13.

### Fixed
- **한/영 전환 직후 입력이 없으면 영문으로 되돌아가던 문제 (macOS)** —
  Shift+Space 전환은 Ctrl+Space 주입(네이티브 경로)으로 이미 성공하는데,
  백그라운드 스레드의 TIS 읽기가 낡은 값을 돌려줘 검증이 헛실패하고, 그때
  도는 TISSelectInputSource 폴백이 반쪽 전환을 일으켜 잠시 후 영문으로
  스냅백시켰다. 폴백을 제거하고 검증은 로그 전용으로 남겼다.
- **콤보 키 홀드 시 오토리피트 재발화** — Shift+Space를 누르고 있으면
  OS 오토리피트 keydown마다 콤보(한/영 토글 등)가 반복 발화했다. 콤보/
  더블탭으로 소비된 키는 keyup까지 리피트 down을 억제한다.
- **CapsLock 레이어 트리거가 Windows에서 Ctrl로 동작하던 문제** —
  `trigger = "CapsLock"`을 설정해도 vim-nav 프리셋의 CapsLock 모드탭
  (tap=Caps/hold=Ctrl)이 함께 주입됐고, 엔진에서 모드탭이 레이어 트리거보다
  먼저 처리되어 CapsLock을 가로챘다 (홀드+HJKL이 Ctrl+H/J/K/L로, 탭 한/영
  불능). 레이어 트리거가 CapsLock이면 프리셋 모드탭을 주입하지 않는다.
- **번들 기본 config에 `global_hotkey = "alt+space"` 활성화** — 지금까지
  Alt+Space 런처 실행은 LAlt 레이어의 Space 매핑이 우연히 대신해 왔다.
  트리거를 CapsLock 등으로 바꾸면 사라지므로, 레이어와 독립인 글로벌
  핫키를 기본으로 켠다.
- **데몬은 살아 있는데 키맵만 죽던 문제 — 키보드 훅 워치독 추가 (Windows)** —
  Windows는 저수준 키보드 훅(WH_KEYBOARD_LL)을 **통지 없이** 제거한다.
  콜백이 `LowLevelHooksTimeout`(기본 300ms)을 넘기거나, **Modern Standby
  (S0 저전력 대기) 복귀** 과정에서 훅 스레드가 스로틀/서스펜드되면 OS가 훅을
  체인에서 떼어낸다. 이때 HHOOK 핸들은 유효해 보이고, `GetMessageW`는 계속
  블로킹하며, 프로세스도 멀쩡하다 — 그래서 `kmd daemon status`는 "실행 중 ·
  레이어 2개"를 그대로 보고하는데 키맵만 죽어 있고, **데몬을 재시작하기
  전까지 영구히 복구되지 않았다.**

  실제 진단 사례: 데몬 22시간 연속 가동, 훅 설치 로그 이후 해제·토글 로그
  전무, 그 사이 이벤트 로그에 Modern Standby 종료(ID 507)가 20회 이상.

  대응은 3단계다.
  1. **하트비트** — 훅 콜백 최상단에서 `GetTickCount`를 기록한다. 주입
     이벤트와 `code < 0`도 포함해야 아래 프로브가 성립한다.
  2. **워치독** — "시스템은 최근 입력을 받았는데(`GetLastInputInfo`) 우리 훅만
     조용한" 상태를 감지하면, 무해한 예약 키(`VK_NONAME` 0xFC)를 주입해 콜백이
     도는지 확인한다. 하트비트가 안 움직이면 훅 사망이 확정된다.
  3. **재설치** — 훅 스레드에 커스텀 메시지를 보내 그 스레드에서 훅을 다시
     건다 (LL 훅은 설치한 스레드에 묶인다). 훅이 없던 동안 놓친 keyup 때문에
     레이어가 눌린 채로 남는 것을 막으려 엔진의 일시 상태도 함께 리셋하고,
     주입해 둔 코드 트리거가 있으면 먼저 해제한다 (stuck-Alt 방지).

  프로브는 **시스템에 최근 입력이 있을 때만** 쏜다 — 유휴 상태에서 키를
  주입하면 시스템 유휴 타이머가 리셋돼 화면 꺼짐·절전이 영영 걸리지 않는다.
  마우스만 쓰는 구간에서 매 주기 주입하지 않도록 레이트 제한도 둔다.

- **`kmd daemon status`에 키보드 훅 생존 표시** — "실행 중"만으로는 키맵이
  살아 있는지 알 수 없었다. 마지막 키 이벤트 경과 시간과 훅 재설치 누적
  횟수를 보여준다 (`키보드 훅:  설치됨 · 마지막 키 이벤트 0.4초 전 · 재설치 2회`).

- **`deploy-local.ps1`이 데몬 로그를 통째로 버리던 문제** — `kmd-daemon.exe`를
  `Start-Process`로 직접 띄워 stdout/stderr가 사라졌다. 정작 진단이 필요한
  로컬 배포본에서만 로그가 없어서, 훅 재설치 경고 같은 정보를 재발 시 확인할
  수 없었다. `kmd daemon start` 경유로 바꿔 `%APPDATA%\kmd\daemon.log`에
  기록되게 했다.

  이때 기동을 **기다리면 안 된다**: `& kmd.exe | ...`는 DETACHED 데몬이
  kmd.exe의 stdout 파이프 핸들을 상속해 PowerShell이 무한 대기하고,
  `Start-Process -Wait`는 PS 5.1이 job 객체로 자손까지 기다려 역시 멈춘다.
  기다리지 않고 띄운 뒤 폴링으로 확인한다 (고정 1초 대기는 kmd.exe 경유로
  한 단계 늘어난 기동 시간을 못 따라가 멀쩡한 배포를 "시작 실패"로 보고했다).

### Changed
- **삭제키 더블탭 제거 — `.`/`/` 는 연타 그대로, 줄 삭제는 `U`로 분리** —
  `Alt+/` 에 걸려 있던 더블탭(1탭 Delete / 2탭 줄 삭제)이 실사용에서 계속
  오발사됐다. 엔진은 **홀드**(오토리피트)는 single만 반복하도록 이미 막아
  뒀지만, 두세 글자를 지우려고 **탭-탭**하는 습관은 그대로 더블탭 판정에
  걸린다. Backspace/Delete는 키보드에서 탭 연타 빈도가 가장 높은 키라
  더블탭 게이팅과 근본적으로 맞지 않는다.

  더블탭은 "연타 개념이 없는 키"에만 남긴다 — `I`/`O`(단어 이동 → 줄 처음/끝)는
  연타가 곧 "더 멀리"라 의미가 이어지므로 유지한다.

  | `LAlt` + | 동작 |
  |---|---|
  | `.` / `/` | Backspace / Delete — 평범한 매핑, 연타 자유 |
  | `,` | 앞 단어 통째 삭제 (`Ctrl+Backspace`, macOS `Alt+Backspace`) |
  | `Y` / `U` | 줄 복사 / 줄 삭제 (vim `yy` / `dd`) |

- **마우스 레이어 이동축 WASD → ESDF** — WASD는 검지를 `D`에 묶어 타이핑
  홈포지션(검지 `F`)을 한 칸 어긋나게 만든다. 포인터를 움직일 때마다 왼손
  전체가 왼쪽으로 이동했다 돌아오는 비용이 생긴다. ESDF는 손을 홈에 둔 채로
  조작되고, 덤으로 `Q/W/R/T/A/G/Z/X/C/V/B`가 확장 자리로 열려 클릭·휠을
  같은 손에 붙일 수 있다 (WASD는 왼쪽 끝이라 붙일 자리가 없어 클릭이 오른손
  `J/K/L`로 밀려나 있었다).

  | `RAlt` 홀드 + | 동작 |
  |---|---|
  | `E`/`S`/`D`/`F` | 포인터 ↑←↓→ |
  | `R` / `V` | 휠 ↑/↓ — 검지 세로열 `R-F-V`, 이동(중지)과 손가락이 갈려 동시 조작 가능 |
  | `Space` | 좌클릭 (홀드 = 드래그) |
  | `C` / `G` | 우 / 중 클릭 |
  | `J`/`K`/`L` | 좌/우/중 클릭 (오른손 병행용 별칭, 유지) |
  | `LShift` | 저속 정밀 모드 |

  WASD와 ESDF는 병행할 수 없다 — `S`가 (아래 → 왼쪽), `D`가 (오른쪽 → 아래)로
  의미가 뒤집혀 서로 덮어쓴다. 되돌리려면 이동키 4개에 더해 잔상으로 남는
  `E`/`F`까지 재정의해야 한다 (`docs/06_config_reference.md` 참고).

  > **기존 사용자 주의**: 배포 스크립트는 이미 있는 `kmd-data/config.toml`을
  > 덮어쓰지 않는다. 라이브 config에 `[launcher.keymap.layers.nav.double_taps.Slash]`
  > 블록이 남아 있으면 기본값을 덮어써 옛 동작이 그대로 유지된다 — 해당 블록을
  > 지우고 `"/" = "Delete"`, `U`, `","` 매핑을 추가해야 한다.

### Internal
- **`fast` 빌드 프로파일 추가 — 로컬 배포 사이클 10분+ → 20초대** — 릴리스
  프로파일의 `lto = true` + `codegen-units = 1`은 378개 의존 크레이트를 하나로
  합쳐 재최적화하므로, 자체 코드가 한 줄만 바뀌어도 링크 단계를 전부 다시 돈다
  (실측: 증분 빌드 10분 초과). 개발·검증 중에는 과한 비용이라 LTO만 끈
  `[profile.fast]`를 추가했다 — 실행 동작(`opt-level`/`panic`/`strip`)은
  release와 같고 바이너리만 약 1.8MB 커진다.

  | 프로파일 | 증분 빌드 | 바이너리 |
  |---|---|---|
  | release (`lto=true`, `cgu=1`) | 10분 초과 | 15.6MB |
  | `lto="thin"`, `cgu=16` | 4분 43초 | 18.4MB |
  | **fast (`lto=false`, `cgu=16`)** | **21초** | 17.4MB |

  `panic = "unwind"`도 함께 지정한다 — cargo는 테스트를 항상 unwind로 빌드하므로,
  release의 `panic="abort"`를 상속하면 `cargo build`와 `cargo test`가 서로 다른
  아티팩트를 만들어 의존성 378개를 두 번 컴파일한다(빌드 9분 + 테스트 9분).
  unwind로 맞추면 테스트 단계 재컴파일이 8개 크레이트(29초)로 줄어든다.

  **엔드투엔드 실측**: 코드 1줄 수정 후 `-Fast` 배포(빌드+테스트+배포+데몬 재시작)
  **82초**, `-SkipTest` 병용 시 **9초**.

  `deploy-local.ps1 -Fast` / `deploy-local.sh --fast`로 사용한다. GitHub 릴리스
  자산은 CI가 기존 release 프로파일로 만들므로 배포물 품질에는 영향이 없다.
  단, 체크아웃마다 `target/fast`가 따로 만들어지므로 **각 체크아웃의 첫 실행은
  클린 빌드**(약 9~13분)다.

## [0.12.0] — 2026-08-05

Windows 투명 창 지원 — 창 리사이즈를 없애 화면 찢어짐을 근본 해결하고,
Windows/macOS UI를 동일하게 통일.

### Features
- **Windows 투명 창 (DirectComposition) — 리사이즈 제거 + UI 통일** —
  v0.10.1부터 Windows는 불투명 창 + 결과에 맞춘 창 리사이즈를 써 왔다.
  리사이즈는 그 자체로 컴포지터가 이전 프레임을 새 크기로 늘려 합성하는
  구간을 만들어, 결과가 뜨고 사라질 때 화면이 찢어져 보였다(v0.11.1~0.11.3의
  디바운스·cloak은 완화책이었다).

  원인을 추적한 결과 **아키텍처(x86/ARM) 문제가 아니라 스왑체인 종류** 문제였다.
  wgpu는 Windows에서 HWND 스왑체인(`DxgiFromHwnd`)을 쓰는데 이건
  per-pixel 알파를 지원하지 않아 `alpha_modes=[Opaque]`로 고정된다
  (`wgpu-hal/dx12/adapter.rs`). 예전에 투명이 됐던 것은 Vulkan 드라이버가
  있는 환경에서 wgpu가 Vulkan을 잡았기 때문이고, Vulkan이 없는 환경
  (Windows on ARM VM 등)에서만 DX12로 폴백해 검은 화면이 났던 것이다.

  이제 DirectComposition 스왑체인(`DxgiFromVisual`)을 사용한다 —
  실측 `alpha_modes=[Auto, Inherit, Opaque, PostMultiplied, PreMultiplied]`.
  덕분에 Windows도 macOS와 같은 구조를 쓴다:
  - 창 높이 **고정**, 빈 영역은 투명 → **창 리사이즈가 없어 찢어짐 원천 제거**
  - 카드 라운드를 직접 그림 → DWM 고정 8px 코너 클립 대신 **양 플랫폼 동일한
    pill 라운드** (기존에는 Windows만 라운드 사각형이라 UI가 갈렸다)
  - 부팅 시에도 처음부터 full 높이 → 첫 확대 리사이즈까지 제거
- **`general.window_transparency` 설정 추가** (`auto`/`off`) — 기본 `auto`.
  빈 영역이 검게 보이는 환경에서는 `off`로 이전 동작(불투명 창 + 리사이즈)으로
  되돌릴 수 있다. 소프트웨어 렌더러(`renderer="software"`)에서는 자동으로
  `off` 폴백. 긴급 시 환경변수 `KMD_NO_TRANSPARENT=1`.

### Internal
- **`vendor/iced_wgpu` — iced 0.14.0에 한 줄 패치** — 업스트림이 wgpu 인스턴스를
  `InstanceDescriptor { ..Default::default() }`로 만들어 `backend_options`가
  항상 기본값(`DxgiFromHwnd`)으로 고정되고 `WGPU_DX12_PRESENTATION_SYSTEM`
  환경변수가 무시된다. `BackendOptions::from_env_or_default()`를 넘기도록
  수정했다 (업스트림 PR 후보).
- **`tools/gpu-probe`** — 투명 창을 만들고 백엔드/스왑체인 조합별
  `alpha_modes`를 실측하는 진단 도구. 워크스페이스에서 제외되어 릴리스
  빌드에는 포함되지 않는다.

## [0.11.3] — 2026-08-04

### Bug Fixes
- **결과창이 뜨고 사라질 때 화면이 깨지던 문제 해결 (macOS) — 리사이즈 자체를 제거**
  — v0.11.2까지의 수정(레이어 gravity, 축소 디바운스)은 *리사이즈 순간의 왜곡을
  줄이는* 접근이었지만, 왜곡의 원인은 리사이즈 그 자체다. 창 크기가 바뀌는 한
  컴포지터가 이전 프레임을 새 크기로 합성하는 구간은 없앨 수 없다.

  이 증상은 원래 없었다가 **v0.10.1(2026-07-17)에 생겼다.** 그 전까지 창은 항상
  full 높이였고 빈 영역은 투명 픽셀이었다 — 리사이즈가 아예 없었다. v0.10.1에서
  창을 접기 시작한 이유는 오직 하나, 투명 합성이 안 되는 환경(Windows on ARM VM
  등 wgpu alpha mode가 Opaque로 폴백)에서 빈 영역이 거대한 검은 사각형이 되는
  문제였다. **macOS에는 해당되지 않는 문제**였고(Quartz는 알파를 항상 정확히
  합성한다), Windows는 v0.10.2부터 아예 불투명 창이라 그 문제가 따로 해결됐다.

  그래서 macOS는 v0.10.1 이전 구조로 되돌린다 — 창 높이를 full로 고정하고 카드만
  콘텐츠 크기로 늘었다 줄었다 한다. 빈 영역은 투명하고 클릭하면 런처가 닫힌다
  (기존 동작 그대로). 리사이즈가 없으므로 늘어날 이전 프레임도 없다. 부팅 시에도
  처음부터 full 높이로 띄워 첫 확대 리사이즈까지 제거했다.
  Windows/Linux는 검은 사각형 위험이 남아 있어 기존 리사이즈 경로를 유지한다.
- v0.11.2의 CAMetalLayer gravity 교정은 그대로 유지된다 — 좌우 드래그 리사이즈,
  디스플레이 배율 변경, `font_size` 변경처럼 남아 있는 리사이즈 경로를 덮는다.

## [0.11.2] — 2026-08-04

### Bug Fixes
- **창이 커/작아질 때 화면이 찌직 깨지던 문제 근본 수정 (macOS)** — v0.11.1의
  축소 디바운스는 증상 완화였고, 진짜 원인은 컴포지터가 *이전 프레임을 새 창
  크기로 늘려서* 합성하는 것이었다. macOS에서 wgpu는 NSView 루트 레이어에
  `CAMetalLayer`를 서브레이어로 붙이는데, 이 레이어의 `contentsGravity`
  기본값이 `resize`라 창 높이가 바뀌는 순간 직전 프레임이 통째로 늘어나거나
  눌린다. 게다가 뷰가 직접 소유한 레이어가 아니라서 암묵적 애니메이션
  (기본 0.25초)까지 걸려 왜곡이 한참 남았다 — 입력 한 글자에 46px→460px로
  커지는 런처에서 가장 크게 드러난다.
  이제 창 생성 직후 해당 레이어의 gravity를 `topLeft`로 바꾸고 암묵적
  애니메이션을 제거한다. 늘리는 대신 좌상단에 고정되므로, 확대 시에는 검색바가
  제자리에 있고 아래가 잠깐 비어 있을 뿐이며 축소 시에는 위쪽만 남는다.
  소프트웨어 렌더러 폴백처럼 Metal 레이어가 없으면 조용히 건너뛴다.
- **결과를 모두 지워 창이 접힐 때의 깨짐 완화 (Windows)** — DXGI 스왑체인은
  `DXGI_SCALING_STRETCH`로 고정돼 있어 같은 방식의 교정이 불가능하다. 대신
  1.25배 이상 축소되는 순간에만 DWM 합성에서 창을 잠깐 제외(cloak)해, 눌린
  프레임 대신 아무것도 보이지 않게 한다. 해제는 `Resized` 수신 후 한 프레임
  뒤이므로 실제 가려지는 시간은 16ms 안팎이고, 예약이 유실돼도 다음 메시지
  처리에서 강제 해제되어 창이 숨은 채 남지 않는다. 포커스·Z오더·항상 위
  속성은 건드리지 않으며, DWM이 없는 환경에서는 그대로 진행한다.
- v0.11.1의 축소 디바운스(60ms)는 연속 리사이즈를 한 번으로 합치는 역할로
  그대로 유지된다.

## [0.11.1] — 2026-08-04

### Bug Fixes
- **텍스트를 한 번에 지울 때 화면이 찢어지던 문제 수정** — 결과가 사라지는
  순간 창을 곧바로 접으면(full 460px → collapsed 46px) 스왑체인에 남은
  이전 프레임을 DXGI가 새 창 크기로 stretch 합성해 결과 목록이 찌그러져
  보였다. 백스페이스 연타, 더블탭 `/`의 한 줄 삭제(`macro:Home;Shift+End;
  Delete`), `.`→Backspace 매핑에서 모두 같은 원인. 이제 확대는 즉시,
  축소는 60ms 디바운스 후 적용한다 — 연속 축소가 한 번으로 합쳐지고,
  리사이즈 시점에는 이미 "빈 카드"(거의 단색)가 렌더돼 있어 stretch
  왜곡이 드러나지 않는다. 창을 접는 동작 자체는 유지되므로 v0.10.1의
  검은 화면 문제는 재발하지 않는다.

## [0.11.0] — 2026-08-04

데스크톱 비주얼 리뉴얼 — 이모지 아이콘을 테마 틴트 SVG로 전면 교체(+브랜드
모노 모드 옵션), 카드 테두리 창 밀착. 데몬이 인덱싱 소유권을 가져가
데스크톱 부팅이 가벼워졌고, TUI 한글 오입력·키바인드 버그를 수정.

### Performance
- **인덱싱 소유권을 데몬으로 이동 — 데스크톱 실행 시 인덱싱 비용 제거** —
  기존에는 kmd-desktop이 첫 실행(하루 1회)에 24시간 지난 인덱스를 직접
  재빌드했다. 이제 데몬이 시작 시 + `launcher.index_refresh_minutes` 주기
  (기본 360분, 0=off)로 전체/quick 인덱스를 백그라운드 재빌드해 공유
  캐시를 갱신하고 데몬 검색 엔진도 함께 교체한다. kmd-desktop은 언제 떠도
  캐시 히트로 즉시 로드하며, 24시간 freshness 재빌드는 데몬이 꺼져 있을
  때의 폴백으로만 남는다. IPC `RebuildIndex`도 캐시를 함께 저장한다.
- **quick 인덱스 캐시 신선도 적용** — quick 캐시(앱/PATH)는 영구 캐시라
  새로 설치한 앱이 full 엔진 교체 전까지 안 보였다. 데몬 리프레셔가 quick
  캐시도 주기 갱신하고, 데스크톱 쪽에도 24시간 freshness 폴백을 추가.
- **인덱스 캐시 원자적 쓰기 (tmp+rename)** — 데몬이 백그라운드로 캐시를
  쓰는 동안 데스크톱이 읽어도 잘린 파일을 보지 않는다.

### Bug Fixes
- **(Windows) 카드 테두리를 창 가장자리에 완전 밀착 — 비대칭 마진 제거** —
  카드 바깥 링(1px)+간격(2px)이 불투명 창 배경색 띠로 노출되고, DPI 소수
  배율에서 우측이 서브픽셀로 잘려 상하좌우 마진이 미묘하게 달라 보이던
  문제. Windows에서는 링·간격을 제거해 teal 테두리가 창 가장자리와
  일치하고(라운드 8px = DWM 코너 클립 정합), 창 높이 공식·테스트도
  `CARD_PAD` 상수로 정리. macOS 투명 pill의 이중 테두리는 유지.
- **TUI /exit 입력 중 한글 자모 오입력 수정** — 쿼리가 `/e`가 되는 순간
  이모지 프리픽스로 판정해 내장 한글 조합이 자동 활성화되면서, /exit의
  x가 'ㅌ'로 조합돼 `/eㅌit`이 되던 버그. `:emoji`를 치는 중에도 같은
  사고가 났다. 이제 별칭 뒤 공백이 와서 키워드 입력이 실제로 시작된
  뒤(`:e fire`)에만 자동 활성화한다.
- **deploy-local.ps1 이 `--help`를 배포 경로로 해석하던 사고 방지** —
  PowerShell은 이중 대시 토큰을 위치 인자로 바인딩해 `$DeployDir="--help"`
  가 되고, 리포 옆에 `--help` 폴더를 만들어 배포한 뒤 그 안의 데몬을
  띄워버렸다(실제 설치본은 미갱신). 이제 -h/--help/help는 사용법을
  출력하고, 옵션처럼 생긴 값·상대 경로는 배포 경로로 거부한다.
  deploy-local.sh도 --help와 알 수 없는 인자 거부를 추가.
- **레이어 더블탭 오토리피트 오판정 수정** — Alt+I/O/`/`를 누르고 있으면
  OS 오토리피트 down이 매번 새 탭으로 계산돼 single↔double 액션이 교대
  발사됐다 (Alt+`/` 홀드 시 Delete와 "줄 전체 삭제" 매크로가 번갈아 실행
  되는 파괴적 오작동). 이제 up 없이 반복된 down은 오토리피트로 인식해
  single 액션만 반복하고(연속 단어 이동), double 액션 직후의 리피트는
  억제한다 (Windows/macOS 공통 엔진 수정).
- **트리거 선해제 시 맨키 누출 차단** — Alt+H 홀드 중 Alt를 먼저 떼면
  계속 눌려 있는 H의 오토리피트가 맨키 'h'로 새어나가 문자가 입력되던
  문제. 레이어가 소비한 키는 keyup까지 추적·억제한다.
- **레이어 활성 전부터 눌려 있던 매핑 키의 keyup 억제 해제** — 해당 키의
  up이 OS에 전달되지 않아 stuck 상태가 되던 문제 (우리가 소비한 down의
  up만 억제).
- **Cmd/Ctrl+Alt+매핑 키 = OS 조합 보존** — Cmd+Alt+H("다른 앱 가리기")
  같은 조합이 레이어 매핑(Left 등)으로 오발사되던 문제. 트리거 외의
  비-Shift 수정자가 함께 눌린 키는 매핑 대신 트리거 조합으로 OS에
  투과한다 (passthrough 레이어).

### Features
- **브랜드 아이콘 모노 모드 (`general.brand_icons = "mono"`)** — 구글·네이버·
  GPT 등 웹 서비스 브랜드 아이콘을 풀컬러 로고 대신 Simple Icons(CC0) 단색
  글리프로 렌더링하는 옵션. 테마의 WebSearch 색(teal)으로 틴트되고 시스템
  아이콘과 같은 12% 알파 컨테이너에 얹혀 목록 전체가 한 톤으로 통일된다.
  `:set`의 "Brand Icons: Mono" 토글로 즉시 전환·저장 가능. 글리프가 없는
  서비스(grok/daum/papago)는 시스템 아이콘 폴백으로 흘리되 같은 teal로
  틴트해 톤을 유지. 기본값은 "color"(기존 풀컬러 로고).
- **데스크톱 시스템 아이콘 전면 교체 — 이모지 → 테마 틴트 SVG** — 시스템
  명령·프리픽스 명령·파일 확장자·키맵 치트시트 등 kmd-core가 이모지로
  내려주던 아이콘 90여 종을 데스크톱에서 Lucide SVG(ISC)로 오버라이드.
  아이콘은 카테고리별 시맨틱 컬러(teal/green/yellow/peach/red)로 틴트되고
  12% 알파 라운드 컨테이너 위에 얹혀 렌더링된다. `stroke="currentColor"`
  기반이라 5개 테마 전부 자동 추종. 브랜드 PNG → 시스템 SVG → 이모지
  텍스트 3단계 폴백으로 kmd-core/TUI는 무수정 (`system_icons.rs`,
  brand_icons 패턴 미러). `:emoji` 검색 결과는 실제 이모지를 유지한다.
- **Shift+네비 키 = 선택 확장 (macOS)** — Shift를 누른 채 Alt+H/J/K/L을
  누르면 Shift+화살표로 합성돼 텍스트 선택이 확장된다. 레이어 액션 실행
  시 트리거(Alt) 플래그만 지우고 함께 눌린 물리 수정자는 보존하도록 변경
  (Windows는 물리 Shift가 통과해 이미 동작).

## [0.10.2] — 2026-07-19

Windows 렌더링·성능 정비 — 검은 화면 근본 해결(불투명 창 전환) +
부팅/입력 핫패스 최적화.

### Bug Fixes
- **Windows 검은 화면 근본 해결 — 투명 창 포기, 불투명 창 + DWM 라운드
  코너로 전환** — v0.10.1의 창 접기로도 남아 있던 검은 영역(상단 드래그
  스트립 6px, 좌우 리사이즈 엣지 4px, pill 라운드 모서리 바깥, "No results
  found" 힌트 배경)은 모두 투명 픽셀이었다. iced/wgpu는 Windows(DX12·VM·
  소프트웨어 폴백)에서 창 단위 알파 합성이 신뢰할 수 없어 투명 픽셀이
  검게 그려진다. 이제 Windows에서는 창 배경을 테마 색으로 불투명하게
  칠하고, 라운드 코너는 DWM 네이티브 클립(`DWMWCP_ROUND`)으로 처리한다 —
  렌더러가 무엇으로 폴백하든 검은 픽셀이 나올 수 없는 구조.
  (macOS/Linux는 기존 투명 pill 유지)

### Performance
- **부팅: quick 인덱스 로드를 비동기로** — 기존에는 창을 만들기 전에 PATH
  스캔/캐시 로드를 동기로 수행해, 캐시 미스(첫 실행)나 느린 VM에서 창
  표시가 수백 ms~수 초 지연됐다. 이제 빈 엔진으로 즉시 창을 띄우고 quick
  인덱스는 백그라운드에서 로드 후 교체한다 (quick → full 2단계 워밍업은
  기존 그대로).
- **입력: 키 입력마다 하던 SQLite 히스토리 조회 제거** — frecency 부스트가
  매 검색마다 `query_history(500)` + 맵 재구축을 수행했다. 부팅 시 1회
  로드한 맵을 재사용한다 (`history::boost_results_with_map`).
- **입력: 불필요한 리렌더 프레임 제거** — 아이콘 prefetch가 새로 추출한
  아이콘이 없어도 매번 `IconsReady` 리렌더를 유발하던 것을, 실제로 새
  아이콘이 생겼을 때만 보내도록 변경. 소프트웨어 렌더러(VM)에서 프레임
  비용이 커 체감 효과가 크다.
- **`general.renderer` 설정 추가** (`auto`/`software`/`gpu`) — VM·원격
  데스크톱처럼 GPU가 부실한 환경에서 `software`로 지정하면 wgpu 어댑터
  프로빙(수 초 소요 가능)을 생략하고 tiny-skia로 직행한다. 부팅 단계별
  소요 시간 로그도 추가 (`desktop.log`).

### Docs
- **README 전면 개편 — 이야기 → 체험 → 습득 구조** — 프로젝트 동기(세 가지
  "이탈")와 철학 3원칙("홀드 손 ≠ 조작 손" 포함)을 서두에, "첫 60초" 최소
  경로와 미션 시나리오 5개(소환·vim-nav·모드탭·마우스 레이어·크로스 OS)를
  체험 코스로 추가. 기존 레퍼런스 표는 뒤로 재배치, v0.3.x 이력은
  CHANGELOG로 이관. 한국어판 `README.ko.md` 신설.
- **`kmd dojo` 계획 문서** (`docs/10_dojo_plan.md`) — 미션을 점수·콤보가
  있는 TUI 연습 게임으로 만드는 인터랙티브 트레이너 설계: 매핑 결과 판정
  아키텍처, 레벨 5종, 마일스톤 M1~M4.

## [0.10.1] — 2026-07-17

데스크톱 런처 검은 화면 핫픽스 — 투명 합성이 안 되는 환경(Windows on ARM
VM, 소프트웨어 렌더러 폴백 등) 대응.

### Bug Fixes
- **결과 없을 때 검색바 아래 거대한 검은 사각형이 보이던 문제** — 창은 항상
  full 높이(검색바+결과 10여 줄)로 만들고 빈 영역을 투명 픽셀로 채우는
  구조였는데, GPU/드라이버가 창 투명 합성(alpha mode)을 지원하지 않으면
  (VMware의 Windows on ARM 등 wgpu가 `Opaque`로 폴백하는 환경) 투명 영역이
  전부 검은색으로 렌더링됐다. 이제 결과가 없으면 창 자체를 검색바(pill)
  높이로 접고, 결과가 생기면 full 높이로 확장한다 — 렌더러와 무관하게 동작.
  부수 효과: 대기 상태에서 화면 1/3을 덮던 보이지 않는 클릭 차단 영역도
  사라져 아래 앱 클릭이 그대로 통과된다.

## [0.10.0] — 2026-07-17

tap-hold(모드탭)·마우스 레이어 릴리스 — HHKB 스타일 CapsLock 모드탭과
RAlt 홀드 마우스 레이어 추가.

### Features
- **HHKB 스타일 CapsLock 모드탭 (tap-hold)** — 짧게 탭 = CapsLock, 홀드 중
  다른 키 = Ctrl 조합. 다른 키를 누르는 순간 즉시 hold로 판정되어
  Ctrl+C 등이 타임아웃 대기 없이 동작한다. vim-nav 프리셋 기본값(Windows),
  minimal 프리셋은 tap=Esc/hold=Ctrl로 진화. macOS는 OS 자체 tap(한영)/
  hold(캡스락)와 충돌하므로 기본값에서 제외. kanata 프리셋에도 동일 반영
  (`tap-hold-press`). `[launcher.keymap.tap_holds.<키>]`로 커스터마이징.
- **마우스 레이어 (VIA 스타일 mouse keys)** — RAlt 홀드 → 왼손 마우스 조작.
  홀드 손(오른엄지)과 조작 손(왼손)을 분리한 배치: WASD 포인터 이동
  (180→1300px/s 시간 가속, 125Hz 워커), Space 좌클릭(홀드=드래그),
  J/K/L 좌/우/중 클릭, LShift 저속 정밀 모드. 미매핑 키는 차단(오타 방지).
  RAlt 짧게 탭 = 한/영 유지 (Windows 한국어 배열의 물리 RAlt=한/영 키 별칭
  매칭 포함). Windows(SendInput)/macOS(CGEvent, 드래그 이벤트 합성) 네이티브
  구현 + kanata 프리셋(`movemouse-accel-*`) 동일 배치. 레이어 트리거 해제·
  keymap 토글 시 이동/버튼 전체 정지(stuck-mouse 방지).
- **LLM 오토파일럿 (Windows)** — `@gpt`/`@claude`/`@gemini` 프리픽스로 연 창에
  키를 주입해 프롬프트를 자동 제출한다. URL `?q=`로는 프리필만 되고 실행이 안 되는
  서비스를 확장 설치 없이 처리한다. 안전 장치로 **전경창이 알려진 브라우저이고
  기대 타이틀 마커를 포함할 때만** 주입하며, 조건이 어긋나면 조용히 포기해
  프리필/클립보드 상태로 남긴다(회귀 없음). `@@`로 기억된 여러 LLM 창에 후속
  질문을 이어서 전달. Windows 외 플랫폼은 스텁이라 기존 URL 폴백을 쓴다.
  설계 근거는 [docs/09_llm_autopilot_plan.md](docs/09_llm_autopilot_plan.md).

## [0.9.5] — 2026-07-13

리팩토링·보안 정비 릴리스 — 0.9.4 패스쓰루 진단 과정에서 드러난
구조 문제(설정 에러 무시, 기본값 이원화)와 잠복 리스크를 일괄 해소.

### Bug Fixes
- **config.toml 파싱 에러가 조용히 무시되던 문제** — TOML 문법 오류
  (테이블 중복 정의 등) 시 데몬이 로그 한 줄 없이 전체 기본값으로
  폴백했다. 이제 에러 로그(경로 + 줄 번호)를 남기고 `kmd daemon status`에
  ⚠ 경고로 표시된다. 데몬은 여전히 기본값으로 계속 동작한다.
- **`vim-nav.kbd`처럼 확장자 붙은 프로필에서 치트시트가 프리셋을 안 보여주던
  문제** — 프로필 판별을 daemon과 치트시트가 다르게 하던 것을
  `profile_kind()`로 통일.
- **`none` 프로필이 사용자 커스텀 레이어를 끄지 않던 문제** — 문서된 대로
  키맵 전체가 비활성화된다 (global_hotkey는 유지).

### Security / Privacy
- **IPC 인증 토큰이 포터블 설치 위치에 노출되던 문제** — 런타임 파일
  (daemon.port/pid/log)을 포터블 모드와 무관하게 항상 OS 표준 사용자
  디렉터리에 기록한다. USB·공용 폴더에 설치해도 다른 로컬 계정이 토큰을
  읽을 수 없다. 포터블 모드의 이동성(config·데이터 = kmd-data/)은 그대로.
  ⚠ 업데이트 후 구버전 데몬이 떠 있으면 CLI가 찾지 못한다 — 데몬 재시작 필요.
- **훅 로그에 실제 타이핑 키 비기록** — chord engage 디버그 로그 등이
  사용자가 누른 키 이름을 남기던 것을 제거. 트리거(config 값)만 로그한다.

### Refactoring
- **키맵 기본값·병합을 kmd-core `effective_keymap`으로 단일화** — vim-nav
  기본 레이어가 daemon과 kmd-core 두 곳에 하드코딩되어 드리프트가
  반복되던 구조 해소 (-330줄). 병합이 TOML(Option) 수준에서 수행되어
  "생략"과 "명시적 기본값"이 구분된다 — 프리셋 기본이 바뀌어도 사용자
  레이어가 조용히 되돌아가지 않음.
- **macOS 액션 실행을 워커 스레드로 이관** — 탭 콜백에서 sleep 포함
  액션이 동기 실행되어 kCGEventTapDisabledByTimeout을 유발할 수 있던
  구조를 Windows(0.7.0)와 동일한 큐잉 모델로 통일. 실기기 검증 필요.
- **Windows VK 역변환을 정방향 match에서 자동 생성** — 거울상 match
  두 벌 유지로 인한 불일치 가능성 제거, 왕복 테스트 추가.
- 엔진 핫패스(키 이벤트마다)의 불필요한 Vec 할당 제거.

---

## [0.9.4] — 2026-07-12

Passthrough 진단 릴리스 — 0.9.3의 Windows 검증에서 "Alt+Tab이 Tab처럼 동작"
증상이 보고되어, 설정이 엔진까지 도달했는지 원격으로 확인할 수단을 추가.

### Bug Fixes
- **`:keymap` 치트시트가 사용자의 `unmapped` 설정을 무시하던 문제** —
  vim-nav 프리셋 병합(`effective_keymap`)이 새 필드를 복사하지 않았다.
  엔진(데몬) 경로는 영향 없음 — 표시만 잘못됐다.

### Diagnostics
- **`kmd daemon status`에 실행 중인 레이어 요약 표시** — 트리거·unmapped
  모드·매핑 수를 그대로 보여줘, 설정 파일이 실제 엔진에 적용됐는지 즉시
  확인할 수 있다 (`레이어: nav: LAlt 홀드 · unmapped=Passthrough · …`).
- **데몬 로그를 `<데이터 디렉터리>/daemon.log`로 기록** — 기존에는
  stdout/stderr가 전부 버려져 키맵 파싱 경고를 볼 방법이 없었다.
  시작마다 새로 쓰며, 경로는 status 출력에 표시된다.

---

## [0.9.3] — 2026-07-12

VIA-style layer passthrough (docs/08 P0–P3) — 레이어 트리거(Alt)를 눌러도
Alt+Tab 같은 OS 조합을 잃지 않는 코드(chord) 모드 도입.

### Features
- **Layer passthrough (`unmapped = "passthrough"`)** — while a layer is held,
  pressing a key that has no layer mapping now enters *chord mode*: the trigger
  and the key are injected to the OS in order, so native combos (Alt+Tab,
  Alt+F4 on Windows; Option-key characters on macOS) work exactly as without
  keymander. Everything in that hold passes to the OS until the trigger is
  released; the layer's tap action does not fire. Opt-in per layer — the
  default (`"plain"`) keeps the previous behavior, and `"block"` (VIA `KC_NO`)
  suppresses unmapped keys entirely.
- Engine guarantees: chord release is injected on keymap toggle and daemon
  stop (no stuck modifiers); deferred layer `launch:` actions still run after
  the chord ends. 9 new engine unit tests.

### Packaging
- First release shipping `.deb`/`.rpm` packages (x86_64 Linux) as release
  assets, alongside the SHA256SUMS.txt introduced in 0.9.2.

---

## [0.9.2] — 2026-07-12

### Bug Fixes
- **Long-running shell commands no longer killed after 10 s (TUI)** — `>`/`!` user commands in the TUI now open in a real terminal window (same UX as the desktop app) instead of running hidden with a 10-second timeout that aborted commands like `>winget upgrade --all` mid-run. Quick actions (`!ip`, `!uptime`, …) keep the inline capture + clipboard behavior.
- **macOS terminal launch works without Automation permission** — shell commands now run via a self-deleting temp `.command` script opened with `open -a Terminal`, replacing the osascript/AppleEvent approach that silently failed for non-bundled binaries without a TCC prompt. The window shows the exit status and waits for Enter.
- **Windows: quoted arguments survive `cmd /k`** — the command line is passed via `raw_arg`, fixing commands containing quotes that std's `\"` escaping (which cmd.exe doesn't understand) used to mangle.

### Refactoring
- Terminal launch unified into `kmd_core::plugin::builtin_shell::launch_in_terminal` — TUI and desktop share one implementation; the desktop's private copy is removed.

---

## [0.9.1] — 2026-07-11

### Bug Fixes
- **Windows binaries no longer require the VC++ Redistributable** — 0.9.0's MSVC builds dynamically linked the CRT, so `kmd daemon start` failed with a missing-`VCRUNTIME140.dll` error on a clean Windows install. All MSVC-target builds (x86_64/aarch64) now statically link the CRT via `.cargo/config.toml` (`-C target-feature=+crt-static`); the binaries run standalone.

---

## [0.9.0] — 2026-07-10

Command-prefix UX release — 프리픽스 문법을 업계 관례에 맞추고 TUI/데스크톱 명령 표면을 통일.

### Refactoring
- **Prefix parser unified into kmd-core** — the TUI and desktop each carried their own `starts_with` chains that had drifted apart. A single `query_prefix::prefix_of` now serves both, and the `COMMANDS` registry (aliases, help title/usage, quick-template seed, icons) is the single source of truth for command dispatch, the `:help` list, and the docs.
- **Token-boundary alias matching** — aliases match only on exact input or alias-plus-space. `:pto` no longer triggers `:pt`, `:setup` no longer triggers `:set`, `:verbose` no longer triggers `:ver`.

### Features
- **TUI command parity** — `:help`, `:set` (opens the F2 settings modal), `:version`, `:keymap` (with start/stop/profile actions), `:keys` (TUI-specific cheatsheet), and `:f` folder search now work in the TUI, matching the desktop app. Folder search moved to `kmd_core::folder_search` (with `USERPROFILE` fallback for `~` on Windows).
- **Slash command aliases** — every `:` command can be typed with a leading `/` (`/help`, `/set`, `/calc 2+3`), matching Slack/Discord/ChatGPT conventions. The closed `/pattern/` regex form still wins; unknown `/...` falls back to normal search.
- **`>` shell alias** — `>command` works like `!command`, matching the PowerToys Run / Flow Launcher / Alfred convention.
- **DuckDuckGo-bang hint** — typing `!g rust` (a shell command here, a web search there) shows a one-line "switch to `@g rust`" hint under the shell item; Enter switches to the web search.
- **Unknown-command feedback** — a mistyped `:clac` shows an "unknown command → `:help`" hint at the top of the results (suppressed while typing a known command's prefix); normal search still runs underneath.

### Bug Fixes
- **Unix paths no longer misdetected as regex** — `/usr/bin/` (slashes inside the pattern) now falls back to fuzzy search instead of regex mode.
- **Help entries all seed a quick template** — selecting the Fuzzy/Glob/Regex example rows in `:help` now fills a starter query (previously dead rows); detection is keyword-based instead of sniffing the description string.

### Docs
- README prefix table synced with the code: added `:t` `:prompt` `:f` `:keys` `:keymap` `:version`, full multi-search alias list, token-boundary rule, `/` and `>` aliases, bang hint.

---

## [0.8.0] — 2026-07-09

### Refactoring
- **macOS backend unified onto the shared key-binding engine** — `macos.rs`'s own decision logic (~380 lines, a divergent copy of the Windows logic) now delegates to `keybind::engine` (extracted in 0.7.0, 21 unit tests). Both platforms share identical, tested behavior. The CGEventTap callback is now a thin adapter: flagsChanged → down/up translation, OS flag sync, decision execution.
- **Layer Launch deferral promoted to the engine** — launch actions bound inside a layer now wait for the trigger key release on *both* platforms (previously macOS-only; Windows fired immediately while the trigger modifier was still held).
- `KeyDecision::Execute` now carries `layer_trigger` context so macOS can clear residual trigger-modifier flags before synthetic events.

### Bug Fixes (macOS)
- **CapsLock remap now works** — the old flagsChanged branch never consulted `remaps`, so the `minimal` preset (CapsLock → Escape) silently did nothing on macOS.
- **Backend restart applies new config** — same `OnceLock` restart bug fixed on Windows in 0.7.0.

---

## [0.7.1] — 2026-07-08

### Bug Fixes
- **Linux shell timeout was ineffective** (found by CI) — on timeout, `child.kill()` only killed `sh`; grandchildren (e.g. `sleep`) kept the stdout pipe open, so the reader join blocked until the command finished on its own. The child is now spawned as a process-group leader (`setpgid`) and the whole group receives `SIGKILL` on timeout (the Unix counterpart of Windows `taskkill /T`). Reader results are collected via a channel with a 2 s grace `recv_timeout`, so even a process that escaped into a new session can't block the launcher.

### CI
- `cargo fmt` applied to 0.7.0 code (Format check green again).

---

## [0.7.0] — 2026-07-08

Follow-up hardening release — 0.6.0 감사에서 예고된 후속 과제 반영.

### Refactoring
- **Key-binding decision engine extracted** — all binding logic (modifier tracking, toggle, layer tap/hold, layer double-tap, combos, global double-tap, remaps) moved from the unsafe Windows hook callback into a pure, platform-independent `keybind::engine::EngineState`. `process_key(vkey, is_down, tick) → KeyDecision` takes time as a parameter, so timing behavior is now covered by **16 new unit tests** (tap-vs-hold, double-tap timeout, modifier-used-in-combo false-positive guard, toggle keeps Launch combos, u32 tick wraparound). The hook file now only installs the hook, translates events, and queues actions.

### Performance / Reliability
- **Hook actions run on a dedicated worker thread** — the low-level hook callback now queues actions over an mpsc channel (FIFO, key order preserved) and returns immediately. Long macros previously executed inside the callback and risked exceeding Windows' `LowLevelHooksTimeout`, which silently uninstalls the hook.

### Security
- **Windows single instance via named mutex** — replaces the PID-file check (TOCTOU race, PID-recycling false positives). The mutex name is derived from the data directory, so multiple portable installs don't interfere; the OS releases ownership on any kind of process death.

### Dependencies
- **bincode 1 → 2** — bincode 1.x is unmaintained (RUSTSEC advisory). Old-format index caches fail decoding gracefully and fall back to the JSON cache / full rebuild.
- **getrandom 0.2 → 0.3**.

### CI
- **macOS test SIGABRT fixed** — desktop unit tests sending `GotRawWindowId` reached the Carbon TIS API, which requires the main thread + a window-server session and aborts on headless CI runners. TIS calls are now skipped in test builds. (macOS CI has been red since 0.5.0.)

---

## [0.6.0] — 2026-07-08

Stability & security release — 코드 전반 감사에서 발견된 실버그와 보안 보완점 수정.

### Bug Fixes
- **Search engine reload duplication** — `SearchEngine::load()` now calls `nucleo.restart()` before injecting items. Previously every index rebuild (daemon `RebuildIndex`, TUI settings save) left deleted items in fuzzy results, accumulated duplicates, and leaked memory.
- **Filename → URL misdetection** — `is_url()` now uses a curated TLD whitelist. Previously any `name.ext` with a 2–6 letter alphabetic extension (`report.pdf`, `readme.md`, `config.toml`) was classified as a URL, which emptied search results. Extensions clashing with real TLDs (`md`, `rs`, `sh`, `ts`, `zip`, `mov`) are intentionally excluded — use `https://` or `www.` prefix to open those domains explicitly.
- **URL open respects selection** — URL-looking queries now show an "Open <url>" virtual item at the top of normal search results. Enter always executes the selected item (previously it either ignored the selection or did nothing when the list was empty).
- **Shell command timeout** — `!` commands are killed after 10 s (process tree on Windows via `taskkill /T`) with output capped at 256 KB. Previously `!ping -t` froze the launcher permanently.
- **Daemon shutdown hang** — `Shutdown` now wakes the main thread via a channel and unblocks the accept loop with a self-connect. Previously shutdown only completed because `kmd daemon stop` happened to poll with connects; other clients would leave the daemon waiting forever.
- **Keyboard hook restart** — restarting the backend now applies the new config; the `OnceLock` state previously ignored the second `start()` silently.
- **History pruning frequency** — the "5% probabilistic" pruning ran on *every* launch on Windows (`subsec_nanos()` is always a multiple of 100 → always a multiple of 20). Replaced with a deterministic once-per-20-launches counter.

### Security
- **Token file permissions** — `daemon.port` (contains the IPC auth token) is created with `0600` from the start on Unix, eliminating the write-then-chmod exposure window. Stale files are removed before re-creation.
- **IPC request size limit** — client requests are capped at 64 KB, preventing unbounded memory growth from a newline-less stream.
- **LIKE wildcard escaping** — history search no longer interprets `%`/`_` in user queries as SQL wildcards.

### Performance
- **Keyboard hook message loop** — replaced `PeekMessageW` + 1 ms sleep busy-wait (~1000 wakeups/s) with a blocking `GetMessageW` loop; `stop()` posts `WM_QUIT`. Reduces idle CPU/battery drain of the always-on daemon.
- **Daemon main loop** — replaced 200 ms shutdown-flag polling with a blocking channel wait.

### Refactoring
- `IpcError` migrated to `thiserror` (+`#[from]`); `DbError::Io` gains `#[from]`.
- `ProviderConfig` derives `Clone`, removing manual field-by-field copies.
- Silent empty-result fallbacks in history/bookmark queries now emit `tracing::warn!`.

---

## [0.5.0] — 2026-05-16

### New Features
- **Frecency-based ranking** — frequently used programs/files float to the top of search results. Launch history is recorded per item and decays over time (1h ×16, 24h ×8, 1w ×4, 1mo ×2, older ×1). Applies to all search modes including relaxed Hangul fallback.
- **Calculator clipboard** — pressing Enter on a calculation result copies the value to the clipboard. Ctrl+4 shortcut and a dedicated "값 복사" copy button appear in the detail panel.
- **Folder search** (`:f`) — type `:f /path query` or `:f ~/path query` to instantly search inside any directory without adding it to the index. Prefix-match results are ranked higher, folders appear before files, and emoji icons indicate file type.
- **Layer Launch deferral** — layer-key bindings that launch apps now wait until the trigger key is released before executing, eliminating modifier-key interference with IME and launched apps.

### UI / Design
- **Keymander theme** — new default color palette: deep ink background, copper accent, signal-cyan border. Replaces "Midnight". Backward-compatible alias kept.
- **Brand mark** — the `»` icon is now rendered inside a subtle copper circular badge. The double-border card (outer copper glow + inner accent) adds depth without clutter.
- **Dynamic border opacity** — card border brightens (0.28 → 0.52) when the search bar is idle, giving a subtle focus cue.

### Improvements
- **Daemon accept loop** — replaced 50 ms busy-wait polling with a blocking `incoming()` loop on a dedicated thread. Eliminates unnecessary context switches when idle.
- **IPC kind field** — `format!("{:?}", kind)` replaced with `kind.to_string()` (stable `Display` impl). Enum renames no longer silently break the protocol.
- **`filter_contains` performance** — path-segment lookup changed from `Vec::contains` O(n) to `HashSet` O(1) per token in multi-token searches.
- **`copy_multi_llm` config** — no longer re-reads config from disk; uses `self.runtime_config` directly, preserving any in-session edits.
- **DB open error logging** — silent in-memory fallback now emits a `tracing::warn!` so users can diagnose missing frecency persistence.
- **`select_services` dedup** — duplicate provider IDs in config are silently deduplicated instead of creating duplicate search rows.
- **Brand click routing** — `BrandClicked` / `BrandRightClicked` now use `toggle_query_mode()` helper backed by `prefix_of()`, removing hardcoded string checks.

### Refactoring
- `app.rs` split from ~3 400 lines into focused modules: `app/settings.rs`, `app/launch.rs`, `app/view.rs` (retains ~1 700 lines).
- Unified two divergent `is_hangul_jamo` implementations — `App::is_hangul_jamo` now delegates to the free function.
- Removed unused `KeyboardBackend::is_running` trait method and all implementations.
- Removed unused `surface` and `corner_radius` fields from `DesktopTheme`.
- `UiScale.brand_icon` field removed (superseded by `view_brand_mark`).

### CI
- Added Linux system-library installation steps (`libx11`, `libxcb`, `libwayland`, `libxkbcommon`, `libfontconfig`) to `ci.yml` and `release.yml` so `kmd-desktop` (iced) builds correctly on `ubuntu-latest`.

---

## [0.4.0] — 2026-05-10

### New Features
- Multi-LLM provider toggle (`:set`)
- Autostart daemon management via IPC (`AutostartStatus`, `AutostartEnable`, `AutostartDisable`)
- Async autostart status refresh — `:set` no longer blocks the UI
- Dynamic detail-panel title width based on panel pixel width and font size
- IME reset delay on macOS (45 ms) to prevent first-keystroke corruption after hotkey launch

### Improvements
- Icon prefetch optimized: deduplication and cache-key-based lookup
- `VirtualBrowseEntry` struct for web browse items
- `parse_combo_vkeys` helper reduces keybind parsing duplication
- `clippy -D warnings` — fixed `collapsible_match` and `unnecessary_sort_by` lints

### Bug Fixes
- Detail panel title overflow on long Korean filenames
- Ctrl+2 shortcut not firing on non-US keyboard layouts (added `logo()` and shifted-numeral matching)

---

## [0.3.x] and earlier

상세 내역은 [docs/v03-changelog.md](docs/v03-changelog.md)에 있다.
그보다 앞선 이력은 git log를 참고할 것.
