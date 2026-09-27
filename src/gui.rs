//! Interactive window: a control pane on the left and a clean canvas on the right.
//!
//! The pane never overlaps the picture: it is a real column, and `Tab` collapses it so the canvas
//! fills the window for pure viewing. `B` (or the `theme:` button) switches the dark/light theme
//! for the canvas **and** the whole UI together, so the framing, the pane and the strokes always
//! agree.
//!
//! Two rendering modes: up to `MAX_ENUM_STEPS` the curve is a point list drawn with `draw_line`;
//! above that the adaptive deep renderer bakes the viewport into a texture and the reveal animates
//! through block levels instead of single steps.
//!
//! Launch defaults are the Koch arc (`base 4, dir 6, rule fold`); every parameter can also be
//! edited in the pane, and all the keyboard shortcuts keep working.

use macroquad::prelude::*;
use macroquad::{miniquad, Window};

use crate::ui::{self, Language, TextInput, Theme, FIELD_H};
use fun_fractal_base_direction::deep::{self, DeepOpts};
use fun_fractal_base_direction::fractal::{
    self, step_dir, Bbox, Curve, DigitRule, Params, MAX_BASE, MAX_DIR_ABS, MAX_ENUM_STEPS,
    MAX_STEPS, MIN_BASE,
};
use fun_fractal_base_direction::palette::Canvas;
use fun_fractal_base_direction::png;
use fun_fractal_base_direction::render::{self, RenderOpts, Window as Win};

const PANE_W: f32 = 344.0;
const MARGIN: f32 = 8.0;
const ROW_GAP: f32 = 8.0;
const BUTTON_H: f32 = 34.0;
const HEADER_H: f32 = 72.0;
const MAX_STROKES: usize = 100_000;
const GHOST_STROKES: usize = 40_000;
const GHOST_FADE_FRAMES: f32 = 60.0;
const LINE_WIDTH: f32 = 1.6;
const ZOOM_STEP: f64 = 1.25;
const PANEL_PAD: f64 = 0.06;
const MAX_ZOOM: f64 = 1e7;

const F_BASE: usize = 0;
const F_DIR: usize = 1;
const F_RULE: usize = 2;
const F_STEPS: usize = 3;
const F_HUE: usize = 4;
const RULE_CHOICES: [DigitRule; 3] = [DigitRule::Linear, DigitRule::Alternate, DigitRule::Fold];
const PREFIX_BLOCK: usize = 4096;

fn draw_text(text: impl AsRef<str>, x: f32, y: f32, size: f32, color: Color) -> TextDimensions {
    ui::draw_app_text(text, x, y, size, color)
}

fn measure_text(
    text: impl AsRef<str>,
    _font: Option<&Font>,
    size: u16,
    _scale: f32,
) -> TextDimensions {
    ui::measure_app_text(text, size)
}

fn tr(en: &'static str, zh: &'static str) -> &'static str {
    ui::localized(en, zh)
}

/// GUI launch settings. `shot` saves the whole window when the loop ends for smoke tests.
pub struct GuiOptions {
    pub hue: f32,
    pub width: u32,
    pub height: u32,
    pub language: Language,
    pub frames_limit: Option<u32>,
    pub shot: Option<String>,
}

impl Default for GuiOptions {
    fn default() -> Self {
        Self {
            hue: 0.0,
            width: 1500,
            height: 900,
            language: Language::En,
            frames_limit: None,
            shot: None,
        }
    }
}

pub fn run(params: Params, canvas: Canvas, options: GuiOptions) {
    let conf = macroquad::conf::Conf {
        // NOTE: do *not* raise draw_call_vertex_capacity here. With a custom capacity macroquad
        // silently loses geometry once a frame needs more than one batch, which showed up as the
        // curve being drawn in a few leftover fragments ("truncated") at high step counts.
        // The defaults (10k vertices / 5k indices) flush correctly; more draw calls cost nothing.
        miniquad_conf: miniquad::conf::Conf {
            window_title: "Fractal Studio".to_string(),
            window_width: options.width as i32,
            window_height: options.height as i32,
            high_dpi: true,
            sample_count: if options.shot.is_some() { 1 } else { 4 },
            window_resizable: true,
            ..Default::default()
        },
        ..Default::default()
    };
    Window::from_config(
        conf,
        amain(
            params,
            canvas,
            options.hue,
            options.language,
            options.frames_limit,
            options.shot,
        ),
    );
}

async fn amain(
    params: Params,
    canvas: Canvas,
    hue: f32,
    language: Language,
    frames_limit: Option<u32>,
    shot: Option<String>,
) {
    ui::init_fonts().unwrap_or_else(|e| panic!("Failed to load bundled Noto fonts: {e}"));
    ui::set_language(language);
    let mut app = App::new(params);
    app.canvas = canvas;
    app.set_hue(hue);
    let mut frames = 0u32;
    loop {
        if is_key_pressed(KeyCode::Escape) && !app.help && !app.typing() {
            break;
        }
        app.update();
        app.draw();
        frames += 1;
        if frames_limit.is_some_and(|limit| frames >= limit) {
            break;
        }
        next_frame().await;
    }
    // Read the back buffer we just drew into; after a swap it would be gone.
    if let Some(path) = shot {
        let image = get_screen_data();
        let bytes = png::encode_rgba(image.width as u32, image.height as u32, &image.bytes, true);
        match render::write_file(&path, &bytes) {
            Ok(()) => println!("shot -> {path} ({}x{})", image.width, image.height),
            Err(e) => eprintln!("shot failed: {e}"),
        }
    }
}

struct Ghost {
    curve: Curve,
    revealed: usize,
    alpha: f32,
}

struct Bake {
    tex: Texture2D,
    w: u16,
    h: u16,
}

struct App {
    params: Params,
    curve: Option<Curve>,
    framing: Bbox,
    revealed: usize,
    deep_level: f64,
    rendered_levels: u32,
    speed: f64,
    speed_user_set: bool,
    paused: bool,
    hue: f32,
    canvas: Canvas,
    zoom: f64,
    center: Option<[f64; 2]>,
    follow: bool,
    help: bool,
    pane_open: bool,
    language: Language,
    prefix: Bbox,
    ghost: Option<Ghost>,
    bake: Option<Bake>,
    bake_stale: bool,
    deep_stats: Option<deep::Stats>,
    fields: [TextInput; 5],
    focus: Option<usize>,
    progress_dragging: bool,
    prefix_checkpoints: Vec<Bbox>,
    pan_last: Option<Vec2>,
    overview_dragging: bool,
    overview_bake: Option<Bake>,
    note: Option<(String, i32)>,
}

