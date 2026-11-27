//! Mouse and keyboard input that Roblox accepts.
//!
//! Roblox ignores a click unless real relative mouse movement arrives first, and it
//! ignores keys sent without a scan code or released too quickly.

use std::thread::sleep;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

const HOLD: Duration = Duration::from_millis(60);

pub fn click(x: i32, y: i32) {
    unsafe {
        SetCursorPos(x, y);
        for _ in 0..2 {
            mouse_event(MOUSEEVENTF_MOVE, 1, 0, 0, 0);
            sleep(Duration::from_millis(15));
            mouse_event(MOUSEEVENTF_MOVE, -1, 0, 0, 0);
            sleep(Duration::from_millis(15));
        }
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        sleep(HOLD);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
}

/// Move the cursor into a window with real movement events (no click).
pub fn hover(x: i32, y: i32) {
    unsafe {
        SetCursorPos(x, y);
        mouse_event(MOUSEEVENTF_MOVE, 1, 0, 0, 0);
        sleep(Duration::from_millis(15));
        mouse_event(MOUSEEVENTF_MOVE, -1, 0, 0, 0);
    }
}

fn vk_for(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    let vk = match n.as_str() {
        "enter" | "return" => 0x0D,
        "backspace" | "bs" => 0x08,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        "delete" | "del" => 0x2E,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "backslash" | "\\" => 0xDC,
        _ => {
            if let Some(f) = n.strip_prefix('f').and_then(|s| s.parse::<u16>().ok()) {
                if (1..=24).contains(&f) {
                    return Some(0x70 + f - 1);
                }
            }
            let mut chars = n.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u16,
                _ => return None,
            }
        }
    };
    Some(vk)
}

fn is_extended(vk: u16) -> bool {
    matches!(vk, 0x25..=0x28 | 0x2D | 0x2E | 0x21..=0x24)
}

fn down(vk: u16) {
    unsafe {
        let sc = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u8;
        let fl = if is_extended(vk) { KEYEVENTF_EXTENDEDKEY } else { 0 };
        keybd_event(vk as u8, sc, fl, 0);
    }
}

fn up(vk: u16) {
    unsafe {
        let sc = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u8;
        let fl = if is_extended(vk) { KEYEVENTF_EXTENDEDKEY } else { 0 };
        keybd_event(vk as u8, sc, fl | KEYEVENTF_KEYUP, 0);
    }
}

/// Press a key or combo like "enter", "ctrl+a", "f11".
pub fn key(combo: &str) -> bool {
    let vks: Option<Vec<u16>> = combo.split('+').map(|k| vk_for(k.trim())).collect();
    let Some(vks) = vks else { return false };
    for &vk in &vks {
        down(vk);
        sleep(Duration::from_millis(30));
    }
    sleep(HOLD);
    for &vk in vks.iter().rev() {
        up(vk);
        sleep(Duration::from_millis(20));
    }
    true
}

pub fn tap_alt() {
    down(0x12);
    sleep(Duration::from_millis(20));
    up(0x12);
}

fn unicode_char(ch: u16) {
    let make = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: ch, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [make(KEYEVENTF_UNICODE), make(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)];
    unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

/// Type text one character at a time, `interval` seconds apart.
pub fn type_text(text: &str, interval: f64) {
    let gap = Duration::from_secs_f64(interval.max(0.0));
    for unit in text.encode_utf16() {
        let scan = unsafe { VkKeyScanW(unit) };
        if scan == -1 {
            unicode_char(unit);
        } else {
            let vk = (scan & 0xFF) as u16;
            let shift = scan & 0x100 != 0;
            if shift {
                down(0x10);
            }
            down(vk);
            sleep(Duration::from_millis(25));
            up(vk);
            if shift {
                up(0x10);
            }
        }
        sleep(gap);
    }
}

pub fn key_held(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}
