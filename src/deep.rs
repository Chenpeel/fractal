//! Adaptive renderer for step counts that cannot be enumerated.
//!
//! The block identity `z(m*b^L + j) = S^L*z(m) + zeta^(s_b(m))*z(j)` turns the curve into a
//! tree: every node is a rotated, translated copy of a shorter walk, and its bounding disk
//! grows like `|S|^level`. The renderer descends that tree from the top, skips any node whose
//! disk misses the viewport, and stops when a node is smaller than a pixel.
//!
//! Cost therefore depends on the viewport, not on the step count: `2^50` steps render in the
//! same time as `2^20`, and zooming into a deep level stays bounded (only the visible blocks
//! are visited). Precision is `f64`: beyond ~`1e7` magnification the unit decorations start
//! to be lost to rounding, so that is the practical zoom ceiling.

use crate::fractal::{self, cmul, cpow, Params, MAX_STEPS};

/// Per-stroke ink in deep mode. Deep rendering switches the raster to max blending, so this only
/// shapes the soft edge; overlaps no longer accumulate into white.
const STROKE_INK: f32 = 0.95;
/// Beads below this radius are redundant: the chain already passes through their origin.
const MIN_BEAD_PX: f64 = 2.0;
use crate::palette::Canvas;
use crate::render::{Raster, Window};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    pub nodes: u64,
    pub leaves: u64,
    pub stamps: u64,
    /// The traversal hit its node budget and stopped early.
    pub truncated: bool,
    /// Last drawn position (the turtle's tip, approximately).
    pub frontier: Option<[f64; 2]>,
}

#[derive(Clone, Copy, Debug)]
pub struct DeepOpts {
    pub canvas: Canvas,
    pub steps: u64,
    /// Step count used for the colour ramp (kept constant while a trace animates).
    pub color_total: u64,
    pub hue_shift: f32,
    pub ink: f32,
    pub budget_nodes: u64,
    /// Sub-pixel cutoff, in device pixels.
    pub cut_px: f64,
}

impl DeepOpts {
    pub fn for_steps(steps: u64, ss: u32) -> Self {
        DeepOpts {
            canvas: Canvas::Dark,
            steps,
            color_total: steps,
            hue_shift: 0.0,
            ink: 1.0,
            budget_nodes: 6_000_000,
            cut_px: 1.5 * ss.max(1) as f64,
        }
    }
}

struct Ctx<'a> {
    base: u32,
    color_total: u64,
    zeta_pow: Vec<[f64; 2]>,
    /// `S^(level-1) * c_r`: the unrotated offset of child `r` inside a level node.
    child_offset: Vec<Vec<[f64; 2]>>,
    /// Upper bound on `|z(j)|` for `j <= b^level`, indexed by level.
    radius_by_level: Vec<f64>,
    colors: Vec<[f32; 3]>,
    view: &'a crate::render::View,
    raster: &'a mut Raster,
    opts: &'a DeepOpts,
    stats: Stats,
    /// Grows as the node budget is consumed, so a finished picture prefers coarse dots over
    /// stopping halfway.
    cut_scale: f64,
    /// End of the path drawn so far (world units): sub-pixel blocks are stamped, and this keeps
    /// the stroke continuous by connecting consecutive stamps.
    last: Option<[f64; 2]>,
}

