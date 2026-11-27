#![windows_subsystem = "windows"]

mod accounts;
mod avatar;
mod clothing;
mod clothing_link;
mod config;
mod engine;
mod input;
mod ram;
mod screen;
mod win;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::{Config, Mode, Outfit, OutfitsFile, PublishedFile};
use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use engine::Shared;

const WIDTH: f32 = 960.0;
const HEIGHT: f32 = 680.0;

const BG: Color32 = Color32::from_rgb(22, 24, 28);
const CARD: Color32 = Color32::from_rgb(31, 34, 40);
const BORDER: Color32 = Color32::from_rgb(48, 52, 60);
const INPUT: Color32 = Color32::from_rgb(16, 17, 20);
const ACCENT: Color32 = Color32::from_rgb(52, 168, 98);
const DANGER: Color32 = Color32::from_rgb(200, 64, 64);
const WARN: Color32 = Color32::from_rgb(230, 170, 60);
const GOOD: Color32 = Color32::from_rgb(90, 200, 120);
const MUTED: Color32 = Color32::from_rgb(140, 146, 156);
const TEXT: Color32 = Color32::from_rgb(220, 223, 228);

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Run,
    Outfits,
    Accounts,
    Clothing,
    Settings,
}

struct Row {
    name: String,
    items: String,
}

/// Background check of what is open, refreshed every 2 seconds.
#[derive(Default, Clone)]
struct Live {
    roblox: Vec<(String, bool)>,
    multi_on: bool,
}

/// A sign-in or import running in the background.
#[derive(Default)]
struct Job {
    busy: Option<String>,
    result: Option<String>,
    changed: bool,
}

struct App {
    dir: PathBuf,
    cfg: Option<Config>,
    error: Option<String>,
    rows: Vec<Row>,
    published: HashSet<String>,
    tab: Tab,
    shared: Arc<Shared>,
    live: Arc<Mutex<Live>>,
    probe: Arc<Mutex<(String, u16)>>,
    accounts: Vec<accounts::Account>,
    job: Arc<Mutex<Job>>,
    paste_cookie: String,
    redo: bool,
    copies: u32,
    bulk: String,
    filter: String,
    notice: (String, Instant),
    auto_start: bool,
    saved_snapshot: String,
    dirty_since: Option<Instant>,
    confirm_clear: bool,
    run_started: Option<Instant>,
    was_running: bool,
    studio: clothing::Studio,
    avatar_link: String,
    avatar_job: Option<std::sync::mpsc::Receiver<Result<avatar::Worn, String>>>,
    avatar_last: Option<String>,
    avatar_msg: Option<(String, bool)>,
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        use egui::{FontId, TextStyle};
        s.visuals = egui::Visuals::dark();
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = INPUT;
        v.faint_bg_color = Color32::from_rgb(36, 39, 46);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT.linear_multiply(0.6);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.widgets.noninteractive.corner_radius = CornerRadius::same(6);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 48, 56);
        v.widgets.inactive.bg_fill = Color32::from_rgb(44, 48, 56);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(56, 61, 71);
        v.widgets.hovered.bg_fill = Color32::from_rgb(56, 61, 71);
    });
}

/// A titled, rounded box.
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.0));
                ui.add_space(2.0);
            }
            add(ui)
        })
        .inner
}

fn dot(ui: &mut egui::Ui, color: Color32, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new("●").color(color));
    ui.label(RichText::new(text.into()).color(MUTED))
}

fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.linear_multiply(0.25))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| ui.label(RichText::new(text).color(color).size(12.0).strong()));
}

fn rows_from(file: OutfitsFile) -> Vec<Row> {
    file.outfits
        .into_iter()
        .map(|o| Row { name: o.name, items: o.items.iter().map(u64::to_string).collect::<Vec<_>>().join(", ") })
        .collect()
}

fn log_color(line: &str) -> Color32 {
    let l = line.to_ascii_lowercase();
    if l.contains("problem") || l.contains("not found") || l.contains("could not") || l.contains("couldn't")
        || l.contains("refused") || l.contains("no roblox") || l.contains("not answering") || l.contains("skipping")
    {
        Color32::from_rgb(235, 110, 110)
    } else if l.contains("] published \"") || l.contains("] built \"") || l.contains("windowed at") {
        GOOD
    } else if l.contains("test mode") || l.contains("stopping") || l.contains("retrying") || l.contains("skipped") {
        WARN
    } else if l.contains("] building \"") {
        TEXT
    } else {
        MUTED
    }
}