/// Shorten `text` with an ellipsis so it fits `max_w` pixels at `size`.
fn fit_text(text: &str, max_w: f32, size: u16) -> String {
    if measure_text(text, None, size, 1.0).width <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut probe = out.clone();
        probe.push(ch);
        probe.push_str("...");
        if measure_text(&probe, None, size, 1.0).width > max_w {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn overview_viewport_rect(image: Rect, camera: Win, full: Win) -> Option<Rect> {
    let view = full.view(image.w as u32, image.h as u32);
    let a = view.map([
        camera.center[0] - camera.half[0],
        camera.center[1] + camera.half[1],
    ]);
    let b = view.map([
        camera.center[0] + camera.half[0],
        camera.center[1] - camera.half[1],
    ]);
    let left = (image.x + a[0] as f32).clamp(image.x, image.x + image.w);
    let right = (image.x + b[0] as f32).clamp(image.x, image.x + image.w);
    let top = (image.y + a[1] as f32).clamp(image.y, image.y + image.h);
    let bottom = (image.y + b[1] as f32).clamp(image.y, image.y + image.h);
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn format_steps(steps: u64) -> String {
    if steps.is_power_of_two() && steps >= 1024 {
        format!("2^{}", steps.trailing_zeros())
    } else {
        format!("{steps}")
    }
}

fn accepts_queued_chars(before: Option<usize>, after: Option<usize>) -> bool {
    before == after && after.is_some_and(|index| index != F_RULE)
}

fn build_prefix_checkpoints(curve: &Curve) -> Vec<Bbox> {
    let mut bbox = Bbox::point(curve.pts[0][0], curve.pts[0][1]);
    let mut checkpoints = vec![bbox];
    for (index, p) in curve.pts.iter().enumerate().skip(1) {
        bbox.grow(p[0], p[1]);
        if index % PREFIX_BLOCK == 0 {
            checkpoints.push(bbox);
        }
    }
    checkpoints
}

fn cached_prefix_bbox(curve: &Curve, checkpoints: &[Bbox], upto: usize) -> Bbox {
    let upto = upto.min(curve.segments());
    let block = upto / PREFIX_BLOCK;
    let mut bbox = checkpoints[block];
    for p in &curve.pts[block * PREFIX_BLOCK + 1..=upto] {
        bbox.grow(p[0], p[1]);
    }
    bbox
}

fn prefix_bbox(curve: &Curve, upto: usize) -> Bbox {
    let upto = upto.min(curve.segments());
    let mut bbox = Bbox::point(curve.pts[0][0], curve.pts[0][1]);
    for p in &curve.pts[1..=upto] {
        bbox.grow(p[0], p[1]);
    }
    bbox
}

fn materialise(params: &Params) -> (Option<Curve>, Bbox) {
    if params.is_deep() {
        (None, deep::frame_bbox(params, params.steps))
    } else {
        let curve = fractal::generate(params);
        let bbox = curve.bbox;
        (Some(curve), bbox)
    }
}

impl App {
    fn new(params: Params) -> Self {
        let params = params.clamped();
        let (curve, framing) = materialise(&params);
        let default_speed = if curve.is_some() {
            (params.steps.min(MAX_ENUM_STEPS) as f64 / 240.0).max(1.0)
        } else {
            (fractal::steps_to_level(params.base, params.steps) as f64 / 240.0).max(1.0 / 240.0)
        };
        let prefix = curve
            .as_ref()
            .map(|c| c.bbox)
            .unwrap_or_else(|| Bbox::point(0.0, 0.0));
        let revealed = curve.as_ref().map(|c| c.segments()).unwrap_or(1);
        let prefix_checkpoints = curve
            .as_ref()
            .map(build_prefix_checkpoints)
            .unwrap_or_default();
        App {
            params,
            curve,
            framing,
            revealed,
            deep_level: fractal::steps_to_level(params.base, params.steps) as f64,
            rendered_levels: fractal::steps_to_level(params.base, params.steps),
            speed: default_speed,
            speed_user_set: false,
            paused: true,
            hue: 0.0,
            canvas: Canvas::Dark,
            zoom: 1.0,
            center: None,
            follow: false,
            help: false,
            pane_open: true,
            language: ui::language(),
            prefix,
            ghost: None,
            bake: None,
            bake_stale: true,
            deep_stats: None,
            fields: [
                TextInput::new("base", "2..64", params.base.to_string()),
                TextInput::new("dir", "1..64", params.dir.to_string()),
                TextInput::new("rule", "linear|alternate|fold", params.rule.name()),
                TextInput::new("steps", "2^30, 1.5e6, ...", format_steps(params.steps)),
                TextInput::new("hue", "0..1", "0.000"),
            ],
            focus: None,
            progress_dragging: false,
            prefix_checkpoints,
            pan_last: None,
            overview_dragging: false,
            overview_bake: None,
            note: Some((
                tr(
                    "Ready. Replay to draw the curve.",
                    "就绪. 点击重播观看分形绘制.",
                )
                .to_string(),
                260,
            )),
        }
    }

    // ---- layout -----------------------------------------------------------

    fn pane_rect(&self, sw: f32, sh: f32) -> Rect {
        if self.pane_open {
            Rect::new(
                0.0,
                0.0,
                PANE_W.min((sw * 0.45).max(170.0)).min(sw - 24.0),
                sh,
            )
        } else {
            Rect::new(0.0, 0.0, 0.0, 0.0)
        }
    }

    /// The picture area: everything right of the pane, with a small margin.
    fn canvas_rect(&self, sw: f32, sh: f32) -> Rect {
        let pane = self.pane_rect(sw, sh);
        let x = (pane.w + MARGIN).min(sw - 16.0);
        Rect::new(
            x,
            MARGIN,
            (sw - x - MARGIN).max(16.0),
            (sh - 2.0 * MARGIN).max(16.0),
        )
    }

    fn field_rect(&self, index: usize, pane: Rect) -> Rect {
        let y = pane.y + HEADER_H + 22.0 + (FIELD_H + ROW_GAP) * index as f32;
        Rect::new(pane.x + 16.0, y, pane.w - 32.0, FIELD_H)
    }

    fn button_rect(&self, row: usize, index: usize, pane: Rect) -> Rect {
        let y = pane.y
            + HEADER_H
            + 22.0
            + (FIELD_H + ROW_GAP) * self.fields.len() as f32
            + 18.0
            + (BUTTON_H + 8.0) * row as f32;
        let w = (pane.w - 32.0 - 8.0) / 2.0;
        let x = pane.x + 16.0 + (w + 8.0) * index as f32;
        Rect::new(x, y, w, BUTTON_H)
    }

    fn overview_rect(&self, sw: f32, sh: f32) -> Rect {
        if self.pane_open && sh >= 870.0 {
            let pane = self.pane_rect(sw, sh);
            Rect::new(
                pane.x + 16.0,
                self.progress_rect(pane).y + 132.0,
                pane.w - 32.0,
                (sh - self.progress_rect(pane).y - 240.0).min(148.0),
            )
        } else {
            let canvas = self.canvas_rect(sw, sh);
            let w = canvas.w.min(216.0);
            Rect::new(canvas.x + canvas.w - w - 16.0, canvas.y + 16.0, w, 138.0)
        }
    }

    fn overview_image_rect(&self, sw: f32, sh: f32) -> Rect {
        let card = self.overview_rect(sw, sh);
        Rect::new(card.x + 9.0, card.y + 29.0, card.w - 18.0, card.h - 38.0)
    }

    fn overview_window(&self, image: Rect) -> Win {
        Win::from_bbox(
            &self.framing,
            PANEL_PAD,
            1.0,
            [0.0, 0.0],
            (image.w / image.h.max(1.0)) as f64,
        )
    }

    fn recenter_from_overview(&mut self, sw: f32, sh: f32, point: Vec2) {
        let image = self.overview_image_rect(sw, sh);
        let center = world_at(
            &self.overview_window(image),
            image,
            point.x as f64,
            point.y as f64,
        );
        self.center = Some(center);
        self.follow = false;
        self.bake_stale = true;
    }

    fn progress_rect(&self, pane: Rect) -> Rect {
        Rect::new(
            pane.x + 16.0,
            self.button_rect(3, 0, pane).y + BUTTON_H + 54.0,
            pane.w - 32.0,
            12.0,
        )
    }

    fn seek(&mut self, fraction: f32) {
        let fraction = fraction.clamp(0.0, 1.0);
        if let Some(curve) = &self.curve {
            let revealed = ((curve.segments() as f64 * fraction as f64).round() as usize)
                .clamp(1, curve.segments().max(1));
            if self.revealed != revealed {
                self.revealed = revealed;
                self.prefix = cached_prefix_bbox(curve, &self.prefix_checkpoints, revealed);
            }
        } else {
            self.deep_level = self.levels() as f64 * fraction as f64;
            self.bake_stale = true;
        }
        self.ghost = None;
        self.paused = true;
    }

    fn seek_at(&mut self, track: Rect, mouse_x: f32) {
        self.seek(((mouse_x - track.x) / track.w).clamp(0.0, 1.0));
    }

    fn select_rule(&mut self, rule: DigitRule) {
        self.fields[F_RULE].replace(rule.name());
        self.fields[F_RULE].error = 0.0;
    }

    fn replay(&mut self) {
        if let Some(curve) = &self.curve {
            self.revealed = 1;
            self.prefix = prefix_bbox(curve, 1);
        } else {
            self.deep_level = 0.0;
        }
        self.ghost = None;
        self.paused = false;
        self.bake_stale = true;
    }

    fn is_deep(&self) -> bool {
        self.curve.is_none()
    }

    fn levels(&self) -> u32 {
        fractal::steps_to_level(self.params.base, self.params.steps)
    }

    fn steps_now(&self) -> u64 {
        match &self.curve {
            Some(_) => self.revealed as u64,
            None => {
                let progress = self.deep_level.clamp(0.0, self.levels() as f64);
                let lower_level = progress.floor() as u32;
                let low =
                    fractal::level_steps(self.params.base, lower_level).min(self.params.steps);
                let high =
                    fractal::level_steps(self.params.base, (lower_level + 1).min(self.levels()))
                        .min(self.params.steps);
                (low as f64 + (high - low) as f64 * progress.fract())
                    .round()
                    .clamp(1.0, self.params.steps as f64) as u64
            }
        }
    }

    fn typing(&self) -> bool {
        self.focus.is_some()
    }

    fn note(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), 200));
    }

    /// The chrome follows the canvas theme, so `B` switches both together.
    fn theme(&self) -> Theme {
        Theme::for_canvas(self.canvas)
    }

    /// World window for the current camera, framed inside `rect` (the canvas area).
    fn window(&self, rect: Rect) -> Win {
        let bbox = if self.follow && self.curve.is_some() {
            &self.prefix
        } else {
            &self.framing
        };
        let framed = Win::from_bbox(
            bbox,
            PANEL_PAD,
            self.zoom,
            [0.0, 0.0],
            (rect.w / rect.h.max(1.0)) as f64,
        );
        match self.center {
            Some(c) => Win::new(c, framed.half),
            None => framed,
        }
    }

    /// View in *screen* pixels for an explicit framing box (the raster space is canvas-local, so
    /// the origin is offset into the canvas rect).
    fn view_for(&self, rect: Rect, bbox: &Bbox) -> render::View {
        let framed = Win::from_bbox(
            bbox,
            PANEL_PAD,
            self.zoom,
            [0.0, 0.0],
            (rect.w / rect.h.max(1.0)) as f64,
        );
        let win = match self.center {
            Some(c) => Win::new(c, framed.half),
            None => framed,
        };
        let mut view = win.view(rect.w.max(16.0) as u32, rect.h.max(16.0) as u32);
        view.ox += rect.x as f64;
        view.oy += rect.y as f64;
        view
    }

    fn sync_fields(&mut self) {
        self.fields[F_BASE].set(self.params.base.to_string());
        self.fields[F_DIR].set(self.params.dir.to_string());
        self.fields[F_RULE].set(self.params.rule.name());
        self.fields[F_STEPS].set(format_steps(self.params.steps));
        self.fields[F_HUE].set(format!("{:.3}", self.hue));
    }

    fn set_focus(&mut self, focus: Option<usize>) {
        if self.focus != focus {
            if let Some(index) = focus {
                self.fields[index].focus();
            }
        }
        self.focus = focus;
        for (index, field) in self.fields.iter_mut().enumerate() {
            field.focused = focus == Some(index);
        }
    }

    fn cancel_edit(&mut self) {
        self.set_focus(None);
        self.fields[F_BASE].replace(self.params.base.to_string());
        self.fields[F_DIR].replace(self.params.dir.to_string());
        self.fields[F_RULE].replace(self.params.rule.name());
        self.fields[F_STEPS].replace(format_steps(self.params.steps));
        self.fields[F_HUE].replace(format!("{:.3}", self.hue));
    }

    fn apply_fields(&mut self) {
        let mut rejected: Vec<&'static str> = Vec::new();
        let base = ui::parse_count(&self.fields[F_BASE].text)
            .filter(|v| (MIN_BASE as u64..=MAX_BASE as u64).contains(v));
        if base.is_none() {
            rejected.push("base");
        }
        let dir = ui::parse_int(&self.fields[F_DIR].text)
            .filter(|v| *v != 0 && v.unsigned_abs() <= MAX_DIR_ABS as u64);
        if dir.is_none() {
            rejected.push("dir");
        }
        let rule = DigitRule::parse(self.fields[F_RULE].text.trim());
        if rule.is_none() {
            rejected.push("rule");
        }
        let steps =
            ui::parse_count(&self.fields[F_STEPS].text).filter(|v| (1..=MAX_STEPS).contains(v));
        if steps.is_none() {
            rejected.push("steps");
        }
        let hue = ui::parse_float(&self.fields[F_HUE].text)
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v));
        if hue.is_none() {
            rejected.push("hue");
        }
        if !rejected.is_empty() {
            for field in &mut self.fields {
                if rejected.contains(&field.label) {
                    field.mark_error();
                }
            }
            let fields = rejected
                .into_iter()
                .map(ui::field_label)
                .collect::<Vec<_>>()
                .join(", ");
            self.note(if self.language == Language::Zh {
                format!("请检查 {fields}. 进制 2..64, 方向 +/-1..64, 步数 1..2^50, 色相 0..1")
            } else {
                format!("Check {fields}. base 2..64, dir +/-1..64, steps 1..2^50, hue 0..1")
            });
            return;
        }
        let next = Params::new(base.unwrap() as u32, dir.unwrap() as i32, steps.unwrap())
            .with_rule(rule.unwrap());
        let changed = self.params != next;
        self.params = next;
        self.hue = hue.unwrap() as f32;
        self.set_focus(None);
        self.sync_fields();
        if changed {
            self.rebuild(false);
        } else {
            self.bake_stale = true;
            self.overview_bake = None;
        }
        self.note(if self.language == Language::Zh {
            format!(
                "进制 {} 方向 {} 规则 {} 步数 {} ({:.2}°/单位), {}",
                self.params.base,
                self.params.dir,
                self.params.rule.name(),
                self.params.steps,
                self.params.turn_per_digit_deg(),
                if changed { "重新播放" } else { "已更新" }
            )
        } else {
            format!(
                "base {} dir {} rule {} steps {} ({:.2} deg/unit) {}",
                self.params.base,
                self.params.dir,
                self.params.rule.name(),
                self.params.steps,
                self.params.turn_per_digit_deg(),
                if changed { "replaying" } else { "updated" }
            )
        });
    }

    fn rebuild(&mut self, keep_progress: bool) {
        let old_revealed = self.revealed;
        let old_curve = self.curve.take();
        let old_fraction = match &old_curve {
            Some(c) => self.revealed as f64 / c.segments().max(1) as f64,
            None => self.deep_level / self.rendered_levels.max(1) as f64,
        };
        let (curve, framing) = materialise(&self.params);
        // Decide from the *new* curve: `self.curve` is already taken above.
        let deep = curve.is_none();
        if deep {
            self.deep_level = if keep_progress {
                old_fraction * self.levels() as f64
            } else {
                0.0
            };
            self.ghost = None;
        } else {
            let segments = curve.as_ref().map(|c| c.segments()).unwrap_or(0);
            self.revealed = if keep_progress {
                ((segments as f64) * old_fraction).round() as usize
            } else {
                1
            }
            .clamp(1, segments.max(1));
            self.ghost = match (keep_progress, old_curve) {
                (false, Some(previous)) => Some(Ghost {
                    curve: previous,
                    revealed: old_revealed.max(1),
                    alpha: 1.0,
                }),
                _ => None,
            };
            self.prefix = curve
                .as_ref()
                .map(|c| prefix_bbox(c, self.revealed))
                .unwrap_or_else(|| Bbox::point(0.0, 0.0));
        }
        self.prefix_checkpoints = curve
            .as_ref()
            .map(build_prefix_checkpoints)
            .unwrap_or_default();
        self.curve = curve;
        self.framing = framing;
        self.rendered_levels = self.levels();
        self.overview_bake = None;
        if !keep_progress {
            self.paused = false;
            self.zoom = 1.0;
            self.center = None;
            self.follow = false;
        }
        if !self.speed_user_set {
            self.speed = if deep {
                (self.levels() as f64 / 240.0).max(1.0 / 240.0)
            } else {
                (self.params.steps.min(MAX_ENUM_STEPS) as f64 / 240.0).max(1.0)
            };
        }
        self.bake_stale = true;
    }

    fn toggle_language(&mut self) {
        self.language = self.language.toggle();
        ui::set_language(self.language);
        self.note(tr("Language: English", "语言: 中文"));
    }

    fn toggle_canvas(&mut self) {
        self.canvas = if self.canvas.is_light() {
            Canvas::Dark
        } else {
            Canvas::Light
        };
        self.bake_stale = true;
        self.overview_bake = None;
        self.note(if self.language == Language::Zh {
            format!(
                "主题: {} (画布与界面)",
                if self.canvas.is_light() {
                    "浅色"
                } else {
                    "深色"
                }
            )
        } else {
            format!("theme: {} (canvas + UI)", self.canvas.name())
        });
    }

    fn set_hue(&mut self, hue: f32) {
        self.hue = hue.rem_euclid(1.0);
        self.fields[F_HUE].set(format!("{:.3}", self.hue));
        self.bake_stale = true;
        self.overview_bake = None;
    }

    fn use_window(&mut self, canvas: Rect, win: Win) {
        let fit = Win::from_bbox(
            &self.framing,
            PANEL_PAD,
            1.0,
            [0.0, 0.0],
            (canvas.w / canvas.h.max(1.0)) as f64,
        );
        self.follow = false;
        self.zoom = (fit.half[0] / win.half[0]).clamp(1.0, MAX_ZOOM);
        self.center = Some(win.center);
        self.bake_stale = true;
    }

    fn zoom_at(&mut self, canvas: Rect, point: Vec2, factor: f64) {
        let win = self.window(canvas);
        let fit = Win::from_bbox(
            &self.framing,
            PANEL_PAD,
            1.0,
            [0.0, 0.0],
            (canvas.w / canvas.h.max(1.0)) as f64,
        );
        let current_zoom = fit.half[0] / win.half[0];
        let factor = (current_zoom * factor).clamp(1.0, MAX_ZOOM) / current_zoom;
        if (factor - 1.0).abs() < 1e-12 {
            return;
        }
        let anchor = world_at(&win, canvas, point.x as f64, point.y as f64);
        let mut next = win;
        next.zoom_around(factor, anchor);
        self.use_window(canvas, next);
    }

    fn pan_pixels(&mut self, canvas: Rect, delta: Vec2) {
        let mut win = self.window(canvas);
        let scale = win
            .view(canvas.w.max(16.0) as u32, canvas.h.max(16.0) as u32)
            .scale;
        win.pan([-delta.x as f64 / scale, delta.y as f64 / scale]);
        self.use_window(canvas, win);
    }

    fn zoom_by(&mut self, factor: f64) {
        let canvas = self.canvas_rect(screen_width(), screen_height());
        self.zoom_at(
            canvas,
            vec2(canvas.x + canvas.w / 2.0, canvas.y + canvas.h / 2.0),
            factor,
        );
    }

    fn advance(&mut self, _dt: f32) {
        if !self.paused {
            if self.is_deep() {
                let levels = self.levels() as f64;
                if self.deep_level < levels {
                    self.deep_level = (self.deep_level + self.speed).min(levels);
                    self.bake_stale = true;
                }
            } else if let Some(curve) = &self.curve {
                if self.revealed < curve.segments() {
                    let step = self.speed.round().max(1.0) as usize;
                    let target = (self.revealed + step).min(curve.segments());
                    for p in &curve.pts[self.revealed + 1..=target] {
                        self.prefix.grow(p[0], p[1]);
                    }
                    self.revealed = target;
                }
            }
        }
        if let Some((_, left)) = self.note.as_mut() {
            *left -= 1;
            if *left <= 0 {
                self.note = None;
            }
        }
        if let Some(ghost) = self.ghost.as_mut() {
            ghost.alpha -= 1.0 / GHOST_FADE_FRAMES;
        }
        if self.ghost.as_ref().is_some_and(|g| g.alpha <= 0.0) {
            self.ghost = None;
        }
    }

    fn update(&mut self) {
        let dt = get_frame_time().min(0.1);
        // macroquad retains unread characters across frames: consume them even with no focused field.
        let queued_chars: Vec<char> = std::iter::from_fn(get_char_pressed).collect();
        let focus_before = self.focus;
        for field in self.fields.iter_mut() {
            field.tick(dt);
        }

        let (sw, sh) = (screen_width(), screen_height());
        let pane = self.pane_rect(sw, sh);
        let canvas = self.canvas_rect(sw, sh);
        let (mx, my) = mouse_position();
        let cursor = vec2(mx, my);

        // Event handling is separate from paint: update must never draw into the back buffer.
        let icon = Rect::new(pane.x + pane.w - 42.0, pane.y + 12.0, 32.0, 32.0);
        let lan = Rect::new(pane.x + pane.w - 104.0, pane.y + 12.0, 52.0, 32.0);
        if self.pane_open && ui::clicked(lan) {
            self.set_focus(None);
            self.toggle_language();
        }
        if self.pane_open && ui::clicked(icon) {
            self.help = !self.help;
            self.set_focus(None);
        }
        if is_key_pressed(KeyCode::H) && !self.typing() {
            self.help = !self.help;
        }
        if self.help {
            miniquad::window::set_mouse_cursor(miniquad::CursorIcon::Default);
            if is_key_pressed(KeyCode::Escape) {
                self.help = false;
            }
            self.pan_last = None;
            self.overview_dragging = false;
            self.progress_dragging = false;
            self.advance(dt);
            return;
        }

        // Field focus by click.
        if !self.pane_open && ui::clicked(Rect::new(MARGIN, MARGIN, 26.0, 46.0)) {
            self.pane_open = true;
            self.bake_stale = true;
        }
        if self.pane_open {
            let mut clicked_field: Option<usize> = None;
            for index in 0..self.fields.len() {
                if is_mouse_button_pressed(MouseButton::Left)
                    && ui::inside(self.field_rect(index, pane), cursor)
                {
                    clicked_field = Some(index);
                }
            }
            if let Some(index) = clicked_field {
                self.set_focus(Some(index));
                if index == F_RULE {
                    let rect = self.field_rect(F_RULE, pane);
                    for (choice, rule) in RULE_CHOICES.into_iter().enumerate() {
                        if ui::inside(ui::segmented_choice_rect(rect, choice), cursor) {
                            self.select_rule(rule);
                        }
                    }
                }
            } else if is_mouse_button_pressed(MouseButton::Left) && self.focus.is_some() {
                self.set_focus(None);
            }
        } else if is_mouse_button_pressed(MouseButton::Left) && self.focus.is_some() {
            self.set_focus(None);
        }

        // Only deliver characters to a field that was already focused before this frame.
        if let Some(index) = self.focus {
            let command = is_key_down(KeyCode::LeftSuper)
                || is_key_down(KeyCode::RightSuper)
                || is_key_down(KeyCode::LeftControl)
                || is_key_down(KeyCode::RightControl);
            if index == F_RULE {
                let selected = RULE_CHOICES
                    .iter()
                    .position(|rule| rule.name() == self.fields[F_RULE].text)
                    .unwrap_or(0);
                if is_key_pressed(KeyCode::Left) {
                    self.select_rule(RULE_CHOICES[(selected + 2) % RULE_CHOICES.len()]);
                }
                if is_key_pressed(KeyCode::Right) {
                    self.select_rule(RULE_CHOICES[(selected + 1) % RULE_CHOICES.len()]);
                }
            } else {
                if command {
                    if is_key_pressed(KeyCode::A) {
                        self.fields[index].select_all();
                    }
                    if is_key_pressed(KeyCode::C) && self.fields[index].is_selected() {
                        miniquad::window::clipboard_set(&self.fields[index].text);
                    }
                    if is_key_pressed(KeyCode::X) && self.fields[index].is_selected() {
                        miniquad::window::clipboard_set(&self.fields[index].text);
                        self.fields[index].clear();
                    }
                    if is_key_pressed(KeyCode::V) {
                        if let Some(text) = miniquad::window::clipboard_get() {
                            self.fields[index].paste(&text);
                        }
                    }
                }
                if !command && accepts_queued_chars(focus_before, self.focus) {
                    for c in queued_chars {
                        self.fields[index].push_char(c);
                    }
                }
                if is_key_pressed(KeyCode::Backspace) {
                    self.fields[index].backspace();
                }
                if is_key_pressed(KeyCode::Delete) {
                    self.fields[index].delete();
                }
                if is_key_pressed(KeyCode::Left) {
                    let next = self.fields[index].cursor().saturating_sub(1);
                    self.fields[index].move_cursor(next);
                }
                if is_key_pressed(KeyCode::Right) {
                    let next = self.fields[index].cursor() + 1;
                    self.fields[index].move_cursor(next);
                }
                if is_key_pressed(KeyCode::Home) {
                    self.fields[index].move_cursor(0);
                }
                if is_key_pressed(KeyCode::End) {
                    self.fields[index].move_cursor(usize::MAX);
                }
            }
            if is_key_pressed(KeyCode::Tab) {
                let offset = if is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift)
                {
                    self.fields.len() - 1
                } else {
                    1
                };
                self.set_focus(Some((index + offset) % self.fields.len()));
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                self.apply_fields();
            }
            if is_key_pressed(KeyCode::Escape) {
                self.cancel_edit();
            }
        }

        if self.pane_open {
            if ui::clicked(self.button_rect(0, 0, pane)) {
                self.apply_fields();
            }
            if ui::clicked(self.button_rect(0, 1, pane)) {
                self.zoom = 1.0;
                self.center = None;
                self.follow = false;
                self.bake_stale = true;
                self.note(tr("View fitted to curve", "视图已适应曲线"));
            }
            if ui::clicked(self.button_rect(1, 0, pane)) {
                self.paused = !self.paused;
            }
            if ui::clicked(self.button_rect(1, 1, pane)) {
                self.replay();
            }
            if ui::clicked(self.button_rect(2, 0, pane)) {
                self.export(false);
            }
            if ui::clicked(self.button_rect(2, 1, pane)) {
                self.export(true);
            }
            if ui::clicked(self.button_rect(3, 0, pane)) {
                self.toggle_canvas();
            }
            if ui::clicked(self.button_rect(3, 1, pane)) {
                self.set_focus(None);
                self.pane_open = false;
                self.bake_stale = true;
            }
            let track = self.progress_rect(pane);
            let hit = Rect::new(track.x - 9.0, track.y - 7.0, track.w + 18.0, track.h + 14.0);
            if track.y + 110.0 < sh - 66.0
                && is_mouse_button_pressed(MouseButton::Left)
                && ui::inside(hit, cursor)
            {
                self.progress_dragging = true;
                self.seek_at(track, mx);
            }
        }
        if self.progress_dragging {
            if self.pane_open
                && (is_mouse_button_down(MouseButton::Left)
                    || is_mouse_button_released(MouseButton::Left))
            {
                self.seek_at(self.progress_rect(pane), mx);
            }
            if !is_mouse_button_down(MouseButton::Left) {
                self.progress_dragging = false;
            }
        }

        // Keys (only while not typing into a field).
        if !self.typing() {
            if is_key_pressed(KeyCode::Tab) {
                self.pane_open = !self.pane_open;
                self.progress_dragging = false;
                self.bake_stale = true;
            }
            if is_key_pressed(KeyCode::B) {
                self.toggle_canvas();
            }
            if is_key_pressed(KeyCode::L) {
                self.toggle_language();
            }
            let mut dirty = false;
            if is_key_pressed(KeyCode::LeftBracket) {
                self.params.base = (self.params.base.saturating_sub(1)).max(MIN_BASE);
                dirty = true;
            }
            if is_key_pressed(KeyCode::RightBracket) {
                self.params.base = (self.params.base + 1).min(MAX_BASE);
                dirty = true;
            }
            if is_key_pressed(KeyCode::Comma) {
                self.params.dir = step_dir(self.params.dir, -1);
                dirty = true;
            }
            if is_key_pressed(KeyCode::Period) {
                self.params.dir = step_dir(self.params.dir, 1);
                dirty = true;
            }
            if is_key_pressed(KeyCode::Minus) {
                self.params.steps = (self.params.steps / 2).max(2);
                dirty = true;
            }
            if is_key_pressed(KeyCode::Equal) {
                self.params.steps = self.params.steps.saturating_mul(2).min(MAX_STEPS);
                dirty = true;
            }
            if is_key_pressed(KeyCode::T) {
                self.params.rule = match self.params.rule {
                    DigitRule::Linear => DigitRule::Alternate,
                    DigitRule::Alternate => DigitRule::Fold,
                    DigitRule::Fold => DigitRule::Linear,
                };
                dirty = true;
            }
            if dirty {
                let keep = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
                self.sync_fields();
                self.rebuild(keep);
                self.note(if self.language == Language::Zh {
                    format!(
                        "进制 {} 方向 {} 规则 {} 步数 {} ({:.2}°/单位), {}",
                        self.params.base,
                        self.params.dir,
                        self.params.rule.name(),
                        self.params.steps,
                        self.params.turn_per_digit_deg(),
                        if keep { "保持进度" } else { "重新播放" }
                    )
                } else {
                    format!(
                        "base {} dir {} rule {} steps {} ({:.2} deg/unit) {}",
                        self.params.base,
                        self.params.dir,
                        self.params.rule.name(),
                        self.params.steps,
                        self.params.turn_per_digit_deg(),
                        if keep { "hot swap" } else { "replaying" }
                    )
                });
            }

            if is_key_pressed(KeyCode::Up) {
                self.speed = (self.speed * 2.0).min(1e9);
                self.speed_user_set = true;
            }
            if is_key_pressed(KeyCode::Down) {
                self.speed = (self.speed / 2.0).max(1.0 / 240.0);
                self.speed_user_set = true;
            }
            if is_key_pressed(KeyCode::Space) {
                self.paused = !self.paused;
            }
            if is_key_pressed(KeyCode::R) {
                self.replay();
            }
            if is_key_pressed(KeyCode::F) {
                self.zoom = 1.0;
                self.center = None;
                self.follow = false;
                self.bake_stale = true;
            }
            if is_key_pressed(KeyCode::G) {
                self.follow = !self.follow;
            }
            if is_key_pressed(KeyCode::C) {
                self.set_hue(self.hue + 0.09);
            }
            if is_key_pressed(KeyCode::P) {
                self.export(false);
            }
            if is_key_pressed(KeyCode::S) {
                self.export(true);
            }
        }

        // Camera inside the canvas area: wheel zooms around the cursor, drag pans.
        let over_pane = self.pane_open && ui::inside(pane, cursor);
        let in_canvas = ui::inside(canvas, cursor);
        let overview = self.overview_rect(sw, sh);
        let over_overview = ui::inside(overview, cursor);
        let over_field = self.pane_open
            && (0..self.fields.len()).any(|index| ui::inside(self.field_rect(index, pane), cursor));
        let track = self.progress_rect(pane);
        let progress_hit = Rect::new(track.x - 9.0, track.y - 7.0, track.w + 18.0, track.h + 14.0);
        miniquad::window::set_mouse_cursor(
            if self.progress_dragging
                || (self.pane_open && ui::inside(progress_hit, cursor))
                || over_overview
                || (self.pane_open && ui::inside(self.field_rect(F_RULE, pane), cursor))
            {
                miniquad::CursorIcon::Pointer
            } else if in_canvas {
                miniquad::CursorIcon::Move
            } else if over_field {
                miniquad::CursorIcon::Text
            } else {
                miniquad::CursorIcon::Default
            },
        );
        if is_mouse_button_pressed(MouseButton::Left) && over_overview {
            self.overview_dragging = true;
            self.pan_last = None;
        }
        if is_mouse_button_released(MouseButton::Left) || !is_mouse_button_down(MouseButton::Left) {
            self.overview_dragging = false;
        }
        if self.overview_dragging {
            let image = self.overview_image_rect(sw, sh);
            self.recenter_from_overview(
                sw,
                sh,
                vec2(
                    mx.clamp(image.x, image.x + image.w),
                    my.clamp(image.y, image.y + image.h),
                ),
            );
        }
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 && in_canvas && !over_overview {
            // Trackpads may emit large deltas in a single frame; cap the step so the curve
            // cannot disappear from a single scroll event.
            self.zoom_at(
                canvas,
                cursor,
                1.18_f64.powf((wheel as f64).clamp(-2.0, 2.0)),
            );
        }
        if !self.typing() && is_key_pressed(KeyCode::X) {
            self.zoom_by(ZOOM_STEP);
        }
        if !self.typing() && is_key_pressed(KeyCode::Z) {
            self.zoom_by(1.0 / ZOOM_STEP);
        }
        if is_mouse_button_pressed(MouseButton::Left) {
            self.pan_last = (in_canvas && !over_pane && !over_overview).then_some(cursor);
        }
        if is_mouse_button_released(MouseButton::Left) || !is_mouse_button_down(MouseButton::Left) {
            self.pan_last = None;
        }
        if let Some(last) = self.pan_last {
            let delta = cursor - last;
            self.pan_last = Some(cursor);
            if delta.length_squared() > 0.0 {
                self.pan_pixels(canvas, delta);
            }
        }

        self.advance(dt);
    }

    fn export(&mut self, full: bool) {
        let canvas = self.canvas_rect(screen_width(), screen_height());
        let (w, h, ss) = if full {
            let w = (canvas.w as u32).clamp(640, 1920) * 2;
            let h = (canvas.h as u32).clamp(360, 1080) * 2;
            let ss = if (w as u64) * (h as u64) > 8_000_000 {
                1
            } else {
                2
            };
            (w, h, ss)
        } else {
            (canvas.w as u32, canvas.h as u32, 1)
        };
        let win = if full {
            let framed = Win::from_bbox(
                &self.framing,
                PANEL_PAD,
                1.0,
                [0.0, 0.0],
                w as f64 / h as f64,
            );
            match self.center {
                Some(c) if self.zoom > 1.01 => Win::new(c, framed.half),
                _ => framed,
            }
        } else {
            self.window(canvas)
        };
        let opts = RenderOpts {
            canvas: self.canvas,
            width: w.max(16),
            height: h.max(16),
            ss,
            pad: PANEL_PAD,
            hue_shift: self.hue,
            gain: 1.0,
        };
        let label = if full { "full" } else { "frame" };
        let stem = format!(
            "out/{label}_b{}_d{}_r{}_n{}_{}_z{:.2}_h{:.3}",
            self.params.base,
            self.params.dir,
            self.params.rule.name(),
            self.params.steps,
            self.canvas.name(),
            if full { 1.0 } else { self.zoom },
            self.hue,
        );
        let name = available_png_path(&stem);
        let outcome = if let Some(curve) = &self.curve {
            let visible = if full { None } else { Some(self.revealed) };
            let (iw, ih, rgb) = render::render_curve_window(curve, w, h, ss, &win, &opts, visible);
            png::write_rgb(&name, iw, ih, &rgb)
        } else {
            let dopts = DeepOpts {
                canvas: self.canvas,
                steps: if full {
                    self.params.steps
                } else {
                    self.steps_now()
                },
                color_total: self.params.steps,
                hue_shift: self.hue,
                ink: 1.0,
                budget_nodes: 12_000_000,
                cut_px: 1.5,
            };
            let (iw, ih, rgb, _) = deep::render_image(&self.params, w, h, ss, &win, &dopts);
            png::write_rgb(&name, iw, ih, &rgb)
        };
        match outcome {
            Ok(()) => {
                println!("exported {name} ({w}x{h})");
                self.note(if self.language == Language::Zh {
                    format!("已保存 {name} ({w} x {h})")
                } else {
                    format!("Saved {name} ({w} x {h})")
                });
            }
            Err(e) => {
                eprintln!("export failed: {e}");
                self.note(if self.language == Language::Zh {
                    format!("导出失败: {e}")
                } else {
                    format!("Export failed: {e}")
                });
            }
        }
    }

    fn draw(&mut self) {
        let theme = self.theme();
        clear_background(theme.chrome_bg);
        let (sw, sh) = (screen_width(), screen_height());
        let canvas = self.canvas_rect(sw, sh);
        let bg = self.canvas.background();
        ui::rounded_rect(
            Rect::new(canvas.x + 3.0, canvas.y + 4.0, canvas.w, canvas.h),
            14.0,
            Color::new(0.0, 0.0, 0.0, if theme.is_light() { 0.14 } else { 0.30 }),
        );
        ui::rounded_panel(
            canvas,
            14.0,
            Color::new(bg[0], bg[1], bg[2], 1.0),
            theme.panel_border,
        );
        if !self.is_deep() {
            let grid = if theme.is_light() {
                Color::new(0.12, 0.18, 0.25, 0.035)
            } else {
                Color::new(0.65, 0.72, 0.82, 0.027)
            };
            let mut x = canvas.x + 80.0;
            while x < canvas.x + canvas.w - 12.0 {
                draw_line(x, canvas.y + 12.0, x, canvas.y + canvas.h - 12.0, 1.0, grid);
                x += 80.0;
            }
            let mut y = canvas.y + 80.0;
            while y < canvas.y + canvas.h - 12.0 {
                draw_line(canvas.x + 12.0, y, canvas.x + canvas.w - 12.0, y, 1.0, grid);
                y += 80.0;
            }
        }

        if self.is_deep() {
            self.draw_deep(canvas);
        } else {
            self.draw_enum(canvas);
        }
        ui::rounded_outline(canvas, 14.0, theme.panel_border);
        if self.pane_open {
            self.draw_pane(sw, sh);
        } else {
            // Collapsed: a thin tab so the pane can be brought back with the mouse as well.
            let tab = Rect::new(MARGIN, MARGIN, 26.0, 46.0);
            ui::button(tab, ">", false, &theme);
        }
        self.draw_overview(sw, sh);
        if self.help {
            self.draw_help(sw, sh);
        }
    }

    fn draw_overview(&mut self, sw: f32, sh: f32) {
        let theme = self.theme();
        let card = self.overview_rect(sw, sh);
        let image_rect = self.overview_image_rect(sw, sh);
        ui::rounded_panel(card, 11.0, theme.panel_bg, theme.panel_border);
        draw_text(
            tr("OVERVIEW  /  drag to locate", "全图导航  /  拖动定位"),
            card.x + 9.0,
            card.y + 19.0,
            14.0,
            theme.text_hint,
        );
        let bg = self.canvas.background();
        ui::rounded_panel(
            image_rect,
            6.0,
            Color::new(bg[0], bg[1], bg[2], 1.0),
            theme.panel_border,
        );
        let full = self.overview_window(image_rect);
        let view = full.view(image_rect.w as u32, image_rect.h as u32);
        if let Some(curve) = &self.curve {
            let total = curve.segments();
            let stride = render::auto_stride(total.max(1), 4000);
            let mut i = 0;
            while i < total {
                let j = (i + stride).min(total);
                let a = view.map(curve.pts[i]);
                let b = view.map(curve.pts[j]);
                let rgb = self.canvas.ramp(i as f32 / total.max(1) as f32, self.hue);
                draw_line(
                    image_rect.x + a[0] as f32,
                    image_rect.y + a[1] as f32,
                    image_rect.x + b[0] as f32,
                    image_rect.y + b[1] as f32,
                    1.3,
                    Color::new(rgb[0], rgb[1], rgb[2], 0.92),
                );
                i = j;
            }
        } else {
            let w = image_rect.w.max(1.0) as u16;
            let h = image_rect.h.max(1.0) as u16;
            if self
                .overview_bake
                .as_ref()
                .is_none_or(|b| b.w != w || b.h != h)
            {
                let opts = DeepOpts {
                    canvas: self.canvas,
                    steps: self.params.steps,
                    color_total: self.params.steps,
                    hue_shift: self.hue,
                    ink: 1.0,
                    budget_nodes: 500_000,
                    cut_px: 2.0,
                };
                let (_, _, rgb, _) =
                    deep::render_image(&self.params, w as u32, h as u32, 1, &full, &opts);
                let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
                for pixel in rgb.chunks_exact(3) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
                let tex = Texture2D::from_image(&Image {
                    bytes: rgba,
                    width: w,
                    height: h,
                });
                self.overview_bake = Some(Bake { tex, w, h });
            }
            if let Some(bake) = &self.overview_bake {
                draw_texture_ex(
                    &bake.tex,
                    image_rect.x,
                    image_rect.y,
                    WHITE,
                    DrawTextureParams {
                        dest_size: Some(vec2(image_rect.w, image_rect.h)),
                        ..Default::default()
                    },
                );
            }
        }
        // Map the current world-space camera into the fixed full-curve overview.
        let canvas = self.canvas_rect(sw, sh);
        let camera = self.window(canvas);
        if let Some(region) = overview_viewport_rect(image_rect, camera, full) {
            let fill = Color::new(
                theme.field_border_focus.r,
                theme.field_border_focus.g,
                theme.field_border_focus.b,
                0.08,
            );
            draw_rectangle(region.x, region.y, region.w, region.h, fill);
            draw_rectangle_lines(
                region.x,
                region.y,
                region.w,
                region.h,
                2.0,
                theme.field_border_focus,
            );
        }
        let center = view.map(camera.center);
        let dot = vec2(
            (image_rect.x + center[0] as f32).clamp(image_rect.x, image_rect.x + image_rect.w),
            (image_rect.y + center[1] as f32).clamp(image_rect.y, image_rect.y + image_rect.h),
        );
        draw_circle(dot.x, dot.y, 3.0, theme.marker_dot);
        draw_line(
            image_rect.x + 6.0,
            image_rect.y,
            image_rect.x + image_rect.w - 6.0,
            image_rect.y,
            1.0,
            theme.panel_border,
        );
    }

    fn draw_enum(&self, canvas: Rect) {
        let Some(curve) = &self.curve else { return };
        let bbox = if self.follow {
            self.prefix
        } else {
            self.framing
        };
        let view = self.view_for(canvas, &bbox);

        if let Some(ghost) = &self.ghost {
            let total = ghost.curve.segments().max(1);
            let revealed = ghost.revealed.min(ghost.curve.segments());
            let stride = render::auto_stride(revealed.max(1), GHOST_STROKES);
            let ghost_alpha = if self.theme().is_light() { 0.45 } else { 0.30 };
            let mut i = 0;
            while i < revealed {
                let j = (i + stride).min(revealed);
                let c = self.canvas.ramp(i as f32 / total as f32, self.hue);
                let a = view.map(ghost.curve.pts[i]);
                let b = view.map(ghost.curve.pts[j]);
                draw_line(
                    a[0] as f32,
                    a[1] as f32,
                    b[0] as f32,
                    b[1] as f32,
                    LINE_WIDTH,
                    Color::new(c[0], c[1], c[2], ghost_alpha * ghost.alpha),
                );
                i = j;
            }
        }

        let total = curve.segments().max(1);
        let stride = render::auto_stride(self.revealed.max(1), MAX_STROKES);
        let mut i = 0;
        while i < self.revealed && i < total {
            let j = (i + stride).min(self.revealed).min(total);
            let c = self.canvas.ramp(i as f32 / total as f32, self.hue);
            let a = view.map(curve.pts[i]);
            let b = view.map(curve.pts[j]);
            draw_line(
                a[0] as f32,
                a[1] as f32,
                b[0] as f32,
                b[1] as f32,
                LINE_WIDTH,
                Color::new(c[0], c[1], c[2], 1.0),
            );
            i = j;
        }

        if self.revealed > 0 {
            let idx = self.revealed.min(curve.segments());
            let p = view.map(curve.pts[idx]);
            let heading = curve.heading_at(idx.saturating_sub(1));
            let (dx, dy) = (heading.cos() as f32, -(heading.sin() as f32));
            let (px, py) = (-dy, dx);
            let (cx, cy) = (p[0] as f32, p[1] as f32);
            let theme = self.theme();
            draw_triangle(
                vec2(cx + dx * 9.0, cy + dy * 9.0),
                vec2(cx + px * 5.5 - dx * 3.0, cy + py * 5.5 - dy * 3.0),
                vec2(cx - px * 5.5 - dx * 3.0, cy - py * 5.5 - dy * 3.0),
                theme.marker,
            );
            draw_circle(cx, cy, 2.6, theme.marker_dot);
        }
    }

    fn draw_deep(&mut self, canvas: Rect) {
        // Bake at 60% of the canvas area: traversal cost scales with pixels, and the blit hides
        // the softness while exploring. P/S export at full resolution.
        let (w, h) = (
            ((canvas.w.max(16.0) * 0.6) as u16).max(32),
            ((canvas.h.max(16.0) * 0.6) as u16).max(32),
        );
        if self.bake_stale || self.bake.as_ref().is_none_or(|b| b.w != w || b.h != h) {
            let win = self.window(canvas);
            let mut raster = render::Raster::new(w as u32, h as u32, self.canvas);
            let dopts = DeepOpts {
                canvas: self.canvas,
                steps: self.steps_now(),
                color_total: self.params.steps,
                hue_shift: self.hue,
                ink: 1.0,
                budget_nodes: 3_000_000,
                cut_px: 2.2,
            };
            let stats = deep::render_into(&mut raster, &self.params, &win, &dopts);
            self.deep_stats = Some(stats);
            let (_, _, rgb) = raster.downsample(1);
            let mut rgba = Vec::with_capacity(rgb.len() / 3 * 4);
            for px in rgb.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            let image = Image {
                bytes: rgba,
                width: w,
                height: h,
            };
            match self.bake.as_mut() {
                Some(bake) => bake.tex.update(&image),
                None => {
                    self.bake = Some(Bake {
                        tex: Texture2D::from_image(&image),
                        w,
                        h,
                    })
                }
            }
            self.bake_stale = false;
        }
        if let Some(bake) = &self.bake {
            draw_texture_ex(
                &bake.tex,
                canvas.x,
                canvas.y,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(canvas.w, canvas.h)),
                    ..Default::default()
                },
            );
        }
        if let Some(stats) = &self.deep_stats {
            if let Some(frontier) = stats.frontier {
                let p = self
                    .window(canvas)
                    .view(canvas.w as u32, canvas.h as u32)
                    .map(frontier);
                draw_circle(
                    canvas.x + p[0] as f32,
                    canvas.y + p[1] as f32,
                    3.0,
                    self.theme().marker_dot,
                );
            }
        }
    }

    fn draw_pane(&self, sw: f32, sh: f32) {
        let theme = self.theme();
        let pane = self.pane_rect(sw, sh);
        draw_rectangle(pane.x, pane.y, pane.w, pane.h, theme.panel_bg);
        draw_rectangle(pane.x, pane.y, 4.0, pane.h, theme.field_border_focus);
        draw_line(pane.w - 1.0, 0.0, pane.w - 1.0, sh, 1.0, theme.panel_border);
        draw_text(
            tr("EXPLORER", "探索工作台"),
            16.0,
            30.0,
            if pane.w < 330.0 { 20.0 } else { 25.0 },
            theme.text_bright,
        );
        draw_text(
            tr("Explore / render / export", "探索 / 渲染 / 导出"),
            16.0,
            52.0,
            16.0,
            theme.text_hint,
        );
        ui::button(
            Rect::new(pane.w - 104.0, 12.0, 52.0, 32.0),
            tr("LAN", "语言"),
            false,
            &theme,
        );
        ui::icon_button(vec2(pane.w - 26.0, 28.0), 15.0, "?", &theme);
        draw_line(
            16.0,
            HEADER_H,
            pane.w - 16.0,
            HEADER_H,
            1.0,
            theme.panel_border,
        );
        draw_text(
            tr("01  PARAMETERS", "01  参数设置"),
            16.0,
            HEADER_H + 14.0,
            16.0,
            theme.text_hint,
        );
        for index in 0..self.fields.len() {
            let rect = self.field_rect(index, pane);
            if index == F_RULE {
                let selected = RULE_CHOICES
                    .iter()
                    .position(|rule| rule.name() == self.fields[F_RULE].text)
                    .unwrap_or(0);
                ui::rule_selector(
                    rect,
                    RULE_CHOICES.map(DigitRule::name),
                    selected,
                    self.focus == Some(F_RULE),
                    &theme,
                );
            } else {
                let mut field = self.fields[index].clone();
                field.draw(rect, &theme, get_time());
            }
        }
        let labels = [
            [tr("Apply", "应用"), tr("Fit view", "适应画面")],
            [
                if self.paused {
                    tr("Play", "播放")
                } else {
                    tr("Pause", "暂停")
                },
                tr("Replay", "重播"),
            ],
            [
                tr("Save frame PNG", "保存当前 PNG"),
                tr("Save full PNG", "保存完整 PNG"),
            ],
            [
                if self.canvas.is_light() {
                    tr("Dark mode", "深色模式")
                } else {
                    tr("Light mode", "浅色模式")
                },
                tr("Hide panel", "隐藏面板"),
            ],
        ];
        for (row, pair) in labels.iter().enumerate() {
            for (index, label) in pair.iter().enumerate() {
                let rect = self.button_rect(row, index, pane);
                if rect.y + rect.h < sh - 8.0 {
                    ui::button(rect, label, row == 0 && index == 0, &theme);
                }
            }
        }
        let track = self.progress_rect(pane);
        if track.y + 110.0 < sh - 66.0 {
            draw_text(
                tr("02  PLAYBACK", "02  播放进度"),
                16.0,
                track.y - 14.0,
                16.0,
                theme.text_hint,
            );
            let fraction = if let Some(curve) = &self.curve {
                self.revealed as f32 / curve.segments().max(1) as f32
            } else {
                (self.deep_level / self.levels().max(1) as f64) as f32
            }
            .clamp(0.0, 1.0);
            ui::scrubber(track, fraction, self.progress_dragging, &theme);
            draw_text(
                if self.language == Language::Zh {
                    format!("{:.1}%  /  {} 步", fraction * 100.0, self.steps_now())
                } else {
                    format!("{:.1}%  /  {} steps", fraction * 100.0, self.steps_now())
                },
                16.0,
                track.y + 34.0,
                18.0,
                theme.text_bright,
            );
        }
        let status_y = track.y + 62.0;
        let zoom = self.zoom;
        let mut lines = vec![
            if self.language == Language::Zh {
                format!(
                    "{} / 缩放 {:.2}x",
                    if self.is_deep() { "深度" } else { "实时" },
                    zoom
                )
            } else {
                format!(
                    "{} / zoom {:.2}x",
                    if self.is_deep() { "DEEP" } else { "LIVE" },
                    zoom
                )
            },
            if self.language == Language::Zh {
                format!(
                    "转角 {:.2}° / 速度 {:.2}",
                    self.params.turn_per_digit_deg(),
                    self.speed
                )
            } else {
                format!(
                    "Turn {:.2} deg / speed {:.2}",
                    self.params.turn_per_digit_deg(),
                    self.speed
                )
            },
        ];
        if let Some(stats) = &self.deep_stats {
            if self.is_deep() && stats.truncated {
                lines
                    .push(tr("WARNING: render budget reached", "警告: 渲染预算已耗尽").to_string());
            }
        }
        for (i, line) in lines.iter().enumerate() {
            let y = status_y + 20.0 * i as f32;
            if y < sh - 65.0 {
                draw_text(
                    fit_text(line, pane.w - 32.0, 17),
                    16.0,
                    y,
                    17.0,
                    theme.text_dim,
                );
            }
        }
        // Pin feedback to the bottom instead of hiding it behind long diagnostics.
        if let Some((text, _)) = &self.note {
            if sh > self.button_rect(3, 0, pane).y + BUTTON_H + 100.0 {
                draw_line(
                    16.0,
                    sh - 66.0,
                    pane.w - 16.0,
                    sh - 66.0,
                    1.0,
                    theme.panel_border,
                );
                draw_text(
                    fit_text(text, pane.w - 32.0, 17),
                    16.0,
                    sh - 45.0,
                    17.0,
                    theme.text_warn,
                );
            }
        }
        if sh > 450.0 {
            draw_text(
                fit_text(
                    tr(
                        "Wheel zoom / Drag pan / H help",
                        "滚轮缩放 / 拖动平移 / H 帮助",
                    ),
                    pane.w - 32.0,
                    16,
                ),
                16.0,
                sh - 16.0,
                16.0,
                theme.text_hint,
            );
        }
    }

    fn draw_help(&self, sw: f32, sh: f32) {
        let theme = self.theme();
        draw_rectangle(0.0, 0.0, sw, sh, theme.overlay);
        let scale = (sw / 1040.0).min(sh / 780.0).min(1.0);
        let x = 36.0 * scale;
        let mut y = 48.0 * scale;
        let line_h = 28.0 * scale;
        let draw = |text: &str, y: f32, size: u16, color: Color| {
            let size = ((size as f32 * scale).max(9.0)) as u16;
            draw_text(fit_text(text, sw - 2.0 * x, size), x, y, size as f32, color);
        };
        draw(
            tr("SHORTCUTS / Esc to close", "快捷键 / Esc 关闭"),
            y,
            29,
            theme.text_bright,
        );
        y += 43.0 * scale;
        for (en, zh) in [
            (
                "[ ] Base -/+ 1    , . Direction -/+ 1    - = Steps /2 / x2",
                "[ ] 进制 -/+1    , . 方向 -/+1    - = 步数减半/翻倍",
            ),
            (
                "T Cycle rule    Shift + parameter keys Keep playback position",
                "T 切换规则    Shift + 参数键 保留播放进度",
            ),
            (
                "Space Play/pause    R Replay    Up/Down Speed x2 / /2",
                "空格 播放/暂停    R 重播    上/下 调整速度",
            ),
            (
                "F Fit view    G Follow turtle    X/Z Zoom in/out",
                "F 适应画面    G 跟随海龟    X/Z 缩放",
            ),
            (
                "Tab Hide/show panel    B Switch theme    L / LAN Language",
                "Tab 显示/隐藏面板    B 切换主题    L / 语言 切换语言",
            ),
            (
                "C Rotate palette    P Save frame    S Save full PNG (out/)",
                "C 切换配色    P 保存当前    S 保存完整 PNG (out/)",
            ),
        ] {
            draw(tr(en, zh), y, 20, theme.text_dim);
            y += line_h;
        }
        y += 18.0 * scale;
        draw(tr("MOUSE & INPUT", "鼠标与输入"), y, 23, theme.text_bright);
        y += 34.0 * scale;
        for (en, zh) in [
            (
                "Wheel Zoom at pointer    Drag Pan canvas",
                "滚轮 以光标为中心缩放    拖动 平移画布",
            ),
            (
                "Click or drag the overview to recenter; F fits the curve",
                "点击或拖动全图导航重新定位; F 适应完整画面",
            ),
            (
                "Drag the progress scrubber to seek live; release to pause",
                "拖动进度条实时定位; 松开后停在当前进度",
            ),
            (
                "Click field to edit; Enter applies; Tab / Shift+Tab moves focus",
                "点击输入框编辑; Enter 应用; Tab / Shift+Tab 切换字段",
            ),
            (
                "Arrows/Home/End move caret; Cmd/Ctrl+A,C,X,V select/copy/cut/paste",
                "方向键/Home/End 移动光标; Cmd/Ctrl+A,C,X,V 全选/复制/剪切/粘贴",
            ),
            (
                "Esc cancels edits; Apply validates all fields",
                "Esc 取消编辑; 应用前会校验全部字段",
            ),
        ] {
            draw(tr(en, zh), y, 20, theme.text_dim);
            y += line_h;
        }
        y += 18.0 * scale;
        draw(
            tr("PARAMETERS & OUTPUT", "参数与输出"),
            y,
            23,
            theme.text_bright,
        );
        y += 34.0 * scale;
        for (en, zh) in [
            (
                "base: 2..64    dir: +/-1..64 (negative mirrors the curve)",
                "进制: 2..64    方向: +/-1..64 (负值为镜像)",
            ),
            (
                "rule: linear / alternate / fold (fold + 4 + 6 = Koch arc)",
                "规则: linear / alternate / fold (fold + 4 + 6 = 科赫弧)",
            ),
            (
                "steps: 1..2^50; supports 2^30 and 1.5e6; deep mode above 4194304",
                "步数: 1..2^50; 支持 2^30 和 1.5e6; 超过 4194304 进入深度模式",
            ),
            (
                "hue: 0..1; PNG exports use current theme and palette",
                "色相: 0..1; PNG 导出使用当前主题和配色",
            ),
            (
                "Deep zoom limited by f64 precision (about 1e7x)",
                "深度缩放受 f64 精度限制 (约 1e7 倍)",
            ),
        ] {
            draw(tr(en, zh), y, 20, theme.text_dim);
            y += line_h;
        }
    }
}

