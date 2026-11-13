mod lsp;

use crowmation_core::Timeline;
use std::process::ExitCode;

const USAGE: &str = "usage:
  crowmation preview <scene.cwm>
  crowmation render <scene.cwm> <out.mp4> [--encoder auto|nvenc|openh264] [--quality high]
  crowmation frame <scene.cwm> <seconds> <out.png>
  crowmation check <scene.cwm> [--dump-ir]
  crowmation lsp     (language server for editors, over stdio)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = match args.as_slice() {
        ["lsp"] => lsp::run(),
        ["preview", path] => crowmation_preview::run(path.into()),
        ["frame", path, t, out] => frame(path, t, out),
        ["render", path, out, rest @ ..] => render(path, out, rest),
        ["check", path] => load(path).map(|_| ()),
        ["check", path, "--dump-ir"] => load(path).map(|tl| println!("{}", tl.to_ron())),
        _ => Err(USAGE.into()),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: &str) -> Result<Timeline, String> {
    use std::io::IsTerminal;
    crowmation_lang::load(path.as_ref(), std::io::stderr().is_terminal()).0
}

fn frame(path: &str, t: &str, out: &str) -> Result<(), String> {
    let tl = load(path)?;
    let t: f64 = t.parse().map_err(|_| format!("bad time: {t}"))?;
    let mut r = crowmation_render::Headless::new(tl.width, tl.height)?;
    let px = r.render(&tl, t)?;
    image::save_buffer(out, &px, tl.width, tl.height, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

fn render(path: &str, out: &str, flags: &[&str]) -> Result<(), String> {
    use crowmation_encode::{Backend, Settings, encode_mp4};
    let (mut encoder, mut high_quality) = ("auto", false);
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        match (*f, it.next()) {
            ("--encoder", Some(e)) => encoder = e,
            ("--quality", Some(&"high")) => high_quality = true,
            ("--quality", Some(&"fast")) => high_quality = false,
            _ => return Err(USAGE.into()),
        }
    }
    let backend = match encoder {
        "auto" => Backend::Auto,
        "nvenc" => Backend::Nvenc,
        "openh264" => Backend::OpenH264,
        _ => return Err(format!("unknown encoder `{encoder}`, expected auto, nvenc or openh264")),
    };
    let tl = load(path)?;
    let start = std::time::Instant::now();
    let mut r = crowmation_render::Headless::new(tl.width, tl.height)?;
    let frames = tl.frame_count();
    let s = Settings { width: tl.width, height: tl.height, fps: tl.fps, high_quality, frames };
    let (order, lanes) = crowmation_encode::frame_order(&s, backend);
    // A few frames of slack lets rendering run ahead of encoding.
    let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<u8>)>(4);
    let out_path = std::path::PathBuf::from(out);
    let writer = std::thread::spawn(move || encode_mp4(&out_path, &s, backend, rx));
    let rendered = r.render_i420(&tl, &order, lanes, |i, f| tx.send((i, f.to_vec())).map_err(|_| "encoder stopped".to_string()));
    drop(tx);
    let used = writer.join().map_err(|_| "encoder thread panicked")??;
    rendered?;
    let secs = start.elapsed().as_secs_f64();
    eprintln!("{out}: {frames} frames in {secs:.2}s ({:.0} fps, {used})", frames as f64 / secs);
    Ok(())
}
