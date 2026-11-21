//! Windows: 버전 정보·아이콘. macOS: 실행 파일에 Info.plist 를 넣는다 (`__TEXT,__info_plist`).
//!
//! 데몬은 `.app` 꾸러미가 아닌 실행 파일 하나라, CoreLocation 이 권한 창에 보여 줄
//! 사용 목적 문구(NSLocationUsageDescription)와 번들 ID 를 여기서 읽는다 (docs/20).
//! 번들 ID 는 배포 스크립트의 codesign `--identifier` 와 같아야 한다.

use std::path::PathBuf;

/// Windows: 버전 정보와 아이콘. 키보드 훅을 거는 실행 파일이 이름표 없이 배포되면
/// 백신이 더 의심한다 — `kmd.exe`·`kmd-desktop.exe` 와 같은 정보를 넣는다.
#[cfg(windows)]
fn windows_resource() {
    let ico_path = "../../assets/icon.ico";
    let mut res = winresource::WindowsResource::new();
    res.set("OriginalFilename", "kmd-daemon.exe");
    res.set(
        "FileDescription",
        "keymander Daemon - key remapping and search service",
    );
    res.set("ProductName", "keymander");
    if std::path::Path::new(ico_path).exists() {
        res.set_icon(ico_path);
    }
    if let Err(e) = res.compile() {
        eprintln!("winresource: {e}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    windows_resource();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let why =
        "카카오맵 길찾기(:길)의 출발지를 현재 위치로 채우는 데 씁니다. 위치는 저장하지 않습니다.";
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.keymander.kmd-daemon</string>
    <key>CFBundleName</key>
    <string>kmd-daemon</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>NSLocationUsageDescription</key>
    <string>{why}</string>
    <key>NSLocationWhenInUseUsageDescription</key>
    <string>{why}</string>
</dict>
</plist>
"#
    );
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("Info.plist");
    std::fs::write(&out, plist).expect("Info.plist 쓰기");
    println!(
        "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
        out.display()
    );
}
