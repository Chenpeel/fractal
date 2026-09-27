//! A deliberately small Manim-style layer: keyframed scene state plus ready-made stories.
//!
//! A `Scene` is a sorted list of keys `(time, easing, State)`. Sampling the scene at time
//! `t` blends the surrounding keys with the *target* key's easing. `animate` renders one
//! frame per `1 / fps` seconds, so a story is just a script for the camera and the turtle.

use crate::palette::{self, Canvas};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    In,
    Out,
    InOut,
}

impl Easing {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::In => t * t * t,
            Easing::Out => 1.0 - (1.0 - t).powi(3),
            Easing::InOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// Everything the renderer needs for one frame.
#[derive(Clone, Copy, Debug)]
pub struct State {
    /// How many segments of the curve are drawn.
    pub visible: usize,
    /// Turtle position (segment index).
    pub cursor: usize,
    /// `false` hides the turtle marker (used for camera-only shots).
    pub show_cursor: bool,
    /// 1.0 = the whole bounding box fits the frame.
    pub zoom: f64,
    /// Pan, in units of the bounding box size.
    pub shift: [f64; 2],
    pub bg: [f32; 3],
    pub hue_shift: f32,
}

impl State {
    pub fn full(segments: usize) -> Self {
        State::full_on(segments, Canvas::Dark)
    }

