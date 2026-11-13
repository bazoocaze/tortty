//! tortty emulation core.
//!
//! Rule of the crate: **nothing here knows about GUI, pty or
//! GPU**. All this crate does is turn bytes from the child into a grid of cells,
//! and that is why the parser benchmark runs without a display.

pub mod cell;
pub mod charset;
pub mod checks;
pub mod checks_n2;
pub mod checks_n5;
pub mod color;
pub mod config;
pub mod graphics;
pub mod grid;
pub mod keymap;
pub mod prompt;
pub mod search;
pub mod select;
pub mod snapshot;
pub mod status;
pub mod term;

pub use cell::Cell;
pub use color::Color;
pub use grid::Row;
pub use select::{Anchor, Span};
pub use term::Term;
