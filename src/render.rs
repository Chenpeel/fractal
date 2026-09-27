//! Software rasteriser (anti-aliased additive strokes) plus PNG/SVG export.

use std::io;
use std::path::Path;

use crate::fractal::{Bbox, Curve};
use crate::palette::{self, Canvas};

/// World -> pixel mapping (world `+y` is up, screen `+y` is down).
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub scale: f64,
    pub ox: f64,
    pub oy: f64,
    pub w: u32,
    pub h: u32,
}

impl View {
    /// Fit `bbox` into a `w x h` viewport. `zoom` 1.0 is an exact fit; `shift` pans in
    /// units of the bounding box size.
    pub fn fit(bbox: &Bbox, w: u32, h: u32, pad_frac: f64, zoom: f64, shift: [f64; 2]) -> View {
        let bw = bbox.width().max(1e-9);
        let bh = bbox.height().max(1e-9);
        let pad = pad_frac.clamp(0.0, 0.45);
        let avail_w = (w as f64 * (1.0 - 2.0 * pad)).max(1.0);
        let avail_h = (h as f64 * (1.0 - 2.0 * pad)).max(1.0);
        let scale = (avail_w / bw).min(avail_h / bh) * zoom.max(1e-6);
        let c = bbox.center();
        let cx = c[0] + shift[0] * bw;
        let cy = c[1] + shift[1] * bh;
        View { scale, ox: w as f64 * 0.5 - scale * cx, oy: h as f64 * 0.5 + scale * cy, w, h }
    }

    #[inline]
    pub fn map(&self, p: [f64; 2]) -> [f64; 2] {
        [self.ox + self.scale * p[0], self.oy - self.scale * p[1]]
    }

    /// Build a view for an explicit world window (center + half-extents).
    pub fn from_window(center: [f64; 2], half: [f64; 2], w: u32, h: u32) -> View {
        let w = w.max(1);
        let h = h.max(1);
        let sx = w as f64 * 0.5 / half[0].max(1e-12);
        let sy = h as f64 * 0.5 / half[1].max(1e-12);
        let scale = sx.min(sy);
        View {
            scale,
            ox: w as f64 * 0.5 - scale * center[0],
            oy: h as f64 * 0.5 + scale * center[1],
            w,
            h,
        }
    }
}

/// World-space viewport: center plus half-extents, in world units.
///
/// This is the single source of truth for "what the camera sees" in every renderer and in
/// the interactive window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub center: [f64; 2],
    pub half: [f64; 2],
}

impl Window {
    pub fn new(center: [f64; 2], half: [f64; 2]) -> Self {
        Window { center, half: [half[0].max(1e-12), half[1].max(1e-12)] }
    }

    /// Window equivalent to `View::fit`: fit `bbox` with `pad`, `zoom` and a relative `shift`,
    /// keeping `aspect` = width/height.
    pub fn from_bbox(bbox: &Bbox, pad: f64, zoom: f64, shift: [f64; 2], aspect: f64) -> Self {
        let bw = bbox.width().max(1e-9);
        let bh = bbox.height().max(1e-9);
        let avail = (1.0 - 2.0 * pad.clamp(0.0, 0.45)).max(0.05);
        let zoom = zoom.max(1e-6);
        let aspect = aspect.max(1e-6);
        let mut half = [bw * 0.5 / (avail * zoom), bh * 0.5 / (avail * zoom)];
        if half[0] / half[1] < aspect {
            half[0] = half[1] * aspect;
        } else {
            half[1] = half[0] / aspect;
        }
        let c = bbox.center();
        Window::new([c[0] + shift[0] * bw, c[1] + shift[1] * bh], half)
    }

    pub fn view(&self, w: u32, h: u32) -> View {
        View::from_window(self.center, self.half, w, h)
    }

    /// Magnification relative to `bbox` (1.0 = the whole picture fits).
    pub fn zoom_of(&self, bbox: &Bbox) -> f64 {
        let bw = bbox.width().max(1e-9);
        (bw * 0.5 / self.half[0].max(1e-12)).max(1e-9)
    }

    /// Pan by a delta in world units.
    pub fn pan(&mut self, delta: [f64; 2]) {
        self.center[0] += delta[0];
        self.center[1] += delta[1];
    }

