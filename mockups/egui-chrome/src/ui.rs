//! The same chrome built out of **egui widgets** instead of painted rects: panels,
//! a `ScrollArea` with its own scrollbar, `Frame` cards, `Label`s, a `TextEdit`
//! for the rename, `Button`s for the tabs and a context menu. Everything the
//! crate refuses to hand-roll is here for free — and everything egui decides
//! (spacing, font, scrollbar) is what makes it stop looking like the grid.

use eframe::egui::{self, Color32, FontId, RichText};

pub const BG: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
pub const FG: Color32 = Color32::from_rgb(0xd6, 0xd6, 0xd6);
pub const DIM: Color32 = Color32::from_rgb(0x7f, 0x7f, 0x7f);
pub const ALERT: Color32 = Color32::from_rgb(0xcd, 0xcd, 0x00);

pub struct Tab {
    title: String,
    alert: bool,
}

pub struct Workspace {
    name: String,
    tabs: Vec<Tab>,
    front_tab: usize,
}

impl Workspace {
    fn new(name: &str, tabs: &[&str]) -> Workspace {
        Workspace {
            name: name.to_string(),
            tabs: tabs
                .iter()
                .map(|t| Tab {
                    title: (*t).to_string(),
                    alert: false,
                })
                .collect(),
            front_tab: 0,
        }
    }

    fn title(&self) -> &str {
        self.tabs.get(self.front_tab).map_or("", |t| t.title.as_str())
    }

    fn alert(&self) -> bool {
        self.tabs.iter().any(|t| t.alert)
    }
}

pub struct State {
    pub ws: Vec<Workspace>,
    pub front: usize,
    pub renaming: Option<usize>,
    pub rename: String,
    pub cost_ms: f64,
    /// The `ScrollArea`'s own scrollbar, to compare with the 8 px drawn one.
    pub show_scrollbar: bool,
    /// Repaints actually delivered per second, measured.
    pub fps: f64,
    pub last: Option<std::time::Instant>,
}

impl Default for State {
    fn default() -> State {
        State {
            ws: vec![
                Workspace::new("", &["zsh", "vim ~/notas.txt"]),
                Workspace::new("headless", &["cargo watch"]),
                Workspace::new("", &["mc", "ssh build"]),
                Workspace::new("logs", &["tail -f /var/log/syslog"]),
                Workspace::new("", &["pi"]),
            ],
            front: 0,
            renaming: None,
            rename: String::new(),
            cost_ms: 0.0,
            show_scrollbar: true,
            fps: 0.0,
            last: None,
        }
    }
}

impl State {
    fn move_ws(&mut self, from: usize, to: usize) {
        if from == to || from >= self.ws.len() || to >= self.ws.len() {
            return;
        }
        let w = self.ws.remove(from);
        self.ws.insert(to, w);
        self.front = match (self.front == from, from < self.front, to <= self.front) {
            (true, _, _) => to,
            (false, true, true) => self.front - 1,
            (false, false, false) => self.front + 1,
            _ => self.front,
        };
    }

