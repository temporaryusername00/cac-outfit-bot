//! Built-in account manager: stores sign-ins, launches accounts into a game,
//! and keeps the multi-Roblox lock (what Roblox Account Manager did before).
//!
//! Sign-ins (.ROBLOSECURITY) are encrypted with Windows DPAPI, so accounts.json
//! only opens for this Windows user on this PC.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::config;

// ------------------------------------------------------------------ storage

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct StoredAccount {
    name: String,
    #[serde(default)]
    display: String,
    #[serde(default)]
    user_id: u64,
    /// DPAPI-encrypted cookie, hex.
    secret: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct StoreFile {
    #[serde(default)]
    accounts: Vec<StoredAccount>,
}

#[derive(Clone, Debug)]
pub struct Account {
    pub name: String,
    pub display: String,
    pub user_id: u64,
    pub cookie: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = if protect {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(v)
    }
}

fn store_path() -> std::path::PathBuf {
    config::data(&config::data_dir()).join("accounts.json")
}

pub fn load() -> Vec<Account> {
    let file: StoreFile = config::read_json(&store_path()).unwrap_or_default();
    file.accounts
        .into_iter()
        .filter_map(|a| {
            let cookie = String::from_utf8(dpapi(&unhex(&a.secret)?, false)?).ok()?;
            Some(Account { name: a.name, display: a.display, user_id: a.user_id, cookie })
        })
        .collect()
}

static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub fn save(accounts: &[Account]) -> Result<(), String> {
    let _g = SAVE_LOCK.lock().unwrap();
    let file = StoreFile {
        accounts: accounts
            .iter()
            .map(|a| StoredAccount {
                name: a.name.clone(),
                display: a.display.clone(),
                user_id: a.user_id,
                secret: hex(&dpapi(a.cookie.as_bytes(), true).unwrap_or_default()),
            })
            .collect(),
    };
    config::write_json(&store_path(), &file)
}

/// Add or replace (same user) and save. Returns the account name.
pub fn upsert(acc: Account) -> Result<String, String> {
    let mut all = load();
    let name = acc.name.clone();
    all.retain(|a| a.user_id != acc.user_id && a.name != acc.name);
    all.push(acc);
    all.sort_by_key(|a| a.name.to_lowercase());
    save(&all)?;
    Ok(name)
}

pub fn remove(name: &str) -> Result<(), String> {
    let mut all = load();
    all.retain(|a| a.name != name);
    save(&all)
}

// ------------------------------------------------------------------ Roblox web

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn clean_cookie(raw: &str) -> String {
    let c = raw.trim().trim_matches('"');
    c.strip_prefix(".ROBLOSECURITY=").unwrap_or(c).trim().to_string()
}

/// Check a sign-in and look up who it belongs to.
pub fn verify(raw_cookie: &str) -> Result<Account, String> {
    let cookie = clean_cookie(raw_cookie);
    if cookie.len() < 100 {
        return Err("That doesn't look like a .ROBLOSECURITY cookie.".into());
    }
    let mut resp = agent()
        .get("https://users.roblox.com/v1/users/authenticated")
        .header("Cookie", format!(".ROBLOSECURITY={cookie}"))
        .call()
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 401 {
        return Err("Roblox says this sign-in is expired or invalid.".into());
    }
    if status != 200 {
        return Err(format!("Roblox answered {status}"));
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(Account {
        name: v["name"].as_str().unwrap_or("?").to_string(),
        display: v["displayName"].as_str().unwrap_or("").to_string(),
        user_id: v["id"].as_u64().unwrap_or(0),
        cookie,
    })
}

/// One-time ticket the Roblox client uses to sign in.
fn auth_ticket(cookie: &str) -> Result<String, String> {
    let a = agent();
    let mut csrf = String::new();
    for _ in 0..2 {
        let mut req = a
            .post("https://auth.roblox.com/v1/authentication-ticket")
            .header("Cookie", format!(".ROBLOSECURITY={cookie}"))
            .header("Referer", "https://www.roblox.com/")
            .header("Origin", "https://www.roblox.com")
            .header("Content-Type", "application/json")
            .header("RBXAuthenticationNegotiation", "1");
        if !csrf.is_empty() {
            req = req.header("x-csrf-token", csrf.as_str());
        }
        let resp = req.send("{}").map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        if let Some(t) = resp.headers().get("rbx-authentication-ticket").and_then(|v| v.to_str().ok()) {
            return Ok(t.to_string());
        }
        if status == 403 {
            if let Some(t) = resp.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()) {
                csrf = t.to_string();
                continue;
            }
        }
        if status == 401 {
            return Err("sign-in expired; sign in to this account again".into());
        }
        return Err(format!("Roblox refused the launch ticket ({status})"));
    }
    Err("Roblox refused the launch ticket (CSRF)".into())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    h.finish()
}

fn uuid() -> String {
    let (a, b) = (random_u64(), random_u64());
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        b >> 52 & 0xfff,
        b & 0xffff_ffff_ffff
    )
}

