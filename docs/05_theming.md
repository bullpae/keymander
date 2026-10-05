# Theming Specification

## 1. 개요

> ⚠️ **현재 동작 (2026-10-05, v0.16.8 확인) — 문서와 코드가 갈려 있다.**
>
> | | 테마 선택 | `general.theme` 설정 |
> |---|---|---|
> | **데스크톱** (iced) | 내장 **5종** — Keymander/Midnight, Obsidian, Snow, Rose Pine, Nord | ✅ 읽는다 (`kmd-desktop/src/main.rs`의 `theme::from_name`) |
> | **TUI** (ratatui) | 단일 하드코딩 테마 | ❌ **읽지 않는다** — `src/tui/app.rs`가 `Theme::default()`를 쓴다 |
>
> 즉 **TUI에서 `general.theme`를 바꿔도 아무 일이 일어나지 않는다.**
> 설정 항목은 존재하고 저장도 되는데 소비되지 않는 **거짓 설정**이다.
> 아래 §2·§3은 TUI 테마를 전제로 쓰였으나, 그 전제가 아직 성립하지 않는다.
>
> 수정 우선순위: [17_feature_backlog.md](17_feature_backlog.md) **F15**(TUI가
> 설정을 읽게 하기) → **F16**(TOML 외부 테마 + 핫 리로드) → **F17**(저색상 폴백).

keymander의 색상과 스타일을 커스터마이즈하는 시스템.

---

## 2. 현재 구현 (v0.2)

### 2.1 Theme 구조체

```mermaid
classDiagram
    class Theme {
        +Color input_fg
        +Color input_border
        +Color list_selected_bg
        +Color list_selected_fg
        +Color list_normal_fg
        +Color kind_tag_fg
        +Color path_fg
        +Color status_fg
        +Color status_bg
        +Color header_fg
        +input_style() Style
        +input_border_style() Style
        +list_selected_style() Style
        +list_normal_style() Style
        +kind_tag_style() Style
        +path_style() Style
        +status_style() Style
        +header_style() Style
        +default_theme() Theme
    }
```

### 2.2 테마가 적용되는 영역

```mermaid
block-beta
    columns 1
    block:headerArea:1
        HeaderText["header_fg (Cyan Bold): kmd v0.2.0 | 1234 items"]
    end
    block:inputArea:1
        InputBorder["input_border (Cyan): 테두리"]
        InputText["input_fg (White): > fire_"]
    end
    block:contentArea:1
        columns 2
        block:listArea:1
            Selected["list_selected_bg/fg (DarkGray/White Bold): ▸ Firefox"]
            Normal["list_normal_fg (Gray): FileZilla"]
            KindTag["kind_tag_fg (DarkGray): [App]"]
            PathText["path_fg (DarkGray): /usr/bin/firefox"]
        end
        block:previewArea:1
            PreviewContent["list_normal_fg (Gray): Name, Type, Path..."]
        end
    end
    block:statusArea:1
        StatusText["status_fg/bg (DarkGray): [fuzzy] 4 results | ..."]
    end
```

### 2.3 기본 테마 값

| 요소 | 색상 | 수정자 |
|------|------|--------|
| input_fg | White | — |
| input_border | Cyan | — |
| list_selected_bg | DarkGray | — |
| list_selected_fg | White | Bold |
| list_normal_fg | Gray | — |
| kind_tag_fg | DarkGray | — |
| path_fg | DarkGray | — |
| status_fg | DarkGray | — |
| status_bg | Reset | — |
| header_fg | Cyan | Bold |

---

## 3. 향후 계획 — ⬜ 미구현

이 절 전체가 **설계이지 현재 동작이 아니다.** `themes/` 디렉터리 로드,
`load_theme`, `COLORTERM` 감지, 핫 리로드 모두 코드에 없다(grep 0건).

### 3.1 TOML 테마 파일

```toml
# ~/.config/kmd/themes/catppuccin-mocha.toml

[meta]
name = "Catppuccin Mocha"
author = "catppuccin"

[colors]
input_fg = "#CDD6F4"        # Text
input_border = "#89B4FA"     # Blue
list_selected_bg = "#45475A" # Surface1
list_selected_fg = "#CDD6F4" # Text
list_normal_fg = "#BAC2DE"   # Subtext1
kind_tag_fg = "#6C7086"      # Overlay0
path_fg = "#6C7086"          # Overlay0
status_fg = "#585B70"        # Surface2
status_bg = "#1E1E2E"        # Base
header_fg = "#89B4FA"        # Blue
```

### 3.2 테마 로드 흐름

