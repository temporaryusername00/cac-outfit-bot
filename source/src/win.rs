//! Finding, focusing, un-fullscreening, resizing and tiling Roblox windows.

use std::time::{Duration, Instant};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::input;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Win(pub isize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Win {
    pub fn hwnd(self) -> HWND {
        self.0 as HWND
    }

    pub fn exists(self) -> bool {
        unsafe { IsWindow(self.hwnd()) != 0 }
    }

    pub fn title(self) -> String {
        unsafe {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(self.hwnd(), buf.as_mut_ptr(), buf.len() as i32);
            String::from_utf16_lossy(&buf[..n.max(0) as usize])
        }
    }

    pub fn rect(self) -> Rect {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetWindowRect(self.hwnd(), &mut r) };
        Rect { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top }
    }

    pub fn size_key(self) -> String {
        let r = self.rect();
        format!("{}x{}", r.width, r.height)
    }

    pub fn is_foreground(self) -> bool {
        unsafe { GetForegroundWindow() == self.hwnd() }
    }

    pub fn is_minimized(self) -> bool {
        unsafe { IsIconic(self.hwnd()) != 0 }
    }

    pub fn is_maximized(self) -> bool {
        unsafe { IsZoomed(self.hwnd()) != 0 }
    }

    /// Roblox fullscreen is a borderless window covering the whole monitor.
    pub fn is_fullscreen(self) -> bool {
        unsafe {
            let style = GetWindowLongW(self.hwnd(), GWL_STYLE) as u32;
            if style & WS_CAPTION == WS_CAPTION {
                return false;
            }
            let mon = MonitorFromWindow(self.hwnd(), MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(mon, &mut mi) == 0 {
                return false;
            }
            let r = self.rect();
            let m = mi.rcMonitor;
            r.left <= m.left && r.top <= m.top && r.left + r.width >= m.right && r.top + r.height >= m.bottom
        }
    }

    /// Bring the window to the front. Windows only lets a background process do this
    /// right after a key press, so tap Alt first when the plain call fails.
    pub fn focus(self) -> bool {
        unsafe {
            if self.is_minimized() {
                ShowWindow(self.hwnd(), SW_RESTORE);
            }
            if self.is_foreground() {
                return true;
            }
            SetForegroundWindow(self.hwnd());
            if !self.is_foreground() {
                input::tap_alt();
                SetForegroundWindow(self.hwnd());
            }
            if !self.is_foreground() {
                // Borrow the current foreground window's input queue, which Windows
                // accepts as permission to switch the foreground.
                let fg = GetForegroundWindow();
                let fg_thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
                let me = GetCurrentThreadId();
                if fg_thread != 0 && fg_thread != me {
                    AttachThreadInput(me, fg_thread, 1);
                    BringWindowToTop(self.hwnd());
                    SetForegroundWindow(self.hwnd());
                    AttachThreadInput(me, fg_thread, 0);
                }
            }
            BringWindowToTop(self.hwnd());
        }
        std::thread::sleep(Duration::from_millis(120));
        self.is_foreground()
    }

    /// True when a click at screen `x,y` would land on this window (nothing covers it).
    pub fn owns_point(self, x: i32, y: i32) -> bool {
        unsafe {
            let w = WindowFromPoint(POINT { x, y });
            !w.is_null() && GetAncestor(w, GA_ROOT) == self.hwnd()
        }
    }

    /// Put the window at `x,y` with the given outer size.
    pub fn place(self, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            if self.is_maximized() || self.is_minimized() {
                ShowWindow(self.hwnd(), SW_RESTORE);
            }
            SetWindowPos(self.hwnd(), std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Leave Roblox fullscreen (F11) and un-maximize. Returns false if it stays fullscreen.
    pub fn make_windowed(self) -> bool {
        for _ in 0..3 {
            if !self.is_fullscreen() {
                break;
            }
            self.focus();
            std::thread::sleep(Duration::from_millis(200));
            input::key("f11");
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(3) && self.is_fullscreen() {
                std::thread::sleep(Duration::from_millis(150));
            }
        }
        if self.is_fullscreen() {
            return false;
        }
        if self.is_maximized() {
            unsafe { ShowWindow(self.hwnd(), SW_RESTORE) };
            std::thread::sleep(Duration::from_millis(300));
        }
        true
    }
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam as *mut Vec<Win>);
    if IsWindowVisible(hwnd) != 0 {
        out.push(Win(hwnd as isize));
    }
    1
}

/// Visible top-level windows whose title is exactly `title`.
pub fn find_windows(title: &str) -> Vec<Win> {
    let mut all: Vec<Win> = Vec::new();
    unsafe { EnumWindows(Some(enum_cb), &mut all as *mut Vec<Win> as LPARAM) };
    all.into_iter().filter(|w| w.title() == title).collect()
}

/// Close every visible window with this exact title (e.g. Roblox's "Authentication Failed" box).
pub fn close_titled(title: &str) {
    for w in find_windows(title) {
        unsafe { PostMessageW(w.hwnd(), WM_CLOSE, 0, 0) };
    }
}

pub fn foreground() -> Win {
    Win(unsafe { GetForegroundWindow() } as isize)
}

/// Primary monitor's work area (screen minus taskbar).
pub fn work_area() -> Rect {
    let mut r = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut RECT as *mut _, 0) };
    Rect { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top }
}

/// Where window number `index` goes: a grid across the work area, then cascaded
/// layers on top when more windows exist than fit side by side.
pub fn tile_position(index: usize, w: i32, h: i32) -> (i32, i32) {
    let area = work_area();
    let cols = (area.width / w.max(1)).max(1) as usize;
    let rows = (area.height / h.max(1)).max(1) as usize;
    let per_layer = cols * rows;
    let cell = index % per_layer;
    let layer = (index / per_layer) as i32;
    let x = area.left + (cell % cols) as i32 * w + layer * 40;
    let y = area.top + (cell / cols) as i32 * h + layer * 40;
    (x, y)
}

/// Windowed, resized and tiled. Returns a message if something did not stick.
pub fn normalize(win: Win, index: usize, size: [i32; 2]) -> Result<(), String> {
    if !win.make_windowed() {
        return Err("could not leave fullscreen (F11 did nothing)".into());
    }
    let (x, y) = tile_position(index, size[0], size[1]);
    for _ in 0..3 {
        win.place(x, y, size[0], size[1]);
        std::thread::sleep(Duration::from_millis(400));
        let r = win.rect();
        if r.width == size[0] && r.height == size[1] {
            return Ok(());
        }
    }
    let r = win.rect();
    Err(format!("window is {}x{}, wanted {}x{}", r.width, r.height, size[0], size[1]))
}
