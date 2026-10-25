//! IPC 프로토콜 타입 및 소켓 경로 헬퍼
//!
//! 데몬 ↔ 클라이언트(kmd, kmd-desktop) 간 통신에 사용하는 메시지 정의.
//!
//! 와이어 프로토콜 (요청):
//!   line 1: <token-hex>
//!   line 2: <json-request>
//!
//! 토큰은 데몬 시작 시 32바이트 OS RNG로 생성되어 `daemon.port` 파일
//! 두 번째 줄에 저장된다. 동일 사용자만 읽을 수 있도록 Unix에서는
//! 0600 권한으로 기록한다. 인증 실패 시 서버는 응답 없이 연결을 종료한다.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── 요청 ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// 검색 쿼리 실행
    Search {
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    /// 인덱스 리빌드
    RebuildIndex,
    /// 데몬 상태 조회
    Status,
    /// 자동 시작 등록 상태 조회
    AutostartStatus,
    /// 자동 시작 등록
    AutostartEnable,
    /// 자동 시작 해제
    AutostartDisable,
    /// 데몬 종료
    Shutdown,
    /// 연결 확인
    Ping,
    /// LLM 오토파일럿 — URL 열기 + 전경창 검증 후 키 주입으로 자동 제출.
    /// 자동화 대상(chatgpt/claude/gemini)만 잡으로 오며, 데몬은 각 잡을
    /// 순차 처리하고 성공한 창을 세션 레지스트리에 기억한다 (docs/09).
    LlmAutopilot { jobs: Vec<LlmJob> },
    /// 이어서 질문 — 레지스트리에 기억된 LLM 창들에 후속 프롬프트 전달.
    LlmFollowup { prompt: String },
    /// [P3 스파이크] 현재 전경 앱에 붙여넣기(Cmd+V/Ctrl+V) 주입.
    /// 데스크톱이 클립보드를 세팅한 뒤 호출한다 — 클립보드 확장의 핵심 능력을
    /// 분리 구조에서 실증하기 위한 것 (docs/11).
    InjectPaste,
    /// 현재 위치 — `:길` 기본 출발지 (docs/20). 데몬이 메모리에만 들고 있다.
    Location,
    /// 클립보드 히스토리 n번째(1-기반) 항목을 붙여넣기 (docs/12).
    /// `to_previous=false`: 현재 전경 앱(흐름 A). `true`: 런처 열기 전 앱으로
    /// 포커스를 되돌린 뒤(흐름 B — 런처가 결과 선택 시 사용).
    ClipPaste {
        slot: usize,
        #[serde(default)]
        to_previous: bool,
    },
    /// 안정 ID로 클립보드 항목을 붙여넣는다. 슬롯 번호는 새 복사가 들어오면
    /// 밀리므로, 런처 검색 결과 선택은 반드시 이 요청을 사용한다 (docs/12).
    ClipPasteItem {
        id: u64,
        #[serde(default)]
        to_previous: bool,
    },
    /// 안정 ID의 항목을 붙여넣지 않고 시스템 클립보드에 복사만 한다
    /// (런처 Cmd/Ctrl+Enter — 사용자가 클립보드 교체를 명시한 동작).
    ClipCopyItem { id: u64 },
    /// 클립보드 히스토리 검색 (흐름 B 런처). 빈 쿼리는 전체(최신순).
    ClipHistory {
        #[serde(default)]
        query: String,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    /// [검증용] 현재 전경 앱을 흐름 B 붙여넣기 대상으로 기억한다.
    /// 실사용에서는 데몬이 런처 실행(Launch) 시점에 자동 캡처하므로 불필요.
    ClipCaptureForeground,
    /// [macOS] 키 주입 셀프테스트 — 마커 없는 합성 키를 주입해 데몬 탭의
    /// 리맵 출력(레이어 매핑·탭=한영·연타 디바운스)을 실 이벤트 경로에서
    /// 검증한다 (docs/14 Tier 2). `kmd daemon e2e`가 호출.
    KeybindSelfTest,
}

fn default_limit() -> usize {
    20
}

/// LLM 오토파일럿 주입 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmInject {
    /// URL이 프롬프트를 프리필함 — Enter만 주입 (ChatGPT, Claude)
    EnterOnly,
    /// 입력창에 붙여넣고 실행 — Ctrl+V → Enter (Gemini)
    PasteEnter,
}