    /// Zoom by `factor` while keeping the world point `anchor` fixed on screen.
    pub fn zoom_around(&mut self, factor: f64, anchor: [f64; 2]) {
        let factor = factor.clamp(1e-3, 1e3);
        self.half = [self.half[0] / factor, self.half[1] / factor];
        self.center = [
            anchor[0] + (self.center[0] - anchor[0]) / factor,
            anchor[1] + (self.center[1] - anchor[1]) / factor,
        ];
    }
}

/// Additive RGB raster with `u16` headroom, so dense overdraw glows instead of clipping.
pub struct Raster {
    pub w: u32,
    pub h: u32,
    /// Theme the raster was created with (used for `reset` and for the stroke ramp).
    pub canvas: Canvas,
    buf: Vec<[u16; 3]>,
    /// Alpha coverage: each stroke paints over what is already there, which is what the GUI's
    /// `draw_line` does. Overlaps therefore keep their palette colour instead of summing up to
    /// white (deep rendering overdraws every pixel several times).
    alpha_blend: bool,
}

impl Raster {
    pub fn new(w: u32, h: u32, canvas: Canvas) -> Self {
        let b = palette::to_rgb8(canvas.background());
        Raster {
            w: w.max(1),
            h: h.max(1),
            canvas,
            buf: vec![[b[0] as u16, b[1] as u16, b[2] as u16]; w.max(1) as usize * h.max(1) as usize],
            alpha_blend: true,
        }
    }

    pub fn reset(&mut self) {
        let b = palette::to_rgb8(self.canvas.background());
        let px = [b[0] as u16, b[1] as u16, b[2] as u16];
        for slot in self.buf.iter_mut() {
            *slot = px;
        }
    }

    #[inline]
    fn add8(&mut self, x: i64, y: i64, c: [u8; 3], weight: f32) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = y as usize * self.w as usize + x as usize;
        let p = &mut self.buf[i];
        if self.alpha_blend {
            let a = weight.clamp(0.0, 1.0);
            for k in 0..3 {
                let over = c[k] as f32 * a + p[k] as f32 * (1.0 - a);
                p[k] = over.round().clamp(0.0, 65535.0) as u16;
            }
        } else {
            for k in 0..3 {
                p[k] = p[k].saturating_add((c[k] as f32 * weight) as u16);
            }
        }
    }

    /// Switch between alpha coverage (default, matches the GUI) and additive glow.
    pub fn set_alpha_blend(&mut self, on: bool) {
        self.alpha_blend = on;
    }

    pub fn add_px(&mut self, x: i64, y: i64, color: [f32; 3], weight: f32) {
        self.add8(x, y, palette::to_rgb8(color), weight);
    }

    /// 3x3 soft kernel: the cheap anti-aliasing of the whole rasteriser.
    #[inline]
    fn kernel(&mut self, x: i64, y: i64, c: [u8; 3], gain: f32) {
        self.add8(x, y, c, 1.00 * gain);
        self.add8(x - 1, y, c, 0.34 * gain);
        self.add8(x + 1, y, c, 0.34 * gain);
        self.add8(x, y - 1, c, 0.34 * gain);
        self.add8(x, y + 1, c, 0.34 * gain);
        self.add8(x - 1, y - 1, c, 0.12 * gain);
        self.add8(x + 1, y - 1, c, 0.12 * gain);
        self.add8(x - 1, y + 1, c, 0.12 * gain);
        self.add8(x + 1, y + 1, c, 0.12 * gain);
    }

    /// Anti-aliased line in pixel coordinates.
    pub fn segment(&mut self, a: [f64; 2], b: [f64; 2], color: [f32; 3], gain: f32) {
        if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) {
            return;
        }
        let (w, h) = (self.w as f64, self.h as f64);
        let Some((a, b)) = clip_to_rect(a, b, w, h) else {
            return;
        };
        let c = palette::to_rgb8(color);
        let mut x0 = a[0].round() as i64;
        let mut y0 = a[1].round() as i64;
        let x1 = b[0].round() as i64;
        let y1 = b[1].round() as i64;
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.kernel(x0, y0, c, gain);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Box-downsample by `ss` and return RGB8.
    pub fn downsample(&self, ss: u32) -> (u32, u32, Vec<u8>) {
        let ss = ss.clamp(1, 4);
        let (ow, oh) = ((self.w / ss).max(1), (self.h / ss).max(1));
        let mut out = vec![0u8; ow as usize * oh as usize * 3];
        let inv = 1.0 / (ss * ss) as f64;
        for oy in 0..oh {
            for ox in 0..ow {
                let mut acc = [0u32; 3];
                for dy in 0..ss {
                    for dx in 0..ss {
                        let i = (oy * ss + dy) as usize * self.w as usize + (ox * ss + dx) as usize;
                        for (slot, value) in acc.iter_mut().zip(&self.buf[i]) {
                            *slot += *value as u32;
                        }
                    }
                }
                let o = (oy as usize * ow as usize + ox as usize) * 3;
                for (k, value) in acc.iter().enumerate() {
                    out[o + k] = (*value as f64 * inv).min(255.0) as u8;
                }
            }
        }
        (ow, oh, out)
    }
}

