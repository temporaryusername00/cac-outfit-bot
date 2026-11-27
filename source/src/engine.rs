//! The bot: runs the config flow in each Roblox window.
//!
//! Every window gets its own thread. Only one thread may use the mouse and keyboard
//! at a time (the input lock); a thread lets go while it waits for CAC to load, so
//! another window can work in the meantime.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::{self, Config, Mode, Outfit, PublishedEntry, PublishedFile, StateSpec, Step};
use crate::{accounts, input, ram, screen, win};

#[derive(Clone, Debug, Default)]
pub struct SlotStatus {
    pub label: String,
    pub doing: String,
    pub done: u32,
    pub problems: u32,
    pub finished: bool,
}

/// Everything the GUI and the bot threads share.
#[derive(Default)]
pub struct Shared {
    pub running: AtomicBool,
    pub stop: AtomicBool,
    pub log: Mutex<Vec<String>>,
    pub slots: Mutex<Vec<SlotStatus>>,
    pub repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// The app holds the multi-Roblox lock.
    pub multi_on: AtomicBool,
}

impl Shared {
    pub fn log(&self, msg: impl AsRef<str>) {
        let line = format!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg.as_ref());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open({
            let logs = config::logs(&config::data_dir());
            let _ = std::fs::create_dir_all(&logs);
            logs.join("bot.log")
        }) {
            let _ = writeln!(f, "{line}");
        }
        let mut log = self.log.lock().unwrap();
        log.push(line);
        if log.len() > 3000 {
            log.drain(..1000);
        }
        drop(log);
        self.poke();
    }

    fn poke(&self) {
        if let Some(f) = self.repaint.lock().unwrap().as_ref() {
            f();
        }
    }

    fn set_slot(&self, i: usize, f: impl FnOnce(&mut SlotStatus)) {
        if let Some(s) = self.slots.lock().unwrap().get_mut(i) {
            f(s);
        }
        self.poke();
    }
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub mode: Mode,
    pub test: bool,
    pub check_items: bool,
    pub redo: bool,
    pub accounts: Vec<String>,
}

enum Halt {
    Stop,
    Failed(String),
    /// The game isn't ready for this yet (e.g. publishing locked): wait, then retry the outfit.
    Wait(f64, String),
}

type Res<T = ()> = Result<T, Halt>;

/// Only one window may hold the mouse and keyboard.
#[derive(Default)]
struct InputLock {
    owner: Mutex<Option<usize>>,
    cv: Condvar,
}

impl InputLock {
    fn acquire(&self, id: usize, stop: &AtomicBool) -> bool {
        let mut owner = self.owner.lock().unwrap();
        loop {
            if stop.load(Ordering::SeqCst) {
                return false;
            }
            match *owner {
                None => {
                    *owner = Some(id);
                    return true;
                }
                Some(o) if o == id => return true,
                _ => owner = self.cv.wait_timeout(owner, Duration::from_millis(200)).unwrap().0,
            }
        }
    }

    fn release(&self, id: usize) {
        let mut owner = self.owner.lock().unwrap();
        if *owner == Some(id) {
            *owner = None;
            self.cv.notify_all();
        }
    }
}

struct Common {
    cfg: Config,
    opts: RunOptions,
    shared: Arc<Shared>,
    queue: Mutex<VecDeque<Outfit>>,
    lock: InputLock,
    published_lock: Mutex<()>,
    dir: PathBuf,
    multi: bool,
}

struct Slot {
    id: usize,
    label: String,
    win: win::Win,
    points: BTreeMap<String, [i32; 2]>,
    states: BTreeMap<String, StateSpec>,
    count: u32,
    failed: u32,
    has_input: bool,
}

impl Common {
    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    fn sleep(&self, secs: f64) -> Res {
        let end = Instant::now() + Duration::from_secs_f64(secs.max(0.0));
        while Instant::now() < end {
            if self.stopped() {
                return Err(Halt::Stop);
            }
            thread::sleep((end - Instant::now()).min(Duration::from_millis(50)));
        }
        if self.stopped() {
            return Err(Halt::Stop);
        }
        Ok(())
    }

    fn log(&self, slot: &Slot, msg: impl AsRef<str>) {
        self.shared.log(format!("[{}] {}", slot.label, msg.as_ref()));
    }

