//! The number-system turtle.
//!
//! ```text
//! heading(n) = (2 * PI / dir) * s_b(n)        s_b = sum of base-b digits
//! z(n + 1)   = z(n) + (cos heading(n), sin heading(n))
//! ```
//!
//! One unit of base-`b` digit sum turns the turtle by `360 / dir` degrees: `dir` is how many
//! equal parts a full turn is cut into (`dir = 4` -> 90 degrees). The heading is absolute, it
//! depends only on `n` and never on the previous heading, which is what makes the walk
//! exactly self-similar. Step `n` is `zeta^(s_b(n))` with `zeta = exp(2*PI*i/dir)`, and
//! `s_b(b*m + r) = s_b(m) + r` gives the block identity used by the deep renderer:
//!
//! ```text
//! z(m*b^L + j) = S^L * z(m) + zeta^(s_b(m)) * z(j)       0 <= j < b^L
//! S            = sum(zeta^r, r = 0..b-1) = (1 - zeta^b) / (1 - zeta)
//! |S|          = sin(PI*b/dir) / sin(PI/dir)             (b not a multiple of dir)
//! ```
//!
//! With `r = b mod dir`:
//!
//! * `r == 0`  : `S = 0` -> the walk returns to the origin every `b^k` steps: a bounded
//!   polygon with at most `b * dir + 1` distinct vertices. Decorative, not a fractal.
//! * `r == +-1`: `|S| = 1` -> `S` is a pure rotation: bounded and periodic, not a fractal.
//! * otherwise : `|S| > 1` -> expanding self-similar fractal.
//!
//! The defaults `(base, dir) = (2, 4)` are 90 degrees per unit of binary digit sum: the dragon
//! walk with `S = 1 + i`, the similarity ratio of the twindragon number system (complex base
//! `1 + i`, digits `{0, 1}`).
//!
//! # Step counts
//!
//! `steps` goes up to `2^50`, but only `MAX_ENUM_STEPS` of them can be materialised as a point
//! list. Everything above that is rendered through the block identity by `crate::deep`, whose
//! cost depends on the viewport, not on the step count.

pub const MIN_BASE: u32 = 2;
pub const MAX_BASE: u32 = 64;
/// Smallest useful direction number: `dir = 1` turns a full `360` degrees per digit unit,
/// which is a straight ray.
pub const MIN_DIR_ABS: i32 = 1;
pub const MAX_DIR_ABS: i32 = 64;
/// Largest step count the CLI/GUI accepts (`2^50`).
pub const MAX_STEPS: u64 = 1 << 50;
/// Largest step count that is expanded into a point list (4.2M vertices, 67 MB).
pub const MAX_ENUM_STEPS: u64 = 1 << 22;

#[inline]
fn dir_or_one(dir: i32) -> i32 {
    if dir == 0 {
        1
    } else {
        dir
    }
}

/// How one base-`b` digit contributes to the heading.
///
/// The heading is always `(2*PI/dir) * sum_i weight(r_i)`, so every rule stays *additive over
/// digits*: `f(b*m + r) = f(m) + weight(r)`. That is the only property the block identity (and
/// therefore the deep renderer, level animation and zooming) relies on, so new rules get all of
/// it for free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigitRule {
    /// `weight(r) = r`: the plain digit sum. Turn per unit = `360/dir`.
    Linear,
    /// `weight(r) = (-1)^r * r`: sign alternates with the digit value.
    Alternate,
    /// `weight(r) = (-1)^r * min(r, b-1-r)`: fold at the midpoint first, then alternate the sign.
    /// `(base, dir) = (4, 6)` gives the Koch arc (mirrored) with `|S| = 3` exactly.
    Fold,
}

impl DigitRule {
    pub fn parse(s: &str) -> Option<DigitRule> {
        match s.to_ascii_lowercase().as_str() {
            "linear" => Some(DigitRule::Linear),
            "alternate" => Some(DigitRule::Alternate),
            "fold" => Some(DigitRule::Fold),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DigitRule::Linear => "linear",
            DigitRule::Alternate => "alternate",
            DigitRule::Fold => "fold",
        }
    }

