//! Tiny immediate-mode widgets for the interactive window, plus the GUI theme.
//!
//! The editing and parsing logic is pure (no macroquad calls), so it is unit tested without
//! opening a window; `gui.rs` only feeds events and draws with a [`Theme`].
//!
//! The theme follows the canvas: `B` (or the `canvas:` button) switches the picture *and* the
//! chrome between dark and light, so the framing, the pane and the strokes always agree.

use macroquad::prelude::*;

use fun_fractal_base_direction::palette::Canvas;

pub const FIELD_H: f32 = 30.0;
pub const LABEL_W: f32 = 104.0;
const MAX_LEN: usize = 40;

/// `4096`, `2^30`, `1.5e6`, `2_000_000` -> a count.
pub fn parse_count(text: &str) -> Option<u64> {
    let t: String = text.chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
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
    let t: String = text.chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
    let t = t.strip_prefix('+').unwrap_or(&t);
    t.parse().ok()
}

/// `0.35`, `.5`, `1`.
pub fn parse_float(text: &str) -> Option<f64> {
    let t: String = text.chars().filter(|c| !c.is_whitespace() && *c != '_').collect();
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
    pub overlay: Color,
    pub marker: Color,
    pub marker_dot: Color,
}

impl Theme {
    pub fn dark() -> Self {
        let rgba = |r: f32, g: f32, b: f32, a: f32| Color::new(r, g, b, a);
        Theme {
            chrome_bg: rgba(0.031, 0.039, 0.063, 1.0),
            panel_bg: rgba(0.043, 0.055, 0.086, 1.0),
            panel_border: rgba(0.32, 0.40, 0.56, 0.9),
            text_bright: rgba(0.90, 0.94, 1.0, 1.0),
            text_dim: rgba(0.66, 0.74, 0.88, 1.0),
            text_hint: rgba(0.56, 0.64, 0.79, 1.0),
            text_warn: rgba(1.0, 0.85, 0.45, 1.0),
            field_bg: rgba(0.055, 0.066, 0.11, 0.85),
            field_bg_focus: rgba(0.10, 0.12, 0.20, 0.95),
            field_border: rgba(0.35, 0.42, 0.55, 0.8),
            field_border_focus: rgba(0.55, 0.75, 1.0, 0.95),
            field_error: rgba(1.0, 0.38, 0.38, 0.95),
            caret: rgba(0.90, 0.95, 1.0, 0.9),
            button_bg: rgba(0.50, 0.70, 1.0, 0.12),
            button_bg_hover: rgba(0.50, 0.70, 1.0, 0.22),
            button_bg_active: rgba(0.50, 0.70, 1.0, 0.30),
            button_border: rgba(0.45, 0.60, 0.85, 0.75),
            button_text: rgba(0.92, 0.96, 1.0, 1.0),
            overlay: rgba(0.01, 0.02, 0.04, 0.93),
            marker: rgba(1.0, 1.0, 1.0, 0.92),
            marker_dot: rgba(1.0, 0.85, 0.45, 1.0),
        }
    }

