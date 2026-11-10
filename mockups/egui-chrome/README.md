# egui chrome mock

What the sidebar and the tab bar would look like built out of **egui widgets**,
next to the same chrome **painted by hand**. Same palette and proportions as the
real one (`crates/app/src/render.rs`): 18-cell column, 2.4-row card, 1-row bar,
`#000000` / `#d6d6d6` / `#7f7f7f` / `#cdcd00`.

Standalone on purpose — it carries its own `[workspace]` table, so the real
build never compiles egui.

| binary | what it is |
|---|---|
| `cargo run` | `src/main.rs`, the chrome painted with `painter` rects and text, the way the app draws today |
| `cargo run --bin widgets` | `src/ui.rs` + the eframe shell, the same chrome from `SidePanel`, `ScrollArea`, `Frame`, `Label`, `TextEdit` |
| `cargo run --release --bin bench` | what one rebuild costs: `Context::run` (layout + widgets) and `tessellate`, no GPU and no window |

`MOCK_FPS=10`, `60` or `max` sets the repaint rate of the widgets window. 10 is
the idle floor; input repaints immediately, which is what keeps hover and the
text cursor usable at that rate.
