//! Tiny immediate-mode widgets for the interactive window, plus the GUI theme.
//!
//! The editing and parsing logic is pure (no macroquad calls), so it is unit tested without
//! opening a window; `gui.rs` feeds events and draws with a [`Theme`].

use macroquad::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use fun_fractal_base_direction::palette::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    Zh,
}

impl Language {
    pub fn toggle(self) -> Self {
        match self {
            Self::En => Self::Zh,
            Self::Zh => Self::En,
        }
    }
}

static CHINESE: AtomicBool = AtomicBool::new(false);
static FONTS: OnceLock<(Font, Font)> = OnceLock::new();

pub fn set_language(language: Language) {
    CHINESE.store(language == Language::Zh, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CHINESE.load(Ordering::Relaxed) {
        Language::Zh
    } else {
        Language::En
    }
}

pub fn localized(en: &'static str, zh: &'static str) -> &'static str {
    match language() {
        Language::En => en,
        Language::Zh => zh,
    }
}

pub fn field_label(label: &'static str) -> &'static str {
    match (language(), label) {
        (Language::Zh, "base") => "进制",
        (Language::Zh, "dir") => "方向",
        (Language::Zh, "rule") => "规则",
        (Language::Zh, "steps") => "步数",
        (Language::Zh, "hue") => "色相",
        _ => label,
    }
}

/// Use fonts already licensed and installed by macOS; do not bundle Apple font binaries.
pub fn init_fonts() -> Result<(), String> {
    let load = |paths: &[&str]| -> Result<Font, String> {
        let mut errors = Vec::new();
        for path in paths {
            match std::fs::read(path) {
                Ok(bytes) => match load_ttf_font_from_bytes(&bytes) {
                    Ok(font) => return Ok(font),
                    Err(e) => errors.push(format!("{path}: {e}")),
                },
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        Err(errors.join("; "))
    };
    let sf = load(&[
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/SFNSRounded.ttf",
    ])?;
    let pingfang = load(&[
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
    ])?;
    let _ = FONTS.set((sf, pingfang));
    Ok(())
}

fn app_font() -> Option<&'static Font> {
    FONTS.get().map(|(en, zh)| match language() {
        Language::En => en,
        Language::Zh => zh,
    })
}

pub fn draw_app_text(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
) -> TextDimensions {
    macroquad::text::draw_text_ex(
        text,
        x,
        y,
        TextParams {
            font: app_font(),
            font_size: size as u16,
            color,
            ..Default::default()
        },
    )
}

pub fn measure_app_text(text: impl AsRef<str>, size: u16) -> TextDimensions {
    macroquad::text::measure_text(text, app_font(), size, 1.0)
}

fn draw_text(text: impl AsRef<str>, x: f32, y: f32, size: f32, color: Color) -> TextDimensions {
    draw_app_text(text, x, y, size, color)
}

fn measure_text(
    text: impl AsRef<str>,
    _font: Option<&Font>,
    size: u16,
    _scale: f32,
) -> TextDimensions {
    measure_app_text(text, size)
}

pub const FIELD_H: f32 = 30.0;
pub const LABEL_W: f32 = 104.0;
const MAX_LEN: usize = 40;

/// `4096`, `2^30`, `1.5e6`, `2_000_000` -> a count.
pub fn parse_count(text: &str) -> Option<u64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    if t.is_empty() {
        return None;
    }
    if let Some((base, exp)) = t.split_once('^') {
        let base: u64 = base.parse().ok()?;
        let exp: u32 = exp.parse().ok()?;
        if exp > 63 {
            return Some(u64::MAX);
        }
        return Some(base.saturating_pow(exp));
    }
    if let Some((mantissa, exp)) = t.split_once(['e', 'E']) {
        let mantissa: f64 = mantissa.parse().ok()?;
        let exp: i32 = exp.parse().ok()?;
        let value = mantissa * 10f64.powi(exp);
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        return Some(value.min(u64::MAX as f64) as u64);
    }
    t.parse().ok()
}

/// `-6`, `+4`, `12`.
pub fn parse_int(text: &str) -> Option<i64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    let t = t.strip_prefix('+').unwrap_or(&t);
    t.parse().ok()
}

