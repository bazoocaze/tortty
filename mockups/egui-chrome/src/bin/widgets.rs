//! eframe shell: the chrome from `ui.rs`, a footer with the cost, and the
//! repaint rate as a knob (`MOCK_FPS=10`, `60` or `max`).

#[path = "../ui.rs"]
mod ui;

use std::time::Instant;

use eframe::egui;
use ui::State;

struct App {
    state: State,
    cost_ms: f64,
    fps: f64,
    last: Option<Instant>,
}

impl Default for App {
    fn default() -> App {
        App {
            state: State::default(),
            cost_ms: 0.0,
            fps: 0.0,
            last: None,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        ui::chrome(ctx, &mut self.state);
        egui::TopBottomPanel::bottom("foot")
            .frame(egui::Frame::new().fill(ui::BG).inner_margin(egui::Margin::same(4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.state.show_scrollbar, "egui's scrollbar");
                    ui.label(
                        egui::RichText::new(format!(
                            "chrome built in {:.2} ms at {:.0} fps — right click a card for the menu",
                            self.cost_ms, self.fps
                        ))
                        .monospace()
                        .color(ui::DIM),
                    );
                });
            });
        self.cost_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if let Some(now) = self.last {
            let dt = now.elapsed().as_secs_f64();
            if dt > 0.0 {
                self.fps = 0.9 * self.fps + 0.1 / dt;
            }
        }
        self.last = Some(Instant::now());
        match std::env::var("MOCK_FPS").ok().as_deref() {
            Some("max") => ctx.request_repaint(),
            other => {
                let fps: f64 = other.and_then(|v| v.parse().ok()).unwrap_or(4.0);
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / fps));
            },
        }
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_title("tortty chrome — egui widgets"),
        ..Default::default()
    };
    eframe::run_native(
        "egui-widgets",
        opts,
        Box::new(|_cc| Ok(Box::<App>::default())),
    )
}