/// LLM 오토파일럿 잡 한 건 (서비스 하나)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmJob {
    /// 서비스 식별자 — 세션 레지스트리 키 ("chatgpt" 등)
    pub service_id: String,
    /// 열 URL (프리필 포함)
    pub url: String,
    /// 원문 프롬프트 — PasteEnter 주입 및 폴백용
    pub prompt: String,
    /// 주입 방식
    pub method: LlmInject,
    /// 전경창 검증용 타이틀 마커 (하나라도 포함되면 매치)
    pub title_markers: Vec<String>,
}

// ── 응답 ─────────────────────────────────────────────────────────────────────

/// 클립보드 히스토리 검색 결과 항목 (docs/12 흐름 B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipHit {
    /// 데몬 수명 동안 변하지 않는 항목 식별자 — 붙여넣기는 ClipPasteItem.id로.
    /// 0 = 구버전 데몬 응답(id 없음) → 클라이언트는 slot 폴백을 쓴다.
    #[serde(default)]
    pub id: u64,
    /// 조회 시점의 1-기반 슬롯 번호 (표시/구버전 폴백용). 새 복사가 들어오면
    /// 밀리므로 실행 식별자로 쓰지 않는다.
    pub slot: usize,
    /// 첫 줄 미리보기 (최대 200자). 전체 내용은 응답에 담지 않는다.
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// 검색 결과
    SearchResults { items: Vec<SearchHit> },
    /// 클립보드 히스토리 검색 결과 (흐름 B)
    ClipHistory { hits: Vec<ClipHit> },
    /// 데몬 상태
    Status {
        uptime_secs: u64,
        index_items: usize,
        pid: u32,
        /// 실행 중인 키맵 레이어 요약 (구버전 데몬 응답에는 없음 → 기본 빈 목록)
        #[serde(default)]
        keymap_layers: Vec<String>,
        /// config.toml 로드 실패 메시지 — 있으면 데몬이 기본값으로 동작 중이라는
        /// 뜻이다 (구버전 데몬 응답에는 없음 → 기본 None)
        #[serde(default)]
        config_error: Option<String>,
        /// 키보드 훅 설치 실패 메시지 — 있으면 데몬은 살아 있지만 키/핫키가
        /// 하나도 잡히지 않는 상태다 (구버전 데몬 응답에는 없음 → 기본 None)
        #[serde(default)]
        keybind_error: Option<String>,
        /// 키보드 훅 생존 요약 (예: "정상 · 마지막 이벤트 0.4초 전 · 재설치 2회").
        /// OS가 LL 훅을 조용히 제거하면 데몬은 살아 있는데 키맵만 죽으므로,
        /// "실행 중" 표시만으로는 진단이 안 된다 (구버전 데몬 → 기본 None).
        #[serde(default)]
        hook_health: Option<String>,
    },
    /// 자동 시작 등록 상태
    AutostartStatus { installed: bool },
    /// 단순 성공
    Ok { message: String },
    /// 에러
    Error { message: String },
    /// Ping 응답
    Pong,
    /// 현재 위치 (`accuracy_m` = 정확도 반경, `age_secs` = 측정 후 지난 초)
    Location {
        lat: f64,
        lng: f64,
        accuracy_m: f64,
        age_secs: u64,
    },
}