/// Liang-Barsky clip to the raster rectangle.
fn clip_to_rect(a: [f64; 2], b: [f64; 2], w: f64, h: f64) -> Option<([f64; 2], [f64; 2])> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = [b[0] - a[0], b[1] - a[1]];
    let checks = [(-d[0], a[0]), (d[0], w - a[0]), (-d[1], a[1]), (d[1], h - a[1])];
    for (p, q) in checks {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    let at = |t: f64| [a[0] + t * d[0], a[1] + t * d[1]];
    Some((at(t0), at(t1)))
}

/// One pass over a range of segments: skip factor plus appearance modifiers.
#[derive(Clone, Copy, Debug)]
pub struct Stroke {
    pub stride: usize,
    pub hue_shift: f32,
    pub gain: f32,
}

/// Stroke segments `[from, to)` of the curve. `stride > 1` skips segments by drawing a
/// chord across them (no gaps, keeps huge step counts cheap).
pub fn stroke_range(
    raster: &mut Raster,
    curve: &Curve,
    view: &View,
    from: usize,
    to: usize,
    stroke: Stroke,
) {
    let total = curve.segments().max(1);
    let stride = stroke.stride.max(1);
    let to = to.min(curve.segments());
    let mut i = from.min(to);
    while i < to {
        let j = (i + stride).min(to);
        let color = raster.canvas.ramp(i as f32 / total as f32, stroke.hue_shift);
        raster.segment(view.map(curve.pts[i]), view.map(curve.pts[j]), color, stroke.gain);
        i = j;
    }
}

/// Automation-friendly stride: keep at most ~`max_segments` strokes on screen.
pub fn auto_stride(segments: usize, max_segments: usize) -> usize {
    (segments / max_segments.max(1)).max(1)
}

/// Turtle marker: a dot at vertex `idx` plus a tick along the heading.
pub fn turtle_marker(raster: &mut Raster, curve: &Curve, view: &View, idx: usize, radius_px: f64) {
    if curve.pts.is_empty() {
        return;
    }
    let last = curve.pts.len() - 1;
    let idx = idx.min(last);
    let p = view.map(curve.pts[idx]);
    let r = radius_px.max(1.0);
    let ri = r.ceil() as i64;
    let (cx, cy) = (p[0].round() as i64, p[1].round() as i64);
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            let d = ((dx * dx + dy * dy) as f64).sqrt();
            if d <= r {
                let w = (1.0 - 0.55 * (d / r)) as f32;
                raster.add_px(cx + dx, cy + dy, [1.0, 1.0, 1.0], w);
            }
        }
    }
    let head = curve.heading_at(idx.min(curve.segments().saturating_sub(1)));
    let tip = [p[0] + head.cos() * r * 2.6, p[1] - head.sin() * r * 2.6];
    raster.segment(p, tip, [1.0, 0.95, 0.75], 1.1);
}

#[derive(Clone, Copy, Debug)]
pub struct RenderOpts {
    pub canvas: Canvas,
    pub width: u32,
    pub height: u32,
    pub ss: u32,
    pub pad: f64,
    pub hue_shift: f32,
    pub gain: f32,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts {
            canvas: Canvas::Dark,
            width: 1600,
            height: 1200,
            ss: 2,
            pad: 0.06,
            hue_shift: 0.0,
            gain: 1.0,
        }
    }
}

