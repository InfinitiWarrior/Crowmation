//! Benchmark: `cargo run --release -p crowmation-render --example bench <scene.cwm>`
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let tl = crowmation_lang::load(path.as_ref(), false).0.unwrap();
    let mut r = crowmation_render::Headless::new(tl.width, tl.height).unwrap();
    let n = tl.frame_count();
    let t = std::time::Instant::now();
    let mut scene = crowmation_render::vello::Scene::new();
    for i in 0..n { crowmation_render::build_scene(&tl, i as f64 / tl.fps as f64, Default::default(), &mut scene); }
    eprintln!("build_scene only: {:.0} fps", n as f64 / t.elapsed().as_secs_f64());
    let t = std::time::Instant::now();
    r.render_i420(&tl, n, |_, _| Ok(())).unwrap();
    eprintln!("render+convert+readback: {:.0} fps", n as f64 / t.elapsed().as_secs_f64());
}