/// Open Roblox as this account, straight into the place.
/// Is this sign-in still usable? Roblox answers 403 "User is moderated" for banned or warned
/// accounts; the Roblox player then fails with "Authentication Failed, Error Code: 403".
pub fn check_session(cookie: &str) -> Result<(), String> {
    let mut resp = agent()
        .get("https://users.roblox.com/v1/users/authenticated")
        .header("Cookie", format!(".ROBLOSECURITY={cookie}"))
        .call()
        .map_err(|e| format!("could not reach Roblox: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => Ok(()),
        401 => Err("the sign-in has expired. Remove the account and sign in again on the Accounts tab".into()),
        403 if body.to_ascii_lowercase().contains("moderated") => Err(
            "Roblox has banned this account or given it a warning to accept. Sign in at roblox.com in a browser to see the notice"
                .into(),
        ),
        _ => Err(format!("Roblox didn't accept the sign-in (HTTP {status})")),
    }
}

/// Did a Roblox player started after `since` fail to sign in? Reads Roblox's own logs, because the
/// player only shows "Authentication Failed" in a dialog the app can't otherwise see.
pub fn launch_failure(since: SystemTime) -> Option<String> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Roblox").join("logs");
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.contains("_Player_") || name.contains("Crash") {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
        if modified < since {
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(entry.path()).ok()?).into_owned();
        if !text.contains("WebLogin HTTP error") && !text.contains("WebLogin authentication is failed") {
            continue;
        }
        let redeem = text.lines().find(|l| l.contains("authentication-ticket/redeem") && l.contains("status:"));
        let status = redeem
            .and_then(|l| l.split("status:").nth(1))
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("?")
            .to_string();
        return Some(match status.as_str() {
            "429" => "Roblox is limiting sign-ins (429) after several tries close together. Wait 5-10 minutes before starting again".into(),
            "403" => "Roblox refused this account's sign-in when the player opened (Authentication Failed, 403)"
                .into(),
            s => format!("the Roblox player couldn't sign in (HTTP {s})"),
        });
    }
    None
}

/// The Roblox player keeps its own saved sign-in (LocalStorage/RobloxCookies.dat) and sends it when
/// it redeems a launch ticket. If that saved sign-in belongs to a banned account, every launch fails
/// with "Authentication Failed, 403". Moving the file aside (kept as .bak) fixes it; the player
/// writes a fresh one for the account it signs in to.
pub fn reset_player_signin() -> bool {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else { return false };
    let dir = PathBuf::from(base).join("Roblox").join("LocalStorage");
    let file = dir.join("RobloxCookies.dat");
    file.exists() && std::fs::rename(&file, dir.join("RobloxCookies.dat.bak")).is_ok()
}

/// Launch, then watch Roblox's log for a few seconds; on a 403 clear the player's old saved
/// sign-in and launch once more. Used by Play in CAC (the bot's run loop does its own watching).
pub fn launch_checked(acc: &Account, place_id: u64) -> Result<(), String> {
    for attempt in 0..2 {
        let started = SystemTime::now() - Duration::from_secs(2);
        launch(acc, place_id)?;
        let end = std::time::Instant::now() + Duration::from_secs(20);
        let mut failed = None;
        while std::time::Instant::now() < end && failed.is_none() {
            std::thread::sleep(Duration::from_millis(500));
            failed = launch_failure(started);
        }
        match failed {
            None => return Ok(()),
            Some(f) if f.contains("403") && attempt == 0 => {
                crate::win::close_titled("Authentication Failed");
                if !reset_player_signin() {
                    return Err(f);
                }
                std::thread::sleep(Duration::from_secs(3));
            }
            Some(f) => return Err(f),
        }
    }
    Ok(())
}