/// Pick a free output filename without silently replacing a previous GUI export.
fn available_png_path(stem: &str) -> String {
    let original = format!("{stem}.png");
    if !std::path::Path::new(&original).exists() {
        return original;
    }
    for index in 2.. {
        let path = format!("{stem}_{index}.png");
        if !std::path::Path::new(&path).exists() {
            return path;
        }
    }
    unreachable!()
}

/// Inverse of `View::map`: screen pixels -> world coordinates (the view is canvas-local).
fn world_at(win: &Win, canvas: Rect, x: f64, y: f64) -> [f64; 2] {
    let view = win.view(canvas.w.max(16.0) as u32, canvas.h.max(16.0) as u32);
    [
        (x - canvas.x as f64 - view.ox) / view.scale,
        (view.oy - (y - canvas.y as f64)) / view.scale,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_formatting_round_trips_through_the_parser() {
        for steps in [2u64, 1024, 1 << 16, 1 << 30, 1 << 50, 200_000, 1_500_000] {
            let text = format_steps(steps);
            assert_eq!(crate::ui::parse_count(&text), Some(steps), "text = {text}");
        }
    }

    #[test]
    fn typing_into_the_panel_updates_the_parameters() {
        let mut app = App::new(Params::default());
        app.fields[F_BASE].text = "4".into();
        app.fields[F_DIR].text = "6".into();
        app.fields[F_RULE].text = "fold".into();
        app.fields[F_STEPS].text = "2^18".into();
        app.fields[F_HUE].text = "0.25".into();
        app.apply_fields();
        assert_eq!(app.params.base, 4);
        assert_eq!(app.params.dir, 6);
        assert_eq!(app.params.rule, DigitRule::Fold);
        assert_eq!(app.params.steps, 1 << 18);
        assert!((app.hue - 0.25).abs() < 1e-6);
        assert!(app.curve.is_some(), "2^18 steps stays in enum mode");
        assert_eq!(app.fields[F_STEPS].text, "2^18", "the notation is kept");

        app.set_focus(Some(F_BASE));
        assert!(app.fields[F_BASE].focused);
        app.fields[F_BASE].text = "999".into();
        app.fields[F_DIR].text = "-9223372036854775808".into();
        app.fields[F_STEPS].text = "2^80".into();
        app.fields[F_HUE].text = "NaN".into();
        let before = app.params;
        app.apply_fields();
        assert_eq!(app.params, before, "invalid fields cannot partially apply");
        assert_eq!(app.hue, 0.25);
        assert_eq!(
            app.fields[F_BASE].text, "999",
            "keep rejected text for correction"
        );
        assert!(app.fields[F_BASE].error > 0.0);
        assert!(app.fields[F_DIR].error > 0.0);
        assert!(app.fields[F_STEPS].error > 0.0);
        assert!(app.fields[F_HUE].error > 0.0);
        app.cancel_edit();
        assert!(!app.fields[F_BASE].focused);
        assert_eq!(app.fields[F_BASE].text, "4");
        app.fields[F_STEPS].text = "2^30".into();
        app.apply_fields();
        assert_eq!(app.params.steps, 1 << 30);
        assert!(app.curve.is_none(), "large valid input uses deep mode");
        app.fields[F_RULE].text = "quadratic".into();
        let before = app.params;
        app.apply_fields();
        assert_eq!(app.params, before);
        assert!(app.fields[F_RULE].error > 0.0);
    }

    #[test]
    fn exported_png_names_do_not_overwrite() {
        let stem = std::env::temp_dir().join(format!("fractal-export-{}", std::process::id()));
        let stem = stem.to_string_lossy();
        let first = available_png_path(&stem);
        std::fs::write(&first, []).unwrap();
        assert_eq!(available_png_path(&stem), format!("{stem}_2.png"));
        std::fs::remove_file(first).unwrap();
    }

    #[test]
    fn wheel_keeps_world_anchor_and_clamps_at_fit() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let canvas = Rect::new(344.0, 8.0, 1148.0, 884.0);
        let cursor = vec2(1100.0, 312.0);
        let anchor = world_at(
            &app.window(canvas),
            canvas,
            cursor.x as f64,
            cursor.y as f64,
        );
        for _ in 0..8 {
            app.zoom_at(canvas, cursor, 1.18);
        }
        let after = world_at(
            &app.window(canvas),
            canvas,
            cursor.x as f64,
            cursor.y as f64,
        );
        assert!((anchor[0] - after[0]).abs() < 1e-9);
        assert!((anchor[1] - after[1]).abs() < 1e-9);
        assert!((app.zoom - 1.18_f64.powi(8)).abs() < 1e-9);
        app.zoom_at(canvas, cursor, 1e-9);
        assert!((app.zoom - 1.0).abs() < 1e-9);
    }

    #[test]
    fn dragging_moves_image_by_same_number_of_pixels() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let canvas = Rect::new(344.0, 8.0, 1148.0, 884.0);
        let win = app.window(canvas);
        let point = win.center;
        let before = win.view(canvas.w as u32, canvas.h as u32).map(point);
        app.pan_pixels(canvas, vec2(86.0, -34.0));
        let after = app
            .window(canvas)
            .view(canvas.w as u32, canvas.h as u32)
            .map(point);
        assert!((after[0] - before[0] - 86.0).abs() < 1e-8);
        assert!((after[1] - before[1] + 34.0).abs() < 1e-8);
    }

    #[test]
    fn overview_maps_click_to_world_and_survives_small_windows() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        for (sw, sh) in [(1500.0, 900.0), (800.0, 720.0)] {
            let card = app.overview_rect(sw, sh);
            let image = app.overview_image_rect(sw, sh);
            assert!(image.x >= card.x && image.x + image.w <= card.x + card.w);
            let point = vec2(image.x + image.w * 0.7, image.y + image.h * 0.3);
            let expected = world_at(
                &app.overview_window(image),
                image,
                point.x as f64,
                point.y as f64,
            );
            app.recenter_from_overview(sw, sh, point);
            let center = app.center.unwrap();
            assert!((center[0] - expected[0]).abs() < 1e-9);
            assert!((center[1] - expected[1]).abs() < 1e-9);
        }
    }

    #[test]
    fn overview_marks_zoomed_camera_and_clips_outside() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let canvas = app.canvas_rect(1500.0, 900.0);
        let image = app.overview_image_rect(1500.0, 900.0);
        let full = app.overview_window(image);
        let initial = overview_viewport_rect(image, app.window(canvas), full).unwrap();
        app.zoom_at(
            canvas,
            vec2(canvas.x + canvas.w / 2.0, canvas.y + canvas.h / 2.0),
            4.0,
        );
        let zoomed = overview_viewport_rect(image, app.window(canvas), full).unwrap();
        assert!(zoomed.w < initial.w && zoomed.h < initial.h);
        app.center = Some([1e12, 1e12]);
        assert!(overview_viewport_rect(image, app.window(canvas), full).is_none());
    }

    #[test]
    fn rule_choice_stays_draft_until_apply_and_input_does_not_leak_focus() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        app.select_rule(DigitRule::Alternate);
        assert_eq!(app.fields[F_RULE].text, "alternate");
        assert_eq!(app.params.rule, DigitRule::Fold);
        app.apply_fields();
        assert_eq!(app.params.rule, DigitRule::Alternate);
        assert!(!accepts_queued_chars(None, None));
        assert!(!accepts_queued_chars(None, Some(F_BASE)));
        assert!(!accepts_queued_chars(Some(F_BASE), Some(F_STEPS)));
        assert!(!accepts_queued_chars(Some(F_RULE), Some(F_RULE)));
        assert!(accepts_queued_chars(Some(F_BASE), Some(F_BASE)));
    }

    #[test]
    fn cached_prefix_matches_full_scan_at_block_edges() {
        let app = App::new(Params::new(4, 6, 12_345).with_rule(DigitRule::Fold));
        let curve = app.curve.as_ref().unwrap();
        for index in [
            0,
            1,
            PREFIX_BLOCK - 1,
            PREFIX_BLOCK,
            PREFIX_BLOCK + 1,
            8192,
            12_345,
        ] {
            assert_eq!(
                cached_prefix_bbox(curve, &app.prefix_checkpoints, index),
                prefix_bbox(curve, index)
            );
        }
    }

    #[test]
    fn scrubber_clamps_beyond_track_and_pauses_on_each_move() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let track = Rect::new(16.0, 200.0, 312.0, 12.0);
        app.paused = false;
        app.seek_at(track, 16.0 + 312.0 * 0.375);
        assert_eq!(app.revealed, 1536);
        assert!(app.paused);
        app.seek_at(track, -400.0);
        assert_eq!(app.revealed, 1);
        app.seek_at(track, 999.0);
        assert_eq!(app.revealed, 4096);
    }

    #[test]
    fn deep_scrubbing_interpolates_between_reveal_levels() {
        let mut app = App::new(Params::new(2, 4, 1 << 24));
        app.deep_level = 10.5;
        assert_eq!(app.steps_now(), 1536);
        app.seek(0.0);
        assert_eq!(app.steps_now(), 1);
        app.seek(1.0);
        assert_eq!(app.steps_now(), 1 << 24);
    }

    #[test]
    fn playback_seek_is_reversible() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        app.seek(0.5);
        assert!(app.paused);
        assert_eq!(app.revealed, 2048);
        app.replay();
        assert_eq!(app.revealed, 1);
        assert!(!app.paused);
    }

    #[test]
    fn rebuilding_keeps_the_rule_and_resets_the_reveal() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        app.reveal_all();
        app.rebuild(false);
        assert_eq!(
            app.revealed, 1,
            "a plain rebuild replays from the first segment"
        );
        assert!(
            !app.paused,
            "a new curve must animate even from the initial paused preview"
        );
        assert_eq!(app.params.rule, DigitRule::Fold);
        app.reveal_all();
        app.rebuild(true);
        assert!(app.revealed > 1, "a hot swap keeps the progress");
    }

    #[test]
    fn deep_hot_swap_preserves_progress_fraction() {
        let mut app = App::new(Params::new(2, 4, 1 << 30));
        app.deep_level = 15.0;
        app.params = Params::new(4, 6, 1 << 30).with_rule(DigitRule::Fold);
        app.rebuild(true);
        assert!((app.deep_level / app.levels() as f64 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn language_toggle_preserves_fractal_and_playback() {
        ui::set_language(Language::En);
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let params = app.params;
        let revealed = app.revealed;
        let paused = app.paused;
        app.toggle_language();
        assert_eq!(app.language, Language::Zh);
        assert_eq!(ui::language(), Language::Zh);
        assert_eq!(tr("LAN", "语言"), "语言");
        assert_eq!(
            (app.params, app.revealed, app.paused),
            (params, revealed, paused)
        );
        app.toggle_language();
        assert_eq!(app.language, Language::En);
        assert_eq!(ui::language(), Language::En);
    }

    #[test]
    fn theme_toggle_switches_canvas_and_ui_together() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let params = app.params;
        assert_eq!(app.canvas, Canvas::Dark);
        assert_eq!(app.theme(), Theme::dark());
        app.toggle_canvas();
        assert_eq!(app.canvas, Canvas::Light);
        assert_eq!(
            app.theme(),
            Theme::light(),
            "the pane must follow the canvas"
        );
        assert_eq!(
            app.params, params,
            "switching the theme must not touch the parameters"
        );
        app.toggle_canvas();
        assert_eq!(app.canvas, Canvas::Dark);
        assert_eq!(app.theme(), Theme::dark());
    }

    #[test]
    fn pane_and_canvas_do_not_overlap() {
        let app = App::new(Params::default());
        let (sw, sh) = (1500.0, 900.0);
        let pane = app.pane_rect(sw, sh);
        let canvas = app.canvas_rect(sw, sh);
        assert!(pane.w > 0.0);
        assert!(
            canvas.x >= pane.x + pane.w,
            "canvas must start right of the pane"
        );
        assert!(canvas.x + canvas.w <= sw && canvas.y + canvas.h <= sh);
        let collapsed = App {
            pane_open: false,
            ..App::new(Params::default())
        };
        let full = collapsed.canvas_rect(sw, sh);
        assert!(
            full.w > canvas.w,
            "collapsing the pane gives the canvas more room"
        );
    }

    impl App {
        fn reveal_all(&mut self) {
            if let Some(curve) = &self.curve {
                self.revealed = curve.segments();
                self.prefix = curve.bbox;
            } else {
                self.deep_level = self.levels() as f64;
            }
        }
    }
}
