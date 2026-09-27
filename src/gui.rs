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

use crate::ui::{self, TextInput, Theme, FIELD_H};
use fun_fractal_base_direction::deep::{self, DeepOpts};
use fun_fractal_base_direction::fractal::{
    self, step_dir, Bbox, Curve, DigitRule, Params, MAX_BASE, MAX_DIR_ABS, MAX_ENUM_STEPS, MAX_STEPS,
    MIN_BASE,
};
use fun_fractal_base_direction::palette::Canvas;
use fun_fractal_base_direction::png;
use fun_fractal_base_direction::render::{self, RenderOpts, Window as Win};

const PANE_W: f32 = 344.0;
const MARGIN: f32 = 8.0;
const ROW_GAP: f32 = 6.0;
const BUTTON_H: f32 = 30.0;
const MAX_STROKES: usize = 100_000;
const GHOST_STROKES: usize = 40_000;
const GHOST_FADE_FRAMES: f32 = 60.0;
const LINE_WIDTH: f32 = 1.6;
const ZOOM_STEP: f64 = 1.25;
const PANEL_PAD: f64 = 0.06;
const MAX_ZOOM: f64 = 1e12;

const F_BASE: usize = 0;
const F_DIR: usize = 1;
const F_RULE: usize = 2;
const F_STEPS: usize = 3;
const F_HUE: usize = 4;

/// `shot`: optional path; the whole window (pane included) is written there when the loop ends.
/// Handy to eye-check the theme, and it turns MSAA off because readback is unreliable with it.
pub fn run(params: Params, canvas: Canvas, frames_limit: Option<u32>, shot: Option<String>) {
    let title = format!(
        "fractal  base={} dir={} rule={} steps={}",
        params.base,
        params.dir,
        params.rule.name(),
        params.steps
    );
    let conf = macroquad::conf::Conf {
        // NOTE: do *not* raise draw_call_vertex_capacity here. With a custom capacity macroquad
        // silently loses geometry once a frame needs more than one batch, which showed up as the
        // curve being drawn in a few leftover fragments ("truncated") at high step counts.
        // The defaults (10k vertices / 5k indices) flush correctly; more draw calls cost nothing.
        miniquad_conf: miniquad::conf::Conf {
            window_title: title,
            window_width: 1500,
            window_height: 900,
            high_dpi: true,
            sample_count: if shot.is_some() { 1 } else { 4 },
            window_resizable: true,
            ..Default::default()
        },
        ..Default::default()
    };
    Window::from_config(conf, amain(params, canvas, frames_limit, shot));
}