/// One-shot render of a curve (all segments, or the first `visible`) into an explicit window.
pub fn render_curve_window(
    curve: &Curve,
    width: u32,
    height: u32,
    ss: u32,
    win: &Window,
    opts: &RenderOpts,
    visible: Option<usize>,
) -> (u32, u32, Vec<u8>) {
    let ss = ss.clamp(1, 4);
    let (w, h) = (width.max(1), height.max(1));
    let mut raster = Raster::new(w * ss, h * ss, opts.canvas);
    let view = win.view(w * ss, h * ss);
    let to = visible.unwrap_or(curve.segments()).min(curve.segments());
    let stride = auto_stride(to, 400_000);
    stroke_range(
        &mut raster,
        curve,
        &view,
        0,
        to,
        Stroke { stride, hue_shift: opts.hue_shift, gain: opts.gain },
    );
    raster.downsample(ss)
}

/// Same render, framed by the curve's bounding box.
pub fn render_curve(curve: &Curve, opts: &RenderOpts, visible: Option<usize>) -> (u32, u32, Vec<u8>) {
    let win = Window::from_bbox(
        &curve.bbox,
        opts.pad,
        1.0,
        [0.0, 0.0],
        opts.width.max(1) as f64 / opts.height.max(1) as f64,
    );
    render_curve_window(curve, opts.width, opts.height, opts.ss, &win, opts, visible)
}

/// A dot plus a soft halo, at an explicit world position (deep-mode turtle marker).
pub fn marker_dot(raster: &mut Raster, view: &View, world: [f64; 2], radius_px: f64) {
    let p = view.map(world);
    if !(p[0].is_finite() && p[1].is_finite()) {
        return;
    }
    let r = radius_px.max(1.0);
    let ri = r.ceil() as i64;
    let (cx, cy) = (p[0].round() as i64, p[1].round() as i64);
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            let d = ((dx * dx + dy * dy) as f64).sqrt();
            if d <= r * 2.2 {
                let weight = if d <= r { 1.0 } else { 0.25 };
                raster.add_px(cx + dx, cy + dy, [1.0, 0.85, 0.45], weight);
            }
        }
    }
}

/// Vector export framed by an explicit window.
pub fn curve_svg_window(
    curve: &Curve,
    width: u32,
    height: u32,
    win: &Window,
    max_points: usize,
    hue_shift: f32,
    canvas: Canvas,
) -> String {
    let (w, h) = (width.max(1), height.max(1));
    let view = win.view(w, h);
    let segments = curve.segments();
    let stride = auto_stride(segments, max_points.max(1));
    let mut points = String::with_capacity(segments.min(max_points.max(1)) * 16 + 64);
    let mut i = 0;
    while i <= segments {
        let p = view.map(curve.pts[i]);
        points.push_str(&format!("{:.2},{:.2} ", p[0], p[1]));
        if i == segments {
            break;
        }
        i = (i + stride).min(segments);
    }
    let mut stops = String::new();
    for k in 0..=8 {
        let t = k as f32 / 8.0;
        stops.push_str(&format!(
            r#"<stop offset="{:.1}%" stop-color="{}"/>"#,
            t * 100.0,
            palette::hex(canvas.ramp(t, hue_shift))
        ));
    }
    let bg = palette::hex(canvas.background());
    let p = &curve.params;
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            "\n<!-- number-system turtle: base={b} dir={d} steps={n} turn/digit={turn:.4} deg -->\n",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
            "\n",
            r#"<desc>base {b}, direction number {d}, {n} unit segments, stride {stride}</desc>"#,
            "\n",
            r#"<rect width="{w}" height="{h}" fill="{bg}"/>"#,
            "\n",
            r#"<defs><linearGradient id="g" gradientUnits="objectBoundingBox" x1="0" y1="0" x2="1" y2="1">{stops}</linearGradient></defs>"#,
            "\n",
            r#"<polyline fill="none" stroke="url(#g)" stroke-width="1.1" stroke-linejoin="round" points="{points}"/>"#,
            "\n</svg>\n"
        ),
        b = p.base,
        d = p.dir,
        n = p.steps,
        turn = p.turn_per_digit_deg(),
        stride = stride,
        w = w,
        h = h,
        bg = bg,
        stops = stops,
        points = points,
    )
}

pub fn ensure_parent_dir(path: &str) -> io::Result<()> {    if let Some(dir) = Path::new(path).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    Ok(())
}

