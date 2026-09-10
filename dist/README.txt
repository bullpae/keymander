============================================================
  keymander (kmd) — Portable Edition
  키보드 하나로 모든 것을 지휘한다
============================================================

이 폴더를 원하는 위치에 두고 바로 사용하세요.
설치가 필요 없습니다. 모든 데이터는 kmd-data/ 안에 저장됩니다.

------------------------------------------------------------
  포함된 파일
------------------------------------------------------------

  kmd(.exe)           CLI / TUI 런처
  kmd-desktop(.exe)   데스크탑 GUI 오버레이 (Spotlight 스타일)
  kmd-daemon(.exe)    백그라운드 데몬 (핫키, 키 바인딩, IPC)
  kmd-data/           설정·DB·캐시 저장 (포터블 모드)
  README.txt          이 파일

------------------------------------------------------------
  빠른 시작
------------------------------------------------------------

1. TUI 모드 (터미널)
   > kmd
   검색어 입력 → 화살표로 선택 → Enter 실행

2. Desktop 모드 (GUI 오버레이)
   > kmd-desktop
   검색창이 화면 중앙에 나타남 → 검색 → Enter 실행 → 자동 닫힘

3. 데몬 모드 (백그라운드 서비스)
   > kmd-daemon start       데몬 시작 (키 바인딩 + IPC 서버)
   > kmd-daemon status      상태 확인
   > kmd-daemon stop        데몬 종료
   > kmd-daemon install     부팅 시 자동 시작 등록
   > kmd-daemon uninstall   자동 시작 해제

------------------------------------------------------------
  글로벌 핫키 설정 (권장)
------------------------------------------------------------

  ** Windows — AutoHotkey **
  !Space::Run "C:\path\to\keymander\kmd-desktop"

  ** Windows — PowerToys **
  Keyboard Manager → Remap Shortcut → Alt+Space → kmd-desktop

  ** 데몬 사용 시 (별도 설정 불필요) **
  kmd-daemon start 만 실행하면 Alt+Space 로 자동 등록됨

  ** 부팅 시 자동 시작 **
  kmd-daemon install 한 번만 실행하면 다음 로그인부터 자동 시작
  (Windows: Startup 폴더, macOS: LaunchAgent, Linux: systemd)

------------------------------------------------------------
  주요 명령어
------------------------------------------------------------

  검색창에서:
    @g query          Google 검색
    @yt query         YouTube 검색
    @gh query         GitHub 검색
    @ai query         AI (Perplexity) 질문
    @ll query         여러 LLM 동시 비교
    @sp 문장          맞춤법 검사
    @tr text          번역 (자동 감지)
    :emoji fire       이모지 검색
    :set              설정 열기
    :help             전체 명령어 보기
    !command          셸 명령 실행

  CLI에서:
    kmd search "query"       검색
    kmd launch "Firefox"     실행
    kmd index --rebuild      인덱스 재빌드
    kmd config edit          설정 편집
    kmd keymap init vim-nav  Vim 키맵 프리셋 설치

------------------------------------------------------------
  설정 변경
------------------------------------------------------------

  방법 1: kmd-data/config.toml 직접 편집
  방법 2: kmd-desktop 에서 :set 입력
  방법 3: kmd (TUI) 에서 F2 키
  방법 4: kmd config set general.theme "nord"

  (소스에서 번들을 만들 때) Windows/macOS 키맵이 섞이지 않도록
  저장소의 dist/config.shared.toml + dist/config.keymap.<os>.toml 을
  합쳐 kmd-data/config.toml 을 만듭니다. 자세한 내용은 dist/CONFIG.md 참고.

------------------------------------------------------------
  커스텀 키 매핑 설정
------------------------------------------------------------

  config.toml 에서 키 바인딩을 자유롭게 설정할 수 있습니다.

  1. active_profile = "custom" 으로 변경
  2. [launcher.keymap.remaps], [launcher.keymap.layers.이름] 등 정의
  3. kmd-daemon restart 또는 stop → start

  예) Alt+H/J/K/L 방향키, CapsLock→Escape, Shift+Space→한영:

    [launcher.keymap]
    active_profile = "custom"

    [launcher.keymap.remaps]
    CapsLock = "Escape"

    [launcher.keymap.layers.nav]
    trigger = "LAlt"
    tap_action = "Escape"

    [launcher.keymap.layers.nav.mappings]
    H = "Left"
    J = "Down"
    K = "Up"
    L = "Right"

    [[launcher.keymap.combos]]
    trigger = "Shift+Space"
    action = "Hangul"

  자세한 형식은 config.toml 의 주석을 참고하세요.

------------------------------------------------------------
  포터블 모드 안내
------------------------------------------------------------

  kmd-data/ 폴더가 실행 파일 옆에 있으면 자동으로
  포터블 모드로 동작합니다.

  - 설정: kmd-data/config.toml
  - DB:   kmd-data/kmd.db
  - 캐시: kmd-data/index.bin, index.json

  이 폴더를 USB 등에 복사하면 설정·기록을 함께 이동 가능.

  포터블 모드를 끄려면: kmd portable disable
  (kmd-data/ 삭제 후 시스템 경로 사용)

------------------------------------------------------------
  자세한 정보
------------------------------------------------------------

  GitHub: https://github.com/bullpae/keymander
  License: MIT

============================================================