fn start_prober(live: Arc<Mutex<Live>>, probe: Arc<Mutex<(String, u16)>>, shared: Arc<Shared>, ctx: egui::Context) {
    std::thread::spawn(move || {
        // The hidden helper process holds the multi-Roblox lock (see accounts.rs).
        accounts::app_started();
        loop {
            let (title, _) = probe.lock().unwrap().clone();
            let wins = win::find_windows(&title);
            accounts::ensure_lock_holder();
            let multi_on = accounts::multi_on();
            shared.multi_on.store(multi_on, Ordering::SeqCst);
            let roblox = wins.into_iter().map(|w| (w.size_key(), w.is_fullscreen())).collect::<Vec<_>>();
            let changed = {
                let mut l = live.lock().unwrap();
                let changed = l.roblox != roblox || l.multi_on != multi_on;
                *l = Live { roblox, multi_on };
                changed
            };
            if changed {
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        style(&cc.egui_ctx);
        let shared = Arc::new(Shared::default());
        let ctx = cc.egui_ctx.clone();
        *shared.repaint.lock().unwrap() = Some(Box::new(move || ctx.request_repaint()));
        let live = Arc::new(Mutex::new(Live::default()));
        let probe = Arc::new(Mutex::new(("Roblox".to_string(), 7963u16)));
        start_prober(live.clone(), probe.clone(), shared.clone(), cc.egui_ctx.clone());
        let mut app = App {
            dir: config::data_dir(),
            cfg: None,
            error: None,
            rows: Vec::new(),
            published: HashSet::new(),
            tab: Tab::Run,
            shared,
            live,
            probe,
            accounts: accounts::load(),
            job: Arc::new(Mutex::new(Job::default())),
            paste_cookie: String::new(),
            redo: false,
            copies: 5,
            bulk: String::new(),
            filter: String::new(),
            notice: (String::new(), Instant::now()),
            auto_start: std::env::args().any(|a| a == "--start"),
            saved_snapshot: String::new(),
            dirty_since: None,
            confirm_clear: false,
            run_started: None,
            was_running: false,
            studio: clothing::Studio::default(),
            avatar_link: String::new(),
            avatar_job: None,
            avatar_last: None,
            avatar_msg: None,
        };
        app.reload();
        app
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.notice = (msg.into(), Instant::now());
    }

    fn reload(&mut self) {
        self.error = None;
        match config::load_config(&self.dir) {
            Ok(c) => self.cfg = Some(c),
            Err(e) => self.error = Some(e),
        }
        match config::read_json::<OutfitsFile>(&config::data(&self.dir).join("outfits.json")) {
            Ok(f) => self.rows = rows_from(f),
            Err(e) => self.error = Some(e),
        }
        self.reload_published();
        self.saved_snapshot = self.snapshot();
        self.dirty_since = None;
    }

    fn reload_published(&mut self) {
        self.published = config::read_json::<PublishedFile>(&config::data(&self.dir).join("published.json"))
            .map(|p| p.published.into_iter().map(|e| e.name).collect())
            .unwrap_or_default();
    }

    fn outfits(&self) -> Vec<Outfit> {
        self.rows.iter().map(|r| Outfit { name: r.name.trim().to_string(), items: config::parse_ids(&r.items) }).collect()
    }

    fn snapshot(&self) -> String {
        let cfg = self.cfg.as_ref().and_then(|c| serde_json::to_string(c).ok()).unwrap_or_default();
        let outfits = serde_json::to_string(&self.outfits()).unwrap_or_default();
        cfg + &outfits
    }

    fn save(&mut self) -> Result<(), String> {
        if let Some(cfg) = &self.cfg {
            config::write_json(&config::data(&self.dir).join("config.json"), cfg)?;
        }
        config::write_json(&config::data(&self.dir).join("outfits.json"), &OutfitsFile { outfits: self.outfits() })?;
        self.saved_snapshot = self.snapshot();
        self.dirty_since = None;
        Ok(())
    }

    /// Save on its own a moment after the last change.
    fn autosave(&mut self, ctx: &egui::Context) {
        if self.cfg.is_none() {
            return;
        }
        if self.snapshot() == self.saved_snapshot {
            self.dirty_since = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= Duration::from_millis(800) {
            match self.save() {
                Ok(()) => self.say("Saved"),
                Err(e) => self.say(format!("Could not save: {e}")),
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(850));
        }
    }

    fn running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// Outfits a run would build right now.
    fn queued(&self) -> usize {
        let test = self.cfg.as_ref().is_some_and(|c| c.gui.test_mode);
        self.outfits()
            .iter()
            .filter(|o| !o.items.is_empty() && !o.name.is_empty())
            .filter(|o| self.redo || test || !self.published.contains(&o.name))
            .count()
    }

    fn window_count(&self) -> usize {
        let Some(cfg) = &self.cfg else { return 0 };
        match cfg.gui.mode {
            Mode::Single => 1,
            Mode::Attach => self.live.lock().unwrap().roblox.len().max(1),
            Mode::Launch => cfg.accounts.len(),
        }
    }

    fn start(&mut self) {
        if self.running() {
            return;
        }
        if let Err(e) = self.save() {
            self.say(format!("Could not save: {e}"));
            return;
        }
        let Some(cfg) = &self.cfg else { return };
        let opts = engine::RunOptions {
            mode: cfg.gui.mode,
            test: cfg.gui.test_mode,
            check_items: cfg.gui.check_items,
            redo: self.redo,
            accounts: cfg.accounts.clone(),
        };
        // Claim the run first, so a double click can never start two runs.
        if self.shared.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return;
        }
        self.shared.log.lock().unwrap().clear();
        self.shared.stop.store(false, Ordering::SeqCst);
        engine::watch_stop_key(self.shared.clone());
        let shared = self.shared.clone();
        std::thread::spawn(move || engine::run(shared, opts));
        self.tab = Tab::Run;
        self.run_started = Some(Instant::now());
    }

    fn stop(&self) {
        if !self.shared.stop.swap(true, Ordering::SeqCst) {
            self.shared.log("stopping...");
        }
    }

    // ---------------------------------------------------------------- header / footer

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("CAC Outfit Bot").size(19.0).strong().color(Color32::WHITE));
            ui.add_space(14.0);
            for (tab, name) in [(Tab::Run, "Run"), (Tab::Outfits, "Outfits"), (Tab::Accounts, "Accounts"), (Tab::Clothing, "Clothing"), (Tab::Settings, "Settings")] {
                let selected = self.tab == tab;
                let text = RichText::new(name).size(15.0).color(if selected { Color32::WHITE } else { MUTED });
                let b = egui::Button::new(text)
                    .fill(if selected { Color32::from_rgb(48, 53, 62) } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(84.0, 30.0));
                if ui.add(b).clicked() {
                    self.tab = tab;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let running = self.running();
                let stopping = running && self.shared.stop.load(Ordering::SeqCst);
                let (text, color) = match (running, stopping) {
                    (true, true) => ("Stopping...", Color32::from_rgb(120, 60, 60)),
                    (true, false) => ("Stop   (End)", DANGER),
                    _ => ("Start", ACCENT),
                };
                let b = egui::Button::new(RichText::new(text).size(17.0).strong().color(Color32::WHITE))
                    .fill(color)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(150.0, 36.0));
                if ui.add_enabled(self.cfg.is_some() && !stopping, b).clicked() {
                    if running {
                        self.stop();
                    } else {
                        self.start();
                    }
                }
                if let Some(cfg) = &self.cfg {
                    if cfg.gui.test_mode {
                        badge(ui, "TEST MODE", WARN);
                    } else {
                        badge(ui, "LIVE", GOOD);
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let live = self.live.lock().unwrap().clone();
        ui.horizontal(|ui| {
            match live.roblox.len() {
                0 => {
                    dot(ui, DANGER, "Roblox: not open");
                }
                n => {
                    let sizes: Vec<String> =
                        live.roblox.iter().map(|(s, fs)| if *fs { format!("{s} fullscreen") } else { s.clone() }).collect();
                    let known = self.cfg.as_ref().map(|c| c.sizes()).unwrap_or_default();
                    let ok = live.roblox.iter().all(|(s, _)| known.contains(s));
                    dot(ui, if ok { GOOD } else { WARN }, format!("Roblox: {n} open ({})", sizes.join(", ")));
                }
            }
            ui.separator();
            if live.multi_on {
                dot(ui, GOOD, "Multi-Roblox: on");
            } else {
                dot(ui, WARN, "Multi-Roblox: off (close all Roblox once)")
                    .on_hover_text("Roblox was already open when the app started. Close every Roblox window and it turns on by itself.");
            }
            ui.separator();
            dot(ui, MUTED, format!("{} account(s)", self.accounts.len()));
            if let (true, Some(t)) = (self.running(), self.run_started) {
                ui.separator();
                let s = t.elapsed().as_secs();
                dot(ui, ACCENT, format!("running {:02}:{:02}", s / 60, s % 60));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (msg, at) = &self.notice;
                if !msg.is_empty() && at.elapsed() < Duration::from_secs(6) {
                    let l = msg.to_ascii_lowercase();
                    let bad = ["could not", "refused", "not ", "error", "answered"].iter().any(|w| l.contains(w));
                    let c = if bad { WARN } else { MUTED };
                    ui.label(RichText::new(msg).color(c));
                }
                if self.dirty_since.is_some() {
                    ui.label(RichText::new("saving...").color(MUTED));
                }
            });
        });
    }

    // ---------------------------------------------------------------- Run

    fn run_tab(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let queued = self.queued();
        let windows = self.window_count();
        let live_count = self.live.lock().unwrap().roblox.len();
        let avg_items = {
            let o = self.outfits();
            let n = o.iter().filter(|o| !o.items.is_empty()).count().max(1);
            o.iter().map(|o| o.items.len()).sum::<usize>() as f64 / n as f64
        };
        let published = self.published.len();
        let mut window_all = false;
        let mut open_failures = false;
        let Some(cfg) = self.cfg.as_mut() else { return };

        ui.add_enabled_ui(!running, |ui| {
            ui.columns(2, |cols| {
                card(&mut cols[0], "Windows", |ui| {
                    ui.radio_value(&mut cfg.gui.mode, Mode::Single, "The open Roblox window");
                    ui.radio_value(&mut cfg.gui.mode, Mode::Attach, format!("Every open Roblox window ({live_count} open)"));
                    ui.radio_value(&mut cfg.gui.mode, Mode::Launch, format!("Launch ticked accounts ({})", cfg.accounts.len()));
                    let s = cfg.launch_window_size.unwrap_or([816, 638]);
                    ui.label(
                        RichText::new(format!("Several windows: each leaves fullscreen, is set to {}x{} and tiled.", s[0], s[1]))
                            .small()
                            .color(MUTED),
                    );
                    ui.horizontal(|ui| {
                        window_all = ui
                            .button("Window all Roblox now")
                            .on_hover_text("Take every Roblox window out of fullscreen, resize and tile them")
                            .clicked();
                    });
                });
                card(&mut cols[1], "This run", |ui| {
                    ui.checkbox(&mut cfg.gui.test_mode, "Test mode: build, then Cancel instead of Publish");
                    ui.checkbox(&mut cfg.gui.check_items, "Check item IDs with the Roblox catalog first");
                    ui.checkbox(&mut self.redo, "Publish again outfits already published");
                    ui.checkbox(&mut cfg.gui.minimize_while_running, "Minimize this app while running (End stops)");
                    ui.add_space(2.0);
                    let per = 10.0 + 4.0 * avg_items + cfg.seconds_between_outfits;
                    let per_hour = (3600.0 / per) * windows as f64;
                    let cap = (cfg.max_publish_per_run as usize * windows).min(queued);
                    ui.horizontal(|ui| {
                        stat(ui, &queued.to_string(), "to build");
                        stat(ui, &cap.to_string(), "this run");
                        stat(ui, &published.to_string(), "published");
                        stat(ui, &format!("~{per_hour:.0}"), "per hour");
                    });
                });
            });
        });
        if window_all {
            let shared = self.shared.clone();
            std::thread::spawn(move || engine::window_all(shared));
        }
        let limit = cfg.max_publish_per_run.max(1);

        ui.add_space(4.0);
        let slots = self.shared.slots.lock().unwrap().clone();
        if !slots.is_empty() {
            card(ui, "Windows in this run", |ui| {
                egui::Grid::new("slots").num_columns(4).spacing([14.0, 6.0]).show(ui, |ui| {
                    for s in &slots {
                        ui.label(RichText::new(&s.label).strong());
                        let frac = s.done as f32 / limit as f32;
                        ui.add(
                            egui::ProgressBar::new(frac.min(1.0))
                                .desired_width(180.0)
                                .fill(if s.finished { MUTED } else { ACCENT })
                                .text(format!("{} / {}", s.done, limit)),
                        );
                        if s.problems > 0 {
                            ui.label(RichText::new(format!("{} problem(s)", s.problems)).color(WARN));
                        } else {
                            ui.label("");
                        }
                        ui.label(RichText::new(&s.doing).color(MUTED));
                        ui.end_row();
                    }
                });
            });
            ui.add_space(4.0);
        }

        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Log").strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    open_failures = ui.small_button("Open failures folder").clicked();
                    if ui.small_button("Clear").clicked() {
                        self.shared.log.lock().unwrap().clear();
                    }
                });
            });
            let log = self.shared.log.lock().unwrap().clone();
            egui::Frame::new().fill(INPUT).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(RichText::new("Press Start. The End key stops the bot from anywhere.").color(MUTED));
                    }
                    for line in &log {
                        ui.label(RichText::new(line).monospace().color(log_color(line)));
                    }
                });
            });
        });
        if open_failures {
            let dir = config::logs(&self.dir).join("failures");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer").arg(dir).spawn();
        }
    }

    // ---------------------------------------------------------------- Outfits

    fn outfits_tab(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        ui.add_enabled_ui(!running, |ui| {
            card(ui, "", |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("+ Add outfit").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                        self.rows.push(Row { name: format!("Outfit {}", self.rows.len() + 1), items: String::new() });
                    }
                    ui.label("Copies per Copy:");
                    ui.add(egui::DragValue::new(&mut self.copies).range(1..=500));
                    ui.separator();
                    ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Search names").desired_width(170.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.confirm_clear { "Click again to delete all" } else { "Delete all" };
                        if ui.add(egui::Button::new(RichText::new(label).color(if self.confirm_clear { DANGER } else { MUTED }))).clicked() {
                            if self.confirm_clear {
                                self.rows.clear();
                            }
                            self.confirm_clear = !self.confirm_clear;
                        }
                        let n = self.rows.len();
                        ui.label(RichText::new(format!("{n} outfit{}", if n == 1 { "" } else { "s" })).color(MUTED));
                    });
                });
            });
            ui.add_space(4.0);

            let mut remove = None;
            let mut copy = None;
            let filter = self.filter.to_lowercase();
            let names: Vec<String> = self.rows.iter().map(|r| r.name.trim().to_string()).collect();
            card(ui, "", |ui| {
                egui::ScrollArea::vertical().max_height(215.0).auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("outfits").striped(true).num_columns(5).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label(RichText::new("Name").color(MUTED));
                        ui.label(RichText::new("Item IDs or links").color(MUTED));
                        ui.label("");
                        ui.label("");
                        ui.label("");
                        ui.end_row();
                        for (i, row) in self.rows.iter_mut().enumerate() {
                            if !filter.is_empty() && !row.name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            let dup = names.iter().filter(|n| **n == row.name.trim()).count() > 1;
                            let mut name_edit = egui::TextEdit::singleline(&mut row.name);
                            if dup {
                                name_edit = name_edit.text_color(WARN);
                            }
                            let r = ui.add_sized([210.0, 26.0], name_edit);
                            if dup {
                                r.on_hover_text("Same name as another outfit");
                            }
                            ui.add_sized([330.0, 26.0], egui::TextEdit::singleline(&mut row.items).hint_text("e.g. 1374269, 48474313"));
                            let n = config::parse_ids(&row.items).len();
                            ui.horizontal(|ui| {
                                ui.set_min_width(120.0);
                                if n == 0 {
                                    badge(ui, "no items", DANGER);
                                } else {
                                    ui.label(RichText::new(format!("{n} item{}", if n == 1 { "" } else { "s" })).color(MUTED));
                                }
                                if self.published.contains(row.name.trim()) {
                                    badge(ui, "published", GOOD);
                                }
                            });
                            if ui.button("Copy").on_hover_text("Add copies with the same items and numbered names").clicked() {
                                copy = Some(i);
                            }
                            if ui.button(RichText::new("Delete").color(Color32::from_rgb(235, 120, 120))).clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                    if self.rows.is_empty() {
                        ui.label(RichText::new("No outfits yet. Press + Add outfit, or paste a list below.").color(MUTED));
                    }
                });
            });
            if let Some(i) = remove {
                self.rows.remove(i);
            }
            if let Some(i) = copy {
                self.copy_row(i);
            }
            ui.add_space(4.0);
            self.avatar_card(ui);
            ui.add_space(4.0);
            card(ui, "Add many at once", |ui| {
                ui.label(RichText::new("One outfit per line:   Name: 1374269, 48474313").color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.bulk).desired_rows(2).desired_width(ui.available_width() - 110.0));
                    if ui.add(egui::Button::new("Add these").min_size(Vec2::new(96.0, 44.0))).clicked() {
                        self.add_bulk();
                    }
                });
            });
        });
    }

    /// Copy what someone is wearing into a new outfit.
    fn avatar_card(&mut self, ui: &mut egui::Ui) {
        self.poll_avatar();
        let busy = self.avatar_job.is_some();
        let mut start: Option<avatar::UserRef> = None;
        card(ui, "Copy an avatar", |ui| {
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.avatar_link)
                        .hint_text("Paste a profile link, or type a username")
                        .desired_width(330.0),
                );
                let who = avatar::parse_user(&self.avatar_link);
                // A pasted profile link is looked up straight away; a typed username waits for Enter.
                let link = self.avatar_link.to_ascii_lowercase().contains("/users/");
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let fresh = self.avatar_last.as_deref() != Some(self.avatar_link.trim());
                if !busy && who.is_some() && ((resp.changed() && link && fresh) || enter) {
                    start = who;
                } else if busy {
                    ui.spinner();
                } else if ui.add_enabled(who.is_some(), egui::Button::new("Copy avatar")).clicked() {
                    start = avatar::parse_user(&self.avatar_link);
                }
                if !self.avatar_link.trim().is_empty() && avatar::parse_user(&self.avatar_link).is_none() {
                    ui.label(RichText::new("Not a profile link or username").color(DANGER));
                }
            });
            if !self.accounts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Your accounts:").color(MUTED));
                    for a in &self.accounts {
                        if ui.add_enabled(!busy, egui::Button::new(&a.name).small()).on_hover_text("Copy what this account is wearing").clicked() {
                            start = Some(avatar::UserRef::Id(a.user_id));
                        }
                    }
                });
            }
            match &self.avatar_msg {
                Some((m, bad)) => {
                    ui.label(RichText::new(m).color(if *bad { DANGER } else { GOOD }));
                }
                None => {
                    ui.label(RichText::new("Adds an outfit with everything they're wearing. Body parts, heads and animations are left out.").color(MUTED));
                }
            }
        });
        if let Some(who) = start {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ui.ctx().clone();
            std::thread::spawn(move || {
                let _ = tx.send(avatar::fetch(&who));
                ctx.request_repaint();
            });
            self.avatar_job = Some(rx);
            self.avatar_last = Some(self.avatar_link.trim().to_string());
            self.avatar_msg = Some(("Looking up the avatar...".into(), false));
        }
    }

    fn poll_avatar(&mut self) {
        let Some(rx) = &self.avatar_job else { return };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("the lookup stopped unexpectedly".into()),
        };
        self.avatar_job = None;
        self.avatar_msg = Some(match r {
            Err(e) => (format!("Couldn't copy that avatar: {e}"), true),
            Ok(w) if w.items.is_empty() => (format!("{} isn't wearing anything that can be copied.", w.user), true),
            Ok(w) => {
                let base = format!("{}'s avatar", w.user);
                let mut name = base.clone();
                let mut n = 2;
                while self.rows.iter().any(|r| r.name.trim() == name) {
                    name = format!("{base} {n}");
                    n += 1;
                }
                let count = w.items.len();
                self.rows.push(Row { name: name.clone(), items: w.items.iter().map(u64::to_string).collect::<Vec<_>>().join(", ") });
                let skipped = if w.skipped > 0 { format!(" ({} body parts/animations left out)", w.skipped) } else { String::new() };
                (format!("Added \"{name}\" with {count} item{}{skipped}.", if count == 1 { "" } else { "s" }), false)
            }
        });
    }

    fn copy_row(&mut self, i: usize) {
        let (name, items) = (self.rows[i].name.trim().to_string(), self.rows[i].items.clone());
        let base = name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end().to_string();
        let base = if base.is_empty() { name.clone() } else { base };
        let mut next = self
            .rows
            .iter()
            .filter_map(|r| r.name.trim().strip_prefix(&base).and_then(|s| s.trim().parse::<u32>().ok()))
            .max()
            .unwrap_or(1)
            + 1;
        for _ in 0..self.copies {
            self.rows.push(Row { name: format!("{base} {next}"), items: items.clone() });
            next += 1;
        }
        self.say(format!("Added {} copies of \"{name}\"", self.copies));
    }

    fn add_bulk(&mut self) {
        let mut added = 0;
        for line in self.bulk.lines() {
            let (name, ids) = match line.split_once('|').or_else(|| line.split_once(':')) {
                Some((n, rest)) if !n.contains("http") && !n.trim().is_empty() => (n.trim().to_string(), rest),
                _ => (format!("Outfit {}", self.rows.len() + 1), line),
            };
            let ids = config::parse_ids(ids);
            if !ids.is_empty() {
                self.rows.push(Row { name, items: ids.iter().map(u64::to_string).collect::<Vec<_>>().join(", ") });
                added += 1;
            }
        }
        self.bulk.clear();
        self.say(format!("Added {added} outfit(s)"));
    }

    // ---------------------------------------------------------------- Accounts

    /// Run a sign-in or import in the background.
    fn spawn_job(&self, what: &str, work: impl FnOnce() -> String + Send + 'static) {
        let job = self.job.clone();
        {
            let mut j = job.lock().unwrap();
            if j.busy.is_some() {
                return;
            }
            j.busy = Some(what.to_string());
            j.result = None;
        }
        std::thread::spawn(move || {
            let msg = work();
            let mut j = job.lock().unwrap();
            j.busy = None;
            j.result = Some(msg);
            j.changed = true;
        });
    }

    fn accounts_tab(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let (busy, result) = {
            let j = self.job.lock().unwrap();
            (j.busy.clone(), j.result.clone())
        };
        let Some(cfg) = self.cfg.clone() else { return };
        let mut sign_in = false;
        let mut import = false;
        let mut add_pasted = false;
        let mut play: Option<accounts::Account> = None;
        let mut remove: Option<String> = None;

        ui.add_enabled_ui(!running, |ui| {
            card(ui, "Add accounts", |ui| {
                ui.horizontal(|ui| {
                    let idle = busy.is_none();
                    sign_in = ui
                        .add_enabled(idle, egui::Button::new(RichText::new("Sign in to Roblox").color(Color32::WHITE)).fill(ACCENT))
                        .on_hover_text("Opens a Roblox sign-in window. You sign in (and solve Roblox's check) yourself; the account is then saved here.")
                        .clicked();
                    import = ui
                        .add_enabled(idle, egui::Button::new("Import from RAM"))
                        .on_hover_text("Copies every account from Roblox Account Manager. RAM must be open with Allow GetAccounts and Allow GetCookie on.")
                        .clicked();
                    if let Some(b) = &busy {
                        ui.spinner();
                        ui.label(RichText::new(b).color(MUTED));
                    } else if let Some(r) = &result {
                        let good = r.starts_with("Signed in") || r.starts_with("Added") || r.starts_with("Imported") || r.starts_with("Launched");
                        ui.label(RichText::new(r).color(if good { GOOD } else { WARN }));
                    }
                });
                egui::CollapsingHeader::new(RichText::new("Or paste a .ROBLOSECURITY cookie").color(MUTED)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.paste_cookie)
                                .password(true)
                                .desired_width(560.0)
                                .hint_text("_|WARNING:-DO-NOT-SHARE-THIS..."),
                        );
                        add_pasted = ui
                            .add_enabled(busy.is_none() && !self.paste_cookie.trim().is_empty(), egui::Button::new("Add"))
                            .clicked();
                    });
                });
                ui.label(
                    RichText::new("Sign-ins are encrypted with your Windows login in accounts.json. Never share that file or a cookie: it signs in to the account.")
                        .small()
                        .color(MUTED),
                );
            });
            ui.add_space(4.0);

            let cfg_mut = self.cfg.as_mut().unwrap();
            card(ui, "", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Accounts").strong().size(15.0));
                    ui.label(
                        RichText::new(format!("{} of {} ticked for launching", cfg_mut.accounts.len(), self.accounts.len())).color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Untick all").clicked() {
                            cfg_mut.accounts.clear();
                        }
                        if ui.small_button("Tick all").clicked() {
                            cfg_mut.accounts = self.accounts.iter().map(|a| a.name.clone()).collect();
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if self.accounts.is_empty() {
                        ui.label(RichText::new("No accounts yet. Press \"Sign in to Roblox\" once for each account.").color(MUTED));
                    }
                    egui::Grid::new("accounts").striped(true).num_columns(4).spacing([16.0, 6.0]).show(ui, |ui| {
                        for a in &self.accounts {
                            let mut on = cfg_mut.accounts.contains(&a.name);
                            if ui.checkbox(&mut on, RichText::new(&a.name).strong()).changed() {
                                if on {
                                    cfg_mut.accounts.push(a.name.clone());
                                } else {
                                    cfg_mut.accounts.retain(|x| *x != a.name);
                                }
                            }
                            let sub = if a.display.is_empty() || a.display == a.name { String::new() } else { a.display.clone() };
                            ui.label(RichText::new(sub).color(MUTED));
                            if ui.button("Play in CAC").on_hover_text("Open this account in Catalog Avatar Creator now").clicked() {
                                play = Some(a.clone());
                            }
                            if ui.small_button(RichText::new("Remove").color(Color32::from_rgb(235, 120, 120))).clicked() {
                                remove = Some(a.name.clone());
                            }
                            ui.end_row();
                        }
                    });
                    let missing: Vec<String> =
                        cfg_mut.accounts.iter().filter(|n| !self.accounts.iter().any(|a| &a.name == *n)).cloned().collect();
                    if !missing.is_empty() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(format!("Ticked but not signed in: {}", missing.join(", "))).color(WARN));
                        if ui.small_button("Untick those").clicked() {
                            cfg_mut.accounts.retain(|n| !missing.contains(n));
                        }
                    }
                });
            });
        });

        if sign_in {
            self.spawn_job("Waiting for you to sign in in the new window...", || match accounts::sign_in_window() {
                Ok(name) => format!("Signed in: {name}"),
                Err(e) => e,
            });
        }
        if import {
            let (port, password) = (cfg.ram.port, cfg.ram.password.clone());
            self.spawn_job("Importing from RAM...", move || match accounts::import_from_ram(port, &password) {
                Ok(results) => {
                    let ok = results.iter().filter(|r| r.is_ok()).count();
                    let errs: Vec<String> = results.into_iter().filter_map(Result::err).collect();
                    if errs.is_empty() {
                        format!("Imported {ok} account(s) from RAM")
                    } else {
                        format!("Imported {ok}; couldn't import: {}", errs.join("; "))
                    }
                }
                Err(e) => e,
            });
        }
        if add_pasted {
            let cookie = std::mem::take(&mut self.paste_cookie);
            self.spawn_job("Checking the sign-in...", move || match accounts::verify(&cookie).and_then(accounts::upsert) {
                Ok(name) => format!("Added {name}"),
                Err(e) => e,
            });
        }
        if let Some(a) = play {
            let place = cfg.ram.place_id;
            let multi = self.shared.multi_on.load(Ordering::SeqCst);
            let others = !self.live.lock().unwrap().roblox.is_empty();
            if others && !multi {
                self.say("Multi-Roblox is off: close every Roblox window first, or this would close the open game.");
            } else {
                let name = a.name.clone();
                self.spawn_job(&format!("Launching {name}..."), move || match accounts::launch_checked(&a, place) {
                    Ok(()) => format!("Launched {name}; its Roblox window opens in a few seconds"),
                    Err(e) => format!("{name}: {e}"),
                });
            }
        }
        if let Some(name) = remove {
            match accounts::remove(&name) {
                Ok(()) => {
                    if let Some(c) = self.cfg.as_mut() {
                        c.accounts.retain(|x| *x != name);
                    }
                    self.accounts = accounts::load();
                    self.say(format!("Removed {name}"));
                }
                Err(e) => self.say(format!("Could not save: {e}")),
            }
        }
    }

    // ---------------------------------------------------------------- Settings

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let running = self.running();
        let Some(cfg) = self.cfg.as_mut() else { return };
        ui.add_enabled_ui(!running, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.columns(2, |cols| {
                    card(&mut cols[0], "Rate", |ui| {
                        egui::Grid::new("rate").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("Outfits per window per run");
                            ui.add(egui::DragValue::new(&mut cfg.max_publish_per_run).range(1..=100000));
                            ui.end_row();
                            ui.label("Seconds between outfits");
                            ui.add(egui::DragValue::new(&mut cfg.seconds_between_outfits).range(0.0..=3600.0).speed(0.5).suffix(" s"));
                            ui.end_row();
                            ui.label("Problems before a window stops");
                            ui.add(egui::DragValue::new(&mut cfg.max_failures_per_window).range(1..=100));
                            ui.end_row();
                        });
                        ui.label(
                            RichText::new("One outfit takes about 10 s plus 4 s per item to build. Roblox may limit how often one account can publish.")
                                .small()
                                .color(MUTED),
                        );
                    });
                    cols[0].add_space(4.0);
                    card(&mut cols[0], "Windows", |ui| {
                        let sizes = cfg.sizes();
                        let mut size = cfg.launch_window_size.map(|s| format!("{}x{}", s[0], s[1])).unwrap_or_else(|| "816x638".into());
                        egui::Grid::new("win").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("Size with several windows");
                            egui::ComboBox::from_id_salt("size").selected_text(&size).show_ui(ui, |ui| {
                                for s in &sizes {
                                    ui.selectable_value(&mut size, s.clone(), s);
                                }
                            });
                            ui.end_row();
                            ui.label("Roblox window title");
                            ui.add(egui::TextEdit::singleline(&mut cfg.window_title).desired_width(140.0));
                            ui.end_row();
                        });
                        if let Some((w, h)) = size.split_once('x') {
                            if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                                cfg.launch_window_size = Some([w, h]);
                            }
                        }
                        ui.label(RichText::new("Only sizes the bot has button positions for are listed.").small().color(MUTED));
                    });
                    card(&mut cols[1], "Launching accounts", |ui| {
                        egui::Grid::new("launch").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            ui.label("Place ID (CAC)");
                            ui.add(egui::DragValue::new(&mut cfg.ram.place_id));
                            ui.end_row();
                            ui.label("Seconds between launches");
                            ui.add(egui::DragValue::new(&mut cfg.ram.seconds_between_launches).range(0.0..=600.0).suffix(" s"));
                            ui.end_row();
                            ui.label("Wait for a window up to");
                            ui.add(egui::DragValue::new(&mut cfg.ram.window_timeout).range(5.0..=600.0).suffix(" s"));
                            ui.end_row();
                            ui.label("Time for CAC to load");
                            ui.add(egui::DragValue::new(&mut cfg.ram.game_load_seconds).range(0.0..=600.0).suffix(" s"));
                            ui.end_row();
                        });
                        egui::CollapsingHeader::new(RichText::new("Importing from Roblox Account Manager").color(MUTED)).show(ui, |ui| {
                            egui::Grid::new("ram").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                                ui.label("RAM web server port");
                                ui.add(egui::DragValue::new(&mut cfg.ram.port));
                                ui.end_row();
                                ui.label("RAM web server password");
                                ui.add(egui::TextEdit::singleline(&mut cfg.ram.password).password(true).desired_width(140.0));
                                ui.end_row();
                            });
                        });
                    });
                    cols[1].add_space(4.0);
                    card(&mut cols[1], "Delays", |ui| {
                        egui::Grid::new("delays").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                            for (k, v) in cfg.delays.iter_mut() {
                                let mut name = k.replace('_', " ");
                                if let Some(f) = name.get_mut(0..1) {
                                    f.make_ascii_uppercase();
                                }
                                ui.label(name);
                                ui.add(egui::DragValue::new(v).range(0.0..=30.0).speed(0.05).suffix(" s"));
                                ui.end_row();
                            }
                        });
                    });
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Changes save on their own.").small().color(MUTED));
            });
        });
    }
}