    fn doing(&self, slot: &Slot, what: impl Into<String>) {
        let what = what.into();
        self.shared.set_slot(slot.id, |s| s.doing = what);
    }

    /// Take the mouse and keyboard and make sure this window is in front.
    fn take_input(&self, slot: &mut Slot) -> Res {
        if !slot.has_input {
            if !self.lock.acquire(slot.id, &self.shared.stop) {
                return Err(Halt::Stop);
            }
            slot.has_input = true;
        }
        if !slot.win.exists() {
            return Err(Halt::Failed("the Roblox window closed".into()));
        }
        if !slot.win.is_foreground() {
            slot.win.focus();
            if !slot.win.is_foreground() {
                slot.win.focus();
            }
            if !slot.win.is_foreground() {
                self.sleep(0.3)?;
                slot.win.focus();
            }
            if !slot.win.is_foreground() {
                return Err(Halt::Failed("Windows wouldn't bring the Roblox window to the front".into()));
            }
            let r = slot.win.rect();
            input::hover(r.left + r.width / 2, r.top + r.height / 2);
            self.sleep(self.cfg.delay("after_focus", 0.3))?;
        }
        Ok(())
    }

    fn give_input(&self, slot: &mut Slot) {
        if slot.has_input {
            self.lock.release(slot.id);
            slot.has_input = false;
        }
    }

    /// Wait without holding the input, so other windows can work.
    fn idle(&self, slot: &mut Slot, secs: f64) -> Res {
        if self.multi && secs >= 0.9 {
            self.give_input(slot);
        }
        self.sleep(secs)
    }

    fn screen_is(&self, slot: &Slot, state: &str) -> bool {
        let Some(spec) = slot.states.get(state) else { return false };
        screen::capture(slot.win.rect()).is_some_and(|shot| shot.matches(spec))
    }

