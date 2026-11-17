//! Mock of the tortty chrome in egui: the workspace sidebar (two-line cards,
//! 2.4 rows each, inverted front card) and the tab bar (one row, right of the
//! sidebar, showing the tabs **of the front workspace**). Same palette and the
//! same proportions as `crates/app/src/render.rs`, so the comparison is about the
//! widgets and not about the colours.
//!
//! Interaction is the point of the mock: click selects, double click renames in
//! place, drag reorders, the wheel walks the list — the behaviour egui would
//! give for free.

use eframe::egui;
use egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, Vec2};

const BG: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
const FG: Color32 = Color32::from_rgb(0xd6, 0xd6, 0xd6);
const DIM: Color32 = Color32::from_rgb(0x7f, 0x7f, 0x7f);
const ALERT: Color32 = Color32::from_rgb(0xcd, 0xcd, 0x00);
/// Sidebar width and card height, in cells — `render.rs` uses 18 and 2.4.
const SIDE_CELLS: f32 = 18.0;
const CARD_ROWS: f32 = 2.4;

struct Tab {
    title: String,
    alert: bool,
}

impl Tab {
    fn new(title: &str) -> Tab {
        Tab {
            title: title.to_string(),
            alert: false,
        }
    }
}

struct Workspace {
    name: String,
    tabs: Vec<Tab>,
    front_tab: usize,
}

impl Workspace {
    fn new(name: &str, tabs: &[&str]) -> Workspace {
        Workspace {
            name: name.to_string(),
            tabs: tabs.iter().map(|t| Tab::new(t)).collect(),
            front_tab: 0,
        }
    }

    /// The title the card shows: the tab in front of this workspace.
    fn title(&self) -> &str {
        self.tabs.get(self.front_tab).map_or("", |t| t.title.as_str())
    }

    /// A card is tinted while any of its tabs waits for attention (`M-K`).
    fn alert(&self) -> bool {
        self.tabs.iter().any(|t| t.alert)
    }
}

struct Mock {
    ws: Vec<Workspace>,
    front: usize,
    /// First card shown, in cards (the wheel walks it, `B42`).
    card_off: f32,
    renaming: Option<usize>,
    rename: String,
    /// The frame's own CPU cost, which is what an adoption would pay for.
    frame_ms: f64,
}

impl Default for Mock {
    fn default() -> Mock {
        Mock {
            ws: vec![
                Workspace::new("", &["zsh", "vim ~/notas.txt"]),
                Workspace::new("headless", &["cargo watch"]),
                Workspace::new("", &["mc", "ssh sandbox"]),
                Workspace::new("logs", &["tail -f /var/log/syslog"]),
                Workspace::new("", &["pi"]),
            ],
            front: 0,
            card_off: 0.0,
            renaming: None,
            rename: String::new(),
            frame_ms: 0.0,
        }
    }
}

impl Mock {
    fn select(&mut self, i: usize) {
        self.front = i;
    }

    fn move_ws(&mut self, from: usize, to: usize) {
        if from == to || from >= self.ws.len() || to >= self.ws.len() {
            return;
        }
        let w = self.ws.remove(from);
        self.ws.insert(to, w);
        // the front is a place in the list, so it travels like any other (`B49`)
        self.front = match (self.front == from, from < self.front, to <= self.front) {
            (true, _, _) => to,
            (false, true, true) => self.front - 1,
            (false, false, false) => self.front + 1,
            _ => self.front,
        };
    }

    /// Body of the window: everything is painted, like the real renderer does
    /// with one instance vector — no widgets are laid out by egui's panels.
    fn draw(&mut self, ui: &mut egui::Ui, cell: Vec2) {
        let area = ui.max_rect();
        let (cw, ch) = (cell.x, cell.y);
        let bar = ch;
        let side = if self.ws.len() > 1 { cw * SIDE_CELLS } else { 0.0 };
        let painter = ui.painter().clone();
        painter.rect_filled(area, 0.0, BG);

        let bar_rect = Rect::from_min_size(
            egui::pos2(area.left() + side, area.top()),
            Vec2::new(area.width() - side, bar),
        );
        self.tab_bar(ui, &painter, bar_rect, cw, ch);

        let side_rect = Rect::from_min_size(
            area.left_top(),
            Vec2::new(side, area.height()),
        );
        if side > 0.0 {
            self.sidebar(ui, &painter, side_rect, cw, ch);
        }

        // the grid, faked: it is the part egui would not own
        let grid = Rect::from_min_max(
            egui::pos2(area.left() + side, area.top() + bar),
            area.right_bottom(),
        );
        let font = FontId::monospace(ch * 0.78);
        let lines = [
            "user@sandbox ~/tortty (main)$ tortty --version",
            "tortty 1.0.56",
            "user@sandbox ~/tortty (main)$ ./tools/quality-check.sh",
            "cargo fmt --check     ok",
            "clippy -D warnings    ok",
            "cargo test            ok",
            "all green",
            "",
            "user@sandbox ~/tortty (main)$ ",
        ];
        painter.with_clip_rect(grid).text(
            grid.left_top() + Vec2::new(cw * 0.5, ch * 0.5),
            Align2::LEFT_TOP,
            lines.join("\n"),
            font,
            FG,
        );

        painter.text(
            area.right_bottom() - Vec2::new(ch * 0.5, ch * 0.4),
            Align2::RIGHT_BOTTOM,
            format!("chrome built in {:.2} ms", self.frame_ms),
            FontId::monospace(ch * 0.55),
            DIM,
        );
    }

