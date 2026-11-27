//! Clothing studio: draw Roblox classic Shirts, Pants and T-shirts on the official
//! 585x559 template (T-shirts: a 512x512 square), preview them on a body, and export
//! a PNG ready for the Creator Dashboard upload page.

use std::path::{Path, PathBuf};

use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};
use image::{Rgba, RgbaImage};

pub const TEMPLATE_W: u32 = 585;
pub const TEMPLATE_H: u32 = 559;
pub const TSHIRT_SIZE: u32 = 512;
pub const UPLOAD_URL: &str = "https://create.roblox.com/dashboard/creations";

const MUTED: Color32 = Color32::from_rgb(140, 146, 156);
const SKIN: [u8; 4] = [163, 162, 165, 255];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Shirt,
    Pants,
    TShirt,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Shirt, Kind::Pants, Kind::TShirt];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Shirt => "Shirt",
            Kind::Pants => "Pants",
            Kind::TShirt => "T-Shirt",
        }
    }
    pub fn size(self) -> (u32, u32) {
        match self {
            Kind::TShirt => (TSHIRT_SIZE, TSHIRT_SIZE),
            _ => (TEMPLATE_W, TEMPLATE_H),
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Torso,
    RightLimb,
    LeftLimb,
    Whole,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Front,
    Back,
    Up,
    Down,
    /// The side facing away from the body's middle (outer side of a limb / the torso's right).
    Right,
    Left,
}

#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub part: Part,
    pub side: Side,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Face {
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn label(&self, kind: Kind) -> String {
        let limb = if kind == Kind::Pants { "leg" } else { "arm" };
        let part = match self.part {
            Part::Torso => "Torso".to_string(),
            Part::RightLimb => format!("Right {limb}"),
            Part::LeftLimb => format!("Left {limb}"),
            Part::Whole => return "Whole image".into(),
        };
        let side = match self.side {
            Side::Front => "front",
            Side::Back => "back",
            Side::Up => "top",
            Side::Down => "bottom",
            Side::Right => "right side",
            Side::Left => "left side",
        };
        format!("{part} {side}")
    }
    fn short(&self) -> &'static str {
        match self.side {
            Side::Front => "F",
            Side::Back => "B",
            Side::Up => "U",
            Side::Down => "D",
            Side::Right => "R",
            Side::Left => "L",
        }
    }
}

const fn f(part: Part, side: Side, x: u32, y: u32, w: u32, h: u32) -> Face {
    Face { part, side, x, y, w, h }
}

/// The classic template layout (same for Shirts and Pants; Pants use the limb blocks for legs).
pub const TEMPLATE_FACES: [Face; 18] = [
    f(Part::Torso, Side::Up, 231, 8, 128, 64),
    f(Part::Torso, Side::Right, 165, 74, 64, 128),
    f(Part::Torso, Side::Front, 231, 74, 128, 128),
    f(Part::Torso, Side::Left, 361, 74, 64, 128),
    f(Part::Torso, Side::Back, 427, 74, 128, 128),
    f(Part::Torso, Side::Down, 231, 204, 128, 64),
    // Right limb: shown on the left of the image. Row order: inner(L), back, outer(R), front.
    f(Part::RightLimb, Side::Up, 217, 289, 64, 64),
    f(Part::RightLimb, Side::Left, 19, 355, 64, 128),
    f(Part::RightLimb, Side::Back, 85, 355, 64, 128),
    f(Part::RightLimb, Side::Right, 151, 355, 64, 128),
    f(Part::RightLimb, Side::Front, 217, 355, 64, 128),
    f(Part::RightLimb, Side::Down, 217, 485, 64, 64),
    // Left limb. Row order: front, outer(L), back, inner(R).
    f(Part::LeftLimb, Side::Up, 308, 289, 64, 64),
    f(Part::LeftLimb, Side::Front, 308, 355, 64, 128),
    f(Part::LeftLimb, Side::Left, 374, 355, 64, 128),
    f(Part::LeftLimb, Side::Back, 440, 355, 64, 128),
    f(Part::LeftLimb, Side::Right, 506, 355, 64, 128),
    f(Part::LeftLimb, Side::Down, 308, 485, 64, 64),
];

const TSHIRT_FACE: Face = f(Part::Whole, Side::Front, 0, 0, TSHIRT_SIZE, TSHIRT_SIZE);

pub fn faces(kind: Kind) -> &'static [Face] {
    match kind {
        Kind::TShirt => std::slice::from_ref(&TSHIRT_FACE),
        _ => &TEMPLATE_FACES,
    }
}

pub fn face_at(kind: Kind, x: u32, y: u32) -> Option<Face> {
    faces(kind).iter().copied().find(|fc| fc.contains(x, y))
}

fn find_face(part: Part, side: Side) -> Face {
    *TEMPLATE_FACES.iter().find(|fc| fc.part == part && fc.side == side).expect("face")
}

// ------------------------------------------------------------------ pixel helpers

