//! Offline animation pipeline: keyframed scene -> frames -> MP4 (ffmpeg) / GIF / PNG.
//!
//! Two renderers share the same scene model: the point-list renderer for
//! `steps <= MAX_ENUM_STEPS`, and the adaptive deep renderer (self-similar block traversal)
//! for anything larger. Deep mode quantises the reveal to block levels, so `2^50` steps still
//! animate as a finite sequence of scales rather than as an impossible step-by-step trace.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::deep::{self, DeepOpts};
use crate::fractal::{self, Params};
use crate::gif::GifWriter;
use crate::palette::Canvas;
use crate::png;
use crate::render::{
    self, auto_stride, stroke_range, turtle_marker, Raster, Stroke, Window,
};
use crate::scene::{self, Story};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimTarget {
    Mp4(String),
    Gif(String),
    PngDir(String),
}

impl AnimTarget {
    pub fn path(&self) -> &str {
        match self {
            AnimTarget::Mp4(p) | AnimTarget::Gif(p) | AnimTarget::PngDir(p) => p,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AnimTarget::Mp4(_) => "mp4",
            AnimTarget::Gif(_) => "gif",
            AnimTarget::PngDir(_) => "png sequence",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnimOpts {
    pub canvas: Canvas,
    pub width: u32,
    pub height: u32,
    pub ss: u32,
    pub fps: u32,
    pub secs: f64,
    pub story: Story,
    pub hue_shift: f32,
    pub gain: f32,
    pub pad: f64,
    pub max_strokes: usize,
    pub target: AnimTarget,
    pub quiet: bool,
}

impl Default for AnimOpts {
    fn default() -> Self {
        AnimOpts {
            canvas: Canvas::Dark,
            width: 1600,
            height: 1000,
            ss: 2,
            fps: 30,
            secs: 8.0,
            story: Story::Draw,
            hue_shift: 0.0,
            gain: 1.0,
            pad: 0.06,
            max_strokes: 250_000,
            target: AnimTarget::Mp4("out/anim.mp4".to_string()),
            quiet: false,
        }
    }
}

pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn spawn_ffmpeg(path: &str, w: u32, h: u32, fps: u32) -> std::io::Result<std::process::Child> {
    crate::render::ensure_parent_dir(path)?;
    Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps.to_string(),
            "-i",
            "-",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-preset",
            "medium",
            path,
        ])
        .stdin(Stdio::piped())
        .spawn()
}

pub fn run(params: &Params, opts: &AnimOpts) -> std::io::Result<()> {
    let p = params.clamped();
    // h264 yuv420p needs even dimensions.
    let w = (opts.width.max(2)) & !1;
    let h = (opts.height.max(2)) & !1;
    let ss = opts.ss.clamp(1, 4);
    let fps = opts.fps.max(1);
    let deep_mode = p.is_deep();
    let curve = if deep_mode { None } else { Some(fractal::generate(&p)) };
    let total = match &curve {
        Some(c) => c.segments() as u64,
        None => p.steps,
    };
    let levels = fractal::steps_to_level(p.base, total.max(1));
    let framing = match &curve {
        Some(c) => c.bbox,
        None => deep::frame_bbox(&p, total),
    };
    let scene = scene::build_on(opts.story, total, fps, opts.secs, opts.canvas);
    let frames = scene.frame_count();
    let mut raster = Raster::new(w * ss, h * ss, opts.canvas);

    let mut ffmpeg = match &opts.target {
        AnimTarget::Mp4(path) => Some(spawn_ffmpeg(path, w, h, fps).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("ffmpeg 启动失败 ({e}); 可用 --gif 或 --frames-dir 输出"),
            )
        })?),
        _ => None,
    };
    let mut gif = match &opts.target {
        AnimTarget::Gif(path) => {
            let delay_cs = (100.0 / fps as f64).round().max(1.0) as u16;
            Some(GifWriter::create(path, w, h, delay_cs)?)
        }
        _ => None,
    };

    for f in 0..frames {
        let state = scene.at(scene.frame_time(f));
        raster.reset();
        let _ = state.bg;
        let win = Window::from_bbox(&framing, opts.pad, state.zoom, state.shift, w as f64 / h as f64);
        let view = win.view(w * ss, h * ss);

        if let Some(curve) = &curve {
            let stride = auto_stride(state.visible.max(1), opts.max_strokes);
            stroke_range(
                &mut raster,
                curve,
                &view,
                0,
                state.visible,
                Stroke { stride, hue_shift: state.hue_shift, gain: opts.gain },
            );
            if state.show_cursor {
                turtle_marker(&mut raster, curve, &view, state.cursor, 2.0 * ss as f64);
            }
        } else {
            // Deep mode: reveal by block level, which keeps every frame cheap.
            let fraction = (state.visible as f64 / total.max(1) as f64).clamp(0.0, 1.0);
            let level = (fraction * levels as f64).round() as u32;
            let steps_now = fractal::level_steps(p.base, level).min(total).max(1);
            let dopts = DeepOpts {
                canvas: opts.canvas,
                steps: steps_now,
                color_total: total,
                hue_shift: state.hue_shift,
                ink: opts.gain,
                budget_nodes: 12_000_000,
                // Deep animation frames are the slowest path, so stamp a little coarser.
                cut_px: 2.0 * ss as f64,
            };
            let stats = deep::render_into(&mut raster, &p, &win, &dopts);
            if state.show_cursor {
                if let Some(frontier) = stats.frontier {
                    render::marker_dot(&mut raster, &view, frontier, 2.0 * ss as f64);
                }
            }
        }

        let (iw, ih, rgb) = raster.downsample(ss);
        if let Some(child) = ffmpeg.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(&rgb)?;
            }
        } else if let Some(writer) = gif.as_mut() {
            writer.add_frame(&rgb)?;
        } else if let AnimTarget::PngDir(dir) = &opts.target {
            png::write_rgb(&format!("{dir}/frame_{f:05}.png"), iw, ih, &rgb)?;
        }

        if !opts.quiet {
            print!("\r  frame {}/{}", f + 1, frames);
            let _ = std::io::stdout().flush();
        }
    }
    if !opts.quiet {
        println!();
    }

    if let Some(mut child) = ffmpeg {
        drop(child.stdin.take());
        let status = child.wait()?;
        if !status.success() {
            return Err(std::io::Error::other(format!("ffmpeg 退出码 {status}")));
        }
    }
    if let Some(writer) = gif {
        writer.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_png_sequence_animates() {
        let params = Params::new(3, 5, 4096);
        let dir = std::env::temp_dir().join("ffbd_anim_test");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = AnimOpts {
            width: 64,
            height: 64,
            ss: 1,
            fps: 8,
            secs: 1.0,
            quiet: true,
            target: AnimTarget::PngDir(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        run(&params, &opts).expect("png sequence");
        let last = dir.join("frame_00007.png");
        assert!(dir.join("frame_00000.png").exists() && last.exists(), "expected 8 frames in {dir:?}");
        let bytes = std::fs::read(&last).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn deep_mode_animates_a_few_frames() {
        let params = Params::new(2, 4, 1 << 50);
        let dir = std::env::temp_dir().join("ffbd_anim_deep_test");
        let _ = std::fs::remove_dir_all(&dir);
        let opts = AnimOpts {
            width: 96,
            height: 64,
            ss: 1,
            fps: 4,
            secs: 1.0,
            quiet: true,
            target: AnimTarget::PngDir(dir.to_string_lossy().to_string()),
            ..Default::default()
        };
        run(&params, &opts).expect("deep png sequence");
        let last = std::fs::read(dir.join("frame_00003.png")).expect("last frame");
        assert_eq!(&last[..8], b"\x89PNG\r\n\x1a\n");
        let bright = std::fs::read(dir.join("frame_00003.png"))
            .unwrap()
            .iter()
            .filter(|b| **b > 90)
            .count();
        assert!(bright > 100, "deep animation frame has no ink ({bright})");
    }
}