/// `0.35`, `.5`, `1`.
pub fn parse_float(text: &str) -> Option<f64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    t.parse().ok()
}

#[inline]
pub fn inside(rect: Rect, p: Vec2) -> bool {
    p.x >= rect.x && p.x <= rect.x + rect.w && p.y >= rect.y && p.y <= rect.y + rect.h
}

/// Colours of the window chrome (pane, fields, buttons, help page, markers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub chrome_bg: Color,
    pub panel_bg: Color,
    pub panel_border: Color,
    pub text_bright: Color,
    pub text_dim: Color,
    pub text_hint: Color,
    pub text_warn: Color,
    pub field_bg: Color,
    pub field_bg_focus: Color,
    pub field_border: Color,
    pub field_border_focus: Color,
    pub field_error: Color,
    pub caret: Color,
    pub button_bg: Color,
    pub button_bg_hover: Color,
    pub button_bg_active: Color,
    pub button_border: Color,
    pub button_text: Color,
    pub accent_bg: Color,
    pub accent_text: Color,
    pub overlay: Color,
    pub marker: Color,
    pub marker_dot: Color,
}

impl Theme {
    pub fn dark() -> Self {
        let rgba = |r: f32, g: f32, b: f32, a: f32| Color::new(r, g, b, a);
        Theme {
            chrome_bg: rgba(0.075, 0.082, 0.094, 1.0),
            panel_bg: rgba(0.12, 0.13, 0.15, 1.0),
            panel_border: rgba(0.30, 0.32, 0.35, 1.0),
            text_bright: rgba(0.96, 0.97, 0.99, 1.0),
            text_dim: rgba(0.77, 0.79, 0.83, 1.0),
            text_hint: rgba(0.64, 0.67, 0.73, 1.0),
            text_warn: rgba(1.0, 0.77, 0.39, 1.0),
            field_bg: rgba(0.17, 0.18, 0.21, 1.0),
            field_bg_focus: rgba(0.20, 0.22, 0.25, 1.0),
            field_border: rgba(0.36, 0.38, 0.43, 1.0),
            field_border_focus: rgba(0.45, 0.68, 1.0, 1.0),
            field_error: rgba(1.0, 0.36, 0.39, 1.0),
            caret: rgba(0.96, 0.97, 0.99, 1.0),
            button_bg: rgba(0.22, 0.24, 0.28, 1.0),
            button_bg_hover: rgba(0.28, 0.31, 0.36, 1.0),
            button_bg_active: rgba(0.35, 0.45, 0.60, 1.0),
            button_border: rgba(0.38, 0.40, 0.45, 1.0),
            button_text: rgba(0.96, 0.97, 0.99, 1.0),
            accent_bg: rgba(0.12, 0.40, 0.79, 1.0),
            accent_text: rgba(1.0, 1.0, 1.0, 1.0),
            overlay: rgba(0.07, 0.08, 0.10, 0.96),
            marker: rgba(1.0, 1.0, 1.0, 0.92),
            marker_dot: rgba(1.0, 0.85, 0.45, 1.0),
        }
    }