    /// Tab strip: `selectable_label` per tab, a `×` on hover, a `+` at the end.
    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut close: Option<usize> = None;
            let mut select: Option<usize> = None;
            let front_tab = self.ws[self.front].front_tab;
            let n = self.ws[self.front].tabs.len();
            for (i, t) in self.ws[self.front].tabs.iter().enumerate() {
                let text = RichText::new(format!("{} {}", i + 1, t.title))
                    .monospace()
                    .color(if t.alert && i != front_tab { ALERT } else { FG });
                let r = ui.selectable_label(i == front_tab, text);
                if r.clicked() {
                    select = Some(i);
                }
                if n > 1 && r.hovered() {
                    let x = ui
                        .add(egui::Button::new(RichText::new("×").monospace()).frame(false))
                        .on_hover_text("close the tab");
                    if x.clicked() {
                        close = Some(i);
                    }
                }
            }
            if ui.button("+").clicked() {
                let tabs = &mut self.ws[self.front].tabs;
                let n = tabs.len() + 1;
                tabs.push(Tab {
                    title: format!("zsh {n}"),
                    alert: false,
                });
                self.ws[self.front].front_tab = tabs.len() - 1;
            }
            if let Some(i) = select {
                self.ws[self.front].front_tab = i;
            }
            if let Some(i) = close {
                self.ws[self.front].tabs.remove(i);
                let len = self.ws[self.front].tabs.len();
                self.ws[self.front].front_tab = self.ws[self.front].front_tab.min(len - 1);
            }
        });
    }

    /// The sidebar: a `ScrollArea` of `Frame` cards, two labels each.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if !self.show_scrollbar {
            area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }
        area.show(ui, |ui| {
            let mut target: Option<usize> = None;
            let mut rename: Option<usize> = None;
            let mut reorder: Option<(usize, usize)> = None;
            for i in 0..self.ws.len() {
                let active = i == self.front;
                let fill = if active { FG } else { BG };
                let (name, title, alert) = {
                    let w = &self.ws[i];
                    (w.name.clone(), w.title().to_string(), w.alert())
                };
                let fg = if active {
                    BG
                } else if alert {
                    ALERT
                } else {
                    DIM
                };
                let frame = egui::Frame::new().fill(fill).inner_margin(egui::Margin {
                    left: 6,
                    right: 6,
                    top: 4,
                    bottom: 6,
                });
                let resp = frame
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if self.renaming == Some(i) {
                            let fe = ui.add(
                                egui::TextEdit::singleline(&mut self.rename)
                                    .font(FontId::monospace(14.0))
                                    .desired_width(f32::INFINITY),
                            );
                            fe.request_focus();
                            if ui.input(|inp| inp.key_pressed(egui::Key::Enter)) {
                                self.ws[i].name = self.rename.clone();
                                self.renaming = None;
                            }
                            if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                                self.renaming = None;
                            }
                        } else if name.is_empty() {
                            ui.label(
                                RichText::new(format!("{} {title}", i + 1))
                                    .monospace()
                                    .color(fg),
                            );
                        } else {
                            ui.label(
                                RichText::new(format!("{} {name}", i + 1))
                                    .monospace()
                                    .color(fg),
                            );
                            ui.label(RichText::new(title).monospace().color(fg));
                        }
                    })
                    .response;
                // the whole card is the target, as it is in the real one
                let resp = resp.interact(egui::Sense::click_and_drag());
                if resp.clicked() {
                    target = Some(i);
                }
                if resp.double_clicked() {
                    rename = Some(i);
                    self.rename = self.ws[i].name.clone();
                }
                resp.context_menu(|ui| {
                    if ui.button("rename").clicked() {
                        self.renaming = Some(i);
                        self.rename = self.ws[i].name.clone();
                        ui.close_menu();
                    }
                    if ui.button("close").clicked() && self.ws.len() > 1 {
                        self.ws.remove(i);
                        self.front = self.front.min(self.ws.len() - 1);
                        ui.close_menu();
                    }
                });
                // egui's drag and drop: the payload is the index, the target is
                // whatever card the pointer is over
                if resp.dragged() {
                    egui::DragAndDrop::set_payload(ui.ctx(), i);
                }
                if resp.hovered() {
                    if let Some(from) = egui::DragAndDrop::payload::<usize>(ui.ctx()) {
                        reorder = Some((*from, i));
                    }
                }
            }
            if let Some((from, to)) = reorder {
                self.move_ws(from, to);
                egui::DragAndDrop::clear_payload(ui.ctx());
            }
            if let Some(t) = target {
                self.front = t;
            }
            if let Some(r) = rename {
                self.renaming = Some(r);
            }
        });
    }
}


/// Everything the window shows, without eframe: the eframe binary and the CPU
/// bench run the same function, so the number they print is the same cost.
pub fn chrome(ctx: &egui::Context, state: &mut State) {
    let mut style = (*ctx.style()).clone();
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::monospace(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::monospace(14.0));
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = Color32::from_gray(16);
    style.visuals.extreme_bg_color = BG;
    style.visuals.override_text_color = Some(FG);
    style.visuals.selection.bg_fill = FG;
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, BG);
    style.visuals.widgets.inactive.bg_fill = Color32::from_gray(20);
    style.visuals.widgets.hovered.bg_fill = Color32::from_gray(40);
    style.visuals.widgets.active.bg_fill = FG;
    style.spacing.item_spacing = egui::vec2(4.0, 2.0);
    style.spacing.button_padding = egui::vec2(4.0, 1.0);
    ctx.set_style(style);

    egui::TopBottomPanel::top("tabs")
        .frame(egui::Frame::new().fill(BG))
        .show(ctx, |ui| state.tabs(ui));
    egui::SidePanel::left("workspaces")
        .exact_width(180.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(BG))
        .show(ctx, |ui| state.sidebar(ui));
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(BG))
        .show(ctx, |ui| {
            for l in [
                "user@box ~/tortty (main)$ tortty --version",
                "tortty 1.0.56",
                "user@box ~/tortty (main)$ ./tools/quality-check.sh",
                "cargo fmt --check     ok",
                "clippy -D warnings    ok",
                "cargo test            ok",
                "all green",
                "",
            ] {
                ui.label(RichText::new(l).monospace().color(FG));
            }
        });
}
