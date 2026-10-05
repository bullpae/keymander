# Product Requirements Document (PRD)

## 1. 개요

### 1.1 제품명
**keymander** (바이너리명: `kmd`)

### 1.2 슬로건
> 키보드 하나로 모든 것을 지휘한다 — CLI-first cross-platform keyboard launcher

### 1.3 문제 정의

컴퓨터에서 새로운 작업을 시작할 때마다 마우스로 손을 이동하여 프로그램을 찾고 실행하는 행위는 키보드 중심 작업 흐름을 크게 방해한다. 기존 키보드 런처들은 다음 한계를 가진다:

| 문제 | 해당 도구 |
|------|----------|
| macOS 전용 | Raycast, Alfred, Spotlight |
| Windows 전용 | PowerToys Run, Flow Launcher |
| Linux 전용 | Rofi, Albert, Ulauncher |
| 크로스플랫폼이나 무거움 (Electron) | ueli, LaunchMenu |
| 크로스플랫폼이나 미완성 | Kunkun, Launchy |

#### 경쟁 포지셔닝

```mermaid
quadrantChart
    title "런처 포지셔닝 맵"
    x-axis "Platform-Specific" --> "Cross-Platform"
    y-axis "Heavy" --> "Lightweight"
    quadrant-1 "Target Zone"
    quadrant-2 "Light but Limited"
    quadrant-3 "Heavy and Limited"
    quadrant-4 "Heavy but Broad"
    Raycast: [0.1, 0.4]
    Alfred: [0.15, 0.6]
    PowerToysRun: [0.2, 0.5]
    Rofi: [0.25, 0.9]
    ueli: [0.8, 0.2]
    FlowLauncher: [0.2, 0.4]
    keymander: [0.85, 0.95]
```

**빈 시장**: "크로스플랫폼 + 경량 + 통일 인터페이스"를 동시에 만족하는 런처가 없다.

### 1.4 해결 방안
Rust 기반 단일 바이너리 TUI 런처. GUI 툴킷 불필요, 터미널에서 동작하므로 진정한 크로스플랫폼. CLI-first 아키텍처로 스크립팅과 자동화 지원.

### 1.5 목표 사용자
- 터미널 중심 개발자
- 여러 OS를 오가며 일관된 워크플로를 원하는 사용자
- SSH/원격 환경에서도 런처를 사용하고 싶은 사용자
- Electron 런처의 무거움에 불만인 사용자

---

## 2. 설계 원칙

| 원칙 | 설명 |
|------|------|
| **CLI-first** | Core → CLI → TUI 레이어 분리. 모든 기능이 CLI로 접근 가능 |
| **런처 중심** | 프로그램/파일 검색·실행이 핵심. 나머지는 확장(Extension/Plugin) |
| **단일 바이너리** | 외부 런타임/의존성 없이 `kmd` 하나로 동작 |
| **극한의 경량성** | 목표: ~3MB 바이너리, ~50ms 시작, <5MB RAM |
| **포터블** | 바이너리 + config.toml + kmd.db = 전체 상태. USB에서도 실행 |
| **확장 가능** | Extension trait + 스크립트 플러그인으로 기능 추가 |

---

## 3. 핵심 기능

### 3.1 런처 (Core)

| 기능 | 설명 |
|------|------|
| 앱 검색 | OS별 설치 앱 인덱싱 및 퍼지 검색 |
| 파일/폴더 검색 | fd/Everything/mdfind/locate/builtin 프로바이더, 폴더도 인덱싱 |
| 폴더 드릴다운 | 폴더 선택 후 Tab/→으로 내부 실시간 탐색, ←/Esc로 복귀 |
| PATH 실행파일 | PATH 환경변수의 모든 실행파일 |
| 시스템 명령 | 종료, 재시작, 잠금, 절전 등 |
| 웹 서비스 | @g, @gh, @yt 등 10종 내장 + 커스텀 정의 |
| AI 서비스 | @ai (Perplexity), @gpt (ChatGPT), @claude, @gemini |
| URL 열기 | URL 자동 감지 → 브라우저 |
| 히스토리 부스팅 | 자주 사용한 항목이 상단 노출 |
| 최근 실행 | 빈 쿼리 시 최근 실행 항목을 올바른 아이콘과 함께 표시 |
| 북마크 | 자주 쓰는 항목 고정 |
| 인라인 계산기 | `:calc` 접두사 또는 수식 자동 감지 — 계산 결과 표시, 클립보드 복사 |
| 단일 인스턴스 토글 | 핫키 재입력 시 기존 인스턴스 종료 (on/off 토글) |
| 포터블 모드 | `kmd portable enable` — exe 옆 kmd-data/ 사용 |
| 윈도우 센터링 | Windows 핫키 실행 시 콘솔 창 화면 중앙 배치 |
| 설정 모달 (F2) | 6개 탭: Priority, Search, Paths, Ignore, Display, Keys |
| 한글 입력 | 내장 2-벌식 한글 조합 엔진 (터미널 raw mode에서 동작) |
| 검색 우선순위 | KindWeights로 항목 종류별 가중치 설정 |
| 스캔 범위 설정 | scan_drives, drive_scan_depth, search_paths 설정 가능 |
| 인덱스 캐시 버전 | 바이너리 업데이트 시 자동 캐시 무효화 |

