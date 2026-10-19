//! macOS 패키지(번들) 판정 — `.app` 등은 폴더지만 항목 하나로 다룬다 (docs/18 §9)
//!
//! Finder는 `Foo.app`·`Doc.pages`·`Photos Library.photoslibrary` 같은 "패키지" 폴더를
//! 파일 하나처럼 보여 준다. 파일 스캐너가 이걸 일반 폴더로 보고 내려가면 번들 내부
//! (`Contents/Info.plist`, `Resources/*.icns`, `*.lproj`…)가 앱 하나에 수백~수천 개씩
//! 인덱스에 들어가 결과를 덮고 할당량을 먹는다. 그래서 스캐너는 패키지를 만나면
//! 그 폴더 하나만 항목으로 넣고 안으로는 내려가지 않는다.
//!
//! 판정은 확장자 목록 + `Contents/Info.plist` 존재로 한다 — 코어에는 Objective-C
//! 의존성이 없어 UTType(`com.apple.package`) 조회를 쓰지 않는다. 동작은 macOS에서만
//! 켜고([`enabled`]), 판정 함수([`classify`])는 테스트를 위해 모든 OS에서 돈다.

use std::path::Path;

/// 패키지 종류 — 인덱스에 어떤 항목으로 넣을지 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    /// `.app` — 앱 항목 (이름은 `.app`을 뗀 것)
    App,
    /// 문서 패키지(`.pages` 등)·라이브러리·개발 번들 — 파일 하나로
    Opaque,
}

/// 앱 번들 확장자 — `Contents/Info.plist`가 있을 때만 번들로 본다
/// (개발자 소스 폴더 이름이 `web.app`인 경우를 앱으로 오판하지 않게).
const APP_EXTENSIONS: &[&str] = &["app"];

/// 확장자만으로 패키지로 보는 목록 — 문서, 앱 전용 보관함, 개발·시스템 번들.
const OPAQUE_EXTENSIONS: &[&str] = &[
    // iWork·리치 텍스트 문서
    "pages",
    "numbers",
    "key",
    "rtfd",
    // 앱 전용 보관함 — 내부를 건드리면 안 된다 (사진 보관함은 수십만 개 파일)
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "fcpbundle",
    "logicx",
    // 개발·시스템 번들
    "framework",
    "bundle",
    "plugin",
    "kext",
    "appex",
    "xpc",
    "prefpane",
    "qlgenerator",
    "mdimporter",
    "saver",
    "xcodeproj",
    "xcworkspace",
    "playground",
    "dsym",
    "sparsebundle",
];

/// 패키지 처리를 켤지 — macOS만.
pub fn enabled() -> bool {
    cfg!(target_os = "macos") || test_force::get()
}

/// 테스트 전용 — 이 스레드에서만 macOS 동작을 강제로 켠다 (Windows·Linux CI에서
/// 스캐너의 패키지 분기를 검증하려고). 다른 테스트 스레드에는 영향이 없다.
#[cfg(test)]
pub(crate) mod test_force {
    use std::cell::Cell;
    thread_local!(static FORCE: Cell<bool> = const { Cell::new(false) });
    pub(crate) fn get() -> bool {
        FORCE.with(Cell::get)
    }
    pub(crate) fn set(on: bool) {
        FORCE.with(|f| f.set(on));
    }
}
#[cfg(not(test))]
mod test_force {
    pub(super) fn get() -> bool {
        false
    }
}

/// `path`(폴더)가 패키지인지 — OS와 무관한 순수 판정. 확장자가 없으면 디스크를 보지 않는다.
pub fn classify(path: &Path) -> Option<PackageKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if APP_EXTENSIONS.contains(&ext.as_str()) {
        return has_info_plist(path).then_some(PackageKind::App);
    }
    if OPAQUE_EXTENSIONS.contains(&ext.as_str()) {
        return path.is_dir().then_some(PackageKind::Opaque);
    }
    // 목록에 없는 확장자라도 번들 구조면 패키지 (서드파티 번들 대비)
    has_info_plist(path).then_some(PackageKind::Opaque)
}

/// 스캐너용 — macOS에서만 판정, 그 밖에서는 항상 None.
pub fn detect(path: &Path) -> Option<PackageKind> {
    if enabled() {
        classify(path)
    } else {
        None
    }
}

/// 순회 필터 — 이 항목의 **부모**가 패키지면 true (= 이 항목은 버린다).
///
/// walkdir의 `filter_entry`는 거른 항목의 자식으로도 내려가지 않으므로, 패키지의
/// 직계 자식만 막으면 번들 내부 전체가 빠지고 패키지 자신은 남는다.
pub fn parent_is_package(path: &Path) -> bool {
    enabled() && path.parent().is_some_and(|p| classify(p).is_some())
}

/// 경로 중간에 패키지가 끼어 있는지 — 외부 프로바이더(fd·locate 등) 결과용 사후 필터.
/// 마지막 구성요소(자기 자신)는 보지 않는다.
pub fn inside_package(path: &Path) -> bool {
    enabled() && path.ancestors().skip(1).any(|a| classify(a).is_some())
}

fn has_info_plist(path: &Path) -> bool {
    path.join("Contents").join("Info.plist").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(root: &Path, name: &str) -> std::path::PathBuf {
        let app = root.join(name);
        std::fs::create_dir_all(app.join("Contents").join("MacOS")).unwrap();
        std::fs::write(app.join("Contents").join("Info.plist"), "<plist/>").unwrap();
        app
    }

    #[test]
    fn 판정_앱은_info_plist가_있어야_한다() {
        let tmp = tempfile::tempdir().unwrap();
        let app = make_app(tmp.path(), "Foo.app");
        let fake = tmp.path().join("web.app");
        std::fs::create_dir(&fake).unwrap();
        let doc = tmp.path().join("Report.pages");
        std::fs::create_dir(&doc).unwrap();
        let plain = tmp.path().join("Projects");
        std::fs::create_dir(&plain).unwrap();

        assert_eq!(classify(&app), Some(PackageKind::App));
        assert_eq!(
            classify(&fake),
            None,
            "Info.plist 없는 .app 폴더는 일반 폴더"
        );
        assert_eq!(classify(&doc), Some(PackageKind::Opaque));
        assert_eq!(classify(&plain), None);
        let upper = make_app(tmp.path(), "Bar.APP");
        assert_eq!(
            classify(&upper),
            Some(PackageKind::App),
            "확장자 대소문자 무시"
        );
    }

    #[test]
    fn 판정_목록에_없어도_번들_구조면_패키지() {
        let tmp = tempfile::tempdir().unwrap();
        let odd = make_app(tmp.path(), "Thing.vendorbundle");
        assert_eq!(classify(&odd), Some(PackageKind::Opaque));
    }

    #[test]
    fn 스캐너용_함수는_macos에서만_켜진다() {
        let tmp = tempfile::tempdir().unwrap();
        let app = make_app(tmp.path(), "Foo.app");
        let inner = app.join("Contents").join("MacOS").join("Foo");
        assert_eq!(detect(&app).is_some(), enabled());
        assert_eq!(parent_is_package(&app.join("Contents")), enabled());
        assert_eq!(inside_package(&inner), enabled());
        assert!(!inside_package(&app), "자기 자신은 보지 않는다");
    }
}