impl Ctx<'_> {
    /// Upper bound on `|z(j)|` for `j <= min(b^level, remaining)` inside a node. Every step is a
    /// unit vector, so a partial node is also bounded by its own step count.
    #[inline]
    fn radius(&self, level: u32, remaining: u64) -> f64 {
        self.radius_by_level[level as usize].min(remaining as f64)
    }

    fn on_screen(&self, p: [f64; 2], r_px: f64) -> bool {
        p[0] > -r_px
            && p[1] > -r_px
            && p[0] < self.view.w as f64 + r_px
            && p[1] < self.view.h as f64 + r_px
    }

    fn stamp(&mut self, p: [f64; 2], r_px: f64, color: [f32; 3]) {
        let radius = r_px.max(0.8);
        let ri = radius.ceil() as i64;
        let (cx, cy) = (p[0].round() as i64, p[1].round() as i64);
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let d = ((dx * dx + dy * dy) as f64).sqrt();
                if d <= radius {
                    let w = self.opts.ink * STROKE_INK * (1.0 - 0.4 * (d / radius)) as f32;
                    self.raster.add_px(cx + dx, cy + dy, color, w);
                }
            }
        }
    }

    fn visit(&mut self, level: u32, remaining: u64, n0: u64, t: [f64; 2], rot: [f64; 2]) {
        if remaining == 0 || self.stats.truncated {
            return;
        }
        let radius = self.radius(level, remaining);
        let center_px = self.view.map(t);
        let r_px = radius * self.view.scale;
        if !self.on_screen(center_px, r_px + 2.0) {
            self.last = None;
            return;
        }
        self.stats.nodes += 1;
        if self.stats.nodes > self.opts.budget_nodes {
            self.stats.truncated = true;
            return;
        }
        // Approaching the budget: stamp bigger dots instead of leaving the rest undrawn.
        if self.stats.nodes * 2 > self.opts.budget_nodes {
            self.cut_scale = 8.0;
        } else if self.stats.nodes * 4 > self.opts.budget_nodes {
            self.cut_scale = 4.0;
        } else if self.stats.nodes * 8 > self.opts.budget_nodes {
            self.cut_scale = 2.0;
        }
        let idx = ((n0 as f64 / self.color_total.max(1) as f64) * 255.0).clamp(0.0, 255.0) as usize;
        let color = self.colors[idx];

        if r_px <= self.opts.cut_px * self.cut_scale {
            self.stats.stamps += 1;
            // Connect to the previous stamp: consecutive stamps are adjacent blocks of the path,
            // so this is the same curve at a coarser resolution instead of a broken bead chain.
            let ink = self.opts.ink * STROKE_INK;
            if let Some(previous) = self.last {
                self.raster.segment(self.view.map(previous), center_px, color, ink);
            }
            if r_px >= MIN_BEAD_PX {
                self.stamp(center_px, r_px, color);
            }
            self.last = Some(t);
            self.stats.frontier = Some(t);
            return;
        }
        if level == 0 {
            self.stats.leaves += 1;
            let ink = self.opts.ink * STROKE_INK;
            if let Some(previous) = self.last {
                self.raster.segment(self.view.map(previous), center_px, color, ink);
            }
            let end = [t[0] + rot[0], t[1] + rot[1]];
            self.raster.segment(center_px, self.view.map(end), color, ink);
            self.last = Some(end);
            self.stats.frontier = Some(end);
            return;
        }

        let sub = fractal::level_steps(self.base, level - 1);
        for r in 0..self.base as usize {
            let start = r as u64 * sub;
            if start >= remaining {
                break;
            }
            let rem = (remaining - start).min(sub);
            let base_offset = self.child_offset[level as usize][r];
            let child_rot_unit = self.zeta_pow[r];
            let offset = cmul(rot, base_offset);
            let child_t = [t[0] + offset[0], t[1] + offset[1]];
            let child_rot = cmul(rot, child_rot_unit);
            self.visit(level - 1, rem, n0 + start, child_t, child_rot);
        }
    }
}

