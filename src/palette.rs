//! Colour helpers and the two canvas themes (dark / light).

/// Background + matching stroke ramp for the rendered picture.
///
/// The GUI chrome (control pane, help page) always stays dark; this only switches the *canvas*
/// that the fractal is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Canvas {
    Dark,
    Light,
}

impl Canvas {
    pub fn parse(s: &str) -> Option<Canvas> {
        match s.to_ascii_lowercase().as_str() {
            "dark" => Some(Canvas::Dark),
            "light" => Some(Canvas::Light),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Canvas::Dark => "dark",
            Canvas::Light => "light",
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Canvas::Light)
    }

    /// Background colour of the canvas.
    pub fn background(self) -> [f32; 3] {
        match self {
            Canvas::Dark => [0.016, 0.020, 0.043],
            // Slightly warm off-white: pure white makes thin strokes look grey.
            Canvas::Light => [0.972, 0.968, 0.952],
        }
    }

    /// Stroke colour along the path: a rainbow, tuned per canvas so it stays readable.
    pub fn ramp(self, t: f32, hue_shift: f32) -> [f32; 3] {
        match self {
            Canvas::Dark => hsv(0.62 * t.clamp(0.0, 1.0) + hue_shift, 0.72, 1.0),
            Canvas::Light => hsv(0.62 * t.clamp(0.0, 1.0) + hue_shift, 0.92, 0.72),
        }
    }
}

/// HSV -> RGB, all channels in `[0, 1]`. `h` wraps around.
pub fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

pub fn to_rgb8(c: [f32; 3]) -> [u8; 3] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
    ]
}

pub fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub fn hex(c: [f32; 3]) -> String {
    let [r, g, b] = to_rgb8(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_hits_the_corners() {
        assert_eq!(to_rgb8(hsv(0.0, 1.0, 1.0)), [255, 0, 0]);
        assert_eq!(to_rgb8(hsv(1.0 / 3.0, 1.0, 1.0)), [0, 255, 0]);
        assert_eq!(to_rgb8(hsv(2.0 / 3.0, 1.0, 1.0)), [0, 0, 255]);
        assert_eq!(to_rgb8(hsv(0.5, 0.0, 0.0)), [0, 0, 0]);
    }

    #[test]
    fn both_canvases_stay_in_gamut_and_differ() {
        for canvas in [Canvas::Dark, Canvas::Light] {
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                for c in [canvas.ramp(t, 0.0), canvas.ramp(t, 0.5)] {
                    assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
                }
            }
            let bg = canvas.background();
            assert!(bg.iter().all(|v| (0.0..=1.0).contains(v)));
        }
        assert_ne!(to_rgb8(Canvas::Dark.background()), to_rgb8(Canvas::Light.background()));
        assert_ne!(to_rgb8(Canvas::Dark.ramp(0.5, 0.0)), to_rgb8(Canvas::Light.ramp(0.5, 0.0)));
        assert!(Canvas::Dark.background().iter().sum::<f32>() < 0.2);
        assert!(Canvas::Light.background().iter().sum::<f32>() > 2.5);
    }

    #[test]
    fn canvas_names_round_trip() {
        for canvas in [Canvas::Dark, Canvas::Light] {
            assert_eq!(Canvas::parse(canvas.name()), Some(canvas));
        }
        assert_eq!(Canvas::parse("LIGHT"), Some(Canvas::Light));
        assert_eq!(Canvas::parse("grey"), None);
    }
}
