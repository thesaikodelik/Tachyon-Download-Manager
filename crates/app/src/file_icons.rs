//! The icon Windows shows for a file in Explorer, so dialogs and the list show the
//! user's real file icons (their video player's, a program's own…). Other systems
//! fall back to Tachyon's category icons.

use std::path::Path;
use std::sync::Arc;

use gpui::{AnyElement, Image, IntoElement, Pixels, Styled, img};

use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::icon;

/// The system icon for `path`, which need not exist yet: until the download is
/// done the icon comes from the file type. Programs and shortcuts get their own
/// icon once they exist. Cached, so it is cheap to call while rendering.
pub fn file_icon(path: &Path) -> Option<Arc<Image>> {
    imp::file_icon(path)
}

/// The system icon for `path`, or the category's icon where there is none.
pub fn file_icon_element(path: &Path, category: &str, size: Pixels, theme: &Theme) -> AnyElement {
    match file_icon(path) {
        Some(image) => img(image).size(size).flex_none().into_any_element(),
        None => icon(Icon::for_category(category), size, theme.accent).into_any_element(),
    }
}

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;

    use gpui::{Image, ImageFormat};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW,
        HBITMAP, HGDIOBJ, ReleaseDC,
    };
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
    use windows::Win32::UI::Controls::{ILD_TRANSPARENT, IImageList};
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHGetImageList, SHIL_EXTRALARGE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
    use windows::core::HSTRING;

    /// File types whose icon depends on the file itself rather than its type.
    const OWN_ICON: &[&str] = &["exe", "ico", "lnk", "url", "cpl", "scr"];

    thread_local! {
        static CACHE: RefCell<HashMap<String, Option<Arc<Image>>>> = RefCell::new(HashMap::new());
    }

    pub fn file_icon(path: &Path) -> Option<Arc<Image>> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let own = OWN_ICON.contains(&ext.as_str()) && path.is_file();
        let key = if own { path.to_string_lossy().to_lowercase() } else { format!(".{ext}") };
        if let Some(hit) = CACHE.with_borrow(|cache| cache.get(&key).cloned()) {
            return hit;
        }
        let loaded = load(path, &ext, own);
        CACHE.with_borrow_mut(|cache| cache.insert(key, loaded.clone()));
        loaded
    }

    fn load(path: &Path, ext: &str, own: bool) -> Option<Arc<Image>> {
        // SAFETY: plain shell/GDI calls; every handle obtained is released below.
        unsafe {
            let mut info = SHFILEINFOW::default();
            let found = if own {
                SHGetFileInfoW(
                    &HSTRING::from(path.as_os_str()),
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    Some(&mut info),
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_SYSICONINDEX,
                )
            } else {
                // By type alone, so the file doesn't have to exist.
                SHGetFileInfoW(
                    &HSTRING::from(format!("file.{ext}")),
                    FILE_ATTRIBUTE_NORMAL,
                    Some(&mut info),
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES,
                )
            };
            if found == 0 {
                return None;
            }
            let list: IImageList = SHGetImageList(SHIL_EXTRALARGE as i32).ok()?;
            let icon = list.GetIcon(info.iIcon, ILD_TRANSPARENT.0).ok()?;
            let png = icon_to_png(icon);
            let _ = DestroyIcon(icon);
            png.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
        }
    }

    /// Reads a bitmap as top-down BGRA rows.
    unsafe fn read_bitmap(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let mut bm = BITMAP::default();
            if GetObjectW(HGDIOBJ(bitmap.0), size_of::<BITMAP>() as i32, Some((&raw mut bm).cast())) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let dc = GetDC(None);
            let rows = GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
            ReleaseDC(None, dc);
            (rows == h).then_some((w as u32, h as u32, pixels))
        }
    }

    unsafe fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
        unsafe {
            let mut parts = ICONINFO::default();
            GetIconInfo(icon, &mut parts).ok()?;
            let color = read_bitmap(parts.hbmColor);
            let mask = read_bitmap(parts.hbmMask);
            let _ = DeleteObject(HGDIOBJ(parts.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(parts.hbmMask.0));
            let (w, h, mut px) = color?;
            // Old icons have no alpha channel: take transparency from the mask.
            if px.chunks_exact(4).all(|p| p[3] == 0) {
                let (_, _, mask) = mask?;
                for (p, m) in px.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            for p in px.chunks_exact_mut(4) {
                p.swap(0, 2); // BGRA -> RGBA
            }
            // Types with only a 32px icon get drawn into the top-left of the 48px
            // canvas; crop so they stay centred.
            let (w, h, px) = crop_small_icon(w, h, px);
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().ok()?.write_image_data(&px).ok()?;
            Some(out)
        }
    }

    fn crop_small_icon(w: u32, h: u32, px: Vec<u8>) -> (u32, u32, Vec<u8>) {
        const SMALL: u32 = 32;
        if w <= SMALL || h <= SMALL {
            return (w, h, px);
        }
        let outside_small = (0..h).any(|y| {
            (0..w).any(|x| (x >= SMALL || y >= SMALL) && px[((y * w + x) * 4 + 3) as usize] != 0)
        });
        if outside_small {
            return (w, h, px);
        }
        let mut cropped = Vec::with_capacity((SMALL * SMALL * 4) as usize);
        for y in 0..SMALL {
            let row = (y * w * 4) as usize;
            cropped.extend_from_slice(&px[row..row + (SMALL * 4) as usize]);
        }
        (SMALL, SMALL, cropped)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;
    use std::sync::Arc;

    use gpui::Image;

    pub fn file_icon(_: &Path) -> Option<Arc<Image>> {
        None
    }
}