fn stat(ui: &mut egui::Ui, value: &str, label: &str) {
    ui.vertical(|ui| {
        ui.set_min_width(76.0);
        ui.label(RichText::new(value).size(20.0).strong().color(Color32::WHITE));
        ui.label(RichText::new(label).small().color(MUTED));
    });
}

impl eframe::App for App {
    /// Runs even while the window is minimized, so the app can come back when a run ends.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let running = self.running();
        let minimize = self.cfg.as_ref().is_some_and(|c| c.gui.minimize_while_running);
        if running && !self.was_running && minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if self.was_running && !running {
            self.reload_published();
            if minimize {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        self.was_running = running;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(cfg) = &self.cfg {
            *self.probe.lock().unwrap() = (cfg.window_title.clone(), cfg.ram.port);
        }
        if self.auto_start && self.cfg.is_some() {
            self.auto_start = false;
            self.start();
        }
        {
            let mut j = self.job.lock().unwrap();
            if j.changed {
                j.changed = false;
                drop(j);
                self.accounts = accounts::load();
            } else if j.busy.is_some() {
                ctx.request_repaint_after(Duration::from_millis(300));
            }
        }
        let running = self.running();

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(Color32::from_rgb(28, 31, 36)).inner_margin(Margin::symmetric(10, 0)))
            .show(ui, |ui| self.header(ui));
        egui::Panel::bottom("footer")
            .frame(egui::Frame::new().fill(Color32::from_rgb(28, 31, 36)).inner_margin(Margin::symmetric(12, 5)))
            .show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(12)))
            .show(ui, |ui| {
                if let Some(e) = self.error.clone() {
                    card(ui, "Problem", |ui| {
                        ui.colored_label(DANGER, e);
                        ui.label(RichText::new(format!("Data folder: {}", self.dir.display())).color(MUTED));
                        if ui.button("Try again").clicked() {
                            self.reload();
                        }
                    });
                    if self.cfg.is_none() {
                        return;
                    }
                    ui.add_space(4.0);
                }
                match self.tab {
                    Tab::Run => self.run_tab(ui),
                    Tab::Outfits => self.outfits_tab(ui),
                    Tab::Accounts => self.accounts_tab(ui),
                    Tab::Clothing => self.studio.ui(ui, &self.dir),
                    Tab::Settings => self.settings_tab(ui),
                }
            });

        self.autosave(&ctx);
        if running {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--hold-lock") {
        let title = config::load_config(&config::data_dir()).map(|c| c.window_title).unwrap_or_else(|_| "Roblox".into());
        accounts::lock_holder_main(&title);
    }
    if args.get(1).map(String::as_str) == Some("--login") {
        let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("cac-bot-login"));
        accounts::login_window_main(&dir);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WIDTH, HEIGHT])
            .with_min_inner_size([WIDTH, HEIGHT])
            .with_max_inner_size([WIDTH, HEIGHT])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_title("CAC Outfit Bot"),
        ..Default::default()
    };
    eframe::run_native("CAC Outfit Bot", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