```mermaid
flowchart TD
    Start["TUI 시작"] --> ReadConfig["config.toml<br/>general.theme 읽기"]
    ReadConfig --> CheckTheme{"theme == 'default'?"}
    CheckTheme -- Yes --> DefaultTheme["하드코딩된 기본 테마"]
    CheckTheme -- No --> FindFile["themes/{name}.toml 탐색"]
    FindFile --> Exists{"파일 존재?"}
    Exists -- Yes --> ParseTOML["TOML 파싱"]
    ParseTOML --> ValidColors{"색상값 유효?"}
    ValidColors -- Yes --> ApplyTheme["테마 적용"]
    ValidColors -- No --> Fallback["기본 테마로 폴백 + 경고"]
    Exists -- No --> Fallback
    DefaultTheme --> Render["렌더링"]
    ApplyTheme --> Render
    Fallback --> Render
```

### 3.3 테마 목록 — 내장된 것 vs 예정

**데스크톱에 이미 내장된 5종** (`crates/kmd-desktop/src/theme.rs`,
`general.theme` 값으로 선택):

| 값 | 설명 | 계열 |
|------|------|------|
| `keymander` \| `default` \| `midnight` | 기본 — 딥 네이비 + 일렉트릭 시안 | Dark |
| `obsidian` | 무채색 고대비 | Dark |
| `snow` \| `light` | 라이트 | Light |
| `rose_pine` | 뮤트 로즈 | Dark |
| `nord` | 차가운 파란 계열 | Dark |

알 수 없는 값은 조용히 `midnight`으로 폴백한다(`from_name`).

**예정** — 아직 없다:

| 이름 | 설명 | 계열 |
|------|------|------|
| catppuccin-mocha | 따뜻한 다크 테마 | Dark |
| catppuccin-latte | 밝은 라이트 테마 | Light |
| tokyo-night | VS Code 인기 테마 포트 | Dark |
| gruvbox | 레트로 컬러 | Dark |
| solarized-dark | 클래식 다크 테마 | Dark |

**TUI는 이 목록 전체와 무관하다** — §1 참조. F15가 먼저다.

### 3.4 색상 시스템

```mermaid
flowchart TD
    Detect["터미널 색상 지원 감지"] --> Check1{"COLORTERM=truecolor?"}
    Check1 -- Yes --> TrueColor["24-bit True Color<br/>Color::Rgb(r, g, b)<br/>1677만 색"]
    Check1 -- No --> Check2{"TERM=*-256color?"}
    Check2 -- Yes --> Color256["256색 팔레트<br/>Color::Indexed(n)<br/>256 색"]
    Check2 -- No --> Color16["16색 기본<br/>Color::Red 등<br/>16 색"]

    TrueColor --> Render["테마 색상 렌더링"]
    Color256 --> Downgrade256["24bit → 가장 가까운 256색 매핑"]
    Downgrade256 --> Render
    Color16 --> Downgrade16["256색 → 가장 가까운 16색 매핑"]
    Downgrade16 --> Render
```

### 3.5 테마 핫 리로드 (향후)

```mermaid
sequenceDiagram
    participant User
    participant FileWatcher as File Watcher
    participant ThemeLoader as Theme Loader
    participant TUI as TUI Renderer

    User->>User: themes/nord.toml 수정 저장
    FileWatcher->>ThemeLoader: 파일 변경 감지
    ThemeLoader->>ThemeLoader: TOML 재파싱
    ThemeLoader->>TUI: 새 Theme 적용
    TUI->>TUI: 다음 프레임에서 반영
    TUI-->>User: 실시간 색상 변경
```

---

## 4. 아이콘 시스템

### 4.1 현재 구현 (Unicode Emoji)

Nerd Font에 의존하지 않고 Unicode 이모지 사용:

| 확장자/종류 | 아이콘 | | 확장자/종류 | 아이콘 |
|-------------|--------|-|-------------|--------|
| .rs | 🦀 | | .json/.yaml/.toml | 📋 |
| .py | 🐍 | | 이미지 | 🖼️ |
| .js/.ts | 📜 | | 음악 | 🎵 |
| .go | 🔵 | | 동영상 | 🎬 |
| .java/.kt | ☕ | | 압축파일 | 📦 |
| .c/.cpp | ⚙️ | | 디렉토리 | 📁 |
| .sh/.bash | 🐚 | | 실행파일 | ⚡ |
| .md/.txt/.doc | 📝 | | 히스토리 | 🕒 |
| .pdf | 📕 | | 기타 | 📄 |

### 4.2 아이콘 설정 (emoji_icons)

config.toml:
```toml
[general]
emoji_icons = true   # 기본값: Unicode 이모지
# emoji_icons = false  # ASCII 폴백 (SSH / 레거시 터미널)
```

단일 매핑 테이블(ICON_TABLE)에 이모지+ASCII 쌍이 정의되어 있으며, `emoji_icons`가 false이면 ASCII 버전을 사용합니다.

### 4.3 향후: Nerd Font

Nerd Font 글리프 지원은 향후 계획입니다 (설치 필요).
