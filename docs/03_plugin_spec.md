# Plugin System Specification

## 1. 개요

keymander 플러그인은 두 가지 형태로 존재한다:

```mermaid
graph TB
    subgraph PluginTypes ["플러그인 유형"]
        direction LR
        BuiltIn["내장 Extension<br/>Rust trait 구현<br/>바이너리에 포함"]
        ScriptPlugin["외부 Script Plugin<br/>별도 프로세스<br/>JSON stdin/stdout 통신"]
    end

    BuiltIn --> Calc[":calc — 계산기"]
    ScriptPlugin --> Todo[":todo — 할일 관리"]
    ScriptPlugin --> Memo[":memo — 메모"]
    ScriptPlugin --> Custom["사용자 정의 플러그인"]
```

---

## 2. Extension Trait (내장 플러그인)

### 2.1 인터페이스

```rust
pub trait Extension: Send + Sync {
    /// 플러그인 이름
    fn name(&self) -> &str;

    /// 활성화 prefix (e.g. ":calc", ":todo")
    /// None이면 글로벌 검색에 참여
    fn prefix(&self) -> Option<&str>;

    /// 검색 결과 반환
    fn search(&self, query: &str) -> Vec<IndexItem>;

    /// 선택 항목 실행
    fn execute(&self, item: &IndexItem) -> ExtensionAction;
}

pub enum ExtensionAction {
    Display(String),          // 결과 표시
    CopyToClipboard(String),  // 클립보드 복사
    OpenUrl(String),          // URL 열기
    Noop,                     // 아무것도 안 함
}
```

### 2.2 내장 플러그인: Calculator

```mermaid
flowchart LR
    Input[":calc 2 + 3 * 4"] --> Tokenize["토크나이저<br/>숫자, 연산자, 괄호"]
    Tokenize --> Parse["재귀 하강 파서<br/>expr → term → factor"]
    Parse --> Eval["평가<br/>연산자 우선순위 적용"]
    Eval --> Result["= 14"]
    Result --> Action["Enter → 클립보드 복사"]
```

지원 연산:

| 연산자 | 의미 | 예시 |
|--------|------|------|
| `+` | 덧셈 | `2 + 3` → `5` |
| `-` | 뺄셈 / 단항 마이너스 | `10 - 3` → `7`, `-5` → `-5` |
| `*` | 곱셈 | `4 * 5` → `20` |
| `/` | 나눗셈 | `15 / 3` → `5` |
| `%` | 나머지 | `10 % 3` → `1` |
| `()` | 그룹 | `(2 + 3) * 4` → `20` |

---

## 3. Script Plugin (외부 플러그인)

> ⚠️ **이 절은 스펙이며, 아직 배선되지 않았다** (2026-10-05, v0.16.8 확인).
> `crates/kmd-core/src/plugin/protocol.rs`에 타입만 정의돼 있고 **`protocol::`
> 참조가 0건**이다 — spawn·search·execute 어디에도 연결돼 있지 않다.
> `loader.rs`도 `discover_plugins`/`default_plugin_dir` 두 함수뿐이고,
> `kmd plugin`은 `list` 서브커맨드만 있다.
>
> 즉 **플러그인을 만들어 넣어도 로드되지 않는다.** 아래 내용은 "이렇게 하기로
> 정했다"는 설계이지 현재 동작이 아니다.
>
> 배선 여부는 **의도적 보류** 상태다 — 사용자 기반이 없는데 프로세스 격리·
> 타임아웃 유지비만 남는다는 판단. 사유는
> [17_feature_backlog.md](17_feature_backlog.md) §3.

### 3.1 디렉토리 구조

```mermaid
graph TB
    PluginsDir["plugins/"] --> ExampleDir["kmd-example/"]
    ExampleDir --> Manifest["manifest.toml<br/>메타데이터 (필수)"]
    ExampleDir --> Script["main.py<br/>실행 파일 (또는 바이너리)"]
```

### 3.2 manifest.toml

```toml
name = "kmd-example"
version = "0.1.0"
description = "An example plugin"
prefix = ":example"          # 활성화 prefix
command = "python main.py"   # 실행 명령 (플러그인 디렉토리 기준)
timeout_secs = 5             # 타임아웃 (기본 5초, 최대 30초)
```

### 3.3 통신 프로토콜