/// Straight-alpha "source over" blend of `src` (with extra coverage 0..1) onto `dst`.
fn blend(dst: [u8; 4], src: [u8; 4], cover: f32) -> [u8; 4] {
    let sa = src[3] as f32 / 255.0 * cover;
    if sa <= 0.0 {
        return dst;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for i in 0..3 {
        let c = (src[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / oa;
        out[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (oa * 255.0).round() as u8;
    out
}

fn erase(dst: [u8; 4], cover: f32) -> [u8; 4] {
    let a = dst[3] as f32 * (1.0 - cover);
    [dst[0], dst[1], dst[2], a.round() as u8]
}

fn c32(c: Color32) -> [u8; 4] {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    [r, g, b, a]
}

fn close(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    if a[3] == 0 && b[3] == 0 {
        return true;
    }
    (0..4).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

/// Copy a face block, optionally mirrored left-right.
fn copy_face(img: &mut RgbaImage, from: Face, to: Face, mirror: bool) {
    let src = image::imageops::crop_imm(img, from.x, from.y, from.w, from.h).to_image();
    let src = if mirror { image::imageops::flip_horizontal(&src) } else { src };
    let src = if src.width() != to.w || src.height() != to.h {
        image::imageops::resize(&src, to.w, to.h, image::imageops::FilterType::Triangle)
    } else {
        src
    };
    image::imageops::replace(img, &src, to.x as i64, to.y as i64);
}

/// Scale `src` to fill (cover) or fit inside a `w`x`h` box, centred, on a transparent canvas.
pub fn fit_image(src: &RgbaImage, w: u32, h: u32, cover: bool) -> RgbaImage {
    let (sw, sh) = (src.width().max(1) as f32, src.height().max(1) as f32);
    let scale = if cover { (w as f32 / sw).max(h as f32 / sh) } else { (w as f32 / sw).min(h as f32 / sh) };
    let nw = ((sw * scale).round() as u32).max(1);
    let nh = ((sh * scale).round() as u32).max(1);
    let resized = image::imageops::resize(src, nw, nh, image::imageops::FilterType::Lanczos3);
    let mut out = RgbaImage::new(w, h);
    let ox = (w as i64 - nw as i64) / 2;
    let oy = (h as i64 - nh as i64) / 2;
    image::imageops::replace(&mut out, &resized, ox, oy);
    out
}

// ------------------------------------------------------------------ document

pub struct Doc {
    pub kind: Kind,
    pub img: RgbaImage,
    pub name: String,
    pub path: Option<PathBuf>,
    undo: Vec<RgbaImage>,
    redo: Vec<RgbaImage>,
    pub dirty_tex: bool,
    pub unsaved: bool,
    tex: Option<egui::TextureHandle>,
}

impl Doc {
    fn new(kind: Kind) -> Self {
        let (w, h) = kind.size();
        Doc {
            kind,
            img: RgbaImage::new(w, h),
            name: format!("My {}", kind.name()),
            path: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty_tex: true,
            unsaved: false,
            tex: None,
        }
    }

    pub fn snapshot(&mut self) {
        self.undo.push(self.img.clone());
        if self.undo.len() > 40 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.unsaved = true;
    }

    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.img, prev));
            self.dirty_tex = true;
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.img, next));
            self.dirty_tex = true;
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        let opts = egui::TextureOptions::NEAREST;
        if self.tex.is_none() || self.dirty_tex {
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [self.img.width() as usize, self.img.height() as usize],
                self.img.as_raw(),
            );
            match &mut self.tex {
                Some(t) => t.set(ci, opts),
                None => self.tex = Some(ctx.load_texture(format!("clothing-{}", self.kind.name()), ci, opts)),
            }
            self.dirty_tex = false;
        }
        self.tex.as_ref().unwrap().id()
    }

    pub fn is_empty(&self) -> bool {
        self.img.pixels().all(|p| p[3] == 0)
    }

    /// Fill every face of a body part (or one face) with a colour.
    pub fn fill_faces(&mut self, pick: impl Fn(&Face) -> bool, color: [u8; 4], replace: bool) {
        for fc in faces(self.kind).iter().filter(|fc| pick(fc)) {
            for y in fc.y..fc.y + fc.h {
                for x in fc.x..fc.x + fc.w {
                    let p = self.img.get_pixel_mut(x, y);
                    p.0 = if replace { color } else { blend(p.0, color, 1.0) };
                }
            }
        }
        self.dirty_tex = true;
    }

    /// Mirror the right limb onto the left limb (or the reverse), so both match.
    pub fn mirror_limbs(&mut self, right_to_left: bool) {
        let (from, to) = if right_to_left { (Part::RightLimb, Part::LeftLimb) } else { (Part::LeftLimb, Part::RightLimb) };
        for (a, b) in [
            (Side::Front, Side::Front),
            (Side::Back, Side::Back),
            (Side::Up, Side::Up),
            (Side::Down, Side::Down),
            (Side::Right, Side::Left),
            (Side::Left, Side::Right),
        ] {
            copy_face(&mut self.img, find_face(from, a), find_face(to, b), true);
        }
        self.dirty_tex = true;
    }

    /// Mirror one half of the torso front and back onto the other half.
    pub fn mirror_torso(&mut self, left_to_right_of_image: bool) {
        for side in [Side::Front, Side::Back, Side::Up, Side::Down] {
            let fc = find_face(Part::Torso, side);
            let half = fc.w / 2;
            for y in fc.y..fc.y + fc.h {
                for i in 0..half {
                    let (a, b) = (fc.x + i, fc.x + fc.w - 1 - i);
                    let (src, dst) = if left_to_right_of_image { (a, b) } else { (b, a) };
                    let p = *self.img.get_pixel(src, y);
                    self.img.put_pixel(dst, y, p);
                }
            }
        }
        // Side panels: copy the torso's right side onto its left side, mirrored.
        let (r, l) = (find_face(Part::Torso, Side::Right), find_face(Part::Torso, Side::Left));
        if left_to_right_of_image {
            copy_face(&mut self.img, r, l, true);
        } else {
            copy_face(&mut self.img, l, r, true);
        }
        self.dirty_tex = true;
    }

    /// Place an image on a face.
    pub fn stamp_image(&mut self, src: &RgbaImage, fc: Face, cover: bool) {
        let fitted = fit_image(src, fc.w, fc.h, cover);
        for y in 0..fc.h {
            for x in 0..fc.w {
                let s = fitted.get_pixel(x, y).0;
                let p = self.img.get_pixel_mut(fc.x + x, fc.y + y);
                p.0 = blend(p.0, s, 1.0);
            }
        }
        self.dirty_tex = true;
    }

    /// Load a PNG/JPG. Wrong-sized templates are scaled to the right size.
    pub fn load(&mut self, path: &Path) -> Result<String, String> {
        let img = image::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?.to_rgba8();
        let (w, h) = self.kind.size();
        self.snapshot();
        let mut note = String::new();
        self.img = if img.width() == w && img.height() == h {
            img
        } else if self.kind == Kind::TShirt {
            note = format!(" (was {}x{}, fitted to {w}x{h})", img.width(), img.height());
            fit_image(&img, w, h, false)
        } else {
            note = format!(" (was {}x{}, scaled to {w}x{h})", img.width(), img.height());
            image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3)
        };
        self.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.name.clone());
        self.path = Some(path.to_path_buf());
        self.unsaved = false;
        self.dirty_tex = true;
        Ok(note)
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        self.img.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        self.path = Some(path.to_path_buf());
        self.unsaved = false;
        Ok(())
    }
}

// ------------------------------------------------------------------ preview

/// A 256x256 front or back view of a blocky body wearing all three items.
pub const PREVIEW: u32 = 264;

pub fn preview(shirt: &RgbaImage, pants: &RgbaImage, tshirt: &RgbaImage, back: bool) -> RgbaImage {
    // Layout (px): arm 64 | gap 4 | torso 128 | gap 4 | arm 64 ; legs 128 tall below a 4px gap.
    let mut out = RgbaImage::from_pixel(PREVIEW, PREVIEW, Rgba([0, 0, 0, 0]));
    let side = if back { Side::Back } else { Side::Front };
    // From the front, the body's right limb is on the viewer's left. From behind, flipped.
    let (left_part, right_part) = if back { (Part::LeftLimb, Part::RightLimb) } else { (Part::RightLimb, Part::LeftLimb) };
    let slots = [
        (Part::Torso, 68u32, 0u32, None),
        (left_part, 0, 0, Some(false)),
        (right_part, 200, 0, Some(false)),
        (left_part, 68, 132, Some(true)),
        (right_part, 132, 132, Some(true)),
    ];
    for (part, ox, oy, leg) in slots {
        let fc = find_face(part, side);
        for y in 0..fc.h {
            for x in 0..fc.w {
                let (sx, sy) = (fc.x + x, fc.y + y);
                let mut px = SKIN;
                let is_leg = leg == Some(true);
                let is_arm = leg == Some(false);
                if part == Part::Torso || is_leg {
                    px = blend(px, pants.get_pixel(sx, sy).0, 1.0);
                }
                if part == Part::Torso || is_arm {
                    px = blend(px, shirt.get_pixel(sx, sy).0, 1.0);
                }
                out.put_pixel(ox + x, oy + y, Rgba(px));
            }
        }
    }
    if !back {
        let t = fit_image(tshirt, 128, 128, false);
        for y in 0..128 {
            for x in 0..128 {
                let p = out.get_pixel_mut(68 + x, y);
                p.0 = blend(p.0, t.get_pixel(x, y).0, 1.0);
            }
        }
    }
    out
}

// ------------------------------------------------------------------ studio UI

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Brush,
    Eraser,
    Fill,
    Rect,
    Line,
    Gradient,
    Text,
    Picker,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str); 8] = [
        (Tool::Brush, "Brush", "B"),
        (Tool::Eraser, "Eraser", "E"),
        (Tool::Fill, "Fill", "G"),
        (Tool::Rect, "Box", "R"),
        (Tool::Line, "Line", "L"),
        (Tool::Gradient, "Gradient", "D"),
        (Tool::Text, "Text", "T"),
        (Tool::Picker, "Pick", "I"),
    ];
}

/// One brush stroke in progress: drawn onto a copy of the image taken when it started,
/// so going over the same spot twice doesn't make see-through colours darker.
struct Stroke_ {
    base: RgbaImage,
    cover: Vec<f32>,
    clip: Option<Face>,
    last: (f32, f32),
}

pub struct Studio {
    docs: Vec<Doc>,
    current: Kind,
    tool: Tool,
    color: Color32,
    size: f32,
    clip: bool,
    guides: bool,
    zoom: f32,
    back_view: bool,
    swatches: Vec<Color32>,
    selected_face: usize,
    stroke: Option<Stroke_>,
    drag_start: Option<(u32, u32)>,
    hover: Option<(u32, u32)>,
    preview_tex: Option<egui::TextureHandle>,
    preview_dirty: bool,
    message: (String, bool),
    cover_mode: bool,
    color2: Color32,
    text: String,
    font: usize,
    text_size: f32,
    fonts: Vec<(String, Option<ab_glyph::FontArc>)>,
    restored: bool,
    hex: String,
    last_edit: Option<std::time::Instant>,
    link: String,
    link_id: Option<u64>,
    link_job: Option<std::sync::mpsc::Receiver<Result<crate::clothing_link::Item, String>>>,
    link_done: Option<(String, PathBuf)>,
    link_err: Option<String>,
}

