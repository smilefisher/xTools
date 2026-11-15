//! 截屏与剪贴板：GDI 抓屏 → RGBA → PNG / 剪贴板。

use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// 行优先 BGRA（GDI 的 32bpp DIB 布局）
    pub bgra: Vec<u8>,
}

impl Frame {
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = self.bgra.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }

    /// 从整帧里裁一块（坐标相对整帧左上角，物理像素）。
    pub fn crop(&self, x: i32, y: i32, w: u32, h: u32) -> Option<Frame> {
        let x = x.max(0) as u32;
        let y = y.max(0) as u32;
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        if w == 0 || h == 0 {
            return None;
        }

        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * self.width + x) * 4) as usize;
            out.extend_from_slice(&self.bgra[start..start + (w * 4) as usize]);
        }
        Some(Frame {
            x: self.x + x as i32,
            y: self.y + y as i32,
            width: w,
            height: h,
            bgra: out,
        })
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let rgba = self.to_rgba();
        let image = image::RgbaImage::from_raw(self.width, self.height, rgba)
            .ok_or_else(|| "图像尺寸不匹配".to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        image
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|e| e.to_string())
    }

    /// 写入剪贴板（CF_DIB，32bpp BGRA，自底向上）。
    pub fn to_clipboard(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use windows::Win32::System::DataExchange::{
                EmptyClipboard, OpenClipboard, SetClipboardData,
            };
            use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
            use windows::Win32::System::Ole::CF_DIB;

            const HEADER: usize = 40; // BITMAPINFOHEADER
            let pixel_bytes = (self.width * self.height * 4) as usize;
            let total = HEADER + pixel_bytes;

            unsafe {
                let handle = GlobalAlloc(GMEM_MOVEABLE, total).map_err(|e| e.to_string())?;
                let ptr = GlobalLock(handle);
                if ptr.is_null() {
                    return Err("GlobalLock 失败".into());
                }
                let bytes = std::slice::from_raw_parts_mut(ptr as *mut u8, total);

                // BITMAPINFOHEADER：高度为正表示自底向上
                bytes[0..4].copy_from_slice(&(HEADER as u32).to_le_bytes());
                bytes[4..8].copy_from_slice(&self.width.to_le_bytes());
                bytes[8..12].copy_from_slice(&(self.height as i32).to_le_bytes());
                bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
                bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
                bytes[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
                bytes[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());

                // 翻转行序：GDI 抓取是自顶向下，剪贴板要自底向上
                let row_bytes = (self.width * 4) as usize;
                for row in 0..self.height as usize {
                    let src = row * row_bytes;
                    let dst = HEADER + (self.height as usize - 1 - row) * row_bytes;
                    bytes[dst..dst + row_bytes]
                        .copy_from_slice(&self.bgra[src..src + row_bytes]);
                }

                let _ = GlobalUnlock(handle);
                OpenClipboard(None).map_err(|e| e.to_string())?;
                let _ = EmptyClipboard();
                SetClipboardData(
                    CF_DIB.0 as u32,
                    Some(windows::Win32::Foundation::HANDLE(handle.0)),
                )
                .map_err(|e| e.to_string())?;
                let _ = windows::Win32::System::DataExchange::CloseClipboard();
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err("仅支持 Windows".into())
        }
    }
}

/// 抓取屏幕的一块矩形（虚拟屏幕物理坐标）。
#[cfg(windows)]
pub fn capture_rect(x: i32, y: i32, width: i32, height: i32) -> Result<Frame, String> {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
    };

    if width <= 0 || height <= 0 {
        return Err("截图区域无效".into());
    }

    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC 失败".into());
        }
        let mem = CreateCompatibleDC(Some(screen));

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // 自顶向下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| e.to_string())?;
        let old = SelectObject(mem, bitmap.into());

        BitBlt(mem, 0, 0, width, height, Some(screen), x, y, SRCCOPY | CAPTUREBLT)
            .map_err(|e| e.to_string())?;

        let len = (width as usize) * (height as usize) * 4;
        let mut bgra = vec![0u8; len];
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(bits as *const u8, bgra.as_mut_ptr(), len);
        }

        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        Ok(Frame {
            x,
            y,
            width: width as u32,
            height: height as u32,
            bgra,
        })
    }
}

/// 写入纯文本剪贴板（CF_UNICODETEXT）。
pub fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
        use windows::Win32::System::Ole::CF_UNICODETEXT;

        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;

        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
            let ptr = GlobalLock(handle);
            if ptr.is_null() {
                return Err("GlobalLock 失败".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr as *mut u8, bytes);
            let _ = GlobalUnlock(handle);

            OpenClipboard(None).map_err(|e| e.to_string())?;
            let _ = EmptyClipboard();
            SetClipboardData(
                CF_UNICODETEXT.0 as u32,
                Some(windows::Win32::Foundation::HANDLE(handle.0)),
            )
            .map_err(|e| e.to_string())?;
            let _ = CloseClipboard();
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("仅支持 Windows".into())
    }
}

/// 贴图 PNG 的落盘目录。
pub fn pin_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("xtools").join("pins"))
}