async fn amain(params: Params, canvas: Canvas, frames_limit: Option<u32>, shot: Option<String>) {
    let mut app = App::new(params);
    app.canvas = canvas;
    let mut frames = 0u32;
    loop {
        if is_key_pressed(KeyCode::Escape) && !app.help {
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
    prefix: Bbox,
    ghost: Option<Ghost>,
    bake: Option<Bake>,
    bake_stale: bool,
    deep_stats: Option<deep::Stats>,
    fields: [TextInput; 5],
    focus: Option<usize>,
    pan_armed: bool,
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

fn format_steps(steps: u64) -> String {
    if steps.is_power_of_two() && steps >= 1024 {
        format!("2^{}", steps.trailing_zeros())
    } else {
        format!("{steps}")
    }
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
            .map(|c| prefix_bbox(c, 1))
            .unwrap_or_else(|| Bbox::point(0.0, 0.0));
        App {
            params,
            curve,
            framing,
            revealed: 1,
            deep_level: 0.0,
            speed: default_speed,
            speed_user_set: false,
            paused: false,
            hue: 0.0,
            canvas: Canvas::Dark,
            zoom: 1.0,
            center: None,
            follow: false,
            help: false,
            pane_open: true,
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
            pan_armed: false,
            note: Some(("Koch arc default | Tab hides this pane | B switches theme".to_string(), 260)),
        }
    }

    // ---- layout -----------------------------------------------------------

    fn pane_rect(&self, sw: f32, sh: f32) -> Rect {
        if self.pane_open {
            Rect::new(0.0, 0.0, PANE_W.min(sw * 0.5), sh)
        } else {
            Rect::new(0.0, 0.0, 0.0, 0.0)
        }
    }

    /// The picture area: everything right of the pane, with a small margin.
    fn canvas_rect(&self, sw: f32, sh: f32) -> Rect {
        let pane = self.pane_rect(sw, sh);
        let x = (pane.w + MARGIN).min(sw - 16.0);
        Rect::new(x, MARGIN, (sw - x - MARGIN).max(16.0), (sh - 2.0 * MARGIN).max(16.0))
    }

    fn field_rect(&self, index: usize, pane: Rect) -> Rect {
        let y = pane.y + 52.0 + (FIELD_H + ROW_GAP) * index as f32;
        Rect::new(pane.x + 10.0, y, pane.w - 20.0, FIELD_H)
    }

    fn button_rect(&self, row: usize, index: usize, pane: Rect) -> Rect {
        let y = pane.y + 52.0
            + (FIELD_H + ROW_GAP) * self.fields.len() as f32
            + 8.0
            + (BUTTON_H + 6.0) * row as f32;
        let w = (pane.w - 20.0 - 8.0) / 2.0;
        let x = pane.x + 10.0 + (w + 8.0) * index as f32;
        Rect::new(x, y, w, BUTTON_H)
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
                let level = self.deep_level.round().clamp(0.0, self.levels() as f64) as u32;
                fractal::level_steps(self.params.base, level).min(self.params.steps).max(1)
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
        let framed = Win::from_bbox(
            &self.framing,
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

    fn apply_fields(&mut self) {
        let mut rejected: Vec<&'static str> = Vec::new();
        match ui::parse_count(&self.fields[F_BASE].text) {
            Some(v) => self.params.base = v.clamp(MIN_BASE as u64, MAX_BASE as u64) as u32,
            None => rejected.push("base"),
        }
        match ui::parse_int(&self.fields[F_DIR].text) {
            Some(v) => self.params.dir = v.clamp(-(MAX_DIR_ABS as i64), MAX_DIR_ABS as i64) as i32,
            None => rejected.push("dir"),
        }
        match DigitRule::parse(self.fields[F_RULE].text.trim()) {
            Some(r) => self.params.rule = r,
            None => rejected.push("rule"),
        }
        match ui::parse_count(&self.fields[F_STEPS].text) {
            Some(v) => self.params.steps = v.clamp(1, MAX_STEPS),
            None => rejected.push("steps"),
        }
        match ui::parse_float(&self.fields[F_HUE].text) {
            Some(v) => self.hue = v.rem_euclid(1.0) as f32,
            None => rejected.push("hue"),
        }
        if !rejected.is_empty() {
            for field in self.fields.iter_mut() {
                if rejected.contains(&field.label) {
                    field.mark_error();
                }
            }
            self.note(format!("rejected: {}  (values unchanged)", rejected.join(", ")));
            self.sync_fields();
            return;
        }
        self.params = self.params.clamped();
        self.focus = None;
        self.sync_fields();
        self.rebuild(false);
        self.note(format!(
            "base {} dir {} rule {} steps {} ({:.2} deg/unit) replaying",
            self.params.base,
            self.params.dir,
            self.params.rule.name(),
            self.params.steps,
            self.params.turn_per_digit_deg()
        ));
    }

    fn rebuild(&mut self, keep_progress: bool) {
        let old_curve = self.curve.take();
        let old_fraction = match &old_curve {
            Some(c) => self.revealed as f64 / c.segments().max(1) as f64,
            None => self.deep_level / self.levels().max(1) as f64,
        };
        let (curve, framing) = materialise(&self.params);
        // Decide from the *new* curve: `self.curve` is already taken above.
        let deep = curve.is_none();
        if deep {
            self.deep_level = if keep_progress { old_fraction * self.levels() as f64 } else { 0.0 };
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
                (false, Some(previous)) => {
                    Some(Ghost { curve: previous, revealed: self.revealed.max(1), alpha: 1.0 })
                }
                _ => None,
            };
            self.prefix = curve
                .as_ref()
                .map(|c| prefix_bbox(c, self.revealed))
                .unwrap_or_else(|| Bbox::point(0.0, 0.0));
        }
        self.curve = curve;
        self.framing = framing;
        if !self.speed_user_set {
            self.speed = if deep {
                (self.levels() as f64 / 240.0).max(1.0 / 240.0)
            } else {
                (self.params.steps.min(MAX_ENUM_STEPS) as f64 / 240.0).max(1.0)
            };
        }
        self.bake_stale = true;
    }

    fn toggle_canvas(&mut self) {
        self.canvas = if self.canvas.is_light() { Canvas::Dark } else { Canvas::Light };
        self.bake_stale = true;
        self.note(format!("theme: {} (canvas + UI)", self.canvas.name()));
    }

    fn set_hue(&mut self, hue: f32) {
        self.hue = hue.rem_euclid(1.0);
        self.fields[F_HUE].set(format!("{:.3}", self.hue));
        self.bake_stale = true;
    }

    fn zoom_by(&mut self, factor: f64) {
        let canvas = self.canvas_rect(screen_width(), screen_height());
        let win = self.window(canvas);
        let mut next = win;
        next.zoom_around(factor, win.center);
        self.zoom = next.zoom_of(&self.framing);
        self.center = Some(next.center);
        self.bake_stale = true;
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
        for field in self.fields.iter_mut() {
            field.tick(dt);
        }

        let (sw, sh) = (screen_width(), screen_height());
        let theme = self.theme();
        let pane = self.pane_rect(sw, sh);
        let canvas = self.canvas_rect(sw, sh);
        let (mx, my) = mouse_position();
        let cursor = vec2(mx, my);

        // `?` icon in the pane header (and `H`) toggles the help page.
        let icon_center = vec2(pane.x + pane.w - 26.0, pane.y + 26.0);
        if self.pane_open && ui::icon_button(icon_center, 15.0, "?", &theme) {
            self.help = !self.help;
            self.focus = None;
        }
        if is_key_pressed(KeyCode::H) && !self.typing() {
            self.help = !self.help;
        }
        if self.help {
            if is_key_pressed(KeyCode::Escape) {
                self.help = false;
            }
            self.advance(dt);
            return;
        }

        // Field focus by click.
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
                self.focus = Some(index);
            } else if is_mouse_button_pressed(MouseButton::Left)
                && !ui::inside(pane, cursor)
                && self.focus.is_some()
            {
                self.apply_fields();
            }
        } else if is_mouse_button_pressed(MouseButton::Left) && self.focus.is_some() {
            self.apply_fields();
        }

        // Typing.
        if let Some(index) = self.focus {
            if let Some(c) = get_char_pressed() {
                self.fields[index].push_char(c);
            }
            if is_key_pressed(KeyCode::Backspace) {
                self.fields[index].backspace();
            }
            if is_key_pressed(KeyCode::Delete) {
                self.fields[index].clear();
            }
            if is_key_pressed(KeyCode::Tab) {
                self.focus = Some((index + 1) % self.fields.len());
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                self.apply_fields();
            }
            if is_key_pressed(KeyCode::Escape) {
                self.focus = None;
            }
        }

        if self.pane_open {
            if ui::button(self.button_rect(0, 0, pane), "Apply", self.focus.is_some(), &theme) {
                self.apply_fields();
            }
            if ui::button(
                self.button_rect(0, 1, pane),
                "Reset view",
                self.zoom > 1.01 || self.center.is_some(),
                &theme,
            ) {
                self.zoom = 1.0;
                self.center = None;
                self.follow = false;
                self.bake_stale = true;
            }
            let label = if self.canvas.is_light() { "theme: light" } else { "theme: dark" };
            if ui::button(self.button_rect(1, 0, pane), label, true, &theme) {
                self.toggle_canvas();
            }
            if ui::button(self.button_rect(1, 1, pane), "Hide (Tab)", true, &theme) {
                self.pane_open = false;
            }
        }

        // Keys (only while not typing into a field).
        if !self.typing() {
            if is_key_pressed(KeyCode::Tab) {
                self.pane_open = !self.pane_open;
                self.bake_stale = true;
            }
            if is_key_pressed(KeyCode::B) {
                self.toggle_canvas();
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
                self.note(format!(
                    "base {} dir {} rule {} steps {} ({:.2} deg/unit) {}",
                    self.params.base,
                    self.params.dir,
                    self.params.rule.name(),
                    self.params.steps,
                    self.params.turn_per_digit_deg(),
                    if keep { "hot swap" } else { "replaying" }
                ));
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
                if let Some(curve) = &self.curve {
                    if self.revealed > 1 {
                        let previous = curve.clone();
                        let revealed = self.revealed;
                        self.ghost = Some(Ghost { curve: previous, revealed, alpha: 1.0 });
                    }
                    self.revealed = 1;
                    self.prefix = prefix_bbox(curve, 1);
                } else {
                    self.deep_level = 0.0;
                }
                self.paused = false;
                self.bake_stale = true;
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
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 && in_canvas {
            let win = self.window(canvas);
            if win.zoom_of(&self.framing) < MAX_ZOOM || wheel < 0.0 {
                let anchor = world_at(&win, canvas, mx as f64, my as f64);
                let mut next = win;
                next.zoom_around(ZOOM_STEP.powf(wheel as f64), anchor);
                self.zoom = next.zoom_of(&self.framing);
                self.center = Some(next.center);
                self.bake_stale = true;
            }
        }
        if !self.typing() && is_key_pressed(KeyCode::X) {
            self.zoom_by(ZOOM_STEP);
        }
        if !self.typing() && is_key_pressed(KeyCode::Z) {
            self.zoom_by(1.0 / ZOOM_STEP);
        }
        if is_mouse_button_pressed(MouseButton::Left) {
            self.pan_armed = in_canvas && !over_pane;
        }
        if is_mouse_button_released(MouseButton::Left) {
            self.pan_armed = false;
        }
        if self.pan_armed && is_mouse_button_down(MouseButton::Left) {
            let d = mouse_delta_position();
            if d.x != 0.0 || d.y != 0.0 {
                let win = self.window(canvas);
                let scale = win.view(canvas.w.max(16.0) as u32, canvas.h.max(16.0) as u32).scale;
                let mut next = win;
                next.pan([d.x as f64 / scale, -(d.y as f64) / scale]);
                self.center = Some(next.center);
                self.bake_stale = true;
            }
        }

        self.advance(dt);
    }

    fn export(&mut self, full: bool) {
        let canvas = self.canvas_rect(screen_width(), screen_height());
        let (w, h, ss) = if full {
            let w = (canvas.w as u32).clamp(640, 3840) * 2;
            let h = (canvas.h as u32).clamp(360, 2160) * 2;
            let ss = if (w as u64) * (h as u64) > 8_000_000 { 1 } else { 2 };
            (w, h, ss)
        } else {
            (canvas.w as u32, canvas.h as u32, 1)
        };
        let win = if full {
            let framed =
                Win::from_bbox(&self.framing, PANEL_PAD, 1.0, [0.0, 0.0], w as f64 / h as f64);
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
        let label = if full { "full" } else { "state" };
        let name = format!(
            "out/{label}_b{}_d{}_n{}_{}_z{:.0}.png",
            self.params.base,
            self.params.dir,
            self.params.steps,
            self.canvas.name(),
            win.zoom_of(&self.framing)
        );
        let outcome = if let Some(curve) = &self.curve {
            let visible = if full { None } else { Some(self.revealed) };
            let (iw, ih, rgb) = render::render_curve_window(curve, w, h, ss, &win, &opts, visible);
            png::write_rgb(&name, iw, ih, &rgb)
        } else {
            let dopts = DeepOpts {
                canvas: self.canvas,
                steps: if full { self.params.steps } else { self.steps_now() },
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
                self.note(format!("exported {name}"));
            }
            Err(e) => {
                eprintln!("export failed: {e}");
                self.note(format!("export failed: {e}"));
            }
        }
    }

    fn draw(&mut self) {
        let theme = self.theme();
        clear_background(theme.chrome_bg);
        let (sw, sh) = (screen_width(), screen_height());
        let canvas = self.canvas_rect(sw, sh);
        let bg = self.canvas.background();
        draw_rectangle(canvas.x, canvas.y, canvas.w, canvas.h, Color::new(bg[0], bg[1], bg[2], 1.0));

        if self.is_deep() {
            self.draw_deep(canvas);
        } else {
            self.draw_enum(canvas);
        }
        if self.pane_open {
            self.draw_pane(sw, sh);
        } else {
            // Collapsed: a thin tab so the pane can be brought back with the mouse as well.
            let tab = Rect::new(MARGIN, MARGIN, 26.0, 46.0);
            if ui::button(tab, ">", false, &theme) {
                self.pane_open = true;
                self.bake_stale = true;
            }
        }
        if self.help {
            self.draw_help(sw, sh);
        }
    }

    fn draw_enum(&self, canvas: Rect) {
        let Some(curve) = &self.curve else { return };
        let bbox = if self.follow { self.prefix } else { self.framing };
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
            let image = Image { bytes: rgba, width: w, height: h };
            match self.bake.as_mut() {
                Some(bake) => bake.tex.update(&image),
                None => self.bake = Some(Bake { tex: Texture2D::from_image(&image), w, h }),
            }
            self.bake_stale = false;
        }
        if let Some(bake) = &self.bake {
            draw_texture_ex(
                &bake.tex,
                canvas.x,
                canvas.y,
                WHITE,
                DrawTextureParams { dest_size: Some(vec2(canvas.w, canvas.h)), ..Default::default() },
            );
        }
        if let Some(stats) = &self.deep_stats {
            if let Some(frontier) = stats.frontier {
                let p = self.window(canvas).view(w as u32, h as u32).map(frontier);
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
        draw_rectangle_lines(pane.x, pane.y, pane.w, pane.h, 1.0, theme.panel_border);
        draw_text("PARAMETERS", pane.x + 12.0, pane.y + 30.0, 22.0, theme.text_bright);

        for index in 0..self.fields.len() {
            let rect = self.field_rect(index, pane);
            let mut field = self.fields[index].clone();
            field.draw(rect, &theme, get_time());
        }

        let status_y = pane.y + 52.0
            + (FIELD_H + ROW_GAP) * self.fields.len() as f32
            + 8.0
            + (BUTTON_H + 6.0) * 2.0
            + 16.0;
        let canvas = self.canvas_rect(sw, sh);
        let win = self.window(canvas);
        let mut lines = vec![
            format!("mode      {}", if self.is_deep() { "deep (blocks)" } else { "enum (steps)" }),
            format!("zoom      {:.4}", win.zoom_of(&self.framing)),
        ];
        if self.is_deep() {
            lines.push(format!("level     {:.1} / {}", self.deep_level, self.levels()));
            if let Some(stats) = &self.deep_stats {
                lines.push(format!("blocks    nodes {}", stats.nodes));
                if stats.truncated {
                    lines.push("WARNING   node budget hit".to_string());
                }
            }
        } else if let Some(curve) = &self.curve {
            lines.push(format!(
                "progress  {:.1}%",
                100.0 * self.revealed as f64 / curve.segments().max(1) as f64
            ));
        }
        lines.push(format!("turn/unit {:.2} deg", self.params.turn_per_digit_deg()));
        lines.push(format!(
            "speed     {:.4}{}",
            self.speed,
            if self.paused { "  (paused)" } else { "" }
        ));
        lines.push(format!("fps       {}", get_fps()));
        if let Some((text, _)) = &self.note {
            lines.push(text.clone());
        }
        for (i, line) in lines.iter().enumerate() {
            let y = status_y + 22.0 * i as f32;
            let color = if i + 1 == lines.len() && self.note.is_some() {
                theme.text_warn
            } else {
                theme.text_dim
            };
            let text = fit_text(line, pane.w - 24.0, 19);
            draw_text(&text, pane.x + 12.0, y, 19.0, color);
        }

        let hint_y = sh - 46.0;
        for (k, hint) in ["wheel: zoom at cursor | drag: pan", "Tab: hide pane | B: theme | ? : help"]
            .iter()
            .enumerate()
        {
            let text = fit_text(hint, pane.w - 24.0, 18);
            draw_text(&text, pane.x + 12.0, hint_y + 20.0 * k as f32, 18.0, theme.text_hint);
        }
    }

    fn draw_help(&self, sw: f32, sh: f32) {
        let theme = self.theme();
        draw_rectangle(0.0, 0.0, sw, sh, theme.overlay);
        let x = 70.0;
        let mut y = 64.0;
        draw_text("KEYS & HELP", x, y, 34.0, theme.text_bright);
        y += 42.0;
        for line in [
            "[ ]            base -1 / +1                , .   dir -1 / +1",
            "T              cycle digit rule           - =   steps /2 / x2",
            "Up Down        speed x2 / /2              Space pause / resume",
            "R              replay from the start",
            "F              reset view (zoom 1, centred)",
            "G              view: whole / follow the turtle",
            "X Z            zoom in / out at the view centre",
            "Tab            hide / show the parameter pane",
            "B              theme dark / light (canvas and this UI together)",
            "C              rotate the palette",
            "P / S          export this frame / whole curve (PNG, to out/)",
            "Shift + [ ] , . - =   hot swap without replaying",
            "H or ?         open / close this page       Esc  close / quit",
        ] {
            draw_text(line, x, y, 20.0, theme.text_dim);
            y += 25.0;
        }
        y += 12.0;
        draw_text("MOUSE", x, y, 23.0, theme.text_bright);
        y += 27.0;
        for line in [
            "wheel          zoom in / out at the cursor (the point under it stays put)",
            "left drag      pan the picture",
            "click a field  edit it; Enter applies, Tab jumps, Esc cancels",
        ] {
            draw_text(line, x, y, 20.0, theme.text_dim);
            y += 25.0;
        }
        y += 12.0;
        draw_text("PARAMETERS", x, y, 23.0, theme.text_bright);
        y += 27.0;
        for line in [
            "base     number base of the digit function, 2..64",
            "dir      equal parts of a full turn: 360/dir degrees per digit unit",
            "rule     per-digit table  linear | alternate | fold   (fold+4+6 = Koch arc)",
            "steps    unit steps up to 2^50; also accepts 2^30 or 1.5e6",
            "         above 4194304 it switches to deep rendering (cost is step independent)",
            "hue      palette rotation 0..1;   theme: dark | light for canvas and UI",
        ] {
            draw_text(line, x, y, 20.0, theme.text_dim);
            y += 25.0;
        }
        y += 8.0;
        draw_text(
            "deep zoom is limited by f64 precision: roughly 1e7x magnification",
            x,
            y,
            19.0,
            theme.text_hint,
        );
    }
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

        app.fields[F_BASE].text = "999".into();
        app.fields[F_DIR].text = "-999".into();
        app.fields[F_STEPS].text = "2^80".into();
        app.apply_fields();
        assert_eq!(app.params.base, MAX_BASE);
        assert_eq!(app.params.dir, -MAX_DIR_ABS);
        assert_eq!(app.params.steps, MAX_STEPS);
        assert_eq!(app.fields[F_BASE].text, MAX_BASE.to_string());
        assert!(app.curve.is_none(), "huge step counts switch to deep mode");

        app.params = Params::new(3, 5, 4096);
        app.sync_fields();
        for field in app.fields.iter_mut() {
            field.error = 0.0;
        }
        app.fields[F_RULE].text = "quadratic".into();
        let before = app.params;
        app.apply_fields();
        assert_eq!(app.params, before, "params must not change on a rejected value");
        assert!(app.fields[F_RULE].error > 0.0, "the offending field is highlighted");
        assert!(app.note.as_ref().is_some_and(|(text, _)| text.contains("rejected")));
    }

    #[test]
    fn rebuilding_keeps_the_rule_and_resets_the_reveal() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        app.reveal_all();
        app.rebuild(false);
        assert_eq!(app.revealed, 1, "a plain rebuild replays from the first segment");
        assert_eq!(app.params.rule, DigitRule::Fold);
        app.reveal_all();
        app.rebuild(true);
        assert!(app.revealed > 1, "a hot swap keeps the progress");
    }

    #[test]
    fn theme_toggle_switches_canvas_and_ui_together() {
        let mut app = App::new(Params::new(4, 6, 4096).with_rule(DigitRule::Fold));
        let params = app.params;
        assert_eq!(app.canvas, Canvas::Dark);
        assert_eq!(app.theme(), Theme::dark());
        app.toggle_canvas();
        assert_eq!(app.canvas, Canvas::Light);
        assert_eq!(app.theme(), Theme::light(), "the pane must follow the canvas");
        assert_eq!(app.params, params, "switching the theme must not touch the parameters");
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
        assert!(canvas.x >= pane.x + pane.w, "canvas must start right of the pane");
        assert!(canvas.x + canvas.w <= sw && canvas.y + canvas.h <= sh);
        let collapsed = App { pane_open: false, ..App::new(Params::default()) };
        let full = collapsed.canvas_rect(sw, sh);
        assert!(full.w > canvas.w, "collapsing the pane gives the canvas more room");
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