    /// The tabs of the front workspace: what the strip above the grid carries.
    fn tab_bar(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        cw: f32,
        ch: f32,
    ) {
        painter.rect_filled(rect, 0.0, BG);
        let font = FontId::monospace(ch * 0.78);
        let mut x = rect.left();
        let mut select: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut add = false;
        let front_tab = self.ws[self.front].front_tab;
        for (i, t) in self.ws[self.front].tabs.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, t.title);
            let width = label.chars().count() as f32 * cw;
            if x + width > rect.right() {
                break;
            }
            let r = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(width, ch));
            let (bg, fg) = if i == front_tab {
                (FG, BG)
            } else if t.alert {
                (ALERT, BG)
            } else {
                (BG, DIM)
            };
            let hovered = ui.rect_contains_pointer(r);
            let bg = if hovered && i != front_tab {
                Color32::from_gray(24)
            } else {
                bg
            };
            painter.rect_filled(r, 0.0, bg);
            painter.text(
                r.left_center() + Vec2::new(cw * 0.5, 0.0),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                fg,
            );
            if ui.interact(r, Id::new(("tab", i)), Sense::click()).clicked() {
                select = Some(i);
            }
            // the `×` shows up on hover, as it does today
            if hovered && self.ws[self.front].tabs.len() > 1 {
                let x_rect = Rect::from_min_size(
                    egui::pos2(r.right() - cw * 1.5, r.top()),
                    Vec2::new(cw, ch),
                );
                let xr = ui.interact(x_rect, Id::new(("tab-close", i)), Sense::click());
                painter.text(
                    x_rect.center(),
                    Align2::CENTER_CENTER,
                    "×",
                    font.clone(),
                    if xr.hovered() { FG } else { fg },
                );
                if xr.clicked() {
                    close = Some(i);
                }
            }
            x += width;
        }
        let plus = Rect::from_min_size(egui::pos2(x + cw * 0.5, rect.top()), Vec2::new(cw * 2.0, ch));
        if plus.right() < rect.right() {
            let pr = ui.interact(plus, Id::new("tab-new"), Sense::click());
            painter.text(
                plus.left_center(),
                Align2::LEFT_CENTER,
                "+",
                font,
                if pr.hovered() { FG } else { DIM },
            );
            add = pr.clicked();
        }
        let ws = &mut self.ws[self.front];
        if let Some(i) = select {
            ws.front_tab = i;
        }
        if add {
            let n = ws.tabs.len() + 1;
            ws.tabs.push(Tab::new(&format!("zsh {n}")));
            ws.front_tab = ws.tabs.len() - 1;
        }
        if let Some(i) = close {
            if ws.tabs.len() > 1 {
                ws.tabs.remove(i);
                ws.front_tab = ws.front_tab.min(ws.tabs.len() - 1);
            }
        }
    }

    fn sidebar(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        cw: f32,
        ch: f32,
    ) {
        let width = rect.width() - cw * 0.5;
        let card_h = ch * CARD_ROWS;
        let (top, bottom) = (rect.top(), rect.bottom());
        let clip = painter.with_clip_rect(Rect::from_min_max(
            egui::pos2(rect.left(), top),
            egui::pos2(rect.left() + width, bottom),
        ));
        let font = FontId::monospace(ch * 0.78);
        let mut target: Option<usize> = None;
        let mut rename: Option<usize> = None;
        let mut move_to: Option<(usize, usize)> = None;

        let first = self.card_off.floor() as usize;
        for (i, w) in self.ws.iter().enumerate().skip(first) {
            let y = top + (i as f32 - self.card_off) * card_h;
            if y >= bottom {
                break;
            }
            let y1 = (y + card_h).min(bottom);
            let r = Rect::from_min_size(
                egui::pos2(rect.left(), y),
                Vec2::new(width, (y1 - y).max(0.0)),
            );
            let active = i == self.front;
            let (bg, fg) = if active { (FG, BG) } else { (BG, DIM) };
            let resp = ui.interact(r, Id::new(("card", i)), Sense::click_and_drag());
            if resp.hovered() && !active {
                clip.rect_filled(r, 0.0, Color32::from_gray(18));
            }
            clip.rect_filled(r, 0.0, bg);
            let fg = if w.alert() { ALERT } else { fg };
            // two lines, exactly like `card_lines` (`B43`): the name on the first
            // row when there is one, the title on the second
            let (first_line, second) = if w.name.is_empty() {
                (format!("{} {}", i + 1, w.title()), String::new())
            } else {
                (format!("{} {}", i + 1, w.name), w.title().to_string())
            };
            clip.text(
                r.left_top() + Vec2::new(cw * 0.5, ch * 0.18),
                Align2::LEFT_TOP,
                first_line,
                font.clone(),
                fg,
            );
            if !second.is_empty() {
                clip.text(
                    r.left_top() + Vec2::new(cw * 0.5, ch * 0.18 + ch),
                    Align2::LEFT_TOP,
                    second,
                    font.clone(),
                    fg,
                );
            }
            if resp.double_clicked() {
                rename = Some(i);
            } else if resp.clicked() {
                target = Some(i);
            }
            if resp.dragged() {
                if let Some(p) = ui.ctx().pointer_interact_pos() {
                    let dest = first + ((p.y - top) / card_h).floor() as usize;
                    if dest != i && dest < self.ws.len() {
                        move_to = Some((i, dest));
                    }
                }
            }
        }
        if let Some((from, to)) = move_to {
            self.move_ws(from, to);
        }
        if let Some(t) = target {
            self.select(t);
        }
        if let Some(i) = rename {
            self.renaming = Some(i);
            self.rename = self.ws[i].name.clone();
        }

        // the rename field lives in the card, so it is drawn there
        if let Some(i) = self.renaming {
            let y = top + (i as f32 - self.card_off) * card_h;
            let r = Rect::from_min_size(
                egui::pos2(rect.left() + cw * 0.5, y + ch * 0.1),
                Vec2::new(width - cw, ch),
            );
            let fe = ui.put(
                r,
                egui::TextEdit::singleline(&mut self.rename)
                    .font(FontId::monospace(ch * 0.78))
                    .desired_width(width - cw),
            );
            fe.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.ws[i].name = self.rename.clone();
                self.renaming = None;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.renaming = None;
            }
        }

        // the wheel walks the list (`B42`)
        if ui.rect_contains_pointer(rect) {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                let visible = (bottom - top) / card_h;
                let max = (self.ws.len() as f32 - visible).max(0.0);
                self.card_off = (self.card_off - dy / card_h * 0.15).clamp(0.0, max.ceil());
            }
        }
    }
}