impl Response {
    /// `Status` 응답을 사람이 읽는 줄 목록으로 만든다. 다른 변형이면 `None`.
    ///
    /// `kmd daemon status`(CLI)와 `kmd-daemon status`(데몬 바이너리)가 같은
    /// 화면을 보여주는데, 예전에는 양쪽이 각자 30줄을 destructure해 출력했다.
    /// 필드를 하나 늘릴 때 한쪽만 고치면 컴파일 에러(E0027)로 걸리거나 —
    /// 더 나쁘게는 — **한쪽에만 표시되는 조용한 불일치**가 났다. 실제로
    /// `hook_health` 추가 때 한 번 물렸다. 그래서 서식은 여기 한 곳에만 둔다.
    ///
    /// `extra_line`은 훅 상태 다음에 끼워 넣을 호출자별 줄이다
    /// (CLI = 로그 경로, 데몬 = 자동 시작 등록 여부).
    pub fn status_lines(&self, extra_line: Option<String>) -> Option<Vec<String>> {
        let Response::Status {
            uptime_secs,
            index_items,
            pid,
            keymap_layers,
            config_error,
            keybind_error,
            hook_health,
        } = self
        else {
            return None;
        };

        let mut lines = vec![
            "데몬 상태: 실행 중".to_string(),
            format!("  PID:        {pid}"),
            format!("  가동 시간:  {uptime_secs}초"),
            format!("  인덱스:     {index_items}개 항목"),
        ];

        // 훅이 죽었으면 아래 레이어 목록은 config에서 온 설정값일 뿐 실제 동작이
        // 아니다 — 오해하지 않도록 목록보다 먼저 못 박는다.
        if keybind_error.is_some() {
            lines.push("  키 훅:      ❌ 미동작 (아래 키 입력이 가로채지지 않습니다)".to_string());
        }
        for layer in keymap_layers {
            lines.push(format!("  레이어:     {layer}"));
        }
        if let Some(hook) = hook_health {
            lines.push(format!("  키보드 훅:  {hook}"));
        }
        lines.extend(extra_line);
        if let Some(err) = keybind_error {
            lines.push(format!("  ⚠ 키 훅:    {err}"));
        }
        if let Some(err) = config_error {
            lines.push(format!("  ⚠ 설정:     {err}"));
            lines.push(
                "              → 데몬이 기본 설정으로 동작 중입니다. config.toml을 고친 뒤 재시작하세요."
                    .to_string(),
            );
        }
        Some(lines)
    }
}

/// IPC 전송용 검색 결과 아이템
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub icon: String,
    pub score: u32,
}

// ── 경로 헬퍼 ────────────────────────────────────────────────────────────────

/// 데몬 런타임 파일 디렉터리.
///
/// 포터블 모드(kmd-data/)와 무관하게 **항상 OS 표준 사용자 데이터 디렉터리**를
/// 쓴다. daemon.port에는 인증 토큰이 들어 있는데, 포터블 설치 위치(USB,
/// C:\ 아래 공용 폴더 등)는 다른 로컬 계정이 읽을 수 있어 토큰이 노출된다.
/// 런타임 파일은 재부팅/재시작마다 새로 만드는 일회성이므로 호스트에 두어도
/// 포터블 설치의 이동성(설정·데이터는 kmd-data/ 유지)을 해치지 않는다.
pub fn runtime_dir() -> PathBuf {
    crate::Config::standard_data_dir()
}

/// 데몬 런타임 파일 경로. 형식:
/// ```text
/// <port>
/// <token-hex>
/// ```
pub fn port_file_path() -> PathBuf {
    runtime_dir().join("daemon.port")
}

/// 데몬 PID 파일 경로
pub fn pid_file_path() -> PathBuf {
    runtime_dir().join("daemon.pid")
}

/// 데몬 로그 파일 경로 (시작마다 새로 씀)
pub fn log_file_path() -> PathBuf {
    runtime_dir().join("daemon.log")
}

// ── 인증 토큰 ────────────────────────────────────────────────────────────────

