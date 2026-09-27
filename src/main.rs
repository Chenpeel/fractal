//! CLI entry point: `fractal gui | render | anim`.

mod gui;
mod ui;

use std::process::ExitCode;
use std::time::Instant;

use fun_fractal_base_direction::animate::{self, AnimOpts, AnimTarget};
use fun_fractal_base_direction::deep::{self, DeepOpts};
use fun_fractal_base_direction::fractal::{
    self, DigitRule, Params, MAX_BASE, MAX_DIR_ABS, MAX_ENUM_STEPS, MAX_STEPS, MIN_BASE,
};
use fun_fractal_base_direction::palette::Canvas;
use fun_fractal_base_direction::png;
use fun_fractal_base_direction::render::{self, RenderOpts, Window};
use fun_fractal_base_direction::scene::Story;

const USAGE: &str = "\
fractal — 用「进制 b + 方向数 dir」驱动海龟生成分形

用法
  fractal                         打开交互窗口 (默认 base=4, dir=6, rule=fold, steps=65536)
  fractal gui    [opts]           同上, 可带参数
  fractal render [opts]           渲染静态图 (PNG; 小步数另有 SVG)
  fractal anim   [opts]           Manim 式动画, 输出 MP4 / GIF / PNG 帧序列

核心公式
  heading(n) = (2 * pi / dir) * s_b(n)       每 1 点数字和转 360/dir 度
  z(n+1)     = z(n) + (cos, sin)(heading(n))               步长恒为 1
  s_b(n) = n 在 b 进制下各位数字之和; dir 是圆周等分数 (dir=4 -> 90 度/点)
  记 S = (1 - zeta^b)/(1 - zeta), zeta = exp(2*pi*i/dir), |S| = sin(pi*b/dir)/sin(pi/dir):
    b mod dir == 0     -> S = 0 : 有界闭合折线 (顶点集有限)
    b mod dir == +-1   -> |S| = 1 : 有界周期, 不是分形
    其它               -> |S| > 1 : 扩张自相似分形 (dir > base 时最密集)

步数与渲染模式
  steps <= 4194304   直接枚举顶点 -> PNG + SVG, 可逐步描线动画
  steps  > 4194304   自适应深渲染 (只走与视口相交的块, 代价与步数无关, 上限 2^50)
                     放大后仍能看到更细的层级; f64 精度下约 1e7 倍放大为实际上限

通用参数
  --base B         进制 2..64                          默认 2
  --dir D          方向数 |D| 1..64, 负号=镜像          默认 4
  --steps N        步数 1..1125899906842624 (支持 2^30 / 1.5e6) 默认 65536
  --bg T          主题 dark | light (GUI 画布与界面, render/anim 输出) 默认 dark
  --lang L        GUI 界面语言 en | zh (窗口 LAN 或 L 可切换) 默认 en
  --rule R         逐位转角表: linear | alternate | fold 默认 linear
                     heading(n) = (2*pi/dir) * sum_i weight(r_i), r_i = n 的 b 进制各位
                       linear    : weight(r) = r                    (数字和, 旧行为)
                       alternate : weight(r) = (-1)^r * r          (符号交替)
                       fold      : weight(r) = (-1)^r*min(r,b-1-r)  (先折到半程再交替;
                                   base=4 dir=6 自动给出科赫弧, |S| = 3)
                     三个表都是逐位相加, 所以块恒等式/深渲染/分级揭示全部照旧成立
  --width W        输出画布宽 / GUI 窗口宽              render/anim 默认 1600, GUI 1500
  --height H       输出画布高 / GUI 窗口高              render/anim 默认 1200, GUI 900
  --ss S           超采样 1..4 (采样像素上限 1600 万)  默认 2
  --hue H          色相偏移 0..1                       默认 0
  --gain G         描边浓度 (0,10]                    默认 1
  --zoom Z         render: 放大 Z 倍                    默认 1
  --center X,Y     render: 视口中心 (世界坐标)          默认图形中心
  --out PATH       输出路径前缀 (自动去扩展名)         默认 out/curve_b{B}_d{D}_n{N}
  --no-svg         render 时不写 SVG
  --quiet          关闭进度输出
  --frames N       gui: 渲染 N 帧后自动退出 (冒烟测试)
  --shot PATH      gui: 退出前把整个窗口截图为 PATH (含参数栏, 便于核对主题)

