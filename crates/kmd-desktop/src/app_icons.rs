//! 네이티브 아이콘 추출 — OS Shell API로 파일/앱/디렉터리 아이콘 추출
//!
//! Windows: SHGetFileInfoW → HICON → BGRA → RGBA pixel data
//! macOS: Info.plist → .icns → PNG 디코딩
//! Linux: .desktop Icon= → 테마/pixmaps 검색 → PNG 로딩
//!
//! App/Executable: 경로 기준 캐시
//! File: 확장자 기준 캐시 (같은 확장자는 같은 아이콘)
//! Directory: 고정 "dir" 키로 캐시

use iced::widget::image::Handle;
use kmd_core::ItemKind;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[cfg(windows)]
const FALLBACK_ICON_SIZE: u32 = 32;
const TARGET_ICON_SIZE: u32 = 64;

static ICON_CACHE: LazyLock<Mutex<HashMap<String, Option<Handle>>>> =
    LazyLock::new(|| Mutex::new(HashMap::with_capacity(128)));

/// 캐시 키 규칙 — 키 하나가 곧 추출 방법을 정한다.
enum IconKey {
    /// App/Executable(경로 기준), `.lnk`(바로가기마다 아이콘이 다름)
    Path(String),
    /// 일반 파일 — 같은 확장자는 같은 아이콘
    Ext(String),
    /// 디렉터리 — 고정 아이콘
    Dir,
}

impl IconKey {
    fn of(kind: ItemKind, path: &str, icon_path: Option<&str>) -> Option<Self> {
        if path.is_empty() || path.starts_with("http") || path.starts_with("kmd:") {
            return None;
        }
        match kind {
            ItemKind::App | ItemKind::Executable => {
                Some(Self::Path(icon_path.unwrap_or(path).to_string()))
            }
            ItemKind::File => {
                let ext = std::path::Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext.is_empty() {
                    None
                } else if ext == "lnk" {
                    Some(Self::Path(path.to_string()))
                } else {
                    Some(Self::Ext(ext))
                }
            }
            ItemKind::Directory => Some(Self::Dir),
            _ => None,
        }
    }

    fn cache_key(&self) -> String {
        match self {
            Self::Path(p) => p.clone(),
            Self::Ext(ext) => format!("ext:{ext}"),
            Self::Dir => "type:dir".to_string(),
        }
    }
}

/// OS 셸 API로 아이콘 추출 (블로킹, 캐시 락 밖에서 호출할 것).
fn extract(kind: ItemKind, path: &str, icon_path: Option<&str>, key: &IconKey) -> Option<Handle> {
    match key {
        IconKey::Path(_) if matches!(kind, ItemKind::App | ItemKind::Executable) => {
            if path.starts_with("shell:") {
                platform::extract_pidl_icon(path)
            } else {
                platform::extract_icon(path, icon_path)
            }
        }
        IconKey::Path(_) => platform::extract_icon(path, None),
        IconKey::Ext(ext) => platform::extract_icon_by_ext(&format!("file.{ext}")),
        IconKey::Dir => platform::extract_folder_icon(),
    }
}

/// 락 poison이어도 캐시는 계속 쓴다 — 예전엔 `lock().ok()?`라 패닉 한 번에 아이콘이 영구히 사라졌다.
fn icon_cache() -> std::sync::MutexGuard<'static, HashMap<String, Option<Handle>>> {
    ICON_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 이미 캐시에 있는 아이콘만 반환 — OS 추출을 수행하지 않는 논블로킹 조회.
///
/// `view()`에서 호출되어 렌더 스레드가 셸 API 호출로 멈추는 것을 막는다.
/// 캐시 미스 시 `None`을 반환하므로 호출부는 텍스트 placeholder를 그린다.
/// 실제 추출은 `prefetch_icons`가 백그라운드 스레드에서 채운다.
pub fn cached_icon_for_item(kind: ItemKind, path: &str, icon_path: Option<&str>) -> Option<Handle> {
    let cache_key = IconKey::of(kind, path, icon_path)?.cache_key();
    icon_cache().get(&cache_key).cloned().flatten()
}