impl Default for Studio {
    fn default() -> Self {
        Studio {
            docs: Kind::ALL.iter().map(|k| Doc::new(*k)).collect(),
            current: Kind::Shirt,
            tool: Tool::Brush,
            color: Color32::from_rgb(40, 110, 220),
            size: 6.0,
            clip: true,
            guides: true,
            zoom: 0.0,
            back_view: false,
            swatches: [
                (255, 255, 255),
                (20, 20, 22),
                (128, 128, 132),
                (200, 40, 40),
                (240, 130, 30),
                (245, 210, 50),
                (60, 170, 80),
                (40, 110, 220),
                (120, 70, 200),
                (235, 110, 170),
                (110, 70, 40),
                (30, 50, 90),
            ]
            .iter()
            .map(|&(r, g, b)| Color32::from_rgb(r, g, b))
            .collect(),
            selected_face: 2,
            stroke: None,
            drag_start: None,
            hover: None,
            preview_tex: None,
            preview_dirty: true,
            message: (String::new(), false),
            cover_mode: true,
            color2: Color32::from_rgba_unmultiplied(0, 0, 0, 0),
            text: "TEXT".into(),
            font: 0,
            text_size: 32.0,
            fonts: FONTS.iter().map(|(name, _)| (name.to_string(), None)).collect(),
            restored: false,
            hex: String::new(),
            last_edit: None,
            link: String::new(),
            link_id: None,
            link_job: None,
            link_done: None,
            link_err: None,
        }
    }
}

pub fn clothing_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("clothing")
}

fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "clothing".into() } else { s }
}

impl Studio {
    fn doc(&mut self) -> &mut Doc {
        let i = self.current.index();
        &mut self.docs[i]
    }