    pub fn light() -> Self {
        let rgba = |r: f32, g: f32, b: f32, a: f32| Color::new(r, g, b, a);
        Theme {
            chrome_bg: rgba(0.90, 0.92, 0.94, 1.0),
            panel_bg: rgba(0.97, 0.98, 0.99, 1.0),
            panel_border: rgba(0.78, 0.80, 0.83, 1.0),
            text_bright: rgba(0.10, 0.12, 0.16, 1.0),
            text_dim: rgba(0.29, 0.32, 0.37, 1.0),
            text_hint: rgba(0.40, 0.43, 0.48, 1.0),
            text_warn: rgba(0.59, 0.32, 0.02, 1.0),
            field_bg: rgba(1.0, 1.0, 1.0, 1.0),
            field_bg_focus: rgba(1.0, 1.0, 1.0, 1.0),
            field_border: rgba(0.74, 0.77, 0.81, 1.0),
            field_border_focus: rgba(0.12, 0.38, 0.78, 1.0),
            field_error: rgba(0.76, 0.12, 0.16, 1.0),
            caret: rgba(0.10, 0.12, 0.16, 1.0),
            button_bg: rgba(0.90, 0.92, 0.95, 1.0),
            button_bg_hover: rgba(0.83, 0.88, 0.95, 1.0),
            button_bg_active: rgba(0.74, 0.83, 0.95, 1.0),
            button_border: rgba(0.72, 0.75, 0.80, 1.0),
            button_text: rgba(0.10, 0.13, 0.20, 1.0),
            accent_bg: rgba(0.09, 0.38, 0.77, 1.0),
            accent_text: rgba(1.0, 1.0, 1.0, 1.0),
            overlay: rgba(0.96, 0.97, 0.99, 0.97),
            marker: rgba(0.10, 0.12, 0.18, 0.95),
            marker_dot: rgba(0.80, 0.35, 0.0, 1.0),
        }
    }

    /// The chrome follows the canvas so the picture and the pane never disagree.
    pub fn for_canvas(canvas: Canvas) -> Self {
        if canvas.is_light() {
            Theme::light()
        } else {
            Theme::dark()
        }
    }

    pub fn is_light(&self) -> bool {
        *self == Theme::light()
    }
}

pub fn rounded_rect(rect: Rect, radius: f32, color: Color) {
    let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
    if r < 1.0 {
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, color);
        return;
    }
    draw_rectangle(rect.x + r, rect.y, rect.w - 2.0 * r, rect.h, color);
    draw_rectangle(rect.x, rect.y + r, rect.w, rect.h - 2.0 * r, color);
    for x in [rect.x + r, rect.x + rect.w - r] {
        for y in [rect.y + r, rect.y + rect.h - r] {
            draw_circle(x, y, r, color);
        }
    }
}

pub fn rounded_panel(rect: Rect, radius: f32, fill: Color, border: Color) {
    rounded_rect(rect, radius, border);
    rounded_rect(
        Rect::new(
            rect.x + 1.0,
            rect.y + 1.0,
            (rect.w - 2.0).max(0.0),
            (rect.h - 2.0).max(0.0),
        ),
        (radius - 1.0).max(0.0),
        fill,
    );
}

pub fn rounded_outline(rect: Rect, radius: f32, color: Color) {
    let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
    let steps = 8;
    let corners = [
        (
            rect.x + rect.w - r,
            rect.y + r,
            -std::f32::consts::FRAC_PI_2,
        ),
        (rect.x + rect.w - r, rect.y + rect.h - r, 0.0),
        (rect.x + r, rect.y + rect.h - r, std::f32::consts::FRAC_PI_2),
        (rect.x + r, rect.y + r, std::f32::consts::PI),
    ];
    let mut first: Option<Vec2> = None;
    let mut previous: Option<Vec2> = None;
    for (cx, cy, start) in corners {
        for i in 0..=steps {
            let angle = start + i as f32 * std::f32::consts::FRAC_PI_2 / steps as f32;
            let next = vec2(cx + r * angle.cos(), cy + r * angle.sin());
            if let Some(prev) = previous {
                draw_line(prev.x, prev.y, next.x, next.y, 1.0, color);
            } else {
                first = Some(next);
            }
            previous = Some(next);
        }
    }
    if let (Some(a), Some(b)) = (previous, first) {
        draw_line(a.x, a.y, b.x, b.y, 1.0, color);
    }
}

#[derive(Clone, Debug)]
pub struct TextInput {
    pub label: &'static str,
    pub hint: &'static str,
    pub text: String,
    pub focused: bool,
    /// Countdown of the "rejected value" highlight, in seconds.
    pub error: f32,
    caret: f32,
    cursor: usize,
    selected: bool,
}