动画参数 (anim)
  --story S        draw | grow | tour                  默认 draw
  --fps F          帧率                                默认 30
  --secs T         总时长 (秒)                         默认 8
  --gif            输出 GIF 而不是 MP4 (无需 ffmpeg)
  --frames-dir D   输出 PNG 帧序列到目录 D

GUI (无参数启动时默认是科赫弧: base 4, dir 6, rule fold)
  左侧面板          base / dir / rule / steps / hue 都是输入框: 点击进入, Enter 应用,
                    Tab 跳到下一个, Esc 取消; 非法值会标红并保留原值
                    steps 支持 2^30 / 1.5e6 / 2000000 这类写法
  右上角 ? 按钮     打开/关闭按键与参数说明页 (H 同效, Esc 关闭)
  Tab               收起/展开左侧参数栏 (收起后画布占满窗口, 参数不遮挡画面)
  B                 切换主题 dark / light (画布 + 参数栏 + 帮助页 一起切)
  鼠标滚轮          以光标所在位置为中心缩放 (光标下的点保持不动)
  左键拖动          平移          点面板外空白处 取消输入焦点
  键盘快捷键        [ ] base ∓1   , . dir ∓1   - = steps /2 x2   T 换转角表
                    ↑ ↓ 速度      空格 暂停     R 重播   F 视图复位   G 跟随/整幅
                    C 配色        P/S 导出 PNG  H 帮助   Esc 退出
                    Shift + 上面几组键 = 保持进度热替换

示例
  fractal render --base 2 --dir 4 --steps 262144 --out out/dragon_90deg
  fractal render --base 4 --dir 6 --rule fold --steps 262144 --out out/koch_arc
  fractal render --base 2 --dir 4 --steps 1125899906842624 --zoom 1e4 --out out/dragon_deep
  fractal anim --base 3 --dir 6 --steps 177147 --story grow --secs 6 --out out/hex_60deg
";

#[derive(Clone, Debug)]
struct Opts {
    base: u32,
    dir: i32,
    steps: u64,
    rule: DigitRule,
    canvas: Canvas,
    language: ui::Language,
    width: u32,
    height: u32,
    ss: u32,
    hue: f32,
    gain: f32,
    zoom: f64,
    center: Option<[f64; 2]>,
    out: Option<String>,
    svg: bool,
    gif: bool,
    frames_dir: Option<String>,
    frames: Option<u32>,
    shot: Option<String>,
    story: Story,
    fps: u32,
    secs: f64,
    quiet: bool,
    /// True when the user supplied `--base/--dir/--rule/--steps`; the GUI launches with the Koch
    /// arc otherwise.
    explicit: bool,
    width_explicit: bool,
    height_explicit: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            base: 2,
            dir: 4,
            steps: 1 << 16,
            rule: DigitRule::Linear,
            canvas: Canvas::Dark,
            language: ui::Language::En,
            width: 1600,
            height: 1200,
            ss: 2,
            hue: 0.0,
            gain: 1.0,
            zoom: 1.0,
            center: None,
            out: None,
            svg: true,
            gif: false,
            frames_dir: None,
            frames: None,
            shot: None,
            story: Story::Draw,
            fps: 30,
            secs: 8.0,
            quiet: false,
            explicit: false,
            width_explicit: false,
            height_explicit: false,
        }
    }
}

/// Parameters the interactive window opens with: the Koch arc.
fn gui_default() -> Params {
    Params::new(4, 6, 1 << 16).with_rule(DigitRule::Fold)
}

impl Opts {
    fn params(&self) -> Params {
        Params::new(self.base, self.dir, self.steps).with_rule(self.rule)
    }
}

fn take(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} 缺少取值"))
}

