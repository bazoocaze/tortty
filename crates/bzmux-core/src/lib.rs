//! Núcleo de emulação do bzmux.
//!
//! Regra herdada do bzmux Python: **nada aqui conhece GUI, pty ou GPU**. Tudo
//! que este crate faz é transformar bytes do filho em grade de células, e é por
//! isso que o benchmark de parser roda sem display.

pub mod cell;
pub mod charset;
pub mod color;
pub mod grid;
pub mod term;

pub use cell::Cell;
pub use color::Color;
pub use grid::Row;
pub use term::Term;