impl TextInput {
    pub fn new(label: &'static str, hint: &'static str, value: impl Into<String>) -> Self {
        let text: String = value.into();
        let cursor = text.chars().count();
        TextInput {
            label,
            hint,
            text,
            focused: false,
            error: 0.0,
            caret: 0.0,
            cursor,
            selected: false,
        }
    }

    /// Programmatic update. Ignored while the field is being edited, so typing is never clobbered.
    pub fn set(&mut self, value: impl Into<String>) {
        if !self.focused {
            self.replace(value);
        }
    }

    fn byte_index(&self, at: usize) -> usize {
        self.text
            .char_indices()
            .nth(at)
            .map_or(self.text.len(), |(i, _)| i)
    }

    pub fn focus(&mut self) {
        self.focused = true;
        self.cursor = self.text.chars().count();
        self.selected = false;
        self.caret = 0.0;
    }

    pub fn select_all(&mut self) {
        self.selected = true;
    }

    pub fn push_char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        if self.selected {
            self.clear();
        }
        if self.text.chars().count() < MAX_LEN {
            self.text.insert(self.byte_index(self.cursor), c);
            self.cursor += 1;
        }
    }

    pub fn paste(&mut self, text: &str) {
        for ch in text.chars().filter(|c| !c.is_control()) {
            self.push_char(ch);
        }
    }

    pub fn backspace(&mut self) {
        if self.selected {
            self.clear();
            return;
        }
        if self.cursor > 0 {
            self.text.remove(self.byte_index(self.cursor - 1));
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.selected {
            self.clear();
            return;
        }
        if self.cursor < self.text.chars().count() {
            self.text.remove(self.byte_index(self.cursor));
        }
    }

    pub fn move_cursor(&mut self, position: usize) {
        self.cursor = position.min(self.text.chars().count());
        self.selected = false;
        self.caret = 0.0;
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selected = false;
    }

    pub fn replace(&mut self, value: impl Into<String>) {
        self.text = value.into();
        self.cursor = self.text.chars().count();
        self.selected = false;
        self.error = 0.0;
    }

    pub fn mark_error(&mut self) {
        self.error = 1.2;
    }

    pub fn tick(&mut self, dt: f32) {
        self.error = (self.error - dt).max(0.0);
        if self.focused {
            self.caret += dt;
        }
    }

    pub fn draw(&mut self, rect: Rect, theme: &Theme, time: f64) {
        let (mx, my) = mouse_position();
        let border = if self.error > 0.0 {
            theme.field_error
        } else if self.focused {
            theme.field_border_focus
        } else if inside(rect, vec2(mx, my)) {
            theme.button_border
        } else {
            theme.field_border
        };
        if self.focused {
            let mut glow = border;
            glow.a = 0.18;
            rounded_rect(
                Rect::new(rect.x - 2.0, rect.y - 2.0, rect.w + 4.0, rect.h + 4.0),
                9.0,
                glow,
            );
        }
        rounded_panel(
            rect,
            7.0,
            if self.focused {
                theme.field_bg_focus
            } else {
                theme.field_bg
            },
            border,
        );
        let label = field_label(self.label);
        draw_text(label, rect.x + 8.0, rect.y + 21.0, 18.0, theme.text_dim);
        let value_x = rect.x + LABEL_W;
        let empty = self.text.is_empty();
        let color = if empty {
            theme.text_hint
        } else {
            theme.text_bright
        };
        let available = (rect.x + rect.w - value_x - 12.0).max(0.0);
        let mut start = 0;
        if self.focused {
            while start < self.cursor
                && measure_text(
                    &self.text[self.byte_index(start)..self.byte_index(self.cursor)],
                    None,
                    20,
                    1.0,
                )
                .width
                    > available
            {
                start += 1;
            }
        }
        let shown = if empty {
            if language() == Language::Zh && self.label == "dir" {
                "+/-1..64"
            } else {
                self.hint
            }
        } else {
            &self.text[self.byte_index(start)..]
        };
        let mut visible = String::new();
        for ch in shown.chars() {
            let candidate = format!("{visible}{ch}");
            if measure_text(&candidate, None, 20, 1.0).width > available {
                break;
            }
            visible.push(ch);
        }
        if self.focused && self.selected {
            draw_rectangle(
                value_x,
                rect.y + 4.0,
                measure_text(&visible, None, 20, 1.0).width,
                rect.h - 8.0,
                theme.button_bg_active,
            );
        }
        draw_text(&visible, value_x, rect.y + 21.0, 20.0, color);
        if self.focused && !self.selected && (time * 2.2) as i64 % 2 == 0 {
            let prefix = &self.text[self.byte_index(start)..self.byte_index(self.cursor)];
            let w = measure_text(prefix, None, 20, 1.0).width;
            draw_rectangle(
                value_x + w + 1.0,
                rect.y + 6.0,
                2.0,
                rect.h - 12.0,
                theme.caret,
            );
        }
    }
}