    fn say(&mut self, text: impl Into<String>, bad: bool) {
        self.message = (text.into(), bad);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, data_dir: &Path) {
        if !self.restored {
            self.restore(data_dir);
        }
        let before: Vec<usize> = self.docs.iter().map(|d| d.undo.len() + d.redo.len()).collect();
        self.handle_drops(ui.ctx());
        self.poll_link(data_dir);
        self.shortcuts(ui.ctx());
        let area = ui.available_rect_before_wrap();
        let (lw, rw, gap) = (200.0, 206.0, 12.0);
        let left = Rect::from_min_size(area.min, vec2(lw, area.height()));
        let right = Rect::from_min_max(pos2(area.max.x - rw, area.min.y), area.max);
        let mid = Rect::from_min_max(pos2(left.max.x + gap, area.min.y), pos2(right.min.x - gap, area.max.y));
        let col = |r: Rect| egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min));
        ui.scope_builder(col(left), |ui| {
            ui.set_clip_rect(left.expand(2.0));
            egui::ScrollArea::vertical().id_salt("cl_tools").max_width(lw).show(ui, |ui| {
                ui.set_width(lw - 10.0);
                self.tools_panel(ui)
            });
        });
        ui.scope_builder(col(mid), |ui| {
            ui.set_clip_rect(mid);
            self.canvas(ui, mid.size());
        });
        ui.scope_builder(col(right), |ui| {
            ui.set_clip_rect(right.expand(2.0));
            egui::ScrollArea::vertical().id_salt("cl_side").max_width(rw).show(ui, |ui| {
                ui.set_width(rw - 10.0);
                self.side_panel(ui, data_dir)
            });
        });
        ui.allocate_rect(area, Sense::hover());
        let after: Vec<usize> = self.docs.iter().map(|d| d.undo.len() + d.redo.len()).collect();
        if before != after || self.stroke.is_some() {
            self.last_edit = Some(std::time::Instant::now());
        }
        // Keep the work-in-progress on disk so closing the app never loses a design.
        if let Some(t) = self.last_edit {
            if self.stroke.is_none() && t.elapsed().as_secs_f32() > 1.0 {
                self.last_edit = None;
                self.autosave(data_dir);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
            }
        }
    }

    /// Look up the pasted link on a background thread.
    fn start_link(&mut self, ctx: &egui::Context, id: u64) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let cookies: Vec<String> = crate::accounts::load().into_iter().map(|a| a.cookie).collect();
            let _ = tx.send(crate::clothing_link::fetch(id, &cookies));
            ctx.request_repaint();
        });
        self.link_id = Some(id);
        self.link_job = Some(rx);
        self.link_done = None;
        self.link_err = None;
        self.say(format!("Looking up item {id}..."), false);
    }

    fn poll_link(&mut self, data_dir: &Path) {
        let Some(rx) = &self.link_job else { return };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("the lookup stopped unexpectedly".into()),
        };
        self.link_job = None;
        let item = match r {
            Ok(item) => item,
            Err(e) => {
                let e = format!("Couldn't get that item: {e}");
                self.link_err = Some(e.clone());
                self.say(e, true);
                return;
            }
        };
        let label = if item.name.is_empty() { format!("item {}", item.id) } else { item.name.clone() };
        let path = clothing_dir(data_dir).join("downloaded").join(format!("{} ({}).png", safe_name(&label), item.id));
        if let Err(e) = path.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).map_err(|e| e.to_string()).and_then(|_| {
            item.img.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())
        }) {
            self.say(format!("Got the item but couldn't save it: {e}"), true);
            return;
        }
        if let Some(k) = item.kind {
            self.current = k;
        }
        let kind = self.current;
        let r = self.doc().load(&path);
        let note = match r {
            Ok(n) => n,
            Err(e) => {
                self.say(format!("Saved {} but couldn't open it: {e}", path.display()), true);
                return;
            }
        };
        self.doc().name = label.clone();
        self.preview_dirty = true;
        self.last_edit = Some(std::time::Instant::now());
        let by = if item.creator.is_empty() { String::new() } else { format!(" by {}", item.creator) };
        let what = match item.kind {
            Some(k) => k.name().to_string(),
            None => format!("image, opened as {}", kind.name()),
        };
        self.link_done = Some((format!("{label}{by} ({what}, template {})", item.template_id), path.clone()));
        self.say(format!("Saved {}{note}. It's open in the editor (Ctrl+Z brings back what was there).", path.display()), false);
    }

    fn link_box(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Get from a link").strong());
        let busy = self.link_job.is_some();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.link)
                .hint_text("Paste a Shirt, Pants or T-Shirt link")
                .desired_width(190.0),
        );
        let id = crate::clothing_link::parse_id(&self.link);
        // A paste (or typing a new link) looks it up straight away.
        if resp.changed() && !busy {
            if let Some(id) = id {
                if self.link_id != Some(id) {
                    self.start_link(ui.ctx(), id);
                }
            }
        }
        ui.horizontal(|ui| {
            if busy {
                ui.spinner();
                ui.label(RichText::new("Downloading...").small().color(MUTED));
            } else {
                let go = ui.add_enabled(id.is_some(), egui::Button::new("Get"));
                if go.clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && id.is_some()) {
                    self.start_link(ui.ctx(), id.unwrap());
                }
                if !self.link.trim().is_empty() && id.is_none() {
                    ui.label(RichText::new("No item ID in that link").small().color(Color32::from_rgb(235, 110, 110)));
                }
            }
        });
        if let Some(e) = &self.link_err {
            ui.add(egui::Label::new(RichText::new(e).small().color(Color32::from_rgb(235, 110, 110))).wrap());
        }
        if let Some((info, path)) = &self.link_done {
            ui.add(egui::Label::new(RichText::new(info).small()).wrap());
            let path = path.clone();
            if ui.small_button("Show file").clicked() {
                let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
            }
        }
        ui.add(
            egui::Label::new(
                RichText::new("Saved to clothing/downloaded. Only re-upload designs you made or have permission to use.")
                    .small()
                    .color(MUTED),
            )
            .wrap(),
        );
    }

    fn work_dir(data_dir: &Path) -> PathBuf {
        clothing_dir(data_dir).join(".work")
    }

    fn autosave(&self, data_dir: &Path) {
        let dir = Self::work_dir(data_dir);
        let _ = std::fs::create_dir_all(&dir);
        for d in &self.docs {
            let path = dir.join(format!("{}.png", d.kind.name()));
            let _ = d.img.save_with_format(&path, image::ImageFormat::Png);
            let meta = serde_json::json!({ "name": d.name, "path": d.path, "unsaved": d.unsaved });
            let _ = std::fs::write(dir.join(format!("{}.json", d.kind.name())), meta.to_string());
        }
    }

    fn restore(&mut self, data_dir: &Path) {
        self.restored = true;
        let dir = Self::work_dir(data_dir);
        for d in &mut self.docs {
            let path = dir.join(format!("{}.png", d.kind.name()));
            if let Ok(img) = image::open(&path) {
                let img = img.to_rgba8();
                if img.dimensions() == d.kind.size() {
                    d.img = img;
                    d.dirty_tex = true;
                }
            }
            if let Ok(text) = std::fs::read_to_string(dir.join(format!("{}.json", d.kind.name()))) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(n) = v["name"].as_str() {
                        d.name = n.to_string();
                    }
                    d.path = v["path"].as_str().map(PathBuf::from);
                    d.unsaved = v["unsaved"].as_bool().unwrap_or(false);
                }
            }
        }
        self.preview_dirty = true;
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(path) = dropped.first() {
            // A dropped full-size template replaces the item; anything else goes onto the chosen part.
            match image::image_dimensions(path) {
                Ok((w, h)) if (w, h) == self.current.size() || self.current == Kind::TShirt => {
                    let r = self.doc().load(path);
                    self.after_load(r, path);
                }
                Ok(_) => self.stamp_from(path),
                Err(e) => self.say(format!("Could not read that file: {e}"), true),
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo, keys) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            let undo = cmd && i.key_pressed(egui::Key::Z) && !i.modifiers.shift;
            let redo = cmd && (i.key_pressed(egui::Key::Y) || (i.key_pressed(egui::Key::Z) && i.modifiers.shift));
            let keys: Vec<egui::Key> = if cmd {
                Vec::new()
            } else {
                [egui::Key::B, egui::Key::E, egui::Key::G, egui::Key::R, egui::Key::L, egui::Key::D, egui::Key::T, egui::Key::I, egui::Key::X, egui::Key::OpenBracket, egui::Key::CloseBracket]
                    .into_iter()
                    .filter(|k| i.key_pressed(*k))
                    .collect()
            };
            (undo, redo, keys)
        });
        if undo {
            self.doc().undo();
            self.preview_dirty = true;
        }
        if redo {
            self.doc().redo();
            self.preview_dirty = true;
        }
        for k in keys {
            match k {
                egui::Key::B => self.tool = Tool::Brush,
                egui::Key::E => self.tool = Tool::Eraser,
                egui::Key::G => self.tool = Tool::Fill,
                egui::Key::R => self.tool = Tool::Rect,
                egui::Key::L => self.tool = Tool::Line,
                egui::Key::I => self.tool = Tool::Picker,
                egui::Key::D => self.tool = Tool::Gradient,
                egui::Key::T => self.tool = Tool::Text,
                egui::Key::X => std::mem::swap(&mut self.color, &mut self.color2),
                egui::Key::OpenBracket => self.size = (self.size - 1.0).max(1.0),
                egui::Key::CloseBracket => self.size = (self.size + 1.0).min(64.0),
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------- left: tools

    fn tools_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Item").strong());
        ui.horizontal(|ui| {
            for k in Kind::ALL {
                let unsaved = self.docs[k.index()].unsaved;
                let label = if unsaved { format!("{}*", k.name()) } else { k.name().to_string() };
                if ui.selectable_label(self.current == k, label).clicked() && self.current != k {
                    self.current = k;
                    self.stroke = None;
                    self.selected_face = if k == Kind::TShirt { 0 } else { 2 };
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Tool").strong());
        egui::Grid::new("cl_toolgrid").num_columns(3).min_col_width(10.0).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for (n, (tool, name, key)) in Tool::ALL.iter().enumerate() {
                let b = egui::Button::selectable(self.tool == *tool, *name).min_size(vec2(58.0, 24.0));
                if ui.add(b).on_hover_text(format!("Shortcut: {key}")).clicked() {
                    self.tool = *tool;
                }
                if n % 3 == 2 {
                    ui.end_row();
                }
            }
        });
        if self.tool == Tool::Text {
            ui.add(egui::TextEdit::singleline(&mut self.text).hint_text("Text to stamp").desired_width(180.0));
            ui.horizontal(|ui| {
                let names: Vec<String> = self.fonts.iter().map(|f| f.0.clone()).collect();
                egui::ComboBox::from_id_salt("cl_font").width(96.0).selected_text(names[self.font].clone()).show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut self.font, i, n);
                    }
                });
                ui.add(egui::DragValue::new(&mut self.text_size).range(6.0..=200.0).suffix(" px"));
            });
            ui.add(egui::Label::new(RichText::new("Click the template to stamp it (centred on the click).").small().color(MUTED)).wrap());
        }
        if self.tool == Tool::Gradient {
            ui.add(egui::Label::new(RichText::new("Drag across a part: colour 1 to colour 2. Right-click a swatch to set colour 2.").small().color(MUTED)).wrap());
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Size");
            ui.add(egui::Slider::new(&mut self.size, 1.0..=64.0).integer());
        });
        ui.checkbox(&mut self.clip, "Stay inside the part")
            .on_hover_text("Strokes, boxes and fills stop at the edge of the part you started on.");
        ui.add_space(6.0);
        ui.label(RichText::new("Colour").strong());
        ui.horizontal(|ui| {
            egui::color_picker::color_edit_button_srgba(ui, &mut self.color, egui::color_picker::Alpha::OnlyBlend).on_hover_text("Colour 1 (left-click a swatch)");
            if ui.small_button("\u{21C4}").on_hover_text("Swap colours (X)").clicked() {
                std::mem::swap(&mut self.color, &mut self.color2);
            }
            egui::color_picker::color_edit_button_srgba(ui, &mut self.color2, egui::color_picker::Alpha::OnlyBlend).on_hover_text("Colour 2, used by Gradient (right-click a swatch)");
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Hex").color(MUTED));
            let [r, g, b, a] = self.color.to_srgba_unmultiplied();
            let current = if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") };
            let id = egui::Id::unique("cl_hex");
            let focused = ui.ctx().memory(|m| m.has_focus(id));
            // While typing, keep what was typed; otherwise show the current colour.
            if !focused {
                self.hex = current;
            }
            let resp = ui.add(egui::TextEdit::singleline(&mut self.hex).id(id).desired_width(90.0));
            if resp.changed() {
                if let Some(c) = parse_hex(&self.hex) {
                    self.color = c;
                }
            }
        });
        let mut picked = None;
        let mut picked2 = None;
        for row in self.swatches.chunks(6) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for c in row {
                    let (rect, resp) = ui.allocate_exact_size(vec2(25.0, 20.0), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, *c);
                    if *c == self.color {
                        ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                    }
                    if resp.clicked() {
                        picked = Some(*c);
                    }
                    if resp.secondary_clicked() {
                        picked2 = Some(*c);
                    }
                }
            });
        }
        if let Some(c) = picked2 {
            self.color2 = c;
        }
        if let Some(c) = picked {
            self.color = c;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Parts").strong());
        let kind = self.current;
        let fs = faces(kind);
        self.selected_face = self.selected_face.min(fs.len() - 1);
        let sel = fs[self.selected_face];
        egui::ComboBox::from_id_salt("cl_face")
            .width(180.0)
            .selected_text(sel.label(kind))
            .show_ui(ui, |ui| {
                for (i, fc) in fs.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_face, i, fc.label(kind));
                }
            });
        if kind != Kind::TShirt {
            ui.add(egui::Label::new(RichText::new("Tip: right-click a part on the template to pick it.").small().color(MUTED)).wrap());
        }
        let color = c32(self.color);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Fill part").clicked() {
                self.doc().snapshot();
                self.doc().fill_faces(|fc| fc.x == sel.x && fc.y == sel.y, color, true);
                self.preview_dirty = true;
            }
            if kind != Kind::TShirt && ui.button("Fill whole piece").on_hover_text("Every side of this torso / arm / leg").clicked() {
                self.doc().snapshot();
                self.doc().fill_faces(|fc| fc.part == sel.part, color, true);
                self.preview_dirty = true;
            }
            if ui.button("Clear part").clicked() {
                self.doc().snapshot();
                self.doc().fill_faces(|fc| fc.x == sel.x && fc.y == sel.y, [0, 0, 0, 0], true);
                self.preview_dirty = true;
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Image on part...").on_hover_text("Put a PNG/JPG (logo, print) on this part. You can also drag a file onto the window.").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file() {
                    self.stamp_from(&path);
                }
            }
            ui.checkbox(&mut self.cover_mode, "Fill").on_hover_text("On: the image covers the whole part (edges may be cut). Off: it fits inside.");
        });
        if kind != Kind::TShirt {
            ui.add_space(4.0);
            let limb = if kind == Kind::Pants { "leg" } else { "arm" };
            ui.label(RichText::new("Symmetry").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button(format!("Right {limb} → left")).clicked() {
                    self.doc().snapshot();
                    self.doc().mirror_limbs(true);
                    self.preview_dirty = true;
                }
                if ui.button(format!("Left {limb} → right")).clicked() {
                    self.doc().snapshot();
                    self.doc().mirror_limbs(false);
                    self.preview_dirty = true;
                }
                if ui.button("Torso ◧ → ◨").on_hover_text("Mirror the left half of the torso (as seen in the template) onto the right half").clicked() {
                    self.doc().snapshot();
                    self.doc().mirror_torso(true);
                    self.preview_dirty = true;
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Quick start").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Colour everything").on_hover_text("Fill every part with the current colour").clicked() {
                    self.doc().snapshot();
                    self.doc().fill_faces(|_| true, color, true);
                    self.preview_dirty = true;
                }
                if ui.button("Add shading").on_hover_text("Darken sides, back and the edges of every part so flat colours look 3D in game").clicked() {
                    self.doc().snapshot();
                    shade_sides(&mut self.doc().img);
                    self.doc().dirty_tex = true;
                    self.preview_dirty = true;
                }
                if ui.button("Outline parts").on_hover_text("Draw a 2px border in the current colour around every part").clicked() {
                    self.doc().snapshot();
                    let img = &mut self.doc().img;
                    for fc in faces(kind) {
                        outline(img, fc, color, 2);
                    }
                    self.doc().dirty_tex = true;
                    self.preview_dirty = true;
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (can_undo, can_redo) = (!self.doc().undo.is_empty(), !self.doc().redo.is_empty());
            if ui.add_enabled(can_undo, egui::Button::new("↶ Undo")).on_hover_text("Ctrl+Z").clicked() {
                self.doc().undo();
                self.preview_dirty = true;
            }
            if ui.add_enabled(can_redo, egui::Button::new("↷ Redo")).on_hover_text("Ctrl+Y").clicked() {
                self.doc().redo();
                self.preview_dirty = true;
            }
            if ui.button("New").on_hover_text("Clear this item").clicked() {
                self.doc().snapshot();
                let (w, h) = self.current.size();
                self.doc().img = RgbaImage::new(w, h);
                self.doc().dirty_tex = true;
                self.doc().path = None;
                self.preview_dirty = true;
            }
        });
    }

    fn stamp_from(&mut self, path: &Path) {
        match image::open(path) {
            Ok(img) => {
                let img = img.to_rgba8();
                let fc = faces(self.current)[self.selected_face.min(faces(self.current).len() - 1)];
                let cover = self.cover_mode;
                self.doc().snapshot();
                self.doc().stamp_image(&img, fc, cover);
                self.preview_dirty = true;
                let label = fc.label(self.current);
                self.say(format!("Put {} on {label}.", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), false);
            }
            Err(e) => self.say(format!("Could not open that image: {e}"), true),
        }
    }

    fn after_load(&mut self, r: Result<String, String>, path: &Path) {
        self.preview_dirty = true;
        match r {
            Ok(note) => self.say(format!("Opened {}{note}.", path.display()), false),
            Err(e) => self.say(e, true),
        }
    }

    // ---------------------------------------------------------- centre: canvas

    fn canvas(&mut self, ui: &mut egui::Ui, avail: Vec2) {
        let kind = self.current;
        let (iw, ih) = kind.size();
        let fit = ((avail.x) / iw as f32).min((avail.y - 26.0) / ih as f32).max(0.2);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Zoom").color(MUTED));
            for (z, name) in [(0.0, "Fit"), (1.0, "1x"), (2.0, "2x"), (3.0, "3x")] {
                if ui.selectable_label(self.zoom == z, name).clicked() {
                    self.zoom = z;
                }
            }
            ui.checkbox(&mut self.guides, "Guides");
            let status = match self.hover {
                Some((x, y)) => match face_at(kind, x, y) {
                    Some(fc) => format!("{x}, {y}  ·  {}", fc.label(kind)),
                    None => format!("{x}, {y}  ·  unused (not shown in game)"),
                },
                None => format!("{iw}x{ih}"),
            };
            ui.label(RichText::new(status).color(MUTED).small());
        });
        let scale = if self.zoom == 0.0 { fit } else { self.zoom };
        let size = vec2(iw as f32 * scale, ih as f32 * scale);
        let tex = self.doc().texture(ui.ctx());
        egui::ScrollArea::both().id_salt("cl_canvas").auto_shrink([false, false]).show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            checker(&painter, rect, 8.0 * scale.max(0.5));
            painter.image(tex, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let to_screen = |x: f32, y: f32| pos2(rect.min.x + x * scale, rect.min.y + y * scale);
            if self.guides && kind != Kind::TShirt {
                // Dim the unused area so it's clear what shows in game.
                let dim = Color32::from_black_alpha(110);
                painter.rect_filled(rect, 0.0, dim);
                for fc in faces(kind) {
                    let r = Rect::from_min_max(to_screen(fc.x as f32, fc.y as f32), to_screen((fc.x + fc.w) as f32, (fc.y + fc.h) as f32));
                    painter.image(tex, r, Rect::from_min_max(
                        pos2(fc.x as f32 / iw as f32, fc.y as f32 / ih as f32),
                        pos2((fc.x + fc.w) as f32 / iw as f32, (fc.y + fc.h) as f32 / ih as f32),
                    ), Color32::WHITE);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(70)), StrokeKind::Outside);
                    painter.text(r.min + vec2(3.0, 2.0), Align2::LEFT_TOP, fc.short(), FontId::proportional(10.0 * scale.clamp(0.8, 1.6)), Color32::from_white_alpha(120));
                }
                // Part names.
                for (part, x, y) in [(Part::Torso, 198.0, 40.0), (Part::RightLimb, 151.0, 283.0), (Part::LeftLimb, 438.0, 283.0)] {
                    let limb = if kind == Kind::Pants { "LEG" } else { "ARM" };
                    let name = match part {
                        Part::Torso => "TORSO".to_string(),
                        Part::RightLimb => format!("RIGHT {limb}"),
                        _ => format!("LEFT {limb}"),
                    };
                    painter.text(to_screen(x, y), Align2::CENTER_TOP, name, FontId::proportional(11.0 * scale.clamp(0.8, 1.6)), Color32::from_white_alpha(150));
                }
            }
            // Highlight the chosen part.
            let sel = faces(kind)[self.selected_face.min(faces(kind).len() - 1)];
            let sr = Rect::from_min_max(to_screen(sel.x as f32, sel.y as f32), to_screen((sel.x + sel.w) as f32, (sel.y + sel.h) as f32));
            painter.rect_stroke(sr, 0.0, Stroke::new(1.5, Color32::from_rgb(90, 200, 120)), StrokeKind::Outside);

            let to_img = |p: Pos2| ((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
            self.hover = resp.hover_pos().map(to_img).and_then(|(x, y)| {
                (x >= 0.0 && y >= 0.0 && x < iw as f32 && y < ih as f32).then_some((x as u32, y as u32))
            });
            if resp.secondary_clicked() {
                if let Some((x, y)) = self.hover {
                    if let Some(i) = faces(kind).iter().position(|fc| fc.contains(x, y)) {
                        self.selected_face = i;
                    }
                }
            }
            self.pointer(&resp, to_img);

            // Brush outline / box / line preview.
            if let Some(p) = resp.hover_pos() {
                if self.tool == Tool::Text {
                    if let Some(mask) = self.text_mask() {
                        let (w, h) = (mask.width() as f32 * scale, mask.height() as f32 * scale);
                        let r = Rect::from_center_size(p, vec2(w, h));
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(180)), StrokeKind::Outside);
                        painter.text(p, Align2::CENTER_CENTER, &self.text, FontId::proportional((self.text_size * scale * 0.8).max(6.0)), self.color.gamma_multiply(0.6));
                    }
                }
                if matches!(self.tool, Tool::Brush | Tool::Eraser | Tool::Line) {
                    painter.circle_stroke(p, (self.size * scale / 2.0).max(2.0), Stroke::new(1.0, Color32::from_white_alpha(200)));
                }
                if let (Some((sx, sy)), true) = (self.drag_start, resp.dragged()) {
                    let a = to_screen(sx as f32, sy as f32);
                    match self.tool {
                        Tool::Rect => {
                            let r = Rect::from_two_pos(a, p);
                            painter.rect_filled(r, 0.0, self.color.gamma_multiply(0.6));
                        }
                        Tool::Line => {
                            painter.line_segment([a, p], Stroke::new(self.size * scale, self.color.gamma_multiply(0.7)));
                        }
                        Tool::Gradient => {
                            painter.line_segment([a, p], Stroke::new(2.0, Color32::WHITE));
                            painter.circle_filled(a, 5.0, self.color);
                            painter.circle_filled(p, 5.0, self.color2);
                        }
                        _ => {}
                    }
                }
            }
            if resp.hovered() && self.tool != Tool::Text {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        });
    }

    fn pointer(&mut self, resp: &egui::Response, to_img: impl Fn(Pos2) -> (f32, f32)) {
        let kind = self.current;
        let (iw, ih) = kind.size();
        let Some(pos) = resp.interact_pointer_pos().map(&to_img) else {
            if resp.drag_stopped() {
                self.finish_stroke();
            }
            return;
        };
        let inside = |(x, y): (f32, f32)| x >= 0.0 && y >= 0.0 && x < iw as f32 && y < ih as f32;
        let primary = resp.ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_released());
        if !primary {
            return;
        }
        let ipos = (pos.0.clamp(0.0, iw as f32 - 1.0) as u32, pos.1.clamp(0.0, ih as f32 - 1.0) as u32);
        let clip_face = || if self.clip { face_at(kind, ipos.0, ipos.1) } else { None };
        match self.tool {
            Tool::Brush | Tool::Eraser => {
                if self.stroke.is_none() && (resp.drag_started() || resp.clicked() || resp.is_pointer_button_down_on()) && inside(pos) {
                    let clip = clip_face();
                    if self.clip && clip.is_none() {
                        return; // started outside every part
                    }
                    let doc = self.doc();
                    doc.snapshot();
                    let (w, h) = (doc.img.width(), doc.img.height());
                    self.stroke = Some(Stroke_ { base: doc.img.clone(), cover: vec![0.0; (w * h) as usize], clip, last: pos });
                    self.dab(pos, pos);
                }
                if self.stroke.is_some() && resp.dragged() {
                    let last = self.stroke.as_ref().unwrap().last;
                    self.dab(last, pos);
                }
                if resp.drag_stopped() || resp.clicked() {
                    self.finish_stroke();
                }
            }
            Tool::Rect | Tool::Line => {
                if resp.drag_started() && inside(pos) {
                    self.drag_start = Some(ipos);
                }
                if resp.drag_stopped() {
                    if let Some(start) = self.drag_start.take() {
                        let clip = if self.clip { face_at(kind, start.0, start.1) } else { None };
                        if self.clip && clip.is_none() {
                            return;
                        }
                        let color = c32(self.color);
                        let size = self.size;
                        let is_rect = self.tool == Tool::Rect;
                        let doc = self.doc();
                        doc.snapshot();
                        if is_rect {
                            fill_rect(&mut doc.img, start, ipos, color, clip);
                        } else {
                            let base = doc.img.clone();
                            let mut cover = vec![0.0f32; (iw * ih) as usize];
                            stamp_line(&mut cover, iw, ih, (start.0 as f32 + 0.5, start.1 as f32 + 0.5), (pos.0, pos.1), size, clip);
                            apply_cover(&mut doc.img, &base, &cover, color, false, None);
                        }
                        doc.dirty_tex = true;
                        self.preview_dirty = true;
                    }
                }
            }
            Tool::Gradient => {
                if resp.drag_started() && inside(pos) {
                    self.drag_start = Some(ipos);
                }
                if resp.drag_stopped() {
                    if let Some(start) = self.drag_start.take() {
                        let clip = if self.clip { face_at(kind, start.0, start.1) } else { None };
                        if self.clip && clip.is_none() {
                            return;
                        }
                        let (c1, c2) = (c32(self.color), c32(self.color2));
                        let doc = self.doc();
                        doc.snapshot();
                        gradient(&mut doc.img, (start.0 as f32 + 0.5, start.1 as f32 + 0.5), pos, c1, c2, clip);
                        doc.dirty_tex = true;
                        self.preview_dirty = true;
                    }
                }
            }
            Tool::Text => {
                if resp.clicked() && inside(pos) {
                    let clip = clip_face();
                    if self.clip && clip.is_none() {
                        return;
                    }
                    match self.text_mask() {
                        Some(mask) => {
                            let color = c32(self.color);
                            let doc = self.doc();
                            doc.snapshot();
                            stamp_mask(&mut doc.img, &mask, ipos, color, clip);
                            doc.dirty_tex = true;
                            self.preview_dirty = true;
                        }
                        None => self.say("That font could not be loaded.", true),
                    }
                }
            }
            Tool::Fill => {
                if resp.clicked() && inside(pos) {
                    let clip = clip_face();
                    let color = c32(self.color);
                    let doc = self.doc();
                    doc.snapshot();
                    flood_fill(&mut doc.img, ipos, color, clip);
                    doc.dirty_tex = true;
                    self.preview_dirty = true;
                }
            }
            Tool::Picker => {
                if (resp.clicked() || resp.dragged()) && inside(pos) {
                    let p = self.doc().img.get_pixel(ipos.0, ipos.1).0;
                    if p[3] > 0 {
                        self.color = Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]);
                    }
                }
                if resp.drag_stopped() || resp.clicked() {
                    self.tool = Tool::Brush;
                }
            }
        }
    }

    /// The current text rendered as a coverage mask (alpha only), or None if the font failed.
    fn text_mask(&mut self) -> Option<image::GrayImage> {
        let i = self.font.min(self.fonts.len() - 1);
        if self.fonts[i].1.is_none() {
            self.fonts[i].1 = load_font(i);
        }
        let font = self.fonts[i].1.clone()?;
        Some(render_text(&font, &self.text, self.text_size))
    }

    fn dab(&mut self, from: (f32, f32), to: (f32, f32)) {
        let color = c32(self.color);
        let eraser = self.tool == Tool::Eraser;
        let size = self.size;
        let i = self.current.index();
        let Some(st) = self.stroke.as_mut() else { return };
        let doc = &mut self.docs[i];
        let (w, h) = (doc.img.width(), doc.img.height());
        let bbox = stamp_line(&mut st.cover, w, h, from, to, size, st.clip);
        apply_cover(&mut doc.img, &st.base, &st.cover, color, eraser, Some(bbox));
        st.last = to;
        doc.dirty_tex = true;
        self.preview_dirty = true;
    }

    fn finish_stroke(&mut self) {
        self.stroke = None;
    }

    // ---------------------------------------------------------- right: preview + export

    fn side_panel(&mut self, ui: &mut egui::Ui, data_dir: &Path) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Preview").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(self.back_view, "Back").clicked() {
                    self.back_view = true;
                    self.preview_dirty = true;
                }
                if ui.selectable_label(!self.back_view, "Front").clicked() {
                    self.back_view = false;
                    self.preview_dirty = true;
                }
            });
        });
        if self.preview_dirty || self.preview_tex.is_none() {
            let img = preview(&self.docs[0].img, &self.docs[1].img, &self.docs[2].img, self.back_view);
            let ci = egui::ColorImage::from_rgba_unmultiplied([PREVIEW as usize, PREVIEW as usize], img.as_raw());
            match &mut self.preview_tex {
                Some(t) => t.set(ci, egui::TextureOptions::LINEAR),
                None => self.preview_tex = Some(ui.ctx().load_texture("clothing-preview", ci, egui::TextureOptions::LINEAR)),
            }
            self.preview_dirty = false;
        }
        let (rect, _) = ui.allocate_exact_size(vec2(192.0, 192.0), Sense::hover());
        ui.painter().rect_filled(rect, 6.0, Color32::from_rgb(44, 48, 56));
        if let Some(t) = &self.preview_tex {
            ui.painter().image(t.id(), rect.shrink(8.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.label(RichText::new("Shows the Shirt, Pants and T-Shirt together.").small().color(MUTED));

        ui.add_space(8.0);
        ui.separator();
        self.link_box(ui);

        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Save").strong());
        let kind = self.current;
        if ui.add(egui::TextEdit::singleline(&mut self.doc().name).hint_text("Name").desired_width(190.0)).changed() {
            self.last_edit = Some(std::time::Instant::now());
        }
        let default_path = clothing_dir(data_dir).join(format!("{}.png", safe_name(&self.doc().name)));
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Save PNG").strong()).on_hover_text(default_path.display().to_string()).clicked() {
                let warn = self.check(kind);
                let r = self.doc().save(&default_path);
                match r {
                    Ok(()) => self.say(format!("Saved {}{warn}", default_path.display()), false),
                    Err(e) => self.say(format!("Could not save: {e}"), true),
                }
            }
            if ui.button("Save as...").clicked() {
                let name = format!("{}.png", safe_name(&self.doc().name));
                if let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(&name).save_file() {
                    let path = if path.extension().is_none() { path.with_extension("png") } else { path };
                    let r = self.doc().save(&path);
                    match r {
                        Ok(()) => self.say(format!("Saved {}", path.display()), false),
                        Err(e) => self.say(format!("Could not save: {e}"), true),
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Open...").on_hover_text("Open a template PNG to keep editing it").clicked() {
                let start = clothing_dir(data_dir);
                let _ = std::fs::create_dir_all(&start);
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
                    .set_directory(&start)
                    .pick_file()
                {
                    let r = self.doc().load(&path);
                    self.after_load(r, &path);
                }
            }
            if ui.button("Folder").on_hover_text("Open the clothing folder").clicked() {
                let d = clothing_dir(data_dir);
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer").arg(&d).spawn();
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new("Upload to Roblox").strong());
        ui.label(
            RichText::new(
                "Roblox only takes clothing through the Creator Dashboard: Avatar Items → Classics → Upload Asset, \
                 pick the type and choose the PNG. Uploading costs 80 Robux and needs ID verification.",
            )
            .small()
            .color(MUTED),
        );
        if ui.button(format!("Open upload page ({})", kind.name())).clicked() {
            if let Some(p) = &self.docs[kind.index()].path {
                // Select the file in Explorer so it's easy to drag into the browser.
                let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
            }
            open_url(UPLOAD_URL);
        }
        if self.docs[kind.index()].path.is_none() {
            ui.label(RichText::new("Save the PNG first, then upload it.").small().color(MUTED));
        }

        if !self.message.0.is_empty() {
            ui.add_space(8.0);
            let color = if self.message.1 { Color32::from_rgb(235, 110, 110) } else { Color32::from_rgb(90, 200, 120) };
            ui.label(RichText::new(&self.message.0).small().color(color));
        }
    }

    /// Warnings about a design before it's saved.
    fn check(&self, kind: Kind) -> String {
        let doc = &self.docs[kind.index()];
        if doc.is_empty() {
            return " (it's empty - nothing will show in game)".into();
        }
        if kind == Kind::TShirt {
            return String::new();
        }
        let blank: Vec<String> = faces(kind)
            .iter()
            .filter(|fc| fc.side == Side::Front)
            .filter(|fc| (fc.y..fc.y + fc.h).all(|y| (fc.x..fc.x + fc.w).all(|x| doc.img.get_pixel(x, y)[3] == 0)))
            .map(|fc| fc.label(kind))
            .collect();
        if blank.is_empty() { String::new() } else { format!(" (blank: {})", blank.join(", ")) }
    }
}

// ------------------------------------------------------------------ drawing ops

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some(Color32::from_rgb(v(0)?, v(2)?, v(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(v(0)?, v(2)?, v(4)?, v(6)?)),
        _ => None,
    }
}

fn checker(painter: &egui::Painter, rect: Rect, cell: f32) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(70, 72, 78));
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    let alt = Color32::from_rgb(92, 95, 102);
    for r in 0..rows {
        for c in 0..cols {
            if (r + c) % 2 == 0 {
                let min = rect.min + vec2(c as f32 * cell, r as f32 * cell);
                painter.rect_filled(Rect::from_min_size(min, Vec2::splat(cell)).intersect(rect), 0.0, alt);
            }
        }
    }
}