    /// Contribution of one digit, in units of `360/dir` degrees.
    pub fn weight(self, digit: u32, base: u32) -> i64 {
        let r = digit as i64;
        let last = base.saturating_sub(1) as i64;
        match self {
            DigitRule::Linear => r,
            DigitRule::Alternate => if digit % 2 == 0 { r } else { -r },
            DigitRule::Fold => {
                let folded = r.min(last - r).max(0);
                if digit % 2 == 0 {
                    folded
                } else {
                    -folded
                }
            }
        }
    }

    /// True when this rule is a no-op for `base` (every weight is the same, so the walk is a ray).
    pub fn is_degenerate_for(self, base: u32) -> bool {
        let first = self.weight(0, base);
        (0..base).all(|r| self.weight(r, base) == first)
    }
}

/// Move the direction number by `delta`, skipping `0` and clamping to `+-MAX_DIR_ABS`.
pub fn step_dir(dir: i32, delta: i32) -> i32 {
    let mut next = dir_or_one(dir) + delta;
    if next == 0 {
        next = if delta > 0 { 1 } else { -1 };
    }
    next.clamp(-MAX_DIR_ABS, MAX_DIR_ABS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub base: u32,
    pub dir: i32,
    pub steps: u64,
    pub rule: DigitRule,
}

impl Default for Params {
    /// `(2, 4, 65536, linear)`: 90 degrees per unit of binary digit sum, the classic dragon.
    fn default() -> Self {
        Params::new(2, 4, 1 << 16)
    }
}

impl Params {
    pub fn new(base: u32, dir: i32, steps: u64) -> Self {
        Params { base, dir, steps, rule: DigitRule::Linear }.clamped()
    }

    pub fn with_rule(self, rule: DigitRule) -> Self {
        Params { rule, ..self }.clamped()
    }

    pub fn clamped(self) -> Self {
        let abs = self.dir.unsigned_abs().clamp(MIN_DIR_ABS as u32, MAX_DIR_ABS as u32) as i32;
        Params {
            base: self.base.clamp(MIN_BASE, MAX_BASE),
            dir: if self.dir < 0 { -abs } else { abs },
            steps: self.steps.clamp(1, MAX_STEPS),
            rule: self.rule,
        }
    }

    /// True when `steps` is too large for a point list and needs `crate::deep`.
    pub fn is_deep(&self) -> bool {
        self.steps > MAX_ENUM_STEPS
    }

    /// Signed degrees the turtle turns per unit of the digit function (`360 / dir`).
    pub fn turn_per_digit_deg(&self) -> f64 {
        360.0 / dir_or_one(self.dir) as f64
    }

    pub fn heading_at(&self, n: u64) -> f64 {
        std::f64::consts::TAU / dir_or_one(self.dir) as f64 * digit_weight(n, self.base, self.rule) as f64
    }
}

pub fn digit_sum(mut n: u64, base: u32) -> u32 {
    let b = base.max(2) as u64;
    let mut sum = 0u32;
    while n > 0 {
        sum += (n % b) as u32;
        n /= b;
    }
    sum
}

/// `f(n) = sum_i weight(r_i)`: the additive digit function behind every rule.
pub fn digit_weight(mut n: u64, base: u32, rule: DigitRule) -> i64 {
    let b = base.max(2) as u64;
    let mut sum = 0i64;
    while n > 0 {
        sum += rule.weight((n % b) as u32, base);
        n /= b;
    }
    sum
}

pub fn heading_rule(n: u64, base: u32, dir: i32, rule: DigitRule) -> f64 {
    std::f64::consts::TAU / dir_or_one(dir) as f64 * digit_weight(n, base, rule) as f64
}

pub fn heading(n: u64, base: u32, dir: i32) -> f64 {
    heading_rule(n, base, dir, DigitRule::Linear)
}

/// `zeta = exp(2*PI*i / dir)`, the unit step rotation.
pub fn zeta(dir: i32) -> [f64; 2] {
    let angle = std::f64::consts::TAU / dir_or_one(dir) as f64;
    [angle.cos(), angle.sin()]
}

/// `c_r = sum(zeta^weight(q), q < r)`: the first `r` steps of one base-`b` block (`r <= b`).
pub fn block_prefix_rule(base: u32, dir: i32, rule: DigitRule, r: u64) -> [f64; 2] {
    let z = zeta(dir);
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for q in 0..r.min(base as u64) {
        let w = cpow(z, rule.weight(q as u32, base));
        x += w[0];
        y += w[1];
    }
    [x, y]
}

pub fn block_prefix(base: u32, dir: i32, r: u64) -> [f64; 2] {
    block_prefix_rule(base, dir, DigitRule::Linear, r)
}

/// Self-similarity factor `S = c_b = sum(zeta^weight(q), q < b)`.
pub fn scale_factor_rule(base: u32, dir: i32, rule: DigitRule) -> [f64; 2] {
    block_prefix_rule(base, dir, rule, base as u64)
}

pub fn scale_factor(base: u32, dir: i32) -> [f64; 2] {
    scale_factor_rule(base, dir, DigitRule::Linear)
}

/// `|S|`: `0` closed, `1` bounded/periodic, `> 1` expanding (and `< 1` contracting, reachable with
/// the non-linear rules only).
pub fn gain_rule(base: u32, dir: i32, rule: DigitRule) -> f64 {
    let s = scale_factor_rule(base, dir, rule);
    (s[0] * s[0] + s[1] * s[1]).sqrt()
}

pub fn gain(base: u32, dir: i32) -> f64 {
    gain_rule(base, dir, DigitRule::Linear)
}

/// `b^level`, saturated at `MAX_STEPS`.
pub fn level_steps(base: u32, level: u32) -> u64 {
    let b = base.max(2) as u64;
    let mut n: u64 = 1;
    for _ in 0..level.min(63) {
        n = n.saturating_mul(b);
    }
    n.clamp(1, MAX_STEPS)
}

/// Smallest `level` with `b^level >= steps`.
pub fn steps_to_level(base: u32, steps: u64) -> u32 {
    let b = base.max(2) as u64;
    let mut level = 0u32;
    let mut n: u64 = 1;
    while n < steps && level < 63 {
        n = n.saturating_mul(b);
        level += 1;
    }
    level
}

#[inline]
pub fn cmul(z: [f64; 2], w: [f64; 2]) -> [f64; 2] {
    [z[0] * w[0] - z[1] * w[1], z[0] * w[1] + z[1] * w[0]]
}

pub fn cpow(z: [f64; 2], k: i64) -> [f64; 2] {
    let n = k.unsigned_abs().min(128);
    let mut acc = [1.0f64, 0.0f64];
    for _ in 0..n {
        acc = cmul(acc, z);
    }
    if k < 0 {
        let d = acc[0] * acc[0] + acc[1] * acc[1];
        [acc[0] / d, -acc[1] / d]
    } else {
        acc
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bbox {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bbox {
    pub fn point(x: f64, y: f64) -> Self {
        Bbox { min_x: x, min_y: y, max_x: x, max_y: y }
    }

    pub fn grow(&mut self, x: f64, y: f64) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn center(&self) -> [f64; 2] {
        [(self.min_x + self.max_x) * 0.5, (self.min_y + self.max_y) * 0.5]
    }
}

/// The materialised turtle path: `steps + 1` vertices, `steps` unit segments.
#[derive(Clone, Debug)]
pub struct Curve {
    pub params: Params,
    pub pts: Vec<[f64; 2]>,
    pub bbox: Bbox,
    /// Heading of the last step (the turtle's final orientation).
    pub head_heading: f64,
}

impl Curve {
    pub fn segments(&self) -> usize {
        self.pts.len().saturating_sub(1)
    }

    pub fn heading_at(&self, i: usize) -> f64 {
        heading_rule(i as u64, self.params.base, self.params.dir, self.params.rule)
    }
}

/// Materialise the first `min(params.steps, MAX_ENUM_STEPS)` steps. The returned curve carries
/// the *effective* step count in `params.steps`.
pub fn generate(params: &Params) -> Curve {
    let clamped = params.clamped();
    let effective = clamped.steps.min(MAX_ENUM_STEPS);
    let p = Params { steps: effective, ..clamped };
    let n = effective as usize;
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(n + 1);
    let (mut x, mut y) = (0.0f64, 0.0f64);
    pts.push([x, y]);
    let mut bbox = Bbox::point(x, y);
    let mut head = 0.0;
    for i in 0..n as u64 {
        head = p.heading_at(i);
        x += head.cos();
        y += head.sin();
        pts.push([x, y]);
        bbox.grow(x, y);
    }
    Curve { params: p, pts, bbox, head_heading: head }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn cmul_test(z: [f64; 2], w: [f64; 2]) -> [f64; 2] {
        cmul(z, w)
    }

    fn distinct_vertices(c: &Curve) -> usize {
        c.pts
            .iter()
            .map(|p| (p[0].round() as i64, p[1].round() as i64))
            .collect::<HashSet<_>>()
            .len()
    }

    fn max_abs(c: &Curve) -> f64 {
        c.pts.iter().map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt()).fold(0.0, f64::max)
    }

    #[test]
    fn digit_sums_are_right() {
        assert_eq!(digit_sum(0, 2), 0);
        assert_eq!(digit_sum(8, 2), 1);
        assert_eq!(digit_sum(255, 16), 30);
        assert_eq!(digit_sum(13, 3), 3); // 111_3
        assert_eq!(digit_sum(1 << 21, 2), 1);
        assert_eq!(digit_sum(1 << 21, 10), 26); // 2097152
    }

    #[test]
    fn headings_follow_the_rule() {
        assert!((heading(0, 2, 4) - 0.0).abs() < 1e-12);
        assert!((heading(2, 2, 4) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((heading(3, 3, 4) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((heading(5, 10, 4) - 2.5 * std::f64::consts::PI).abs() < 1e-12);
        assert!((heading(1, 2, -8) + std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        assert!((Params::new(2, 4, 10).turn_per_digit_deg() - 90.0).abs() < 1e-12);
        assert!((Params::new(2, -5, 10).turn_per_digit_deg() + 72.0).abs() < 1e-12);
        assert!((Params::new(2, 1, 10).turn_per_digit_deg() - 360.0).abs() < 1e-12);
    }

    #[test]
    fn params_are_clamped() {
        let zero = Params::new(0, 0, 0);
        assert_eq!((zero.base, zero.dir, zero.steps), (MIN_BASE, 1, 1));
        let high = Params::new(999, 999, u64::MAX);
        assert_eq!((high.base, high.dir, high.steps), (MAX_BASE, MAX_DIR_ABS, MAX_STEPS));
        assert_eq!(Params::new(2, -999, 10).dir, -MAX_DIR_ABS);
        assert_eq!(Params::new(2, i32::MIN, 10).dir, -MAX_DIR_ABS);
        assert!(Params::new(2, 4, 1 << 50).is_deep());
        assert!(!Params::new(2, 4, 1 << 20).is_deep());
    }

    #[test]
    fn step_dir_skips_zero_and_clamps() {
        assert_eq!(step_dir(1, -1), -1);
        assert_eq!(step_dir(-1, 1), 1);
        assert_eq!(step_dir(2, -1), 1);
        assert_eq!(step_dir(-2, 1), -1);
        assert_eq!(step_dir(MAX_DIR_ABS, 1), MAX_DIR_ABS);
        assert_eq!(step_dir(-MAX_DIR_ABS, -1), -MAX_DIR_ABS);
        assert_eq!(step_dir(0, 1), 2);
        assert_eq!(step_dir(3, 1), 4);
    }

    #[test]
    fn level_helpers_are_consistent() {
        assert_eq!(level_steps(2, 0), 1);
        assert_eq!(level_steps(2, 10), 1024);
        assert_eq!(level_steps(3, 4), 81);
        assert_eq!(level_steps(2, 50), 1 << 50);
        assert_eq!(steps_to_level(2, 1), 0);
        assert_eq!(steps_to_level(2, 1024), 10);
        assert_eq!(steps_to_level(2, 1025), 11);
        assert_eq!(steps_to_level(3, 81), 4);
        assert_eq!(steps_to_level(2, 1 << 50), 50);
        assert_eq!(cpow([0.0, 1.0], 4)[0], 1.0);
        assert!((cpow(scale_factor(2, 4), 2)[0] - 0.0).abs() < 1e-12); // (1+i)^2 = 2i
        assert!((cpow(scale_factor(2, 4), 2)[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn bbox_covers_every_vertex() {
        let c = generate(&Params::new(3, 5, 40_000));
        for p in &c.pts {
            assert!(p[0] >= c.bbox.min_x && p[0] <= c.bbox.max_x);
            assert!(p[1] >= c.bbox.min_y && p[1] <= c.bbox.max_y);
        }
    }

    #[test]
    fn generate_caps_at_enum_limit() {
        let c = generate(&Params::new(2, 4, MAX_STEPS));
        assert_eq!(c.params.steps, MAX_ENUM_STEPS);
        assert_eq!(c.segments(), MAX_ENUM_STEPS as usize);
    }

    #[test]
    fn block_identity_holds() {
        for (b, d) in [(2u32, 4i32), (2, -4), (3, 3), (3, 5), (4, 7), (5, 8)] {
            let s = scale_factor(b, d);
            let z = zeta(d);
            let limit = 1u64 << 15;
            let curve = generate(&Params::new(b, d, limit));
            for (m, level) in [(0u64, 1u32), (1, 1), (3, 2), (5, 2), (7, 3), (11, 3)] {
                let block = level_steps(b, level);
                let Some(span) = m.checked_mul(block) else { continue };
                if span + block > limit || m >= limit {
                    continue;
                }
                let sl = cpow(s, level as i64);
                let rot = cpow(z, digit_sum(m, b) as i64);
                let step = (block / 8).max(1);
                let mut j = 0u64;
                while j <= block {
                    let n = (span + j) as usize;
                    let lhs = curve.pts[n];
                    let rhs = {
                        let a = cmul(sl, curve.pts[m as usize]);
                        let bb = cmul(rot, curve.pts[j as usize]);
                        [a[0] + bb[0], a[1] + bb[1]]
                    };
                    let err = ((lhs[0] - rhs[0]).powi(2) + (lhs[1] - rhs[1]).powi(2)).sqrt();
                    assert!(err < 1e-6, "b={b} d={d} m={m} L={level} j={j} err={err}");
                    if j == block {
                        break;
                    }
                    j = (j + step).min(block);
                }
            }
        }
    }

    #[test]
    fn self_similarity_holds() {
        let steps = 1u64 << 12;
        for (b, d) in [
            (2u32, 4i32),
            (2, 5),
            (2, 8),
            (2, 3),
            (2, 2),
            (3, 3),
            (3, 5),
            (3, 6),
            (4, 3),
            (4, 4),
            (5, 7),
            (7, 5),
            (2, -4),
            (3, -5),
            (2, 1),
        ] {
            let c = generate(&Params::new(b, d, steps));
            let s = scale_factor(b, d);
            let n = (steps / b as u64) as usize;
            let lhs = c.pts[n * b as usize];
            let rhs = cmul_test(s, c.pts[n]);
            let err = ((lhs[0] - rhs[0]).powi(2) + (lhs[1] - rhs[1]).powi(2)).sqrt();
            assert!(err < 1e-6, "b={b} d={d} err={err}");
        }
    }

    #[test]
    fn dir_dividing_base_is_a_closed_polygon() {
        let steps = 1u64 << 10;
        for (b, d) in [(2u32, 2i32), (2, -2), (3, 3), (4, 2), (4, 4), (6, 2), (6, 3), (7, 7), (6, -3)] {
            let s = scale_factor(b, d);
            assert!(s[0].abs() < 1e-9 && s[1].abs() < 1e-9, "b={b} d={d} S={s:?}");
            let c = generate(&Params::new(b, d, steps));
            let n = (steps / b as u64) as usize;
            let z = c.pts[n * b as usize];
            assert!(z[0].abs() < 1e-6 && z[1].abs() < 1e-6, "b={b} d={d} z={z:?}");
            let bound = b as usize * d.unsigned_abs() as usize + 1;
            assert!(distinct_vertices(&c) <= bound, "b={b} d={d} vertices={}", distinct_vertices(&c));
        }
    }

    #[test]
    fn base_plus_minus_one_mod_dir_is_bounded() {
        for (b, d) in [
            (2u32, 3i32),
            (2, -3),
            (3, 2),
            (3, 4),
            (4, 3),
            (4, 5),
            (5, 4),
            (6, 5),
            (6, 7),
            (8, 7),
            (9, 5),
            (11, 3),
            (12, 11),
        ] {
            let m = d.unsigned_abs();
            let r = b % m;
            assert!(r == 1 || r == m - 1, "b={b} d={d} is not a +-1 residue");
            let c = generate(&Params::new(b, d, 1 << 13));
            let bound = 4.0 * (b as f64).powi(2);
            assert!(max_abs(&c) <= bound, "b={b} d={d} max={}", max_abs(&c));
            // Bounded means: four times as many steps must not push the walk outwards.
            let c4 = generate(&Params::new(b, d, 1 << 15));
            assert!(
                max_abs(&c4) <= max_abs(&c) * 1.2 + 1.0,
                "b={b} d={d} grew from {} to {}",
                max_abs(&c),
                max_abs(&c4)
            );
        }
    }

    #[test]
    fn middle_residue_is_expanding() {
        for (b, d) in [
            (2u32, 4i32),
            (2, 5),
            (2, 8),
            (3, 5),
            (3, 8),
            (4, 7),
            (5, 7),
            (5, 8),
            (6, 4),
            (6, 8),
            (10, 7),
        ] {
            let m = d.unsigned_abs();
            let r = b % m;
            assert!(r >= 2 && r <= m - 2, "b={b} d={d} is not a middle residue");
            let gain = gain(b, d);
            assert!(gain > 1.2, "b={b} d={d} |S|={gain}");
            let steps = 1u64 << 12;
            let small = max_abs(&generate(&Params::new(b, d, steps)));
            let big = max_abs(&generate(&Params::new(b, d, steps * (b as u64).pow(2))));
            assert!(big > small * gain.powi(2) * 0.9, "b={b} d={d}: {small} -> {big}");
        }
    }

    #[test]
    fn dir_above_base_is_dense() {
        let steps = 1u64 << 12;
        for (b, d) in [
            (2u32, 4i32),
            (2, 5),
            (2, 8),
            (3, 5),
            (3, 7),
            (4, 6),
            (4, 9),
            (5, 8),
            (6, 11),
            (7, 12),
            (9, 11),
        ] {
            assert!(d as u32 > b, "b={b} d={d} is not in the dense regime");
            let c = generate(&Params::new(b, d, steps));
            let distinct = distinct_vertices(&c) as f64;
            assert!(distinct > steps as f64 / 4.0, "b={b} d={d}: {distinct} distinct vertices");
        }
    }

    #[test]
    fn small_dir_on_a_big_base_is_sparse() {
        let steps = 1u64 << 12;
        for (b, d) in [(6u32, 4i32), (10, 4), (12, 5), (9, 6), (10, 7)] {
            assert!(d as u32 <= b, "b={b} d={d} is not in the sparse regime");
            let c = generate(&Params::new(b, d, steps));
            let distinct = distinct_vertices(&c) as f64;
            assert!(distinct < steps as f64 / 6.0, "b={b} d={d}: {distinct} distinct vertices");
            assert!(max_abs(&c) > 4.0, "b={b} d={d} did not expand at all");
        }
    }

    #[test]
    fn dir_one_is_a_straight_ray() {
        for d in [1, -1] {
            let steps = 4096u64;
            let c = generate(&Params::new(2, d, steps));
            let tip = c.pts[steps as usize];
            assert!((tip[0] - steps as f64).abs() < 1e-9 && tip[1].abs() < 1e-9, "d={d} tip={tip:?}");
            for p in &c.pts {
                assert!(p[1].abs() < 1e-9, "d={d} left the x axis: {p:?}");
            }
        }
    }

    #[test]
    fn block_identity_holds_for_every_rule() {
        for rule in [DigitRule::Linear, DigitRule::Alternate, DigitRule::Fold] {
            for (b, d) in [(2u32, 4i32), (3, 5), (3, 6), (4, 6), (4, 10), (5, 8), (6, 8)] {
                let p = Params::new(b, d, 1 << 15).with_rule(rule);
                let curve = generate(&p);
                let s = scale_factor_rule(b, d, rule);
                let zeta = zeta(d);
                for (m, level) in [(0u64, 1u32), (1, 1), (3, 2), (5, 2), (7, 3)] {
                    let block = level_steps(b, level);
                    let span = m * block;
                    if span + block > 1 << 15 {
                        continue;
                    }
                    let sl = cpow(s, level as i64);
                    let rot = cpow(zeta, digit_weight(m, b, rule));
                    let mut j = 0u64;
                    let step = (block / 8).max(1);
                    while j <= block {
                        let lhs = curve.pts[(span + j) as usize];
                        let a = cmul(sl, curve.pts[m as usize]);
                        let bb = cmul(rot, curve.pts[j as usize]);
                        let rhs = [a[0] + bb[0], a[1] + bb[1]];
                        let err = ((lhs[0] - rhs[0]).powi(2) + (lhs[1] - rhs[1]).powi(2)).sqrt();
                        assert!(err < 1e-6, "{} b={b} d={d} m={m} L={level} j={j} err={err}", rule.name());
                        if j == block {
                            break;
                        }
                        j = (j + step).min(block);
                    }
                }
            }
        }
    }

    #[test]
    fn fold_rule_produces_the_koch_arc() {
        let p = Params::new(4, 6, 1 << 12).with_rule(DigitRule::Fold);
        assert!((gain_rule(4, 6, DigitRule::Fold) - 3.0).abs() < 1e-12);
        let curve = generate(&p);
        // Mirrored Koch generator: directions 0, -60, +60, 0 degrees.
        let expected: [f64; 16] =
            [0.0, -60.0, 60.0, 0.0, -60.0, -120.0, 0.0, -60.0, 60.0, 0.0, 120.0, 60.0, 0.0, -60.0, 60.0, 0.0];
        for (i, want) in expected.iter().enumerate() {
            let got = curve.heading_at(i).to_degrees().rem_euclid(360.0);
            let want = (*want).rem_euclid(360.0);
            assert!((got - want).abs() < 1e-9, "segment {i}: {got} vs {want}");
        }
        // Stage endpoints: `z(4^k) = 3^k` on the x axis (the Koch arc closes with ratio 3).
        let mut level = 1u32;
        while level_steps(4, level) < 4096 {
            let n = level_steps(4, level) as usize;
            let want = 3.0f64.powi(level as i32);
            let got = curve.pts[n];
            assert!(
                (got[0] - want).abs() < 1e-6 && got[1].abs() < 1e-6,
                "stage {level}: {got:?} vs ({want}, 0)"
            );
            level += 1;
        }
        // The arc never fills area: every vertex is new (the dragon only reaches ~75%).
        let distinct: std::collections::HashSet<(i64, i64)> = curve
            .pts
            .iter()
            .map(|p| (p[0].round() as i64, p[1].round() as i64))
            .collect();
        assert!(distinct.len() > 4096 * 9 / 10, "only {} distinct vertices", distinct.len());
        let span = curve.pts[4096];
        assert!((span[0] - 729.0).abs() < 1e-6 && span[1].abs() < 1e-6, "z(4^6) = {span:?}");
    }

    #[test]
    fn non_linear_rules_have_expected_weights() {
        assert_eq!(DigitRule::Linear.weight(3, 4), 3);
        assert_eq!(DigitRule::Alternate.weight(3, 4), -3);
        assert_eq!(DigitRule::Alternate.weight(2, 4), 2);
        assert_eq!(DigitRule::Fold.weight(0, 4), 0);
        assert_eq!(DigitRule::Fold.weight(1, 4), -1);
        assert_eq!(DigitRule::Fold.weight(2, 4), 1);
        assert_eq!(DigitRule::Fold.weight(3, 4), 0);
        assert_eq!(digit_weight(0b1011, 2, DigitRule::Alternate), -3);
        assert!(DigitRule::Fold.is_degenerate_for(2), "fold collapses on base 2");
        assert!(!DigitRule::Fold.is_degenerate_for(4));
        assert!(!DigitRule::Alternate.is_degenerate_for(2));
        assert_eq!(DigitRule::parse("FOLD"), Some(DigitRule::Fold));
        assert_eq!(DigitRule::parse("nope"), None);
        assert_eq!(Params::new(4, 6, 10).with_rule(DigitRule::Fold).rule, DigitRule::Fold);
    }
}