/// Hit-test a control during update; drawing and event handling are kept separate.
pub fn clicked(rect: Rect) -> bool {
    let (mx, my) = mouse_position();
    inside(rect, vec2(mx, my)) && is_mouse_button_pressed(MouseButton::Left)
}

pub fn button(rect: Rect, label: &str, active: bool, theme: &Theme) {
    let (mx, my) = mouse_position();
    let hovered = inside(rect, vec2(mx, my));
    let bg = if active {
        theme.accent_bg
    } else if hovered {
        theme.button_bg_hover
    } else {
        theme.button_bg
    };
    let shadow = Color::new(0.0, 0.0, 0.0, if theme.is_light() { 0.09 } else { 0.20 });
    rounded_rect(Rect::new(rect.x, rect.y + 2.0, rect.w, rect.h), 8.0, shadow);
    rounded_panel(
        rect,
        8.0,
        bg,
        if active {
            theme.accent_bg
        } else {
            theme.button_border
        },
    );
    let natural = measure_text(label, None, 19, 1.0).width;
    let size = if natural > rect.w - 12.0 {
        (19.0 * (rect.w - 12.0) / natural).max(11.0)
    } else {
        19.0
    };
    let dim = measure_text(label, None, size as u16, 1.0);
    draw_text(
        label,
        rect.x + (rect.w - dim.width) * 0.5,
        rect.y + rect.h * 0.5 + dim.height * 0.35,
        size,
        if active {
            theme.accent_text
        } else {
            theme.button_text
        },
    );
}

/// The three rule choices share one field row. Keep hit regions separate from drawing.
pub fn segmented_choice_rect(rect: Rect, index: usize) -> Rect {
    debug_assert!(index < 3);
    let slot = (rect.w - 60.0).max(0.0) / 3.0;
    Rect::new(
        rect.x + 56.0 + slot * index as f32 + 2.0,
        rect.y + 3.0,
        (slot - 4.0).max(0.0),
        (rect.h - 6.0).max(0.0),
    )
}

/// Draw the draft rule selection without modifying the underlying text field.
pub fn rule_selector(
    rect: Rect,
    choices: [&str; 3],
    selected: usize,
    focused: bool,
    theme: &Theme,
) {
    if focused {
        let mut glow = theme.field_border_focus;
        glow.a = 0.18;
        rounded_rect(
            Rect::new(rect.x - 2.0, rect.y - 2.0, rect.w + 4.0, rect.h + 4.0),
            9.0,
            glow,
        );
    }
    rounded_panel(
        rect,
        7.0,
        theme.field_bg,
        if focused {
            theme.field_border_focus
        } else {
            theme.field_border
        },
    );
    draw_text(
        field_label("rule"),
        rect.x + 8.0,
        rect.y + rect.h * 0.5 + 5.0,
        16.0,
        theme.text_dim,
    );
    let (mx, my) = mouse_position();
    for (index, choice) in choices.iter().enumerate() {
        let segment = segmented_choice_rect(rect, index);
        let active = selected == index;
        let hovered = inside(segment, vec2(mx, my));
        if active {
            rounded_panel(segment, 6.0, theme.accent_bg, theme.field_border_focus);
        } else if hovered {
            rounded_rect(segment, 6.0, theme.button_bg_hover);
        }
        let max_width = (segment.w - 6.0).max(0.0);
        let mut size = 15u16;
        while size > 10 && measure_app_text(choice, size).width > max_width {
            size -= 1;
        }
        let dimensions = measure_app_text(choice, size);
        draw_text(
            choice,
            segment.x + (segment.w - dimensions.width) * 0.5,
            segment.y + segment.h * 0.5 + dimensions.height * 0.35,
            size as f32,
            if active {
                theme.accent_text
            } else {
                theme.text_bright
            },
        );
    }
}

