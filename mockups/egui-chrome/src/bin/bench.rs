//! What the egui layer costs per rebuild, with no GPU and no window: `Context::run`
//! (layout + widgets + the shapes) and `tessellate` (shapes -> triangles). This is
//! the number that the repaint frequency multiplies — the app's own frame rate
//! does not, because between rebuilds the same primitives are re-issued.

#[path = "../ui.rs"]
mod ui;

use std::time::Instant;

use eframe::egui;

fn main() {
    let ctx = egui::Context::default();
    let mut state = ui::State::default();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(980.0, 640.0),
        )),
        ..Default::default()
    };

    let frames = 200;
    let warm = 20;
    let (mut run_ms, mut tess_ms, mut tris) = (0.0, 0.0, 0usize);
    for i in 0..(frames + warm) {
        let t0 = Instant::now();
        let out = ctx.run(raw(), |ctx| ui::chrome(ctx, &mut state));
        let run = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let tess = t1.elapsed().as_secs_f64() * 1000.0;
        if i >= warm {
            run_ms += run;
            tess_ms += tess;
            tris = prims
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.indices.len() / 3,
                    egui::epaint::Primitive::Callback(_) => 0,
                })
                .sum();
        }
    }
    let n = f64::from(frames);
    let (run, tess) = (run_ms / n, tess_ms / n);
    println!("run (layout + widgets): {run:.2} ms");
    println!("tessellate:             {tess:.2} ms");
    println!("total per rebuild:      {:.2} ms, {tris} triangles", run + tess);
    for fps in [10.0, 60.0] {
        println!(
            "  at {fps:.0} fps: {:.1} ms/s of CPU = {:.1}% of a core",
            (run + tess) * fps,
            (run + tess) * fps / 10.0
        );
    }
}