```mermaid
sequenceDiagram
    actor User
    participant kmd
    participant Plugin as Plugin Process

    User->>kmd: ":example hello"
    kmd->>Plugin: spawn(command)
    kmd->>Plugin: stdin: {"type":"search","query":"hello"}
    Plugin-->>kmd: stdout: {"type":"results","items":[...]}
    kmd-->>User: 결과 리스트 표시

    User->>kmd: Enter (항목 선택)
    kmd->>Plugin: stdin: {"type":"execute","item_id":"item-123"}
    Plugin-->>kmd: stdout: {"type":"action","action":{"type":"display","text":"Hello!"}}
    kmd-->>User: 결과 표시

    Note over kmd,Plugin: timeout_secs 초과 시 프로세스 kill
```

#### Request (kmd → plugin, stdin JSON)

```json
{"type": "search", "query": "hello world"}
```

```json
{"type": "execute", "item_id": "item-123"}
```

#### Response (plugin → kmd, stdout JSON)

```json
{
  "type": "results",
  "items": [
    {
      "id": "item-123",
      "name": "Hello World",
      "description": "A greeting",
      "icon": "👋"
    }
  ]
}
```

```json
{
  "type": "action",
  "action": {"type": "copy", "text": "copied text"}
}
```

### 3.4 생명주기

```mermaid
stateDiagram-v2
    [*] --> Idle: kmd 시작
    Idle --> Discover: plugin_dir 스캔
    Discover --> Registered: manifest.toml 파싱

    Registered --> Spawned: 사용자 prefix 입력
    Spawned --> SearchReq: stdin: search request
    SearchReq --> SearchRes: stdout: results
    SearchRes --> Display: 결과 표시

    Display --> ExecuteReq: Enter 선택
    ExecuteReq --> ActionRes: stdout: action
    ActionRes --> Applied: 액션 적용

    Applied --> Idle: 완료
    Spawned --> Killed: timeout 초과
    Killed --> Idle
```

### 3.5 보안

| 정책 | 내용 |
|------|------|
| 프로세스 격리 | 각 플러그인은 별도 프로세스로 실행 |
| 타임아웃 | 기본 5초, manifest에서 최대 30초 설정 가능 |
| 환경 | 플러그인 디렉토리를 CWD로 설정, 최소 환경변수 전달 |
| 파일시스템 | 별도 샌드박스 없음 (사용자 책임) |

---

## 4. 플러그인 관리

### 4.1 CLI

```bash
kmd plugin list              # 설치된 플러그인 목록
# (향후)
kmd plugin install <name>    # 플러그인 설치
kmd plugin remove <name>     # 플러그인 제거
kmd plugin update <name>     # 플러그인 업데이트
```

### 4.2 디스커버리 흐름

```mermaid
flowchart TD
    Start["kmd plugin list"] --> ScanDir["plugin_dir 스캔"]
    ScanDir --> ForEach["각 하위 디렉토리"]
    ForEach --> CheckManifest{"manifest.toml 존재?"}
    CheckManifest -- No --> Skip["건너뛰기"]
    CheckManifest -- Yes --> ParseTOML["TOML 파싱"]
    ParseTOML --> Valid{"유효한 매니페스트?"}
    Valid -- No --> Warn["경고 로그"]
    Valid -- Yes --> Register["PluginManifest 등록"]
    Register --> Display["목록 출력"]
```

기본 디렉토리:
- Linux: `~/.local/share/kmd/plugins/`
- macOS: `~/Library/Application Support/kmd/plugins/`
- Windows: `%LOCALAPPDATA%/kmd/plugins/`

---

## 5. 향후 계획

### 5.1 예정 공식 플러그인

| 이름 | Prefix | 기능 |
|------|--------|------|
| kmd-todo | `:todo` | 할일 관리 (SQLite) |
| kmd-memo | `:memo` | 메모 관리 (SQLite) |
| kmd-clipboard | `:clip` | 클립보드 히스토리 |
| kmd-snippet | `:snip` | 텍스트 스니펫 확장 |

### 5.2 플러그인 레지스트리

향후 중앙 레지스트리 도입 가능:
- `kmd plugin install kmd-todo` → GitHub 릴리스에서 다운로드
- `manifest.toml`에 `repository` 필드 추가
- 버전 관리 및 업데이트 알림