pub fn write_file(path: &str, bytes: &[u8]) -> io::Result<()> {
    ensure_parent_dir(path)?;
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::{generate, Params};

    #[test]
    fn view_fits_and_flips_y() {
        let bbox = Bbox { min_x: 0.0, min_y: 0.0, max_x: 10.0, max_y: 5.0 };
        let v = View::fit(&bbox, 100, 100, 0.0, 1.0, [0.0, 0.0]);
        assert!((v.scale - 10.0).abs() < 1e-9);
        let p = v.map([0.0, 0.0]);
        assert!((p[0] - 0.0).abs() < 1e-9 && (p[1] - 75.0).abs() < 1e-9, "{p:?}");
        let q = v.map([10.0, 5.0]);
        assert!((q[0] - 100.0).abs() < 1e-9 && (q[1] - 25.0).abs() < 1e-9, "{q:?}");
    }

    #[test]
    fn degenerate_bbox_does_not_blow_up() {
        let bbox = Bbox { min_x: 0.0, min_y: 3.0, max_x: 2000.0, max_y: 3.0 };
        let v = View::fit(&bbox, 800, 600, 0.06, 1.0, [0.0, 0.0]);
        assert!(v.scale.is_finite() && v.scale > 0.0);
        assert!(v.ox.is_finite() && v.oy.is_finite());
    }

    #[test]
    fn render_produces_expected_dimensions_and_ink() {
        let curve = generate(&Params::new(2, 4, 4096));
        let opts = RenderOpts { width: 64, height: 48, ss: 2, ..Default::default() };
        let _ = opts.canvas;
        let (w, h, rgb) = render_curve(&curve, &opts, None);
        assert_eq!((w, h), (64, 48));
        assert_eq!(rgb.len(), 64 * 48 * 3);
        let bright = rgb.chunks(3).filter(|p| p.iter().any(|&v| v > 90)).count();
        assert!(bright > 200, "expected visible strokes, got {bright} bright pixels");
    }

    #[test]
    fn light_canvas_render_differs_from_dark() {
        let curve = generate(&Params::new(2, 4, 4096));
        let dark = RenderOpts { width: 64, height: 48, ss: 1, ..Default::default() };
        let light = RenderOpts { canvas: Canvas::Light, ..dark };
        let (_, _, a) = render_curve(&curve, &dark, None);
        let (_, _, b) = render_curve(&curve, &light, None);
        assert_ne!(a, b);
        let first_pixel = |rgb: &[u8]| rgb[..3].iter().map(|v| *v as u32).sum::<u32>();
        assert!(first_pixel(&a) < 40, "dark background, got {}", first_pixel(&a));
        assert!(first_pixel(&b) > 600, "light background, got {}", first_pixel(&b));
    }

    #[test]
    fn clip_rejects_far_away_segments() {
        assert!(clip_to_rect([-50.0, -50.0], [-10.0, -10.0], 100.0, 100.0).is_none());
        assert!(clip_to_rect([10.0, 10.0], [90.0, 90.0], 100.0, 100.0).is_some());
    }

    #[test]
    fn svg_contains_a_polyline_with_expected_point_count() {
        let curve = generate(&Params::new(3, 5, 1024));
        let win = Window::from_bbox(&curve.bbox, 0.06, 1.0, [0.0, 0.0], 4.0 / 3.0);
        let svg = curve_svg_window(&curve, 400, 300, &win, 200, 0.0, Canvas::Dark);
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("base=3 dir=5 steps=1024"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn window_zoom_and_pan_keep_anchors() {
        let bbox = Bbox { min_x: 0.0, min_y: 0.0, max_x: 10.0, max_y: 10.0 };
        let mut win = Window::from_bbox(&bbox, 0.0, 1.0, [0.0, 0.0], 1.0);
        assert!((win.zoom_of(&bbox) - 1.0).abs() < 1e-9);
        let anchor = [2.0, 3.0];
        win.zoom_around(2.0, anchor);
        assert!((win.zoom_of(&bbox) - 2.0).abs() < 1e-9);
        // The anchor must stay put relative to the window.
        let before = [2.0 - 5.0, 3.0 - 5.0];
        let after = [anchor[0] - win.center[0], anchor[1] - win.center[1]];
        for k in 0..2 {
            assert!((after[k] - before[k] / 2.0).abs() < 1e-9, "{before:?} {after:?}");
        }
        win.pan([1.0, -1.0]);
        // Anchored zoom: center = anchor + (center - anchor) / 2, then pan.
        assert!((win.center[0] - 4.5).abs() < 1e-9, "{:?}", win.center);
        assert!((win.center[1] - 3.0).abs() < 1e-9, "{:?}", win.center);
    }
}