### 3.2 검색 모드 자동 감지

```mermaid
flowchart TD
    Input["사용자 입력"] --> URLCheck{"URL 형태?"}
    URLCheck -- Yes --> URLMode["URL 모드: 브라우저 열기"]
    URLCheck -- No --> GlobCheck{"* 또는 ? 포함?"}
    GlobCheck -- Yes --> GlobMode["Glob 모드: 패턴 필터"]
    GlobCheck -- No --> RegexCheck{"/pattern/ 형태?"}
    RegexCheck -- Yes --> RegexMode["Regex 모드: 정규식 필터"]
    RegexCheck -- No --> ExtCheck{".ext 형태?"}
    ExtCheck -- Yes --> ExtMode["Extension: *.ext로 변환 후 Glob"]
    ExtCheck -- No --> CJKCheck{"비ASCII 문자?"}
    CJKCheck -- Yes --> ContainsMode["Contains 모드: 부분문자열"]
    CJKCheck -- No --> FuzzyMode["Fuzzy 모드: Nucleo 매칭"]
```

| 입력 패턴 | 모드 | 동작 |
|-----------|------|------|
| 일반 텍스트 | Fuzzy | Nucleo 퍼지 매칭 |
| `*`, `?` 포함 | Glob | 글로브 패턴 필터 |
| `/pattern/` | Regex | 정규식 필터 (ReDoS 방어) |
| `.ext` | Extension | `*.ext`로 변환 후 Glob |
| 비ASCII (한글 등) | Contains | 정확한 부분문자열 매칭 |
| URL 형태 | URL | 브라우저에서 열기 |
| `@prefix query` | WebSearch | 외부 서비스 검색 |
| `:calc expr` | Calculator | 수식 계산 (Plugin) |

### 3.3 CLI 명령 체계

```mermaid
flowchart LR
    kmd["kmd"] --> noarg["(인자 없음)"]
    kmd --> search["search"]
    kmd --> launch["launch"]
    kmd --> index["index"]
    kmd --> config["config"]
    kmd --> history["history"]
    kmd --> plugin["plugin"]
    kmd --> daemon["daemon"]

    noarg --> TUI["TUI 모드 실행"]
    search --> SearchQuery["query --limit --json"]
    launch --> LaunchTarget["target"]
    index --> IndexOpts["--rebuild --stats"]
    config --> ConfigGet["get key"]
    config --> ConfigSet["set key value"]
    config --> ConfigEdit["edit"]
    config --> ConfigPath["path"]
    history --> HistList["list --limit --json"]
    history --> HistClear["clear"]
    plugin --> PlugList["list"]
    daemon --> DaemonStart["start"]
    daemon --> DaemonStop["stop"]
    daemon --> DaemonStatus["status"]
```

### 3.4 TUI 화면 설계

#### 메인 화면 (미리보기 ON)

```mermaid
block-beta
    columns 1
    block:header:1
        H["⌨ key·mander v0.2.0 · 1234 items indexed"]
    end
    block:inputBar:1
        IB["⌨ Command: > fire_"]
    end
    block:content:1
        columns 2
        block:resultsList:1
            R1["▸ 📦 Firefox          [App]"]
            R2["  📦 FileZilla        [App]"]
            R3["  ⚙️ Firewall 설정   [System]"]
            R4["  📄 fire.py          [File]"]
        end
        block:previewPanel:1
            P1["Name: Firefox"]
            P2["Type: App"]
            P3["Path: C:\Program Files\Mozilla..."]
            P4["Keywords: firefox browser web"]
        end
    end
    block:statusBar:1
        SB["[fuzzy] 4 results | ↑↓ navigate  Enter launch  F2 settings  Esc quit"]
    end
```

#### 메인 화면 (미리보기 OFF)

```mermaid
block-beta
    columns 1
    block:header2:1
        H2["⌨ key·mander v0.2.0 · 1234 items indexed"]
    end
    block:inputBar2:1
        IB2["⌨ Command: > @g rust tutorial_"]
    end
    block:content2:1
        R2a["▸ 🔍 Google: 'rust tutorial'  →  https://google.com/search?q=rust+tutorial"]
    end
    block:statusBar2:1
        SB2["[contains] 1 result | ↑↓ navigate  Enter open  Esc quit"]
    end
```

#### 빈 쿼리 (히스토리 표시)

```mermaid
block-beta
    columns 1
    block:header3:1
        H3["⌨ key·mander v0.2.0 · 1234 items indexed"]
    end
    block:inputBar3:1
        IB3["⌨ Command: > _"]
    end
    block:content3:1
        R3a["▸ 🕒 Firefox           [3x]"]
        R3b["  🕒 VS Code           [2x]"]
        R3c["  🕒 Slack             [1x]"]
    end
    block:statusBar3:1
        SB3["[fuzzy] Recent launches | ↑↓ navigate  Enter launch  Esc quit"]
    end
```