/// 주어진 아이템들의 아이콘을 OS에서 추출해 캐시에 채운다 (블로킹).
///
/// 백그라운드 스레드(`spawn_blocking`)에서 호출되어야 한다. 이미 캐시된
/// 항목은 건너뛴다. 새로 추출한 아이콘 수를 반환한다 — 0이면 호출부가
/// 리렌더(IconsReady)를 생략할 수 있다.
pub fn prefetch_icons(items: &[(ItemKind, String, Option<String>)]) -> usize {
    let mut newly_extracted = 0;
    for (kind, path, icon_path) in items {
        let Some(key) = IconKey::of(*kind, path, icon_path.as_deref()) else {
            continue;
        };
        let cache_key = key.cache_key();
        if icon_cache().contains_key(&cache_key) {
            continue;
        }
        // 추출은 락 밖에서 — 렌더 스레드의 cached_icon_for_item이 셸 API 호출을
        // 기다리지 않게 한다. 같은 키를 두 번 추출해도 결과가 같아 무해하다.
        let handle = extract(*kind, path, icon_path.as_deref(), &key);
        icon_cache().insert(cache_key, handle);
        newly_extracted += 1;
    }
    newly_extracted
}

/// PNG/이미지 바이트 → TARGET_ICON_SIZE 리사이즈 → Handle
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn png_bytes_to_handle(png_data: &[u8]) -> Option<Handle> {
    use image::codecs::png::PngEncoder;
    use image::imageops::FilterType;
    use image::ImageEncoder;

    let img = image::load_from_memory(png_data).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    let target = TARGET_ICON_SIZE;

    let resized = if w == target && h == target {
        img
    } else {
        image::imageops::resize(&img, target, target, FilterType::Lanczos3)
    };

    let mut buf = Vec::new();
    PngEncoder::new(&mut buf)
        .write_image(
            resized.as_raw(),
            target,
            target,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(Handle::from_bytes(buf))
}

// ── Windows 구현 ────────────────────────────────────────────────────────────

#[cfg(windows)]
mod platform {
    use super::FALLBACK_ICON_SIZE;
    use iced::widget::image::Handle;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{
        SHGetFileInfoW, SHFILEINFOW, SHGFI_FLAGS, SHGFI_ICON, SHGFI_LARGEICON,
        SHGFI_USEFILEATTRIBUTES,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    pub fn extract_icon(path: &str, _icon_path: Option<&str>) -> Option<Handle> {
        extract_shell_icon(path, FILE_FLAGS_AND_ATTRIBUTES(0), false)
    }

    /// shell:appsFolder\... URI에서 PIDL을 통해 아이콘 추출 (UWP/Store 앱용)
    pub fn extract_pidl_icon(uri: &str) -> Option<Handle> {
        use windows::Win32::UI::Shell::{SHParseDisplayName, SHGFI_PIDL};

        unsafe {
            let wide: Vec<u16> = uri.encode_utf16().chain(std::iter::once(0)).collect();
            let mut pidl = std::ptr::null_mut();

            SHParseDisplayName(
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                &mut pidl,
                0,
                None,
            )
            .ok()?;

            if pidl.is_null() {
                return None;
            }

            let mut info = SHFILEINFOW::default();
            let result = SHGetFileInfoW(
                windows::core::PCWSTR(pidl as *const u16),
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_FLAGS(SHGFI_PIDL.0 | SHGFI_ICON.0 | SHGFI_LARGEICON.0),
            );

            windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const _));

            if result == 0 || info.hIcon.0.is_null() {
                return None;
            }

            let rgba = hicon_to_rgba(info.hIcon);
            let _ = DestroyIcon(info.hIcon);

            rgba.and_then(|(pixels, w, h)| normalize_and_encode(w, h, &pixels))
        }
    }

    /// 확장자 기반 연결 프로그램 아이콘 추출 (File용, 파일 존재 불필요)
    pub fn extract_icon_by_ext(dummy_name: &str) -> Option<Handle> {
        extract_shell_icon(
            dummy_name,
            FILE_FLAGS_AND_ATTRIBUTES(0x80), // FILE_ATTRIBUTE_NORMAL
            true,
        )
    }

    /// 폴더 아이콘 추출
    pub fn extract_folder_icon() -> Option<Handle> {
        extract_shell_icon(
            "directory",
            FILE_FLAGS_AND_ATTRIBUTES(0x10), // FILE_ATTRIBUTE_DIRECTORY
            true,
        )
    }

    fn extract_shell_icon(
        path: &str,
        file_attrs: FILE_FLAGS_AND_ATTRIBUTES,
        use_file_attrs: bool,
    ) -> Option<Handle> {
        unsafe {
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut info = SHFILEINFOW::default();

            let mut flags = SHGFI_ICON.0 | SHGFI_LARGEICON.0;
            if use_file_attrs {
                flags |= SHGFI_USEFILEATTRIBUTES.0;
            }

            let result = SHGetFileInfoW(
                windows::core::PCWSTR(wide.as_ptr()),
                file_attrs,
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_FLAGS(flags),
            );

            if result == 0 || info.hIcon.0.is_null() {
                return None;
            }

            let rgba = hicon_to_rgba(info.hIcon);
            let _ = DestroyIcon(info.hIcon);

            rgba.and_then(|(pixels, w, h)| normalize_and_encode(w, h, &pixels))
        }
    }

    /// 원본 RGBA를 TARGET_ICON_SIZE 정사각형으로 리사이즈 후 PNG 인코딩
    fn normalize_and_encode(width: u32, height: u32, rgba: &[u8]) -> Option<Handle> {
        use image::codecs::png::PngEncoder;
        use image::imageops::FilterType;
        use image::{ImageEncoder, RgbaImage};

        let target = super::TARGET_ICON_SIZE;
        let img = RgbaImage::from_raw(width, height, rgba.to_vec())?;

        let resized = if width == target && height == target {
            img
        } else {
            image::imageops::resize(&img, target, target, FilterType::Lanczos3)
        };

        let mut buf = Vec::new();
        PngEncoder::new(&mut buf)
            .write_image(
                resized.as_raw(),
                target,
                target,
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
        Some(Handle::from_bytes(buf))
    }

    /// HICON → (RGBA pixel data, width, height)
    unsafe fn hicon_to_rgba(
        hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
    ) -> Option<(Vec<u8>, u32, u32)> {
        use windows::Win32::Graphics::Gdi::*;
        use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

        let mut icon_info = ICONINFO::default();
        GetIconInfo(hicon, &mut icon_info).ok()?;

        // 실제 비트맵 크기 조회
        let (width, height) = if !icon_info.hbmColor.0.is_null() {
            bitmap_dimensions(icon_info.hbmColor)
        } else {
            (0, 0)
        };
        let (width, height) = if width > 0 && height > 0 {
            (width, height)
        } else {
            (FALLBACK_ICON_SIZE, FALLBACK_ICON_SIZE)
        };

        let hdc_screen = GetDC(None);
        let hdc = CreateCompatibleDC(hdc_screen);

        if hdc.0.is_null() {
            ReleaseDC(None, hdc_screen);
            cleanup_bitmaps(&icon_info);
            return None;
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: height as i32, // bottom-up
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let pixel_count = (width * height * 4) as usize;
        let mut pixels = vec![0u8; pixel_count];

        let old = SelectObject(hdc, icon_info.hbmColor);
        let lines = GetDIBits(
            hdc,
            icon_info.hbmColor,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        SelectObject(hdc, old);

        let _ = DeleteDC(hdc);
        ReleaseDC(None, hdc_screen);
        cleanup_bitmaps(&icon_info);

        if lines == 0 {
            return None;
        }

        // bottom-up → top-down 변환
        let row_bytes = (width * 4) as usize;
        let mut top_down = Vec::with_capacity(pixel_count);
        for row in pixels.chunks_exact(row_bytes).rev() {
            top_down.extend_from_slice(row);
        }

        // BGRA → RGBA + 알파 처리
        let has_alpha = top_down.chunks_exact(4).any(|c| c[3] != 0);
        for chunk in top_down.chunks_exact_mut(4) {
            chunk.swap(0, 2);
            if !has_alpha {
                chunk[3] = 255;
            }
        }

        Some((top_down, width, height))
    }

    /// HBITMAP의 실제 픽셀 크기 조회
    unsafe fn bitmap_dimensions(hbm: windows::Win32::Graphics::Gdi::HBITMAP) -> (u32, u32) {
        use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP};

        let mut bm = BITMAP::default();
        let ret = GetObjectW(
            hbm,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        if ret > 0 && bm.bmWidth > 0 && bm.bmHeight > 0 {
            (bm.bmWidth as u32, bm.bmHeight as u32)
        } else {
            (0, 0)
        }
    }

    unsafe fn cleanup_bitmaps(info: &windows::Win32::UI::WindowsAndMessaging::ICONINFO) {
        use windows::Win32::Graphics::Gdi::DeleteObject;

        if !info.hbmColor.0.is_null() {
            let _ = DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.0.is_null() {
            let _ = DeleteObject(info.hbmMask);
        }
    }
}

// ── macOS 구현 ──────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use iced::widget::image::Handle;
    use std::path::Path;

    pub fn extract_icon(path: &str, _icon_path: Option<&str>) -> Option<Handle> {
        if path.ends_with(".app") {
            extract_app_bundle_icon(path)
        } else {
            None
        }
    }

    fn extract_app_bundle_icon(app_path: &str) -> Option<Handle> {
        let bundle = Path::new(app_path);
        let info_plist = bundle.join("Contents/Info.plist");

        let plist_value = plist::Value::from_file(&info_plist).ok()?;
        let dict = plist_value.as_dictionary()?;

        let icon_name = dict
            .get("CFBundleIconFile")
            .and_then(|v| v.as_string())
            .unwrap_or("AppIcon");

        let icon_filename = if icon_name.ends_with(".icns") {
            icon_name.to_string()
        } else {
            format!("{icon_name}.icns")
        };

        let icns_path = bundle.join("Contents/Resources").join(&icon_filename);
        let icns_data = std::fs::read(&icns_path).ok()?;
        extract_png_from_icns(&icns_data)
    }

    /// ICNS 파일에서 가장 적합한 PNG 엔트리 추출
    fn extract_png_from_icns(data: &[u8]) -> Option<Handle> {
        if data.len() < 8 || &data[0..4] != b"icns" {
            return None;
        }

        let mut pos = 8usize;
        let mut best_png: Option<Vec<u8>> = None;
        let mut best_size = 0u32;

        while pos + 8 <= data.len() {
            let entry_type: [u8; 4] = data[pos..pos + 4].try_into().ok()?;
            let entry_len = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;

            if entry_len < 8 || pos + entry_len > data.len() {
                break;
            }

            let entry_data = &data[pos + 8..pos + entry_len];

            // PNG 시그니처 확인
            if entry_data.len() >= 8
                && entry_data[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
            {
                let size = match &entry_type {
                    b"icp4" => 16,
                    b"icp5" => 32,
                    b"icp6" => 64,
                    b"ic07" => 128,
                    b"ic08" => 256,
                    b"ic09" => 512,
                    b"ic10" => 1024,
                    b"ic11" => 64,
                    b"ic12" => 128,
                    b"ic13" => 512,
                    b"ic14" => 1024,
                    _ => 0,
                };

                // 128px에 가장 가까운 엔트리 선호 (32 이상만)
                if size >= 32 {
                    let dist = (size as i32 - 128).unsigned_abs();
                    let best_dist = (best_size as i32 - 128).unsigned_abs();
                    if best_png.is_none() || best_size < 32 || dist < best_dist {
                        best_png = Some(entry_data.to_vec());
                        best_size = size;
                    }
                } else if best_png.is_none() {
                    best_png = Some(entry_data.to_vec());
                    best_size = size;
                }
            }

            pos += entry_len;
        }

        let png_data = best_png?;
        super::png_bytes_to_handle(&png_data)
    }

    pub fn extract_pidl_icon(_uri: &str) -> Option<Handle> {
        None
    }

    pub fn extract_icon_by_ext(_dummy_name: &str) -> Option<Handle> {
        None
    }

    pub fn extract_folder_icon() -> Option<Handle> {
        None
    }
}

// ── Linux 구현 ──────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    use iced::widget::image::Handle;

    pub fn extract_icon(_path: &str, icon_path: Option<&str>) -> Option<Handle> {
        let icon_file = icon_path?;
        if icon_file.is_empty() {
            return None;
        }

        // PNG/BMP 등 image crate가 읽을 수 있는 파일만 처리
        let lower = icon_file.to_lowercase();
        if lower.ends_with(".svg") || lower.ends_with(".xpm") {
            return None;
        }

        let data = std::fs::read(icon_file).ok()?;
        super::png_bytes_to_handle(&data)
    }

    pub fn extract_pidl_icon(_uri: &str) -> Option<Handle> {
        None
    }

    pub fn extract_icon_by_ext(_dummy_name: &str) -> Option<Handle> {
        None
    }

    pub fn extract_folder_icon() -> Option<Handle> {
        None
    }
}

// ── 기타 플랫폼 스텁 ────────────────────────────────────────────────────────

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod platform {
    use iced::widget::image::Handle;

    pub fn extract_icon(_path: &str, _icon_path: Option<&str>) -> Option<Handle> {
        None
    }

    pub fn extract_pidl_icon(_uri: &str) -> Option<Handle> {
        None
    }

    pub fn extract_icon_by_ext(_dummy_name: &str) -> Option<Handle> {
        None
    }

    pub fn extract_folder_icon() -> Option<Handle> {
        None
    }
}
