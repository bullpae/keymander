//! 바이트열을 텍스트로 읽는 규칙 — UTF-8 우선, 아니면 CP949.
//!
//! 한국어 Windows에는 CP949(EUC-KR의 상위집합)로 저장된 텍스트 파일과, 콘솔
//! 코드페이지(949)로 출력하는 명령이 흔하다. UTF-8로만 읽으면 한글이 `�`로
//! 깨진다. 본문 색인(`content_index`)과 셸 출력(`builtin_shell`)이 같은 규칙을
//! 쓰도록 여기 한 곳에 둔다.

/// 문자열을 NFC(완성형)로 정규화한다. 이미 NFC면 복사하지 않는다.
///
/// macOS는 한글 파일명을 **NFD(자모 분리)**로 저장하는 경우가 많다 — Safari
/// 다운로드, AirDrop, HFS+ 시절 파일 등. 화면에는 똑같이 보이지만 `미닉스`(NFC,
/// 3글자)와 디스크의 `미닉스`(NFD, 7글자)는 서로 다른 문자열이라 검색이 0건이
/// 된다. 매칭에 쓰는 문자열은 양쪽 다 이 함수를 거친다. **경로 자체는 바꾸지
/// 않는다** — 파일을 열 때는 원래 바이트가 필요하다(Linux는 NFC/NFD를 다른
/// 이름으로 본다).
pub fn nfc(s: &str) -> std::borrow::Cow<'_, str> {
    use unicode_normalization::{is_nfc_quick, IsNormalized, UnicodeNormalization};
    if is_nfc_quick(s.chars()) == IsNormalized::Yes {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(s.nfc().collect())
    }
}

/// 디코딩 결과.
pub struct Decoded {
    pub text: String,
    /// CP949로 읽었는데 깨진 문자(U+FFFD)가 10%를 넘는다 — 사실상 텍스트가 아니다.
    pub mostly_garbage: bool,
}

/// UTF-8로 읽고, 실패하면 CP949로 읽는다.
pub fn decode_utf8_or_cp949(bytes: Vec<u8>) -> Decoded {
    match String::from_utf8(bytes) {
        Ok(text) => Decoded {
            text,
            mostly_garbage: false,
        },
        Err(e) => {
            let bytes = e.into_bytes();
            // encoding_rs의 EUC_KR은 WHATWG 정의상 windows-949(UHC) — CP949 전체를 덮는다.
            let (decoded, _, had_errors) = encoding_rs::EUC_KR.decode(&bytes);
            let mostly_garbage = had_errors && {
                let bad = decoded.chars().filter(|&c| c == '\u{FFFD}').count();
                bad * 10 > decoded.chars().count().max(1)
            };
            Decoded {
                text: decoded.into_owned(),
                mostly_garbage,
            }
        }
    }
}

/// 외부 명령의 출력을 읽는다.
///
/// Windows에선 콘솔 코드페이지로 출력하는 명령이 있어 CP949 폴백을 쓴다.
/// 다른 OS에서 UTF-8이 아닌 출력은 CP949라는 근거가 없으므로 `�` 치환으로 둔다
/// (엉뚱한 한글로 바꿔 보여주는 것보다 깨졌다는 걸 보여주는 게 낫다).
pub fn decode_command_output(bytes: Vec<u8>) -> String {
    if cfg!(windows) {
        decode_utf8_or_cp949(bytes).text
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfd_한글은_nfc로_합친다() {
        let nfd = "\u{1106}\u{1175}\u{1102}\u{1175}\u{11A8}\u{1109}\u{1173}"; // 미닉스 (NFD)
        assert_ne!(nfd, "미닉스", "화면엔 같아 보여도 다른 문자열이다");
        assert_eq!(nfc(nfd), "미닉스");
        assert!(
            matches!(nfc("미닉스"), std::borrow::Cow::Borrowed(_)),
            "NFC는 복사 없음"
        );
        assert_eq!(nfc("cafe\u{301}"), "café", "라틴 결합 문자도");
    }

    #[test]
    fn utf8은_그대로() {
        let d = decode_utf8_or_cp949("안녕 hello".as_bytes().to_vec());
        assert_eq!(d.text, "안녕 hello");
        assert!(!d.mostly_garbage);
    }

    #[test]
    fn cp949는_한글로_읽는다() {
        // "가나다" in CP949
        let d = decode_utf8_or_cp949(vec![0xB0, 0xA1, 0xB3, 0xAA, 0xB4, 0xD9]);
        assert_eq!(d.text, "가나다");
        assert!(!d.mostly_garbage);
    }

    #[test]
    fn 잡음_바이트는_텍스트가_아니라고_본다() {
        let d = decode_utf8_or_cp949(vec![0xFF; 64]);
        assert!(d.mostly_garbage);
    }
}