/// Render `steps` segments of the walk into `raster` for the given world window.
pub fn render_into(
    raster: &mut Raster,
    params: &Params,
    win: &Window,
    opts: &DeepOpts,
) -> Stats {
    let p = params.clamped();
    let steps = opts.steps.clamp(1, MAX_STEPS);
    let level = fractal::steps_to_level(p.base, steps);
    let zeta = fractal::zeta(p.dir);
    let s = fractal::scale_factor_rule(p.base, p.dir, p.rule);
    let mut zeta_pow = Vec::with_capacity(p.base as usize + 1);
    let mut c_prefix = Vec::with_capacity(p.base as usize + 1);
    let mut max_c = 0.0f64;
    for r in 0..=p.base as usize {
        zeta_pow.push(cpow(zeta, p.rule.weight(r as u32, p.base)));
        let c = fractal::block_prefix_rule(p.base, p.dir, p.rule, r as u64);
        max_c = max_c.max((c[0] * c[0] + c[1] * c[1]).sqrt());
        c_prefix.push(c);
    }
    let max_c = max_c.max(1.0);
    let gain = fractal::gain_rule(p.base, p.dir, p.rule);

    // `S^(level-1) * c_r` per level and child: the only geometry the traversal needs.
    let mut child_offset: Vec<Vec<[f64; 2]>> = Vec::with_capacity(level as usize + 1);
    child_offset.push(vec![[0.0, 0.0]; p.base as usize + 1]);
    for l in 1..=level {
        let s_pow = cpow(s, l as i64 - 1);
        child_offset.push(c_prefix.iter().map(|c| cmul(s_pow, *c)).collect());
    }
    // Per-level bounding radii (no `powi` in the hot loop).
    let radius_by_level: Vec<f64> = (0..=level)
        .map(|l| {
            if gain > 1.0 + 1e-12 {
                max_c * (gain.powi(l as i32 + 1) - 1.0) / (gain - 1.0)
            } else {
                max_c * p.base as f64 * (l as f64 + 1.0)
            }
        })
        .collect();
    let colors: Vec<[f32; 3]> =
        (0..256).map(|i| opts.canvas.ramp(i as f32 / 255.0, opts.hue_shift)).collect();

    let view = win.view(raster.w, raster.h);
    let mut ctx = Ctx {
        base: p.base,
        color_total: opts.color_total.max(1),
        zeta_pow,
        child_offset,
        radius_by_level,
        colors,
        view: &view,
        raster,
        opts,
        stats: Stats { nodes: 0, leaves: 0, stamps: 0, truncated: false, frontier: None },
        cut_scale: 1.0,
        last: None,
    };
    ctx.visit(level, steps, 0, [0.0, 0.0], [1.0, 0.0]);
    ctx.stats
}

/// Convenience wrapper: supersampled image of the window.
pub fn render_image(
    params: &Params,
    width: u32,
    height: u32,
    ss: u32,
    win: &Window,
    opts: &DeepOpts,
) -> (u32, u32, Vec<u8>, Stats) {
    let ss = ss.clamp(1, 4);
    let (w, h) = (width.max(1), height.max(1));
    let mut raster = Raster::new(w * ss, h * ss, opts.canvas);
    let mut local = *opts;
    local.cut_px = opts.cut_px.max(1.0) * ss as f64;
    let stats = render_into(&mut raster, params, win, &local);
    let (iw, ih, rgb) = raster.downsample(ss);
    (iw, ih, rgb, stats)
}

/// Steps enumerated to estimate the framing box. One million vertices is a few tens of
/// milliseconds and keeps the coarse-shape extrapolation accurate to a fraction of a percent.
pub const FRAME_REF_STEPS: u64 = 1 << 20;