    pub fn full_on(segments: usize, canvas: Canvas) -> Self {
        State {
            visible: segments,
            cursor: segments,
            show_cursor: true,
            zoom: 1.0,
            shift: [0.0, 0.0],
            bg: canvas.background(),
            hue_shift: 0.0,
        }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl State {
    fn blend(a: &State, b: &State, t: f64) -> State {
        State {
            visible: lerp(a.visible as f64, b.visible as f64, t).round() as usize,
            cursor: lerp(a.cursor as f64, b.cursor as f64, t).round() as usize,
            show_cursor: b.show_cursor,
            zoom: lerp(a.zoom, b.zoom, t),
            shift: [lerp(a.shift[0], b.shift[0], t), lerp(a.shift[1], b.shift[1], t)],
            bg: palette::lerp(a.bg, b.bg, t as f32),
            hue_shift: lerp(a.hue_shift as f64, b.hue_shift as f64, t) as f32,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub time: f64,
    pub ease: Easing,
    pub state: State,
}

#[derive(Clone, Debug)]
pub struct Scene {
    keys: Vec<Key>,
    pub fps: u32,
    pub duration: f64,
}

impl Scene {
    pub fn new(fps: u32, duration: f64) -> Self {
        Scene { keys: Vec::new(), fps: fps.max(1), duration: duration.max(1.0 / fps.max(1) as f64) }
    }

    /// Append a key. Times must be pushed in ascending order.
    pub fn key(&mut self, time: f64, ease: Easing, state: State) {
        self.keys.push(Key { time, ease, state });
    }

    pub fn frame_count(&self) -> u32 {
        ((self.duration * self.fps as f64).round() as i64).max(1) as u32
    }

    /// Time of frame `f`.
    pub fn frame_time(&self, f: u32) -> f64 {
        f as f64 / self.fps as f64
    }

    pub fn at(&self, t: f64) -> State {
        let Some(first) = self.keys.first() else {
            return State::full(0);
        };
        if t <= first.time {
            return first.state;
        }
        let last = self.keys.last().expect("non-empty");
        if t >= last.time {
            return last.state;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t >= a.time && t < b.time {
                let span = (b.time - a.time).max(1e-9);
                return State::blend(&a.state, &b.state, b.ease.apply((t - a.time) / span));
            }
        }
        last.state
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Story {
    /// Trace the whole fractal, then a slow push-in.
    Draw,
    /// Start zoomed in on the origin while tracing, then pull back to the whole curve.
    Grow,
    /// Trace fast, then tour two details and return.
    Tour,
}

impl Story {
    pub fn parse(s: &str) -> Option<Story> {
        match s.to_ascii_lowercase().as_str() {
            "draw" => Some(Story::Draw),
            "grow" => Some(Story::Grow),
            "tour" => Some(Story::Tour),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Story::Draw => "draw",
            Story::Grow => "grow",
            Story::Tour => "tour",
        }
    }
}

/// Build the keyframes for a story. `secs` is the total duration, `segments` the number of
/// unit steps of the curve being drawn.
pub fn build(story: Story, segments: u64, fps: u32, secs: f64) -> Scene {
    build_on(story, segments, fps, secs, Canvas::Dark)
}

/// Same as [`build`], but the default canvas background comes from `canvas`.
pub fn build_on(story: Story, segments: u64, fps: u32, secs: f64, canvas: Canvas) -> Scene {
    let n = segments.max(1) as usize;
    let dur = secs.max(1.0 / fps.max(1) as f64);
    let at = |frac: f64| frac.clamp(0.0, 1.0) * dur;
    let mut scene = Scene::new(fps, dur);

    match story {
        Story::Draw => {
            scene.key(at(0.00), Easing::Linear, State { visible: 1, cursor: 0, ..State::full_on(n, canvas) });
            scene.key(at(0.08), Easing::Linear, State { visible: 1, cursor: 1, ..State::full_on(n, canvas) });
            scene.key(at(0.82), Easing::InOut, State { visible: n, cursor: n, ..State::full_on(n, canvas) });
            scene.key(
                at(1.00),
                Easing::In,
                State { visible: n, cursor: n, zoom: 1.06, ..State::full_on(n, canvas) },
            );
        }
        Story::Grow => {
            let start = State {
                visible: 1,
                cursor: 0,
                show_cursor: true,
                zoom: 1.5,
                shift: [0.18, 0.18],
                bg: if canvas.is_light() { [1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0] },
                hue_shift: 0.0,
            };
            scene.key(at(0.00), Easing::Linear, start);
            scene.key(
                at(0.15),
                Easing::Out,
                State {
                    visible: (n / 8).max(1),
                    cursor: (n / 8).max(1),
                    zoom: 1.3,
                    shift: [0.1, 0.1],
                    bg: canvas.background(),
                    ..State::full_on(n, canvas)
                },
            );
            scene.key(at(0.88), Easing::InOut, State { visible: n, cursor: n, ..State::full_on(n, canvas) });
            scene.key(
                at(1.00),
                Easing::In,
                State { visible: n, cursor: n, zoom: 1.04, ..State::full_on(n, canvas) },
            );
        }
        Story::Tour => {
            scene.key(at(0.00), Easing::Linear, State { visible: 1, cursor: 0, ..State::full_on(n, canvas) });
            scene.key(at(0.45), Easing::InOut, State { visible: n, cursor: n, ..State::full_on(n, canvas) });
            scene.key(
                at(0.62),
                Easing::InOut,
                State {
                    visible: n,
                    cursor: n,
                    show_cursor: false,
                    zoom: 3.4,
                    shift: [-0.30, 0.24],
                    ..State::full_on(n, canvas)
                },
            );
            scene.key(
                at(0.78),
                Easing::InOut,
                State {
                    visible: n,
                    cursor: n,
                    show_cursor: false,
                    zoom: 2.1,
                    shift: [0.28, -0.20],
                    ..State::full_on(n, canvas)
                },
            );
            scene.key(at(1.00), Easing::InOut, State { visible: n, cursor: n, show_cursor: false, ..State::full_on(n, canvas) });
        }
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::{generate, Params};

    #[test]
    fn easing_is_monotonic_and_anchored() {
        for e in [Easing::Linear, Easing::In, Easing::Out, Easing::InOut] {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-12);
            assert!((e.apply(1.0) - 1.0).abs() < 1e-12);
            let mut prev = -1.0;
            for i in 0..=50 {
                let v = e.apply(i as f64 / 50.0);
                assert!(v >= prev - 1e-12, "{e:?} not monotonic at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn sampling_clamps_and_interpolates() {
        let curve = generate(&Params::new(2, 1, 4096));
        for story in [Story::Draw, Story::Grow, Story::Tour] {
            let scene = build(story, curve.segments() as u64, 30, 4.0);
            let n = curve.segments();
            let first = scene.at(-1.0);
            assert_eq!(first.visible, 1);
            let last = scene.at(1e9);
            assert_eq!(last.visible, n);
            let mut prev = 0;
            for f in 0..scene.frame_count() {
                let s = scene.at(scene.frame_time(f));
                assert!(s.visible >= prev.min(1)); // monotonic while tracing
                assert!(s.cursor <= n && s.visible <= n);
                assert!(s.zoom > 0.0 && s.zoom.is_finite());
                prev = s.visible;
            }
        }
    }

    #[test]
    fn frame_count_matches_duration() {
        let scene = build(Story::Draw, 64, 30, 6.0);
        assert_eq!(scene.frame_count(), 180);
    }
}