/// Add a round, anti-aliased brush line to the coverage mask. Returns the touched box.
fn stamp_line(cover: &mut [f32], w: u32, h: u32, a: (f32, f32), b: (f32, f32), size: f32, clip: Option<Face>) -> (u32, u32, u32, u32) {
    let r = (size / 2.0).max(0.5);
    let (minx, maxx) = (a.0.min(b.0) - r - 1.0, a.0.max(b.0) + r + 1.0);
    let (miny, maxy) = (a.1.min(b.1) - r - 1.0, a.1.max(b.1) + r + 1.0);
    let mut x0 = minx.floor().max(0.0) as u32;
    let mut y0 = miny.floor().max(0.0) as u32;
    let mut x1 = (maxx.ceil() as i64).clamp(0, w as i64 - 1) as u32;
    let mut y1 = (maxy.ceil() as i64).clamp(0, h as i64 - 1) as u32;
    if let Some(fc) = clip {
        x0 = x0.max(fc.x);
        y0 = y0.max(fc.y);
        x1 = x1.min(fc.x + fc.w - 1);
        y1 = y1.min(fc.y + fc.h - 1);
    }
    if x0 > x1 || y0 > y1 {
        return (0, 0, 0, 0);
    }
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            // Small brushes stay crisp (pixel art); bigger ones get a 1px soft edge.
            let c = if size <= 2.0 { if d <= r { 1.0 } else { 0.0 } } else { (r + 0.5 - d).clamp(0.0, 1.0) };
            let i = (y * w + x) as usize;
            if c > cover[i] {
                cover[i] = c;
            }
        }
    }
    (x0, y0, x1, y1)
}