pub fn launch(acc: &Account, place_id: u64) -> Result<(), String> {
    check_session(&acc.cookie)?;
    let ticket = auth_ticket(&acc.cookie)?;
    let tracker = 100_000_000_000 + random_u64() % 900_000_000_000;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let place_url = format!(
        "https://www.roblox.com/Game/PlaceLauncher.ashx?request=RequestGame&browserTrackerId={tracker}&placeId={place_id}&isPlayTogetherGame=false&joinAttemptId={}&joinAttemptOrigin=PlayButton",
        uuid()
    );
    let uri = format!(
        "roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{now}+placelauncherurl:{}+browsertrackerid:{tracker}+robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp",
        enc(&place_url)
    );
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (op, target) = (wide("open"), wide(&uri));
    let r = unsafe {
        ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    if (r as isize) <= 32 {
        return Err("Windows couldn't open roblox-player:// (is Roblox installed?)".into());
    }
    Ok(())
}

// ------------------------------------------------------------------ multi-Roblox

/// Roblox closes older copies of itself through a named lock. Holding that lock
/// ourselves before Roblox starts lets several copies run side by side.
pub struct MultiRoblox {
    handles: Vec<HANDLE>,
    pub on: bool,
}

unsafe impl Send for MultiRoblox {}

impl MultiRoblox {
    /// Take the lock. Only works while no Roblox is running.
    pub fn try_enable(&mut self) -> bool {
        if self.on {
            return true;
        }
        for h in self.handles.drain(..) {
            unsafe { CloseHandle(h) };
        }
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        unsafe {
            let name = wide("ROBLOX_singletonMutex");
            let m = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
            let existed = GetLastError() == ERROR_ALREADY_EXISTS;
            if !m.is_null() {
                self.handles.push(m);
            }
            let ev = wide("ROBLOX_singletonEvent");
            let e = CreateEventW(std::ptr::null(), 1, 0, ev.as_ptr());
            if !e.is_null() {
                self.handles.push(e);
            }
            self.on = !m.is_null() && !existed;
        }
        self.on
    }
}

/// Accounts saved in Roblox Account Manager, with their sign-ins (needs RAM's
/// web server with "Allow GetAccounts" and "Allow GetCookie").
pub fn import_from_ram(port: u16, password: &str) -> Result<Vec<Result<String, String>>, String> {
    let names = crate::ram::accounts(port, password)?;
    if names.is_empty() {
        return Err("RAM has no accounts.".into());
    }
    let mut results = Vec::new();
    for name in names {
        let r = crate::ram::cookie(port, password, &name).and_then(|c| verify(&c)).and_then(upsert);
        results.push(r.map_err(|e| format!("{name}: {e}")));
    }
    Ok(results)
}

/// Run the sign-in window (this exe with --login) and save the account it returns.
pub fn sign_in_window() -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join(format!("cac-bot-login-{}", random_u64()));
    let mut child = std::process::Command::new(exe)
        .arg("--login")
        .arg(&dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut cookie = None;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(c) = line.strip_prefix("COOKIE:") {
                cookie = Some(c.to_string());
            }
        }
    }
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(800));
    let _ = std::fs::remove_dir_all(&dir);
    let cookie = cookie.ok_or("Sign-in window closed before signing in.")?;
    upsert(verify(&cookie)?)
}