/// 32바이트 OS RNG에서 hex 64자 토큰 생성
pub fn generate_token() -> String {
    use std::fmt::Write;
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    let mut s = String::with_capacity(64);
    for b in buf {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// 상수 시간 바이트 비교 — 타이밍 공격 방지
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 데몬 런타임 파일을 (port, token)으로 파싱
pub fn read_runtime() -> Result<(u16, String), IpcError> {
    let content = std::fs::read_to_string(port_file_path()).map_err(|_| IpcError::NoDaemon)?;
    let mut lines = content.lines();
    let port: u16 = lines
        .next()
        .ok_or(IpcError::NoDaemon)?
        .trim()
        .parse()
        .map_err(|_| IpcError::NoDaemon)?;
    let token = lines.next().ok_or(IpcError::NoDaemon)?.trim().to_string();
    if token.is_empty() {
        return Err(IpcError::NoDaemon);
    }
    Ok((port, token))
}

// ── 직렬화 헬퍼 ──────────────────────────────────────────────────────────────

/// 요청을 JSON Line 형식으로 직렬화
pub fn encode_request(req: &Request) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string(req)?;
    s.push('\n');
    Ok(s)
}

/// 응답을 JSON Line 형식으로 직렬화
pub fn encode_response(res: &Response) -> Result<String, serde_json::Error> {
    let mut s = serde_json::to_string(res)?;
    s.push('\n');
    Ok(s)
}

/// JSON Line에서 요청 역직렬화
pub fn decode_request(line: &str) -> Result<Request, serde_json::Error> {
    serde_json::from_str(line.trim())
}

/// JSON Line에서 응답 역직렬화
pub fn decode_response(line: &str) -> Result<Response, serde_json::Error> {
    serde_json::from_str(line.trim())
}

// ── 에러 타입 ────────────────────────────────────────────────────────────────

/// IPC 통신 오류
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// 데몬이 실행 중이지 않음 (포트 파일 없음 / 파싱 실패)
    #[error("데몬이 실행 중이지 않습니다")]
    NoDaemon,
    #[error("IO 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 오류: {0}")]
    Json(#[from] serde_json::Error),
}

// ── 클라이언트 헬퍼 ─────────────────────────────────────────────────────────

/// 포트 파일에서 데몬 포트 읽기 (실패 시 에러 반환)
pub fn read_port() -> Result<u16, IpcError> {
    read_runtime().map(|(port, _)| port)
}

/// 데몬이 실행 중이면 포트를 반환
pub fn daemon_port() -> Option<u16> {
    read_port().ok()
}

/// 오래된 런타임 파일(port/pid) 정리
pub fn cleanup_stale_files() {
    let _ = std::fs::remove_file(port_file_path());
    let _ = std::fs::remove_file(pid_file_path());
}

/// 데몬에 요청 전송 후 응답 반환 (실패 시 에러 반환). 응답 대기 5초.
pub fn send_request_result(request: &Request) -> Result<Response, IpcError> {
    send_request_with_timeout(request, std::time::Duration::from_secs(5))
}

/// [`send_request_result`]의 응답 대기 시간 지정판 — 키 주입 셀프테스트처럼
/// 데몬이 수 초 이상 작업한 뒤 응답하는 요청에 쓴다.
pub fn send_request_with_timeout(
    request: &Request,
    read_timeout: std::time::Duration,
) -> Result<Response, IpcError> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let (port, token) = read_runtime()?;
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).map_err(IpcError::Io)?;
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(IpcError::Io)?;

    // line 1: token, line 2: JSON request
    let mut payload = String::with_capacity(token.len() + 64);
    payload.push_str(&token);
    payload.push('\n');
    let encoded = encode_request(request).map_err(IpcError::Json)?;
    payload.push_str(&encoded);

    stream.write_all(payload.as_bytes()).map_err(IpcError::Io)?;
    stream.flush().map_err(IpcError::Io)?;

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(IpcError::Io)?;

    decode_response(&line).map_err(IpcError::Json)
}

/// 데몬에 요청 전송 후 응답 반환. 연결 실패 시 None.
pub fn send_request(request: &Request) -> Option<Response> {
    send_request_result(request).ok()
}