/// A draggable playback track. The caller owns seeking and drag capture.
pub fn scrubber(rect: Rect, fraction: f32, dragging: bool, theme: &Theme) {
    let fraction = fraction.clamp(0.0, 1.0);
    let center = vec2(rect.x + rect.w * fraction, rect.y + rect.h * 0.5);
    let (mx, my) = mouse_position();
    let hovered = inside(
        Rect::new(rect.x - 9.0, rect.y - 7.0, rect.w + 18.0, rect.h + 14.0),
        vec2(mx, my),
    );
    rounded_panel(rect, rect.h * 0.5, theme.field_bg, theme.field_border);
    let filled = rect.w * fraction;
    if filled > 0.0 {
        rounded_rect(
            Rect::new(rect.x, rect.y, filled, rect.h),
            rect.h * 0.5,
            theme.field_border_focus,
        );
    }
    if dragging || hovered {
        let mut halo = theme.field_border_focus;
        halo.a = if dragging { 0.24 } else { 0.13 };
        draw_circle(center.x, center.y, if dragging { 11.0 } else { 9.5 }, halo);
    }
    let radius = if dragging {
        7.5
    } else if hovered {
        6.5
    } else {
        5.5
    };
    draw_circle(
        center.x,
        center.y + 1.0,
        radius + 1.0,
        Color::new(0.0, 0.0, 0.0, 0.18),
    );
    draw_circle(center.x, center.y, radius + 1.0, theme.field_border_focus);
    draw_circle(center.x, center.y, radius, theme.accent_text);
}