    pub fn light() -> Self {
        let rgba = |r: f32, g: f32, b: f32, a: f32| Color::new(r, g, b, a);
        Theme {
            chrome_bg: rgba(0.90, 0.89, 0.86, 1.0),
            panel_bg: rgba(0.97, 0.96, 0.94, 1.0),
            panel_border: rgba(0.62, 0.62, 0.60, 0.9),
            text_bright: rgba(0.11, 0.13, 0.18, 1.0),
            text_dim: rgba(0.28, 0.31, 0.37, 1.0),
            text_hint: rgba(0.42, 0.45, 0.50, 1.0),
            text_warn: rgba(0.65, 0.36, 0.0, 1.0),
            field_bg: rgba(1.0, 1.0, 1.0, 0.95),
            field_bg_focus: rgba(1.0, 1.0, 1.0, 1.0),
            field_border: rgba(0.70, 0.70, 0.68, 0.95),
            field_border_focus: rgba(0.16, 0.38, 0.78, 0.95),
            field_error: rgba(0.78, 0.15, 0.15, 0.95),
            caret: rgba(0.12, 0.15, 0.22, 0.9),
            button_bg: rgba(0.20, 0.40, 0.80, 0.10),
            button_bg_hover: rgba(0.20, 0.40, 0.80, 0.20),
            button_bg_active: rgba(0.20, 0.40, 0.80, 0.30),
            button_border: rgba(0.35, 0.45, 0.65, 0.80),
            button_text: rgba(0.10, 0.13, 0.20, 1.0),
            overlay: rgba(0.93, 0.92, 0.89, 0.96),
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

/// A labelled single-line text field.
#[derive(Clone, Debug)]
pub struct TextInput {
    pub label: &'static str,
    pub hint: &'static str,
    pub text: String,
    pub focused: bool,
    /// Countdown of the "rejected value" highlight, in seconds.
    pub error: f32,
    caret: f32,
}

impl TextInput {
    pub fn new(label: &'static str, hint: &'static str, value: impl Into<String>) -> Self {
        TextInput { label, hint, text: value.into(), focused: false, error: 0.0, caret: 0.0 }
    }

    /// Programmatic update. Ignored while the field is being edited, so typing is never clobbered.
    pub fn set(&mut self, value: impl Into<String>) {
        if !self.focused {
            self.text = value.into();
        }
    }

    pub fn push_char(&mut self, c: char) {
        if !c.is_control() && self.text.chars().count() < MAX_LEN {
            self.text.push(c);
        }
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }

    pub fn clear(&mut self) {
        self.text.clear();
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
        draw_rectangle(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            if self.focused { theme.field_bg_focus } else { theme.field_bg },
        );
        let border = if self.error > 0.0 {
            theme.field_error
        } else if self.focused {
            theme.field_border_focus
        } else {
            theme.field_border
        };
        draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 1.0, border);
        draw_text(self.label, rect.x + 8.0, rect.y + 21.0, 19.0, theme.text_dim);
        let value_x = rect.x + LABEL_W;
        let empty = self.text.is_empty();
        let color = if empty { theme.text_hint } else { theme.text_bright };
        let shown = if empty { self.hint } else { &self.text };
        draw_text(shown, value_x, rect.y + 21.0, 20.0, color);
        if self.focused && (time * 2.2) as i64 % 2 == 0 {
            let w = measure_text(&self.text, None, 20, 1.0).width;
            let x = (value_x + w + 2.0).min(rect.x + rect.w - 6.0);
            draw_rectangle(x, rect.y + 6.0, 2.0, rect.h - 12.0, theme.caret);
        }
    }
}

/// A clickable labelled button; returns true when it was clicked this frame.
pub fn button(rect: Rect, label: &str, active: bool, theme: &Theme) -> bool {
    let (mx, my) = mouse_position();
    let hovered = inside(rect, vec2(mx, my));
    let bg = if active {
        theme.button_bg_active
    } else if hovered {
        theme.button_bg_hover
    } else {
        theme.button_bg
    };
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, bg);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 1.0, theme.button_border);
    let dim = measure_text(label, None, 19, 1.0);
    draw_text(
        label,
        rect.x + (rect.w - dim.width) * 0.5,
        rect.y + rect.h * 0.5 + dim.height * 0.35,
        19.0,
        theme.button_text,
    );
    hovered && is_mouse_button_pressed(MouseButton::Left)
}

/// A round icon button (`?`); returns true when it was clicked this frame.
pub fn icon_button(center: Vec2, radius: f32, glyph: &str, theme: &Theme) -> bool {
    let (mx, my) = mouse_position();
    let hovered = ((mx - center.x).powi(2) + (my - center.y).powi(2)).sqrt() <= radius;
    let mut fill = theme.button_bg_hover;
    fill.a = if hovered { 0.95 } else { 0.55 };
    draw_circle(center.x, center.y, radius, fill);
    draw_circle_lines(center.x, center.y, radius, 1.5, theme.button_border);
    let dim = measure_text(glyph, None, 24, 1.0);
    draw_text(
        glyph,
        center.x - dim.width * 0.5,
        center.y + dim.height * 0.35,
        24.0,
        theme.button_text,
    );
    hovered && is_mouse_button_pressed(MouseButton::Left)
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
        let (hi, lo) = (luminance(a).max(luminance(b)), luminance(a).min(luminance(b)));
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
            assert!(contrast(theme.text_bright, theme.panel_bg) >= 4.0, "text vs pane");
            assert!(contrast(theme.text_dim, theme.panel_bg) >= 3.0, "labels vs pane");
            assert!(contrast(theme.text_bright, theme.field_bg) >= 4.0, "value vs field");
            assert!(contrast(theme.button_text, theme.panel_bg) >= 3.0, "button vs pane");
            assert!(contrast(theme.field_error, theme.field_bg) >= 2.0, "error vs field");
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