fn apply_cover(img: &mut RgbaImage, base: &RgbaImage, cover: &[f32], color: [u8; 4], eraser: bool, bbox: Option<(u32, u32, u32, u32)>) {
    let (w, h) = (img.width(), img.height());
    let (x0, y0, x1, y1) = bbox.unwrap_or((0, 0, w - 1, h - 1));
    for y in y0..=y1.min(h - 1) {
        for x in x0..=x1.min(w - 1) {
            let c = cover[(y * w + x) as usize];
            if c <= 0.0 {
                continue;
            }
            let b = base.get_pixel(x, y).0;
            let out = if eraser { erase(b, c) } else { blend(b, color, c) };
            img.put_pixel(x, y, Rgba(out));
        }
    }
}

fn fill_rect(img: &mut RgbaImage, a: (u32, u32), b: (u32, u32), color: [u8; 4], clip: Option<Face>) {
    let (mut x0, mut x1) = (a.0.min(b.0), a.0.max(b.0));
    let (mut y0, mut y1) = (a.1.min(b.1), a.1.max(b.1));
    if let Some(fc) = clip {
        x0 = x0.max(fc.x);
        y0 = y0.max(fc.y);
        x1 = x1.min(fc.x + fc.w - 1);
        y1 = y1.min(fc.y + fc.h - 1);
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = img.get_pixel_mut(x, y);
            p.0 = blend(p.0, color, 1.0);
        }
    }
}