fn num<T: std::str::FromStr>(v: String, flag: &str) -> Result<T, String> {
    v.parse::<T>()
        .map_err(|_| format!("{flag} 的取值无法解析: {v}"))
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].clone();
        match flag.as_str() {
            "--base" => {
                o.explicit = true;
                o.base = num(take(args, &mut i, &flag)?, &flag)?;
            }
            "--dir" => {
                o.explicit = true;
                o.dir = num(take(args, &mut i, &flag)?, &flag)?;
            }
            "--steps" => {
                o.explicit = true;
                let value = take(args, &mut i, &flag)?;
                o.steps = ui::parse_count(&value)
                    .ok_or_else(|| format!("--steps 的取值无法解析: {value}"))?;
            }
            "--bg" => {
                let v = take(args, &mut i, &flag)?;
                o.canvas =
                    Canvas::parse(&v).ok_or_else(|| format!("--bg 只能是 dark|light, 收到 {v}"))?;
            }
            "--lang" => {
                let v = take(args, &mut i, &flag)?;
                o.language = match v.as_str() {
                    "en" => ui::Language::En,
                    "zh" => ui::Language::Zh,
                    _ => return Err(format!("--lang 只能是 en|zh, 收到 {v}")),
                };
            }
            "--rule" => {
                o.explicit = true;
                let v = take(args, &mut i, &flag)?;
                o.rule = DigitRule::parse(&v)
                    .ok_or_else(|| format!("--rule 只能是 linear|alternate|fold, 收到 {v}"))?;
            }
            "--width" => {
                o.width_explicit = true;
                o.width = num(take(args, &mut i, &flag)?, &flag)?;
            }
            "--height" => {
                o.height_explicit = true;
                o.height = num(take(args, &mut i, &flag)?, &flag)?;
            }
            "--ss" => o.ss = num(take(args, &mut i, &flag)?, &flag)?,
            "--hue" => o.hue = num(take(args, &mut i, &flag)?, &flag)?,
            "--gain" => o.gain = num(take(args, &mut i, &flag)?, &flag)?,
            "--zoom" => o.zoom = num(take(args, &mut i, &flag)?, &flag)?,
            "--center" => {
                let v = take(args, &mut i, &flag)?;
                let mut parts = v.split(',');
                let x = parts.next().and_then(|s| s.trim().parse::<f64>().ok());
                let y = parts.next().and_then(|s| s.trim().parse::<f64>().ok());
                match (x, y) {
                    (Some(x), Some(y))
                        if x.is_finite() && y.is_finite() && parts.next().is_none() =>
                    {
                        o.center = Some([x, y])
                    }
                    _ => return Err(format!("--center 需要 \"x,y\", 收到 {v}")),
                }
            }
            "--out" => o.out = Some(take(args, &mut i, &flag)?),
            "--frames-dir" => o.frames_dir = Some(take(args, &mut i, &flag)?),
            "--frames" => o.frames = Some(num(take(args, &mut i, &flag)?, &flag)?),
            "--shot" => o.shot = Some(take(args, &mut i, &flag)?),
            "--story" => {
                let v = take(args, &mut i, &flag)?;
                o.story = Story::parse(&v)
                    .ok_or_else(|| format!("--story 只能是 draw|grow|tour, 收到 {v}"))?;
            }
            "--fps" => o.fps = num(take(args, &mut i, &flag)?, &flag)?,
            "--secs" => o.secs = num(take(args, &mut i, &flag)?, &flag)?,
            "--no-svg" => o.svg = false,
            "--gif" => o.gif = true,
            "--quiet" => o.quiet = true,
            other => return Err(format!("未知参数 {other}")),
        }
        i += 1;
    }
    if !(MIN_BASE..=MAX_BASE).contains(&o.base) {
        return Err(format!("--base 必须在 {MIN_BASE}..={MAX_BASE} 之间"));
    }
    if o.dir == 0 || o.dir.unsigned_abs() > MAX_DIR_ABS as u32 {
        return Err(format!(
            "--dir 必须在 -{MAX_DIR_ABS}..=-1 或 1..={MAX_DIR_ABS} 之间"
        ));
    }
    if !(1..=MAX_STEPS).contains(&o.steps) {
        return Err("--steps 必须在 1..=2^50 之间".to_string());
    }
    if o.width == 0 || o.height == 0 || o.width > 8192 || o.height > 8192 {
        return Err("--width/--height 必须在 1..=8192 之间".to_string());
    }
    if !(1..=4).contains(&o.ss) {
        return Err("--ss 必须在 1..=4 之间".to_string());
    }
    let samples = u64::from(o.width) * u64::from(o.height) * u64::from(o.ss).pow(2);
    if samples > 16_000_000 {
        return Err(
            "输出像素数乘超采样平方不能超过 1600 万; 请减小 --width/--height/--ss".to_string(),
        );
    }
    if !o.hue.is_finite() || !(0.0..=1.0).contains(&o.hue) {
        return Err("--hue 必须在 0..=1 之间".to_string());
    }
    if !o.gain.is_finite() || o.gain <= 0.0 || o.gain > 10.0 {
        return Err("--gain 必须在 (0, 10] 之间".to_string());
    }
    if !(1..=240).contains(&o.fps) {
        return Err("--fps 必须在 1..=240 之间".to_string());
    }
    if !o.secs.is_finite() || !(0.0..=600.0).contains(&o.secs) || o.secs == 0.0 {
        return Err("--secs 必须在 (0, 600] 之间".to_string());
    }
    if o.zoom <= 0.0 || !o.zoom.is_finite() {
        return Err("--zoom 必须为有限正数".to_string());
    }
    if o.frames == Some(0) {
        return Err("--frames 必须大于 0".to_string());
    }
    if o.out.as_ref().is_some_and(|path| path.trim().is_empty())
        || o.shot.as_ref().is_some_and(|path| path.trim().is_empty())
        || o.frames_dir
            .as_ref()
            .is_some_and(|path| path.trim().is_empty())
    {
        return Err("输出路径不能为空".to_string());
    }
    Ok(o)
}