pub fn icon_button(center: Vec2, radius: f32, glyph: &str, theme: &Theme) {
    let (mx, my) = mouse_position();
    let hovered = ((mx - center.x).powi(2) + (my - center.y).powi(2)).sqrt() <= radius;
    let rect = Rect::new(
        center.x - radius,
        center.y - radius,
        radius * 2.0,
        radius * 2.0,
    );
    rounded_panel(
        rect,
        9.0,
        if hovered {
            theme.button_bg_hover
        } else {
            theme.button_bg
        },
        theme.button_border,
    );
    let dim = measure_text(glyph, None, 24, 1.0);
    draw_text(
        glyph,
        center.x - dim.width * 0.5,
        center.y + dim.height * 0.35,
        24.0,
        theme.button_text,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance (sRGB channels linearised first).
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            let v = v.clamp(0.0, 1.0);
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (hi, lo) = (
            luminance(a).max(luminance(b)),
            luminance(a).min(luminance(b)),
        );
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn parses_counts_including_powers_and_scientific() {
        assert_eq!(parse_count("4096"), Some(4096));
        assert_eq!(parse_count("2^30"), Some(1 << 30));
        assert_eq!(parse_count("2 ^ 30"), Some(1 << 30));
        assert_eq!(parse_count("1.5e6"), Some(1_500_000));
        assert_eq!(parse_count("2_000_000"), Some(2_000_000));
        assert_eq!(parse_count("2^50"), Some(1 << 50));
        assert_eq!(parse_count("2^900"), Some(u64::MAX));
        assert_eq!(parse_count("nope"), None);
        assert_eq!(parse_count(""), None);
        assert_eq!(parse_count("-4"), None);
    }

    #[test]
    fn parses_signed_ints_and_floats() {
        assert_eq!(parse_int("-6"), Some(-6));
        assert_eq!(parse_int("+4"), Some(4));
        assert_eq!(parse_int(" 12 "), Some(12));
        assert_eq!(parse_int("4.5"), None);
        assert_eq!(parse_float("0.35"), Some(0.35));
        assert_eq!(parse_float(".5"), Some(0.5));
        assert_eq!(parse_float("x"), None);
    }

    #[test]
    fn editing_respects_focus() {
        let mut field = TextInput::new("base", "2..64", "4");
        field.set("7");
        assert_eq!(field.text, "7");
        field.focused = true;
        field.set("9");
        assert_eq!(field.text, "7");
        field.push_char('2');
        assert_eq!(field.text, "72");
        field.backspace();
        assert_eq!(field.text, "7");
        field.clear();
        assert_eq!(field.text, "");
        field.push_char('\n');
        assert_eq!(field.text, "");
        for _ in 0..100 {
            field.push_char('1');
        }
        assert_eq!(field.text.chars().count(), MAX_LEN);
    }

    #[test]
    fn cursor_selection_and_paste_edit_in_place() {
        let mut field = TextInput::new("steps", "2^30", "1234");
        field.focus();
        field.move_cursor(2);
        field.push_char('x');
        assert_eq!(field.text, "12x34");
        field.backspace();
        assert_eq!(field.text, "1234");
        field.delete();
        assert_eq!(field.text, "124");
        field.select_all();
        field.paste("2^30\n");
        assert_eq!(field.text, "2^30");
        field.move_cursor(0);
        field.push_char('λ');
        field.move_cursor(1);
        field.delete();
        assert_eq!(field.text, "λ^30");
    }

    #[test]
    fn segmented_choices_fit_narrow_and_wide_fields_without_overlap() {
        for width in [256.0, 312.0] {
            let outer = Rect::new(16.0, 170.0, width, FIELD_H);
            let choices = [0, 1, 2].map(|i| segmented_choice_rect(outer, i));
            for choice in choices {
                assert!(choice.x >= outer.x && choice.x + choice.w <= outer.x + outer.w);
                assert!(choice.y >= outer.y && choice.y + choice.h <= outer.y + outer.h);
                assert!(choice.w > 0.0 && choice.h > 0.0);
            }
            assert!(choices[0].x + choices[0].w < choices[1].x);
            assert!(choices[1].x + choices[1].w < choices[2].x);
        }
    }

    #[test]
    fn hit_testing_is_inclusive() {
        let rect = Rect::new(10.0, 20.0, 100.0, 30.0);
        assert!(inside(rect, vec2(10.0, 20.0)));
        assert!(inside(rect, vec2(110.0, 50.0)));
        assert!(!inside(rect, vec2(9.9, 20.0)));
        assert!(!inside(rect, vec2(50.0, 50.1)));
    }

    /// Both themes must stay readable: text on the panel, field text on the field.
    #[test]
    fn themes_keep_enough_contrast() {
        for theme in [Theme::dark(), Theme::light()] {
            assert!(
                contrast(theme.text_bright, theme.panel_bg) >= 4.0,
                "text vs pane"
            );
            assert!(
                contrast(theme.text_dim, theme.panel_bg) >= 3.0,
                "labels vs pane"
            );
            assert!(
                contrast(theme.text_bright, theme.field_bg) >= 4.0,
                "value vs field"
            );
            assert!(
                contrast(theme.button_text, theme.panel_bg) >= 3.0,
                "button vs pane"
            );
            assert!(
                contrast(theme.field_error, theme.field_bg) >= 2.0,
                "error vs field"
            );
        }
    }

    #[test]
    fn theme_follows_the_canvas() {
        assert_eq!(Theme::for_canvas(Canvas::Dark), Theme::dark());
        assert_eq!(Theme::for_canvas(Canvas::Light), Theme::light());
        assert!(!Theme::dark().is_light());
        assert!(Theme::light().is_light());
    }
}