fn flood_fill(img: &mut RgbaImage, start: (u32, u32), color: [u8; 4], clip: Option<Face>) {
    let (w, h) = (img.width(), img.height());
    let (bx0, by0, bx1, by1) = match clip {
        Some(fc) => (fc.x, fc.y, fc.x + fc.w - 1, fc.y + fc.h - 1),
        None => (0, 0, w - 1, h - 1),
    };
    let target = img.get_pixel(start.0, start.1).0;
    let mut seen = vec![false; (w * h) as usize];
    let mut stack = vec![start];
    let src = img.clone();
    while let Some((x, y)) = stack.pop() {
        let i = (y * w + x) as usize;
        if seen[i] {
            continue;
        }
        seen[i] = true;
        if !close(src.get_pixel(x, y).0, target, 24) {
            continue;
        }
        let p = img.get_pixel_mut(x, y);
        p.0 = blend(src.get_pixel(x, y).0, color, 1.0);
        if x > bx0 {
            stack.push((x - 1, y));
        }
        if x < bx1 {
            stack.push((x + 1, y));
        }
        if y > by0 {
            stack.push((x, y - 1));
        }
        if y < by1 {
            stack.push((x, y + 1));
        }
    }
}

fn outline(img: &mut RgbaImage, fc: &Face, color: [u8; 4], t: u32) {
    for y in fc.y..fc.y + fc.h {
        for x in fc.x..fc.x + fc.w {
            let edge = x < fc.x + t || y < fc.y + t || x >= fc.x + fc.w - t || y >= fc.y + fc.h - t;
            if edge {
                let p = img.get_pixel_mut(x, y);
                p.0 = blend(p.0, color, 1.0);
            }
        }
    }
}