fn human(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.0} KB", bytes as f64 / (1 << 10) as f64)
    } else {
        format!("{bytes} B")
    }
}

fn stem(o: &Opts, p: &Params) -> String {
    let raw = o
        .out
        .clone()
        .unwrap_or_else(|| format!("out/curve_b{}_d{}_n{}", p.base, p.dir, p.steps));
    for ext in [".png", ".svg", ".mp4", ".gif", ".bmp"] {
        if let Some(stripped) = raw.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    raw
}

/// Framing window for `render`: bbox framed at `--zoom`, then optionally re-centred.
fn window_for(o: &Opts, bbox: &fractal::Bbox) -> Window {
    let aspect = o.width.max(1) as f64 / o.height.max(1) as f64;
    let framed = Window::from_bbox(bbox, 0.06, o.zoom, [0.0, 0.0], aspect);
    match o.center {
        Some(c) => Window::new(c, framed.half),
        None => framed,
    }
}

fn render_cmd(o: &Opts) -> Result<(), String> {
    let p = o.params();
    let base = stem(o, &p);
    let t0 = Instant::now();

    let (w, h, rgb, mode, detail, win, curve) = if p.is_deep() {
        let bbox = deep::frame_bbox(&p, p.steps);
        let win = window_for(o, &bbox);
        let dopts = DeepOpts {
            canvas: o.canvas,
            steps: p.steps,
            color_total: p.steps,
            hue_shift: o.hue,
            ink: o.gain,
            budget_nodes: 12_000_000,
            cut_px: 1.5,
        };
        let (w, h, rgb, stats) = deep::render_image(&p, o.width, o.height, o.ss, &win, &dopts);
        let detail = format!(
            "zoom {:.3} | nodes {} stamps {} leaves {}{}",
            win.zoom_of(&bbox),
            stats.nodes,
            stats.stamps,
            stats.leaves,
            if stats.truncated {
                " | TRUNCATED by node budget"
            } else {
                ""
            }
        );
        (w, h, rgb, "deep", detail, win, None)
    } else {
        let curve = fractal::generate(&p);
        let win = window_for(o, &curve.bbox);
        let opts = RenderOpts {
            canvas: o.canvas,
            width: o.width,
            height: o.height,
            ss: o.ss,
            pad: 0.06,
            hue_shift: o.hue,
            gain: o.gain,
        };
        let (w, h, rgb) =
            render::render_curve_window(&curve, o.width, o.height, o.ss, &win, &opts, None);
        let detail = format!(
            "zoom {:.3} | {} vertices",
            win.zoom_of(&curve.bbox),
            curve.pts.len()
        );
        (w, h, rgb, "enum", detail, win, Some(curve))
    };

    let png_path = format!("{base}.png");
    png::write_rgb(&png_path, w, h, &rgb).map_err(|e| format!("写 {png_path} 失败: {e}"))?;
    let mut extra = String::new();
    if o.svg {
        match &curve {
            Some(curve) => {
                let svg_path = format!("{base}.svg");
                let svg = render::curve_svg_window(curve, w, h, &win, 200_000, o.hue, o.canvas);
                render::write_file(&svg_path, svg.as_bytes())
                    .map_err(|e| format!("写 {svg_path} 失败: {e}"))?;
                extra = format!(", {svg_path} ({})", human(svg.len() as u64));
            }
            None => extra = format!(" (steps > {MAX_ENUM_STEPS}: deep 模式不写 SVG)"),
        }
    }
    println!(
        "base={} dir={} rule={} steps={} turn/digit={:.4}deg",
        p.base,
        p.dir,
        p.rule.name(),
        p.steps,
        p.turn_per_digit_deg()
    );
    println!("mode={mode} {detail}");
    println!(
        "{png_path} ({w}x{h}, {}){extra}  用时 {:.2}s",
        human(rgb.len() as u64),
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

fn anim_cmd(o: &Opts) -> Result<(), String> {
    let p = o.params();
    let base = stem(o, &p);
    let target = if let Some(dir) = &o.frames_dir {
        AnimTarget::PngDir(dir.clone())
    } else if o.gif || o.out.as_deref().is_some_and(|s| s.ends_with(".gif")) {
        AnimTarget::Gif(format!("{base}.gif"))
    } else {
        AnimTarget::Mp4(format!("{base}.mp4"))
    };
    if matches!(target, AnimTarget::Mp4(_)) && !animate::ffmpeg_available() {
        return Err("未找到 ffmpeg: 改用 --gif 或 --frames-dir, 或先安装 ffmpeg".to_string());
    }
    let opts = AnimOpts {
        canvas: o.canvas,
        width: o.width,
        height: o.height,
        ss: o.ss,
        fps: o.fps,
        secs: o.secs,
        story: o.story,
        hue_shift: o.hue,
        gain: o.gain,
        pad: 0.06,
        max_strokes: 250_000,
        target: target.clone(),
        quiet: o.quiet,
    };
    println!(
        "anim story={} {}x{} @{}fps {:.1}s | mode={} base={} dir={} rule={} steps={}",
        o.story.name(),
        o.width,
        o.height,
        o.fps,
        o.secs,
        if p.is_deep() {
            "deep (level trace)"
        } else {
            "enum"
        },
        p.base,
        p.dir,
        p.rule.name(),
        p.steps
    );
    let t0 = Instant::now();
    animate::run(&p, &opts).map_err(|e| format!("动画渲染失败: {e}"))?;
    let size = std::fs::metadata(target.path())
        .map(|m| human(m.len()))
        .unwrap_or_default();
    println!(
        "{} -> {} {size}  用时 {:.1}s",
        target.kind(),
        target.path(),
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("错误: {message}\n\n{USAGE}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = argv.split_first() else {
        gui::run(gui_default(), Canvas::Dark, gui::GuiOptions::default());
        return ExitCode::SUCCESS;
    };
    match command.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "gui" => match parse_opts(rest).and_then(|o| {
            if (o.width_explicit && o.width < 640) || (o.height_explicit && o.height < 720) {
                Err("GUI 窗口至少需要 640x720 像素".to_string())
            } else {
                Ok(o)
            }
        }) {
            Ok(o) => {
                gui::run(
                    if o.explicit {
                        o.params()
                    } else {
                        gui_default()
                    },
                    o.canvas,
                    gui::GuiOptions {
                        hue: o.hue,
                        width: if o.width_explicit { o.width } else { 1500 },
                        height: if o.height_explicit { o.height } else { 900 },
                        language: o.language,
                        frames_limit: o.frames,
                        shot: o.shot.clone(),
                    },
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        "render" => match parse_opts(rest).and_then(|o| render_cmd(&o)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        "anim" => match parse_opts(rest).and_then(|o| anim_cmd(&o)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e),
        },
        other => fail(&format!("未知命令 {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Result<Opts, String> {
        parse_opts(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn steps_accepts_human_notation_and_boundaries() {
        assert_eq!(options(&["--steps", "2^50"]).unwrap().steps, MAX_STEPS);
        assert_eq!(options(&["--steps", "1.5e6"]).unwrap().steps, 1_500_000);
        assert!(options(&["--steps", "2^51"]).is_err());
    }

    #[test]
    fn gui_language_option_accepts_only_supported_locales() {
        assert_eq!(
            options(&["--lang", "zh"]).unwrap().language,
            ui::Language::Zh
        );
        assert_eq!(
            options(&["--lang", "en"]).unwrap().language,
            ui::Language::En
        );
        assert!(options(&["--lang", "fr"]).is_err());
    }

    #[test]
    fn rejects_bad_parameters_before_render() {
        for args in [
            vec!["--base", "1"],
            vec!["--dir", "0"],
            vec!["--steps", "0"],
            vec!["--ss", "5"],
            vec!["--width", "1000000"],
            vec!["--width", "8192", "--height", "8192"],
            vec!["--hue", "NaN"],
            vec!["--gain", "inf"],
            vec!["--center", "NaN,0"],
            vec!["--zoom", "inf"],
            vec!["--out", " "],
            vec!["--frames", "0"],
        ] {
            assert!(options(&args).is_err(), "accepted {args:?}");
        }
    }
}