/// Framing box for a 2D viewport: enumerate a reference prefix and scale/rotate it by the
/// self-similarity that the skipped levels add. Meant for camera framing, where a few percent of
/// error is irrelevant, and exact when `steps <= FRAME_REF_STEPS`.
pub fn frame_bbox(params: &Params, steps: u64) -> fractal::Bbox {
    let p = params.clamped();
    let reference_steps = steps.clamp(1, FRAME_REF_STEPS);
    let curve = fractal::generate(&Params { steps: reference_steps, ..p });
    let mut bbox = curve.bbox;
    if steps > reference_steps {
        let levels = (steps as f64 / reference_steps as f64).ln() / (p.base as f64).ln();
        let s = fractal::scale_factor_rule(p.base, p.dir, p.rule);
        let mag = fractal::gain_rule(p.base, p.dir, p.rule).max(1e-9).powf(levels);
        let angle = s[1].atan2(s[0]) * levels;
        let (c, sn) = (angle.cos(), angle.sin());
        let corners = [
            [bbox.min_x, bbox.min_y],
            [bbox.max_x, bbox.min_y],
            [bbox.min_x, bbox.max_y],
            [bbox.max_x, bbox.max_y],
        ];
        let map = |q: [f64; 2]| {
            let (x, y) = (q[0] * mag, q[1] * mag);
            [x * c - y * sn, x * sn + y * c]
        };
        let first = map(corners[0]);
        let mut out = fractal::Bbox::point(first[0], first[1]);
        for corner in corners.iter().skip(1) {
            let q = map(*corner);
            out.grow(q[0], q[1]);
        }
        bbox = out;
    }
    // A little padding: the coarse-shape estimate does not include unit-level decorations.
    let pad = (bbox.width().max(bbox.height()) * 0.02).max(1.0);
    bbox.min_x -= pad;
    bbox.min_y -= pad;
    bbox.max_x += pad;
    bbox.max_y += pad;
    bbox
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::generate;
    use crate::render::{render_curve, RenderOpts};

    fn ink_mask(rgb: &[u8]) -> Vec<bool> {
        rgb.chunks(3).map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 90).collect()
    }

    /// Symmetric coverage within one pixel: "do both renderers ink the same curve?" It is robust
    /// to stroke width, which matters when unit segments are sub-pixel (thin curves).
    fn coverage(a: &[bool], b: &[bool], w: usize, h: usize) -> f64 {
        let near = |mask: &[bool], other: &[bool]| {
            let total = mask.iter().filter(|v| **v).count().max(1);
            let mut hit = 0;
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    if !mask[(y * w as i64 + x) as usize] {
                        continue;
                    }
                    let mut ok = false;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (nx, ny) = (x + dx, y + dy);
                            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                                continue;
                            }
                            if other[(ny * w as i64 + nx) as usize] {
                                ok = true;
                            }
                        }
                    }
                    if ok {
                        hit += 1;
                    }
                }
            }
            hit as f64 / total as f64
        };
        (near(a, b) + near(b, a)) / 2.0
    }

    /// Deep levels with a non-linear rule too: a window centred on `z(m*4^j) = 3^j z(m)` (a point
    /// on the Koch arc) must contain ink, and the block size there grows like `|S|^j`.
    #[test]
    fn koch_arc_is_drawn_at_deep_levels() {
        use crate::fractal::DigitRule;
        let steps = 1u64 << 50;
        let p = Params::new(4, 6, steps).with_rule(DigitRule::Fold);
        let s = fractal::scale_factor_rule(4, 6, DigitRule::Fold);
        assert!((fractal::gain_rule(4, 6, DigitRule::Fold) - 3.0).abs() < 1e-12);
        let prefix = generate(&Params::new(4, 6, 1 << 17).with_rule(DigitRule::Fold));
        for (j, m) in [(3u32, 7u64), (6, 55), (9, 1000), (12, 30000), (15, 60000)] {
            let n = m * 4u64.pow(j);
            assert!(n < steps, "j={j} m={m} index {n} outside the curve");
            let centre = cmul(cpow(s, j as i64), prefix.pts[m as usize]);
            // Window spans about 30 blocks of this level, which is where the structure lives.
            let half = 30.0 * 3.0f64.powi(j as i32);
            let win = Window::new(centre, [half, half]);
            let (_, _, rgb, stats) = render_image(&p, 160, 160, 1, &win, &DeepOpts::for_steps(steps, 1));
            let ink = ink_mask(&rgb).iter().filter(|v| **v).count();
            assert!(!stats.truncated, "j={j} m={m} truncated at {} nodes", stats.nodes);
            assert!(ink > 150, "j={j} m={m} centre={centre:?} ink={ink} nodes={}", stats.nodes);
        }
    }

    #[test]
    fn frame_bbox_tracks_the_real_extent() {
        for (b, d) in [(2u32, 4i32), (3, 5), (4, 7)] {
            for steps in [1u64 << 16, 1 << 18, 1 << 20] {
                let exact = generate(&Params::new(b, d, steps)).bbox;
                let framed = frame_bbox(&Params::new(b, d, steps), steps);
                assert!(
                    framed.min_x <= exact.min_x + 1e-6 && framed.max_x >= exact.max_x - 1e-6,
                    "b={b} d={d} steps={steps}: framed {framed:?} misses {exact:?}"
                );
                assert!(
                    framed.min_y <= exact.min_y + 1e-6 && framed.max_y >= exact.max_y - 1e-6,
                    "b={b} d={d} steps={steps}: framed {framed:?} misses {exact:?}"
                );
                let ratio = framed.width() / exact.width().max(1e-9);
                assert!(ratio < 4.0, "b={b} d={d} steps={steps}: framed {ratio:.2}x too big");
            }
        }
    }

    /// Deep levels: a unit-scale window centred on a known curve point must be full of ink.
    /// `z(m * b^j) = S^j * z(m)`, so these centres are exactly on the curve. This is the
    /// "no black holes" check for the deepest levels.
    #[test]
    fn ink_appears_where_deep_blocks_are() {
        let steps = 1u64 << 50;
        let p = Params::new(2, 4, steps);
        let s = fractal::scale_factor(2, 4);
        let prefix = generate(&Params::new(2, 4, 1 << 16));
        for (j, m) in [(8u32, 7u64), (16, 123), (24, 1234), (30, 12345), (34, 32768)] {
            let n = m << j;
            assert!(n < steps, "j={j} m={m} index {n} is outside the curve");
            let centre = cmul(cpow(s, j as i64), prefix.pts[m as usize]);
            let win = Window::new(centre, [30.0, 30.0]);
            let (_, _, rgb, stats) =
                render_image(&p, 160, 160, 1, &win, &DeepOpts::for_steps(steps, 1));
            let ink = ink_mask(&rgb).iter().filter(|v| **v).count();
            assert!(!stats.truncated, "j={j} m={m} truncated");
            assert!(
                ink > 500,
                "j={j} m={m} n={n} centre={centre:?} ink={ink} nodes={} stamps={} leaves={}",
                stats.nodes,
                stats.stamps,
                stats.leaves
            );
        }
    }

    #[test]
    fn frame_bbox_handles_deep_levels() {
        let steps = 1u64 << 50;
        let p = Params::new(2, 4, steps);
        let framed = frame_bbox(&p, steps);
        let reference = generate(&Params::new(2, 4, FRAME_REF_STEPS)).bbox;
        let k = 30.0f64;
        let mag = fractal::gain(2, 4).powf(k);
        let s = fractal::scale_factor(2, 4);
        let angle = s[1].atan2(s[0]) * k;
        let (c, sn) = (angle.cos(), angle.sin());
        let map = |p: [f64; 2]| {
            let (x, y) = (p[0] * mag, p[1] * mag);
            [x * c - y * sn, x * sn + y * c]
        };
        let corners = [
            [reference.min_x, reference.min_y],
            [reference.max_x, reference.min_y],
            [reference.min_x, reference.max_y],
            [reference.max_x, reference.max_y],
        ];
        let first = map(corners[0]);
        let mut hull = fractal::Bbox::point(first[0], first[1]);
        for corner in corners.iter().skip(1) {
            let q = map(*corner);
            hull.grow(q[0], q[1]);
        }
        let slack = hull.width() * 0.05;
        assert!(framed.min_x <= hull.min_x + slack && framed.max_x >= hull.max_x - slack);
        assert!(framed.min_y <= hull.min_y + slack && framed.max_y >= hull.max_y - slack);
        assert!(framed.width() < hull.width() * 1.25, "framing too loose");
    }

    #[test]
    fn deep_matches_enumeration() {
        let (w, h) = (240u32, 200u32);
        for (rule, b, d, steps) in [
            (crate::fractal::DigitRule::Linear, 2u32, 4i32, 4096u64),
            (crate::fractal::DigitRule::Linear, 3, 5, 2187),
            (crate::fractal::DigitRule::Linear, 5, 8, 625),
            (crate::fractal::DigitRule::Linear, 2, -4, 4096),
            (crate::fractal::DigitRule::Alternate, 3, 5, 2187),
            (crate::fractal::DigitRule::Fold, 4, 6, 4096),
            (crate::fractal::DigitRule::Fold, 4, 8, 1024),
        ] {
            let params = Params::new(b, d, steps).with_rule(rule);
            let curve = generate(&params);
            let (_, _, rgb_enum) =
                render_curve(&curve, &RenderOpts { width: w, height: h, ss: 1, ..Default::default() }, None);
            let win = Window::from_bbox(&curve.bbox, 0.06, 1.0, [0.0, 0.0], w as f64 / h as f64);
            let opts = DeepOpts::for_steps(steps, 1);
            let (_, _, rgb_deep, stats) = render_image(&params, w, h, 1, &win, &opts);
            let score = coverage(&ink_mask(&rgb_enum), &ink_mask(&rgb_deep), w as usize, h as usize);
            assert!(!stats.truncated, "{rule:?} b={b} d={d} truncated");
            // Thin curves are legitimately drawn as sub-pixel dots at this resolution.
            assert!(
                stats.leaves + stats.stamps > 100,
                "{rule:?} b={b} d={d} leaves={} stamps={}",
                stats.leaves,
                stats.stamps
            );
            assert!(score > 0.9, "{rule:?} b={b} d={d} coverage={score:.3}");
        }
    }

    #[test]
    fn deep_matches_enumeration_at_a_big_level() {
        use crate::fractal::DigitRule;
        let (w, h) = (300u32, 200u32);
        for (b, d, steps) in [(4u32, 6i32, 1u64 << 20), (4, 6, (1 << 22) - 1), (2, 4, 1 << 20)] {
            let rule = if b == 4 { DigitRule::Fold } else { DigitRule::Linear };
            let params = Params::new(b, d, steps).with_rule(rule);
            let curve = generate(&params);
            let win = Window::from_bbox(&curve.bbox, 0.06, 1.0, [0.0, 0.0], w as f64 / h as f64);
            let (_, _, rgb_enum) = render_curve(
                &curve,
                &RenderOpts { width: w, height: h, ss: 1, ..Default::default() },
                None,
            );
            let (_, _, rgb_deep, stats) =
                render_image(&params, w, h, 1, &win, &DeepOpts::for_steps(steps, 1));
            let cov = coverage(
                &ink_mask(&rgb_enum),
                &ink_mask(&rgb_deep),
                w as usize,
                h as usize,
            );
            eprintln!(
                "b={b} d={d} steps={steps}: coverage={cov:.3} nodes={} stamps={} leaves={} truncated={}",
                stats.nodes, stats.stamps, stats.leaves, stats.truncated
            );
            assert!(cov > 0.9, "b={b} d={d} steps={steps} coverage={cov:.3}");
        }
    }

    /// 4-neighbour connected components of an ink mask.
    fn components(mask: &[bool], w: usize, h: usize) -> usize {
        let mut seen = vec![false; mask.len()];
        let mut count = 0;
        let mut stack: Vec<usize> = Vec::new();
        for start in 0..mask.len() {
            if !mask[start] || seen[start] {
                continue;
            }
            count += 1;
            seen[start] = true;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let (x, y) = (i % w, i / w);
                if x > 0 && !seen[i - 1] && mask[i - 1] {
                    seen[i - 1] = true;
                    stack.push(i - 1);
                }
                if x + 1 < w && !seen[i + 1] && mask[i + 1] {
                    seen[i + 1] = true;
                    stack.push(i + 1);
                }
                if y > 0 && !seen[i - w] && mask[i - w] {
                    seen[i - w] = true;
                    stack.push(i - w);
                }
                if y + 1 < h && !seen[i + w] && mask[i + w] {
                    seen[i + w] = true;
                    stack.push(i + w);
                }
            }
        }
        count
    }

    #[test]
    fn deep_stroke_is_continuous() {
        use crate::fractal::DigitRule;
        let (w, h) = (400u32, 300u32);
        for (b, d, steps, rule) in [
            (4u32, 6i32, 1u64 << 30, DigitRule::Fold),
            (2, 4, 1 << 26, DigitRule::Linear),
            (2, 5, 1 << 24, DigitRule::Linear),
        ] {
            let params = Params::new(b, d, steps).with_rule(rule);
            let win = Window::from_bbox(
                &frame_bbox(&params, steps),
                0.06,
                1.0,
                [0.0, 0.0],
                w as f64 / h as f64,
            );
            let (_, _, rgb, stats) = render_image(&params, w, h, 1, &win, &DeepOpts::for_steps(steps, 1));
            let ink = ink_mask(&rgb);
            let n = components(&ink, w as usize, h as usize);
            let count = ink.iter().filter(|v| **v).count();
            eprintln!(
                "b={b} d={d} steps={steps}: components={n} ink={count} nodes={} stamps={} leaves={}",
                stats.nodes, stats.stamps, stats.leaves
            );
            assert!(n <= 4, "b={b} d={d}: the stroke broke into {n} pieces");
        }
    }

    #[test]
    fn deep_zoom_is_bounded_and_not_blank() {
        let (w, h) = (200u32, 200u32);
        let params = Params::new(2, 4, 1 << 50);
        let reference = generate(&Params::new(2, 4, 1 << 20));
        let center = reference.bbox.center();
        let extent = reference.bbox.width().max(reference.bbox.height());
        for zoom in [1.0, 10.0, 100.0] {
            let half = extent / (2.0 * zoom);
            let win = Window::new(center, [half, half * (h as f64 / w as f64)]);
            let (_, _, rgb, stats) = render_image(&params, w, h, 1, &win, &DeepOpts::for_steps(1 << 50, 1));
            let ink = ink_mask(&rgb).iter().filter(|v| **v).count();            assert!(!stats.truncated, "zoom={zoom} truncated at {} nodes", stats.nodes);
            assert!(stats.nodes < 400_000, "zoom={zoom} visited {} nodes", stats.nodes);
            assert!(ink > 200, "zoom={zoom} ink={ink}");
        }
        // Past the unit scale the window can be nearly empty, but the work stays bounded.
        let win = Window::new(center, [extent / 2e6, extent / 2e6]);
        let (_, _, _, stats) = render_image(&params, w, h, 1, &win, &DeepOpts::for_steps(1 << 50, 1));
        assert!(!stats.truncated && stats.nodes < 400_000, "nodes={}", stats.nodes);
    }

    #[test]
    fn cost_is_independent_of_step_count() {
        let params = Params::new(2, 4, 1 << 50);
        let bbox = generate(&Params::new(2, 4, 1 << 10)).bbox;
        let win = Window::from_bbox(&bbox, 0.06, 1.0, [0.0, 0.0], 1.5);
        let small = render_into(
            &mut Raster::new(300, 200, Canvas::Dark),
            &params,
            &win,
            &DeepOpts::for_steps(1 << 20, 1),
        );
        let huge = render_into(
            &mut Raster::new(300, 200, Canvas::Dark),
            &params,
            &win,
            &DeepOpts::for_steps(1 << 50, 1),
        );
        assert!(!small.truncated && !huge.truncated);
        assert!(huge.nodes < small.nodes * 2, "{} vs {}", small.nodes, huge.nodes);
        assert!(huge.nodes > 100, "suspiciously few nodes: {}", huge.nodes);
    }

    #[test]
    fn frontier_matches_the_enumeration_tip() {
        let steps = 4096u64;
        let params = Params::new(2, 4, steps);
        let curve = generate(&params);
        let win = Window::from_bbox(&curve.bbox, 0.06, 1.0, [0.0, 0.0], 1.0);
        let stats = render_into(
            &mut Raster::new(400, 400, Canvas::Dark),
            &params,
            &win,
            &DeepOpts::for_steps(steps, 1),
        );
        let tip = stats.frontier.expect("frontier");
        let want = curve.pts[steps as usize];
        let err = ((tip[0] - want[0]).powi(2) + (tip[1] - want[1]).powi(2)).sqrt();
        assert!(err < 8.0, "frontier {tip:?} vs tip {want:?}");
    }

    #[test]
    fn budget_truncates_instead_of_hanging() {
        let params = Params::new(2, 4, 1 << 50);
        let bbox = generate(&Params::new(2, 4, 1 << 10)).bbox;
        let win = Window::from_bbox(&bbox, 0.0, 1.0, [0.0, 0.0], 1.0);
        let opts = DeepOpts { budget_nodes: 500, ..DeepOpts::for_steps(1 << 50, 1) };
        let stats = render_into(&mut Raster::new(200, 200, Canvas::Dark), &params, &win, &opts);
        assert!(stats.truncated);
        assert!(stats.nodes <= 600, "visited {} nodes", stats.nodes);
    }
}
