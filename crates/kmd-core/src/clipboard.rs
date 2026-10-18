//! 시스템 클립보드 접근 — UI 크레이트(데스크톱·TUI·CLI)가 공유한다.
//!
//! 예전엔 호출부마다 arboard를 직접 감싸 구현이 6벌이었고, 일부는 결과를 버리고
//! 무조건 "복사했습니다"라고 표시했다 — 클립보드가 막힌 환경(원격 세션·권한 제한)에서
//! 복사된 줄 알고 붙여넣다 낭패를 본다. 쓰기는 실패를 그대로 돌려준다.

/// 클립보드에 텍스트를 쓴다. 실패 사유를 돌려주므로 호출부가 사용자에게 알려야 한다.
pub fn set_text(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// 클립보드의 텍스트 (없거나 읽을 수 없으면 None)
pub fn get_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