impl eframe::App for Mock {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(14.0));
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = BG;
        style.visuals.extreme_bg_color = BG;
        style.visuals.widgets.noninteractive.bg_fill = BG;
        style.visuals.widgets.inactive.bg_fill = Color32::from_gray(20);
        style.visuals.widgets.hovered.bg_fill = Color32::from_gray(40);
        style.visuals.widgets.active.bg_fill = FG;
        style.visuals.selection.bg_fill = FG;
        style.visuals.selection.stroke = Stroke::new(1.0_f32, BG);
        style.visuals.override_text_color = Some(FG);
        ctx.set_style(style);

        // the cell comes from the font, as it does in the real glyph atlas
        let font = FontId::monospace(14.0);
        let galley = ctx.fonts(|f| f.layout_no_wrap("M".to_owned(), font.clone(), FG));
        let cell = Vec2::new(galley.size().x.max(7.0), galley.size().y.max(14.0));

        let t0 = std::time::Instant::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ctx, |ui| self.draw(ui, cell));
        self.frame_ms = t0.elapsed().as_secs_f64() * 1000.0;
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_title("tortty chrome — egui mock"),
        ..Default::default()
    };
    eframe::run_native(
        "egui-chrome",
        opts,
        Box::new(|_cc| Ok(Box::<Mock>::default())),
    )
}