    fn wait_for(&self, slot: &Slot, state: &str, timeout: f64, negate: bool) -> Res<bool> {
        let end = Instant::now() + Duration::from_secs_f64(timeout);
        loop {
            if self.stopped() {
                return Err(Halt::Stop);
            }
            if self.screen_is(slot, state) != negate {
                return Ok(true);
            }
            if Instant::now() >= end {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn detect(&self, slot: &Slot) -> Option<String> {
        let shot = screen::capture(slot.win.rect())?;
        self.cfg
            .recovery
            .order
            .iter()
            .find(|name| slot.states.get(*name).is_some_and(|spec| shot.matches(spec)))
            .cloned()
    }

    fn click(&self, slot: &mut Slot, name: &str) -> Res {
        let [x, y] = *slot
            .points
            .get(name)
            .ok_or_else(|| Halt::Failed(format!("no position for '{name}' at this window size")))?;
        self.take_input(slot)?;
        let r = slot.win.rect();
        let (sx, sy) = (r.left + x, r.top + y);
        // Never click blind: something else (another window, a popup) may be on top.
        if !slot.win.owns_point(sx, sy) {
            slot.win.focus();
            self.sleep(0.25)?;
            if !slot.win.owns_point(sx, sy) {
                return Err(Halt::Failed(format!("another window is covering the Roblox window where '{name}' is")));
            }
        }
        input::click(sx, sy);
        Ok(())
    }

    /// Close whatever CAC panel is open until the main screen shows.
    fn recover(&self, slot: &mut Slot) -> Res<bool> {
        self.doing(slot, "getting back to the main screen");
        for _ in 0..8 {
            self.take_input(slot)?;
            match self.detect(slot) {
                Some(s) if s == "main" => return Ok(true),
                Some(s) => {
                    if let Some(point) = self.cfg.recovery.close.get(&s).cloned() {
                        self.log(slot, format!("  on {s}, clicking {point}"));
                        self.click(slot, &point)?;
                    }
                }
                None => {}
            }
            self.sleep(1.2)?;
        }
        Ok(self.detect(slot).as_deref() == Some("main"))
    }

    /// Back to the catalog list after a failed item: close an item page if one is open.
    fn retry_prepare(&self, slot: &mut Slot) -> Res<bool> {
        for _ in 0..3 {
            self.take_input(slot)?;
            match self.detect(slot).as_deref() {
                Some("catalog_list") => return Ok(true),
                Some("item_page") | Some("item_loading") => self.click(slot, "close_item")?,
                _ => {}
            }
            self.sleep(1.0)?;
        }
        Ok(self.screen_is(slot, "catalog_list"))
    }

    fn snapshot(&self, slot: &Slot, why: &str) {
        let dir = config::logs(&self.dir).join("failures");
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = slot.label.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let path = dir.join(format!("{}-{}-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"), safe, why));
        if let Some(shot) = screen::capture(slot.win.rect()) {
            if shot.save_png(&path).is_ok() {
                self.log(slot, format!("  screenshot: logs\\failures\\{}", path.file_name().unwrap().to_string_lossy()));
            }
        }
    }

    fn do_step(&self, slot: &mut Slot, step: &Step, fields: &[(&str, String)]) -> Res {
        let fill = |t: &str| fields.iter().fold(t.to_string(), |acc, (k, v)| acc.replace(&format!("{{{k}}}"), v));
        if let Some(name) = &step.click {
            let (expect, expect_not) = (step.expect.as_deref(), step.expect_not.as_deref());
            let checks = expect.is_some() || expect_not.is_some();
            let tries = 1 + step.retries.unwrap_or(if checks { 2 } else { 0 });
            let timeout = step.timeout.unwrap_or_else(|| self.cfg.delay("expect_timeout", 3.0));
            let mut ok = !checks;
            for attempt in 0..tries {
                for _ in 0..step.times.unwrap_or(1).max(1) {
                    self.click(slot, name)?;
                }
                if !checks {
                    break;
                }
                if let Some(e) = expect {
                    if self.wait_for(slot, e, timeout, false)? {
                        ok = true;
                        break;
                    }
                }
                if let Some(e) = expect_not {
                    if self.wait_for(slot, e, timeout, true)? {
                        ok = true;
                        break;
                    }
                }
                if attempt + 1 < tries {
                    self.log(slot, format!("  {name}: screen didn't change, retrying ({}/{})", attempt + 1, tries - 1));
                }
            }
            if !ok {
                if let Some(secs) = step.extra.get("on_fail_wait").and_then(Value::as_f64) {
                    let why = step.extra.get("on_fail_note").and_then(Value::as_str).unwrap_or("the game wasn't ready");
                    return Err(Halt::Wait(secs, format!("{name} did nothing: {why}")));
                }
                let want = expect.map(String::from).unwrap_or_else(|| format!("not {}", expect_not.unwrap_or("?")));
                return Err(Halt::Failed(format!("after clicking {name}, expected {want}")));
            }
        } else if let Some(state) = &step.wait_for {
            self.take_input(slot)?;
            if !self.wait_for(slot, state, step.timeout.unwrap_or(6.0), false)? {
                return Err(Halt::Failed(format!("{state} never appeared")));
            }
            return Ok(());
        } else if let Some(text) = &step.text {
            self.take_input(slot)?;
            input::type_text(&fill(text), self.cfg.delay("key_interval", 0.03));
        } else if let Some(k) = &step.key {
            self.take_input(slot)?;
            if !input::key(k) {
                return Err(Halt::Failed(format!("unknown key '{k}'")));
            }
        } else {
            return Err(Halt::Failed(format!("unknown step in config flow: {}", serde_json::to_string(step).unwrap_or_default())));
        }
        self.sleep(self.cfg.delay("after_step", 0.4))
    }

    fn run_steps(&self, slot: &mut Slot, steps: &[Step], fields: &[(&str, String)], publish_clicked: &mut bool) -> Res {
        for step in steps {
            if self.opts.test && step.live_only == Some(true) {
                continue;
            }
            if !self.opts.test && step.test_only == Some(true) {
                continue;
            }
            if let Some(w) = &step.wait {
                let secs = match w {
                    Value::Number(n) => n.as_f64().unwrap_or(0.0),
                    Value::String(s) => self.cfg.delay(s, 1.0),
                    _ => 0.0,
                };
                self.idle(slot, secs)?;
                continue;
            }
            if step.publishes == Some(true) {
                *publish_clicked = true;
            }
            self.do_step(slot, step, fields)?;
        }
        Ok(())
    }

    fn record(&self, slot: &Slot, outfit: &Outfit) {
        let _g = self.published_lock.lock().unwrap();
        let path = config::data(&self.dir).join("published.json");
        let mut file: PublishedFile = config::read_json(&path).unwrap_or_default();
        file.published.push(PublishedEntry {
            name: outfit.name.clone(),
            account: slot.label.clone(),
            at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        });
        if let Err(e) = config::write_json(&path, &file) {
            self.log(slot, format!("could not save published.json: {e}"));
        }
    }

    /// Freshly launched windows: wait until CAC shows a screen the bot knows.
    fn wait_until_loaded(&self, slot: &mut Slot) -> Res {
        if self.opts.mode != Mode::Launch {
            return Ok(());
        }
        self.doing(slot, "waiting for CAC to load");
        let end = Instant::now() + Duration::from_secs_f64(self.cfg.ram.game_load_seconds + 60.0);
        let size = window_size(&self.cfg);
        let want = format!("{}x{}", size[0], size[1]);
        while Instant::now() < end {
            if slot.win.size_key() != want {
                // Roblox can go back to fullscreen when it finishes loading.
                if !self.lock.acquire(slot.id, &self.shared.stop) {
                    return Err(Halt::Stop);
                }
                slot.has_input = true;
                if let Err(e) = win::normalize(slot.win, slot.id, size) {
                    self.log(slot, e);
                }
                self.give_input(slot);
            }
            if self.detect(slot).is_some() {
                self.log(slot, "CAC loaded");
                return self.sleep(1.0);
            }
            self.sleep(0.5)?;
        }
        self.log(slot, "CAC didn't show a known screen in time; trying anyway");
        Ok(())
    }

    fn job(&self, slot: &mut Slot) -> Res {
        let limit = self.cfg.max_publish_per_run;
        let max_fail = self.cfg.max_failures_per_window.max(1);
        let mut waits = 0;
        self.wait_until_loaded(slot)?;
        self.take_input(slot)?;
        if !self.recover(slot)? {
            self.log(slot, "CAC isn't on its main screen and couldn't be closed back to it; skipping this window.");
            self.snapshot(slot, "stuck");
            return Ok(());
        }
        while slot.count < limit {
            let Some(outfit) = self.queue.lock().unwrap().pop_front() else { break };
            self.log(slot, format!("building \"{}\" ({} items)", outfit.name, outfit.items.len()));
            self.doing(slot, format!("building \"{}\"", outfit.name));
            let base = vec![("name", outfit.name.clone())];
            let mut publish_clicked = false;
            let mut result = self.run_steps(slot, &self.cfg.flow.start, &base, &mut publish_clicked);
            for (n, id) in outfit.items.iter().enumerate() {
                if result.is_err() {
                    break;
                }
                self.doing(slot, format!("\"{}\": item {}/{}", outfit.name, n + 1, outfit.items.len()));
                let mut fields = base.clone();
                fields.push(("item_id", id.to_string()));
                result = self.run_steps(slot, &self.cfg.flow.per_item, &fields, &mut publish_clicked);
                if let Err(Halt::Failed(why)) = &result {
                    // One dropped click shouldn't cost the whole outfit: tidy up and try this item again.
                    self.log(slot, format!("  item {id}: {why}; retrying it once"));
                    if self.retry_prepare(slot).unwrap_or(false) {
                        result = self.run_steps(slot, &self.cfg.flow.per_item, &fields, &mut publish_clicked);
                    }
                }
            }
            if result.is_ok() {
                self.doing(slot, format!("publishing \"{}\"", outfit.name));
                result = self.run_steps(slot, &self.cfg.flow.finish, &base, &mut publish_clicked);
            }
            match result {
                Err(Halt::Stop) => return Err(Halt::Stop),
                Err(Halt::Wait(secs, why)) => {
                    waits += 1;
                    if waits > 15 {
                        self.log(slot, format!("{why}; still blocked after 15 tries, stopping this window."));
                        self.queue.lock().unwrap().push_front(outfit);
                        return Ok(());
                    }
                    self.log(slot, format!("{why}. Waiting {secs:.0}s in the game, then trying \"{}\" again ({waits}/15).", outfit.name));
                    self.queue.lock().unwrap().push_front(outfit);
                    if !self.recover(slot)? {
                        self.log(slot, "couldn't get back to the main screen; stopping this window.");
                        return Ok(());
                    }
                    self.doing(slot, format!("waiting {secs:.0}s (publishing locked)"));
                    self.give_input(slot);
                    self.sleep(secs)?;
                    continue;
                }
                Err(Halt::Failed(why)) => {
                    slot.failed += 1;
                    let failed = slot.failed;
                    self.shared.set_slot(slot.id, |s| s.problems = failed);
                    self.log(slot, format!("problem with \"{}\": {why}", outfit.name));
                    self.snapshot(slot, "failed");
                    if publish_clicked {
                        self.record(slot, &outfit);
                        slot.count += 1;
                        self.log(slot, "(publish was already clicked; counted as published)");
                    } else {
                        self.log(slot, format!("skipped \"{}\" (still unpublished; the next run retries it)", outfit.name));
                    }
                    if !slot.win.exists() {
                        self.log(slot, "the window closed; stopping this window.");
                        return Ok(());
                    }
                    if !self.recover(slot)? {
                        self.log(slot, "couldn't get back to the main screen; stopping this window.");
                        self.snapshot(slot, "stuck");
                        return Ok(());
                    }
                    if slot.failed >= max_fail {
                        self.log(slot, format!("{} problems; stopping this window.", slot.failed));
                        return Ok(());
                    }
                }
                Ok(()) => {
                    slot.count += 1;
                    if self.opts.test {
                        self.log(slot, format!("built \"{}\" (test mode: cancelled instead of publishing)", outfit.name));
                    } else {
                        self.record(slot, &outfit);
                        self.log(slot, format!("published \"{}\"", outfit.name));
                    }
                }
            }
            let count = slot.count;
            self.shared.set_slot(slot.id, |s| s.done = count);
            if slot.count < limit && !self.queue.lock().unwrap().is_empty() {
                self.doing(slot, format!("waiting {}s before the next outfit", self.cfg.seconds_between_outfits));
                self.give_input(slot);
                self.sleep(self.cfg.seconds_between_outfits)?;
            }
        }
        Ok(())
    }
}

fn layout_for(cfg: &Config, w: win::Win) -> Option<(BTreeMap<String, [i32; 2]>, BTreeMap<String, StateSpec>)> {
    let key = w.size_key();
    Some((cfg.points.get(&key)?.clone(), cfg.states.get(&key)?.clone()))
}

fn window_size(cfg: &Config) -> [i32; 2] {
    cfg.launch_window_size.unwrap_or([816, 638])
}

/// Find or launch the Roblox windows and get each one windowed at a size the bot knows.
fn make_windows(cfg: &Config, opts: &RunOptions, shared: &Shared) -> Result<Vec<(String, win::Win)>, String> {
    let title = &cfg.window_title;
    let stop = || shared.stop.load(Ordering::SeqCst);
    let mut wins: Vec<(String, win::Win)> = Vec::new();
    match opts.mode {
        Mode::Single => {
            let found = win::find_windows(title);
            let fg = win::foreground();
            let w = found.iter().copied().find(|w| *w == fg).or_else(|| found.first().copied());
            let w = w.ok_or("No Roblox window is open. Open Catalog Avatar Creator first.")?;
            wins.push(("main".into(), w));
        }
        Mode::Attach => {
            let found = win::find_windows(title);
            if found.is_empty() {
                return Err("No Roblox windows are open.".into());
            }
            for (i, w) in found.into_iter().enumerate() {
                wins.push((format!("window {}", i + 1), w));
            }
        }
        Mode::Launch => {
            if opts.accounts.is_empty() {
                return Err("No accounts selected. Pick some on the Accounts tab.".into());
            }
            let saved = accounts::load();
            let open_already = win::find_windows(title).len();
            if opts.accounts.len() + open_already > 1 && !shared.multi_on.load(Ordering::SeqCst) {
                return Err("Multi-Roblox is off because Roblox was already running when the app opened. Close every Roblox window, wait a few seconds for the status bar to say 'Multi-Roblox: on', then press Start.".into());
            }
            for (i, account) in opts.accounts.iter().enumerate() {
                if stop() {
                    return Err("stopped".into());
                }
                let Some(acc) = saved.iter().find(|a| &a.name == account) else {
                    shared.log(format!("  {account}: not in the account list (sign in to it on the Accounts tab); skipping"));
                    continue;
                };
                let known: HashSet<win::Win> = win::find_windows(title).into_iter().collect();
                shared.log(format!("launching {account} into CAC ({}/{})", i + 1, opts.accounts.len()));
                let mut new = None;
                let mut failed = None;
                for attempt in 0..2 {
                    let started = std::time::SystemTime::now() - Duration::from_secs(2);
                    if let Err(e) = accounts::launch(acc, cfg.ram.place_id) {
                        failed = Some(e);
                        break;
                    }
                    let end = Instant::now() + Duration::from_secs_f64(cfg.ram.window_timeout);
                    failed = None;
                    while Instant::now() < end && !stop() {
                        new = win::find_windows(title).into_iter().find(|w| !known.contains(w));
                        if new.is_some() {
                            break;
                        }
                        failed = accounts::launch_failure(started);
                        if failed.is_some() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(500));
                    }
                    // A 403 usually means the player's own saved sign-in is a banned account: clear it, try once more.
                    let refused = failed.as_deref().is_some_and(|f| f.contains("403"));
                    if new.is_some() || attempt == 1 || !refused || stop() {
                        break;
                    }
                    win::close_titled("Authentication Failed");
                    if accounts::reset_player_signin() {
                        shared.log(format!("  {account}: the Roblox player had an old saved sign-in; cleared it, trying again"));
                    } else {
                        break;
                    }
                    thread::sleep(Duration::from_secs(3));
                }
                match (new, failed) {
                    (Some(w), _) => {
                        shared.log(format!("  {account}: Roblox window opened"));
                        wins.push((account.clone(), w));
                    }
                    (None, Some(why)) => shared.log(format!("  {account}: {why}; skipping")),
                    (None, None) => shared.log(format!("  {account}: no Roblox window appeared; skipping")),
                }
                if i + 1 < opts.accounts.len() {
                    let end = Instant::now() + Duration::from_secs_f64(cfg.ram.seconds_between_launches);
                    while Instant::now() < end && !stop() {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            if wins.is_empty() {
                return Err("No account got a Roblox window.".into());
            }
        }
    }
    if stop() {
        return Err("stopped".into());
    }

    // More than one window: every one goes windowed at the standard size and gets tiled.
    // One window: keep its size if the bot has a layout for it, otherwise make it standard.
    let size = window_size(cfg);
    let force = wins.len() > 1;
    for (i, (label, w)) in wins.iter().enumerate() {
        let known = layout_for(cfg, *w).is_some();
        if force || !known {
            shared.log(format!("[{label}] setting window to {}x{} (windowed)", size[0], size[1]));
            if let Err(e) = win::normalize(*w, i, size) {
                shared.log(format!("[{label}] {e}"));
            }
            thread::sleep(Duration::from_millis(600));
        }
    }
    Ok(wins)
}

/// Outfits still to do, after dropping published ones (unless redo) and bad items.
fn build_queue(cfg: &Config, opts: &RunOptions, shared: &Shared, dir: &std::path::Path) -> Result<VecDeque<Outfit>, String> {
    let file: config::OutfitsFile = config::read_json(&config::data(dir).join("outfits.json"))?;
    let published: PublishedFile = config::read_json(&config::data(dir).join("published.json")).unwrap_or_default();
    let done: HashSet<&str> = published.published.iter().map(|p| p.name.as_str()).collect();
    let mut queue = VecDeque::new();
    let mut skipped = 0;
    for o in file.outfits {
        if o.items.is_empty() || o.name.trim().is_empty() {
            continue;
        }
        if !opts.redo && !opts.test && done.contains(o.name.as_str()) {
            skipped += 1;
            continue;
        }
        queue.push_back(o);
    }
    if skipped > 0 {
        shared.log(format!("{skipped} outfit(s) already published; skipping them (tick 'Publish again' to redo)"));
    }
    if opts.check_items {
        shared.log("checking item IDs with the Roblox catalog...");
        let mut names: BTreeMap<u64, Result<String, String>> = BTreeMap::new();
        for o in &queue {
            for id in &o.items {
                if !names.contains_key(id) {
                    names.insert(*id, ram::item_name(*id));
                }
            }
        }
        for (id, r) in &names {
            match r {
                Ok(n) => shared.log(format!("  {id}: {n}")),
                Err(e) => shared.log(format!("  {id}: NOT FOUND ({e})")),
            }
        }
        let before = queue.len();
        queue.retain(|o| o.items.iter().all(|id| names.get(id).is_some_and(|r| r.is_ok())));
        if queue.len() < before {
            shared.log(format!("{} outfit(s) have a bad item ID; leaving them out", before - queue.len()));
        }
    }
    let _ = cfg;
    Ok(queue)
}

/// Run the whole thing. Blocks until done or stopped.
pub fn run(shared: Arc<Shared>, opts: RunOptions) {
    shared.stop.store(false, Ordering::SeqCst);
    shared.running.store(true, Ordering::SeqCst);
    shared.slots.lock().unwrap().clear();
    let result = run_inner(&shared, opts);
    if let Err(e) = result {
        shared.log(e);
    }
    shared.running.store(false, Ordering::SeqCst);
    shared.log("finished.");
}

fn run_inner(shared: &Arc<Shared>, opts: RunOptions) -> Result<(), String> {
    let dir = config::data_dir();
    let cfg = config::load_config(&dir)?;
    let queue = build_queue(&cfg, &opts, shared, &dir)?;
    if queue.is_empty() {
        return Err("Nothing to do: no unpublished outfits with items.".into());
    }
    shared.log(format!(
        "{} outfit(s) to build, {} per window max, {}s apart{}",
        queue.len(),
        cfg.max_publish_per_run,
        cfg.seconds_between_outfits,
        if opts.test { " - TEST MODE (cancels instead of publishing)" } else { "" }
    ));

    let wins = make_windows(&cfg, &opts, shared)?;
    let mut slots = Vec::new();
    for (label, w) in wins {
        match layout_for(&cfg, w) {
            Some((points, states)) => slots.push(Slot {
                id: slots.len(),
                label,
                win: w,
                points,
                states,
                count: 0,
                failed: 0,
                has_input: false,
            }),
            None => shared.log(format!(
                "[{label}] window is {}, and the bot only knows {:?}; skipping it",
                w.size_key(),
                cfg.sizes()
            )),
        }
    }
    if slots.is_empty() {
        return Err("No usable Roblox window.".into());
    }
    *shared.slots.lock().unwrap() = slots
        .iter()
        .map(|s| SlotStatus { label: s.label.clone(), doing: "starting".into(), ..Default::default() })
        .collect();

    let common = Arc::new(Common {
        multi: slots.len() > 1,
        cfg,
        opts,
        shared: shared.clone(),
        queue: Mutex::new(queue),
        lock: InputLock::default(),
        published_lock: Mutex::new(()),
        dir,
    });
    let handles: Vec<_> = slots
        .into_iter()
        .map(|mut slot| {
            let c = common.clone();
            thread::spawn(move || {
                let r = c.job(&mut slot);
                c.give_input(&mut slot);
                let note = match r {
                    Err(Halt::Stop) => "stopped".to_string(),
                    _ => format!("done ({} outfit(s))", slot.count),
                };
                c.shared.set_slot(slot.id, |s| {
                    s.doing = note;
                    s.finished = true;
                });
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }
    let left = common.queue.lock().unwrap().len();
    if left > 0 && !common.stopped() {
        shared.log(format!("{left} outfit(s) left over (every window hit its limit or stopped)"));
    }
    Ok(())
}

/// Take every open Roblox window out of fullscreen, set the standard size and tile them.
pub fn window_all(shared: Arc<Shared>) {
    let cfg = match config::load_config(&config::data_dir()) {
        Ok(c) => c,
        Err(e) => return shared.log(e),
    };
    let size = window_size(&cfg);
    let wins = win::find_windows(&cfg.window_title);
    if wins.is_empty() {
        return shared.log("No Roblox windows are open.");
    }
    for (i, w) in wins.iter().enumerate() {
        match win::normalize(*w, i, size) {
            Ok(()) => shared.log(format!("window {}: windowed at {}x{}", i + 1, size[0], size[1])),
            Err(e) => shared.log(format!("window {}: {e}", i + 1)),
        }
    }
}

/// Stop when End is pressed (anywhere; F12 would also start a Roblox video recording), for as long as the bot runs.
pub fn watch_stop_key(shared: Arc<Shared>) {
    thread::spawn(move || {
        while shared.running.load(Ordering::SeqCst) {
            if input::key_held(0x23) && !shared.stop.swap(true, Ordering::SeqCst) {
                shared.log("End pressed: stopping...");
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
}
