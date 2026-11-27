//! Screen capture for state checks and failure snapshots.

use std::path::Path;
use windows_sys::Win32::Graphics::Gdi::*;

use crate::config::StateSpec;
use crate::win::Rect;

/// BGRA pixels of a screen rectangle, top row first.
pub struct Shot {
    pub width: i32,
    pub height: i32,
    data: Vec<u8>,
}

impl Shot {
    pub fn rgb(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([self.data[i + 2], self.data[i + 1], self.data[i]])
    }

    /// True when every pixel of the state is within its tolerance.
    pub fn matches(&self, spec: &StateSpec) -> bool {
        !spec.pixels.is_empty()
            && spec.pixels.iter().all(|(x, y, want)| match self.rgb(*x, *y) {
                Some(got) => got.iter().zip(want).all(|(g, w)| (*g as i32 - *w as i32).abs() <= spec.tol),
                None => false,
            })
    }

    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let mut rgb = Vec::with_capacity((self.width * self.height * 3) as usize);
        for px in self.data.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let img = image::RgbImage::from_raw(self.width as u32, self.height as u32, rgb).ok_or("bad image size")?;
        img.save(path).map_err(|e| e.to_string())
    }
}

pub fn capture(r: Rect) -> Option<Shot> {
    if r.width <= 0 || r.height <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, r.width, r.height);
        let old = SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, r.width, r.height, screen, r.left, r.top, SRCCOPY | CAPTUREBLT) != 0;
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = r.width;
        info.bmiHeader.biHeight = -r.height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut data = vec![0u8; (r.width * r.height * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, r.height as u32, data.as_mut_ptr() as *mut _, &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        if !ok || lines == 0 {
            return None;
        }
        Some(Shot { width: r.width, height: r.height, data })
    }
}