/// 데몬이 실행 중인지 확인 (Ping 시도)
pub fn is_daemon_running() -> bool {
    matches!(send_request(&Request::Ping), Some(Response::Pong))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_roundtrip() {
        let req = Request::Search {
            query: "firefox".into(),
            limit: 10,
        };
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        match decoded {
            Request::Search { query, limit } => {
                assert_eq!(query, "firefox");
                assert_eq!(limit, 10);
            }
            _ => panic!("잘못된 타입"),
        }
    }

    #[test]
    fn test_response_roundtrip() {
        let res = Response::Status {
            uptime_secs: 42,
            index_items: 1000,
            pid: 12345,
            keymap_layers: vec!["nav: LAlt".into()],
            config_error: None,
            keybind_error: None,
            hook_health: None,
        };
        let encoded = encode_response(&res).unwrap();
        let decoded = decode_response(&encoded).unwrap();
        match decoded {
            Response::Status {
                uptime_secs,
                index_items,
                pid,
                keymap_layers,
                ..
            } => {
                assert_eq!(uptime_secs, 42);
                assert_eq!(index_items, 1000);
                assert_eq!(pid, 12345);
                assert_eq!(keymap_layers.len(), 1);
            }
            _ => panic!("잘못된 타입"),
        }
    }

    #[test]
    fn test_status_decodes_without_keymap_layers() {
        // 구버전 데몬 응답(keymap_layers 없음)과의 하위 호환
        let old = r#"{"type":"Status","uptime_secs":1,"index_items":2,"pid":3}"#;
        match decode_response(old).unwrap() {
            Response::Status {
                keymap_layers,
                config_error,
                keybind_error,
                ..
            } => {
                assert!(keymap_layers.is_empty());
                assert!(config_error.is_none());
                assert!(keybind_error.is_none());
            }
            _ => panic!("잘못된 타입"),
        }
    }

    #[test]
    fn clip_paste_item_roundtrip() {
        let req = Request::ClipPasteItem {
            id: 42,
            to_previous: true,
        };
        let encoded = encode_request(&req).unwrap();
        match decode_request(&encoded).unwrap() {
            Request::ClipPasteItem { id, to_previous } => {
                assert_eq!(id, 42);
                assert!(to_previous);
            }
            other => panic!("잘못된 타입: {other:?}"),
        }
    }

    #[test]
    fn clip_copy_item_roundtrip() {
        let req = Request::ClipCopyItem { id: 7 };
        let encoded = encode_request(&req).unwrap();
        assert!(matches!(
            decode_request(&encoded).unwrap(),
            Request::ClipCopyItem { id: 7 }
        ));
    }

    #[test]
    fn clip_hit_구버전_응답은_id_0으로_디코드된다() {
        // 구버전 데몬(id 필드 없음) 응답과의 하위 호환 — 클라이언트는 slot 폴백
        let old = r#"{"type":"ClipHistory","hits":[{"slot":3,"preview":"p"}]}"#;
        match decode_response(old).unwrap() {
            Response::ClipHistory { hits } => {
                assert_eq!(hits[0].id, 0);
                assert_eq!(hits[0].slot, 3);
            }
            _ => panic!("잘못된 타입"),
        }
    }

    #[test]
    fn test_ping_pong() {
        let req = Request::Ping;
        let encoded = encode_request(&req).unwrap();
        assert!(encoded.contains("Ping"));

        let res = Response::Pong;
        let encoded = encode_response(&res).unwrap();
        assert!(encoded.contains("Pong"));
    }

    fn status_sample(hook_health: Option<&str>, keybind_error: Option<&str>) -> Response {
        Response::Status {
            uptime_secs: 42,
            index_items: 7,
            pid: 1234,
            keymap_layers: vec!["nav: CapsLock 홀드".into()],
            config_error: None,
            keybind_error: keybind_error.map(str::to_string),
            hook_health: hook_health.map(str::to_string),
        }
    }

    #[test]
    fn status_lines는_호출자_줄을_훅_상태_다음에_끼운다() {
        let lines = status_sample(Some("설치됨"), None)
            .status_lines(Some("  로그:       /tmp/daemon.log".into()))
            .expect("Status 응답");

        let hook = lines.iter().position(|l| l.contains("키보드 훅")).unwrap();
        let extra = lines.iter().position(|l| l.contains("로그:")).unwrap();
        assert_eq!(extra, hook + 1, "호출자 줄은 훅 상태 바로 다음");
        assert!(lines[0].contains("실행 중"));
        assert!(lines.iter().any(|l| l.contains("1234")), "PID 표시");
    }

    #[test]
    fn 훅_실패는_레이어_목록보다_먼저_경고한다() {
        // 훅이 죽으면 레이어 목록은 config 설정값일 뿐 실제 동작이 아니다
        let lines = status_sample(None, Some("SetWindowsHookEx 실패"))
            .status_lines(None)
            .expect("Status 응답");

        let warn = lines.iter().position(|l| l.contains("미동작")).unwrap();
        let layer = lines.iter().position(|l| l.contains("레이어:")).unwrap();
        assert!(warn < layer, "경고가 레이어 목록보다 앞");
        assert!(lines.iter().any(|l| l.contains("SetWindowsHookEx 실패")));
    }

    #[test]
    fn status가_아닌_응답은_none() {
        assert!(Response::Pong.status_lines(None).is_none());
    }
}