/// The sign-in window itself (runs in its own process).
pub fn login_window_main(dir: &Path) -> ! {
    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use std::io::Write;
    use std::time::Instant;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Sign in to Roblox - CAC Outfit Bot")
        .with_inner_size(tao::dpi::LogicalSize::new(520.0, 760.0))
        .build(&event_loop)
        .expect("window");
    let mut ctx = wry::WebContext::new(Some(dir.to_path_buf()));
    let webview = wry::WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url("https://www.roblox.com/login")
        .build(&window)
        .expect("webview");
    event_loop.run(move |event, _, flow| {
        let _keep = (&ctx, &window);
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(800));
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => std::process::exit(1),
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Ok(cookies) = webview.cookies_for_url("https://www.roblox.com") {
                    if let Some(c) = cookies.iter().find(|c| c.name() == ".ROBLOSECURITY" && !c.value().is_empty()) {
                        let mut out = std::io::stdout();
                        let _ = writeln!(out, "COOKIE:{}", c.value());
                        let _ = out.flush();
                        std::process::exit(0);
                    }
                }
            }
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints each saved account's status (never the sign-in): `cargo test session -- --ignored`.
    #[test]
    #[ignore]
    fn session_status() {
        for a in load() {
            println!("{}: {:?}", a.name, check_session(&a.cookie));
        }
    }

    /// Reads Roblox's logs from the last hour: `cargo test launch_failure -- --ignored`.
    #[test]
    #[ignore]
    fn launch_failure_reads_logs() {
        let since = SystemTime::now() - Duration::from_secs(3600);
        println!("{:?}", launch_failure(since));
    }

    #[test]
    fn dpapi_round_trip() {
        let secret = b"_|WARNING:-test-value-not-a-real-cookie";
        let locked = dpapi(secret, true).unwrap();
        assert_ne!(&locked[..], &secret[..]);
        let text = hex(&locked);
        assert_eq!(dpapi(&unhex(&text).unwrap(), false).unwrap(), secret.to_vec());
    }

    #[test]
    fn launch_link_parts() {
        assert_eq!(enc("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
        assert_eq!(clean_cookie(".ROBLOSECURITY=abc "), "abc");
        assert_eq!(uuid().len(), 36);
    }
}

// ------------------------------------------------------------------ lock helper
//
// Roblox lets every copy but one close as soon as nobody holds its lock. If the
// lock lived in the app, closing or restarting the app would kill the extra
// accounts. So a tiny hidden helper (this exe with --hold-lock) holds it, and
// stays alive while the app or any Roblox window is open.

const HOLDER: &str = r"Local\CACOutfitBot_LockHolder";
const MULTI_ON: &str = r"Local\CACOutfitBot_MultiOn";
const APP_ALIVE: &str = r"Local\CACOutfitBot_App";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn named_exists(name: &str) -> bool {
    use windows_sys::Win32::System::Threading::OpenMutexW;
    let n = wide(name);
    unsafe {
        let h = OpenMutexW(0x0010_0000, 0, n.as_ptr());
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

fn create_named(name: &str) -> (HANDLE, bool) {
    let n = wide(name);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, n.as_ptr());
        (h, GetLastError() == ERROR_ALREADY_EXISTS)
    }
}

/// Called once by the app: mark it alive and make sure the helper runs.
pub fn app_started() {
    // Never closed: the handle lives as long as the app.
    let _ = create_named(APP_ALIVE);
    ensure_lock_holder();
}

pub fn ensure_lock_holder() {
    if named_exists(HOLDER) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new(exe).arg("--hold-lock").creation_flags(0x0000_0008).spawn();
    }
}

/// The helper holds Roblox's lock.
pub fn multi_on() -> bool {
    named_exists(MULTI_ON)
}

pub fn lock_holder_main(title: &str) -> ! {
    let (_me, existed) = create_named(HOLDER);
    if existed {
        std::process::exit(0);
    }
    let mut multi = MultiRoblox { handles: Vec::new(), on: false };
    let mut flag: HANDLE = std::ptr::null_mut();
    let mut idle_checks = 0;
    loop {
        let roblox_open = !crate::win::find_windows(title).is_empty();
        if !multi.on && !roblox_open {
            multi.try_enable();
        }
        if multi.on && flag.is_null() {
            flag = create_named(MULTI_ON).0;
        }
        if !roblox_open && !named_exists(APP_ALIVE) {
            idle_checks += 1;
            if idle_checks >= 3 {
                std::process::exit(0);
            }
        } else {
            idle_checks = 0;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
