//! End-to-end CLI tests: the binary must render a PNG/SVG and an animated GIF.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_fractal")
}

#[test]
fn render_command_writes_png_and_svg() {
    let dir = std::env::temp_dir().join("ffbd_cli_render");
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.join("curve");
    let status = Command::new(bin())
        .args([
            "render",
            "--base",
            "3",
            "--dir",
            "5",
            "--steps",
            "4096",
            "--width",
            "120",
            "--height",
            "90",
            "--ss",
            "1",
            "--out",
            &out.to_string_lossy(),
        ])
        .status()
        .expect("spawn fractal render");
    assert!(status.success(), "render exited with {status}");
    let png = std::fs::read(format!("{}.png", out.display())).expect("png written");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() > 500, "png too small: {}", png.len());
    let svg = std::fs::read_to_string(format!("{}.svg", out.display())).expect("svg written");
    assert!(svg.contains("<polyline"));
}

#[test]
fn anim_command_writes_a_looping_gif() {
    let dir = std::env::temp_dir().join("ffbd_cli_anim");
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.join("loop");
    let status = Command::new(bin())
        .args([
            "anim",
            "--base",
            "2",
            "--dir",
            "5",
            "--steps",
            "2187",
            "--width",
            "96",
            "--height",
            "80",
            "--ss",
            "1",
            "--fps",
            "10",
            "--secs",
            "1",
            "--gif",
            "--quiet",
            "--out",
            &out.to_string_lossy(),
        ])
        .status()
        .expect("spawn fractal anim");
    assert!(status.success(), "anim exited with {status}");
    let gif = std::fs::read(format!("{}.gif", out.display())).expect("gif written");
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(gif[gif.len() - 1], 0x3b, "missing GIF trailer");
    assert!(gif.len() > 200, "gif too small: {}", gif.len());
}

#[test]
fn unknown_flag_reports_usage_and_exits_two() {
    let out = Command::new(bin()).args(["render", "--bogus"]).output().expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("未知参数"), "stderr was: {stderr}");
}