#### 폴더 드릴다운 화면

```mermaid
block-beta
    columns 1
    block:header4:1
        H4["kmd v0.2.0 | 5997 items indexed"]
    end
    block:inputBar4:1
        IB4["Search: > _"]
    end
    block:content4:1
        R4a["▸ 📁 2026년_프로젝트     [Dir]  C:\Users\...\Documents"]
        R4b["  📁 회의록              [Dir]  C:\Users\...\Documents"]
        R4c["  📘 업무계획_r1.hwp     [File] C:\Users\...\Documents"]
        R4d["  📊 예산.xlsx           [File] C:\Users\...\Documents"]
    end
    block:statusBar4:1
        SB4["[contains] 4 results | ↑↓ navigate  Tab/→ open folder  ←/Esc back  Enter launch"]
    end
```

- 단일 화면, 모드 전환 없음 (드릴다운은 스택 기반 상태 관리)
- 플러그인은 prefix(`:calc`, `:todo`)로 활성화
- 빈 쿼리 시 히스토리 표시 (🕒 Recent 타이틀)
- 폴더 선택 시 Tab/→로 드릴다운, ←/Esc로 복귀

---

## 4. 비기능 요구사항

| 항목 | 목표 |
|------|------|
| 바이너리 크기 | < 4MB (release, stripped) |
| 시작 시간 | < 100ms (인덱스 캐시 사용 시 < 50ms) |
| RAM 사용량 | < 10MB (인덱스 5000개 기준) |
| 지원 OS | Windows 10+, macOS 12+, Linux (glibc 2.17+) |
| 최소 터미널 | 80x24 |

---

## 5. 로드맵

```mermaid
gantt
    title keymander 로드맵
    dateFormat YYYY-MM-DD
    axisFormat %b

    section Phase 1 - Core + CLI
    kmd-core 라이브러리           :done, p1a, 2026-02-01, 7d
    CLI 서브커맨드                :done, p1b, after p1a, 7d

    section Phase 2 - TUI
    Ratatui 인터랙티브 런처       :done, p2, after p1b, 10d

    section Phase 3 - Plugin
    Extension trait + Calculator  :done, p3, after p2, 7d

    section Phase 4 - Daemon
    핫키 데몬 인프라              :done, p4, after p3, 7d

    section Phase 5 - Polish
    README, CI/CD, 릴리스         :done, p5, after p4, 5d

    section Phase 6 - Future
    글로벌 핫키 네이티브          :done, future1, after p5, 14d
    FTS 본문 검색                 :done, future1b, after p5, 14d
    테마 핫 리로드                :future2, after p5, 7d
    export/import                 :future2b, after p5, 7d
    공식 플러그인 (todo/memo)     :future3, after future1, 14d
```

### Phase 1: Core + CLI ✅
- kmd-core 라이브러리 (config, db, index, search, history, action)
- CLI 서브커맨드 (search, launch, index, config, history)
- 26개 유닛 테스트

### Phase 2: TUI ✅
- Ratatui 기반 인터랙티브 런처
- 실시간 검색, 히스토리 부스팅, @웹서비스
- 미리보기 패널, 테마 시스템

### Phase 3: Plugin System ✅
- Extension trait 정의
- JSON over stdin/stdout 프로토콜
- 플러그인 로더 (manifest.toml 기반)
- 내장 계산기 (`:calc`)

### Phase 4: Daemon (Infrastructure) ✅
- CLI 스텁 구현 (kmd daemon start/stop/status)
- OS별 바인딩 가이드 제공
- 글로벌 핫키 네이티브 등록 ✅ 완료 — `keybind/macos.rs`(CGEventTap)·
  `keybind/windows.rs`(LL 훅), 콤보 등록은 `kmd-daemon/src/server.rs`

### Phase 5: Polish ✅
- README, LICENSE
- CI/CD (GitHub Actions: check, test, clippy, fmt, release)
- 크로스플랫폼 릴리스 빌드

### Phase 6 ✅ 완료된 것
- 글로벌 핫키 데몬 네이티브 구현 — Phase 4 참조
- FTS (파일 본문 검색) — v0.15.0, `crates/kmd-core/src/content_index.rs`
- 클립보드 — 플러그인이 아니라 **네이티브로** 구현(`;`/`:clip`,
  `crates/kmd-daemon/src/clipboard.rs`). 아래 "공식 플러그인"에서 제외

### Phase 6 남은 것

**정본은 [17_feature_backlog.md](17_feature_backlog.md)다.** 여기 목록은 요약이다.

- 테마 핫 리로드 + TOML 외부 테마 (F15~F17) — 먼저 **TUI가 `general.theme`를
  읽게** 해야 한다. 현재는 설정만 있고 읽지 않는다
- export/import (설정+DB 아카이브) (F10)
- 공식 플러그인 kmd-todo / kmd-memo (F20) — Script Plugin 배선 결정에 종속
- 플러그인 레지스트리/마켓플레이스 — 🧊 **보류**(docs/17 §3)