/// Fonts offered by the Text tool: Windows fonts when present, plus one built in.
const FONTS: [(&str, &str); 6] = [
    ("Arial Bold", "arialbd.ttf"),
    ("Arial Black", "ariblk.ttf"),
    ("Impact", "impact.ttf"),
    ("Segoe UI Bold", "segoeuib.ttf"),
    ("Comic Bold", "comicbd.ttf"),
    ("Built-in", ""),
];

fn load_font(i: usize) -> Option<ab_glyph::FontArc> {
    let file = FONTS.get(i)?.1;
    if file.is_empty() {
        return ab_glyph::FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).ok();
    }
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let bytes = std::fs::read(Path::new(&dir).join("Fonts").join(file)).ok()?;
    ab_glyph::FontArc::try_from_vec(bytes).ok()
}

/// Rasterise one line of text into an alpha mask, tightly sized.
pub fn render_text(font: &ab_glyph::FontArc, text: &str, px: f32) -> image::GrayImage {
    use ab_glyph::{Font, ScaleFont};
    let sf = font.as_scaled(px);
    let mut glyphs = Vec::new();
    let mut x = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        glyphs.push(id.with_scale_and_position(px, ab_glyph::point(x, sf.ascent())));
        x += sf.h_advance(id);
        prev = Some(id);
    }
    let w = (x.ceil() as u32).max(1) + 2;
    let h = (sf.height().ceil() as u32).max(1) + 2;
    let mut mask = image::GrayImage::new(w, h);
    for g in glyphs {
        if let Some(og) = font.outline_glyph(g) {
            let b = og.px_bounds();
            og.draw(|gx, gy, c| {
                let (px_, py_) = (b.min.x as i32 + gx as i32 + 1, b.min.y as i32 + gy as i32 + 1);
                if px_ >= 0 && py_ >= 0 && (px_ as u32) < w && (py_ as u32) < h {
                    let p = mask.get_pixel_mut(px_ as u32, py_ as u32);
                    p[0] = p[0].max((c.clamp(0.0, 1.0) * 255.0) as u8);
                }
            });
        }
    }
    mask
}

/// Paint `color` through an alpha mask centred on `at`.
fn stamp_mask(img: &mut RgbaImage, mask: &image::GrayImage, at: (u32, u32), color: [u8; 4], clip: Option<Face>) {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let ox = at.0 as i64 - mask.width() as i64 / 2;
    let oy = at.1 as i64 - mask.height() as i64 / 2;
    for (mx, my, m) in mask.enumerate_pixels() {
        if m[0] == 0 {
            continue;
        }
        let (x, y) = (ox + mx as i64, oy + my as i64);
        if x < 0 || y < 0 || x >= w || y >= h {
            continue;
        }
        if let Some(fc) = clip {
            if !fc.contains(x as u32, y as u32) {
                continue;
            }
        }
        let p = img.get_pixel_mut(x as u32, y as u32);
        p.0 = blend(p.0, color, m[0] as f32 / 255.0);
    }
}

/// Linear gradient from `a` (colour c1) to `b` (colour c2), over a face or the whole image.
fn gradient(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32), c1: [u8; 4], c2: [u8; 4], clip: Option<Face>) {
    let (w, h) = (img.width(), img.height());
    let (x0, y0, x1, y1) = match clip {
        Some(fc) => (fc.x, fc.y, fc.x + fc.w, fc.y + fc.h),
        None => (0, 0, w, h),
    };
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = (dx * dx + dy * dy).max(1.0);
    for y in y0..y1 {
        for x in x0..x1 {
            if clip.is_none() && face_at(Kind::Shirt, x, y).is_none() && w == TEMPLATE_W {
                continue; // leave the unused template area alone
            }
            let t = (((x as f32 + 0.5 - a.0) * dx + (y as f32 + 0.5 - a.1) * dy) / len2).clamp(0.0, 1.0);
            let mut c = [0u8; 4];
            for i in 0..4 {
                c[i] = (c1[i] as f32 + (c2[i] as f32 - c1[i] as f32) * t).round() as u8;
            }
            let p = img.get_pixel_mut(x, y);
            p.0 = blend(p.0, c, 1.0);
        }
    }
}

/// Darken non-front sides and the edges of every part so flat colours read as 3D in game.
fn shade_sides(img: &mut RgbaImage) {
    for fc in TEMPLATE_FACES.iter() {
        // Soft edge shadow: up to 22% darker at the border, fading out over 10px.
        for y in fc.y..fc.y + fc.h {
            for x in fc.x..fc.x + fc.w {
                let d = (x - fc.x).min(fc.x + fc.w - 1 - x).min(y - fc.y).min(fc.y + fc.h - 1 - y) as f32;
                let k = 1.0 - 0.22 * (1.0 - (d / 10.0).min(1.0)).powi(2);
                let p = img.get_pixel_mut(x, y);
                for c in 0..3 {
                    p[c] = (p[c] as f32 * k) as u8;
                }
            }
        }
    }
    for fc in TEMPLATE_FACES.iter() {
        let k = match fc.side {
            Side::Front | Side::Up => continue,
            Side::Back => 0.88,
            Side::Down => 0.75,
            Side::Left | Side::Right => 0.82,
        };
        for y in fc.y..fc.y + fc.h {
            for x in fc.x..fc.x + fc.w {
                let p = img.get_pixel_mut(x, y);
                for c in 0..3 {
                    p[c] = (p[c] as f32 * k) as u8;
                }
            }
        }
    }
}

pub fn open_url(url: &str) {
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_fit_and_dont_overlap() {
        for (i, a) in TEMPLATE_FACES.iter().enumerate() {
            assert!(a.x + a.w <= TEMPLATE_W && a.y + a.h <= TEMPLATE_H, "{a:?} out of bounds");
            for b in TEMPLATE_FACES.iter().skip(i + 1) {
                let overlap = a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn blend_and_erase() {
        assert_eq!(blend([0, 0, 0, 0], [255, 0, 0, 255], 1.0), [255, 0, 0, 255]);
        assert_eq!(blend([0, 0, 255, 255], [255, 0, 0, 255], 0.0), [0, 0, 255, 255]);
        let half = blend([0, 0, 0, 255], [255, 255, 255, 128], 1.0);
        assert!(half[0] > 120 && half[0] < 135 && half[3] == 255);
        assert_eq!(erase([10, 20, 30, 200], 1.0)[3], 0);
    }

    #[test]
    fn mirror_limbs_copies_front() {
        let mut d = Doc::new(Kind::Shirt);
        d.fill_faces(|fc| fc.part == Part::RightLimb && fc.side == Side::Front, [255, 0, 0, 255], true);
        d.mirror_limbs(true);
        let lf = find_face(Part::LeftLimb, Side::Front);
        assert_eq!(d.img.get_pixel(lf.x + 5, lf.y + 5).0, [255, 0, 0, 255]);
    }

    #[test]
    fn flood_fill_stays_in_face() {
        let mut img = RgbaImage::new(TEMPLATE_W, TEMPLATE_H);
        let fc = find_face(Part::Torso, Side::Front);
        flood_fill(&mut img, (fc.x + 3, fc.y + 3), [0, 255, 0, 255], Some(fc));
        assert_eq!(img.get_pixel(fc.x, fc.y)[1], 255);
        assert_eq!(img.get_pixel(fc.x - 1, fc.y)[3], 0);
    }

    #[test]
    fn preview_is_256() {
        let s = RgbaImage::new(TEMPLATE_W, TEMPLATE_H);
        let t = RgbaImage::new(TSHIRT_SIZE, TSHIRT_SIZE);
        let p = preview(&s, &s, &t, false);
        assert_eq!(p.dimensions(), (PREVIEW, PREVIEW));
        assert_eq!(p.get_pixel(100, 50).0, SKIN);
    }

    #[test]
    fn text_renders() {
        let font = load_font(FONTS.len() - 1).expect("built-in font");
        let m = render_text(&font, "Hi", 40.0);
        assert!(m.width() > 20 && m.height() > 30);
        assert!(m.pixels().any(|p| p[0] > 200));
    }

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#FF0000"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_hex("00ff0080").unwrap().a(), 128);
        assert_eq!(parse_hex("zz"), None);
    }
}
