//! Headless parser benchmark: feeds `Term` with the exact doom-fire frame and
//! reports µs/cell, MB/s and ns/escape.
//!
//! Same load as `tools/doomfire.py` (truecolor fg + truecolor bg + `▀` per cell,
//! with no grouping), so the number is comparable with the bzmux Python one
//! measured in docs/BENCHMARKS.md (~35 µs/cell before P20, 5,5 after).
//!
//! ```sh
//! cargo run --release -p bzmux-core --bin parsebench
//! cargo run --release -p bzmux-core --bin parsebench -- --cols 61 --rows 18 --frames 500
//! ```

use std::time::Instant;

use bzmux_core::Term;

/// The 37 doom-fire tones (the generator picks one per cell, so the color varies
/// from cell to cell — that is what stresses SGR).
const TONS: [(u8, u8, u8); 8] = [
    (0x07, 0x07, 0x07),
    (0x47, 0x0f, 0x07),
    (0x8f, 0x27, 0x07),
    (0xbf, 0x47, 0x07),
    (0xdf, 0x57, 0x07),
    (0xcf, 0x77, 0x0f),
    (0xb7, 0xb7, 0x2f),
    (0xff, 0xff, 0xff),
];

/// One whole frame: `CSI H` + (fg + bg + `▀`) per half-line cell.
fn frame(cols: usize, rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(cols * rows * 40);
    out.extend_from_slice(b"\x1b[H");
    for row in 0..rows {
        for x in 0..cols {
            let i = (row * cols + x) % TONS.len();
            let (r, g, b) = TONS[i];
            let (r2, g2, b2) = TONS[(i + 3) % TONS.len()];
            out.extend_from_slice(format!("\x1b[38;2;{r};{g};{b}m").as_bytes());
            out.extend_from_slice(format!("\x1b[48;2;{r2};{g2};{b2}m").as_bytes());
            out.extend_from_slice("▀".as_bytes());
        }
    }
    out
}

fn arg(name: &str, default: usize) -> usize {
    let args: Vec<String> = std::env::args().collect();
    for (i, a) in args.iter().enumerate() {
        if a == &format!("--{name}") {
            if let Some(v) = args.get(i + 1) {
                if let Ok(n) = v.parse() {
                    return n;
                }
            }
        }
    }
    default
}

fn arg_s(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    for (i, a) in args.iter().enumerate() {
        if a == &format!("--{name}") {
            return args.get(i + 1).cloned();
        }
    }
    None
}

/// "Real load" mode: dumps a file (the system `dmesg`, with and without color)
/// into the parser, the way the Python BENCHMARKS does with the `Term` without GTK.
fn bench_file(path: &str, cols: usize, rows: usize) {
    const ROUNDS: usize = 20;
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not open {path}: {e}");
            return;
        },
    };
    let mut term = Term::new(cols, rows);
    term.scrollback_limit = 100_000;
    // warm up
    for _ in 0..3 {
        term.feed(&data);
        term.clear_damage();
    }
    let mut best = f64::MAX;
    for _ in 0..ROUNDS {
        let t0 = Instant::now();
        term.feed(&data);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        term.clear_damage();
        if ms < best {
            best = ms;
        }
    }
    #[allow(clippy::naive_bytecount)] // one pass, outside the timer
    let lines = data.iter().filter(|&&b| b == b'\n').count();
    println!("file         {} ({} B, {} lines)", path, data.len(), lines);
    println!("grid         {cols}x{rows}");
    println!(
        "parse        {:.2} ms  ({:.2} MB/s, {:.1} µs/line) — best of {}",
        best,
        data.len() as f64 / (best / 1e3) / 1e6,
        best * 1000.0 / lines.max(1) as f64,
        ROUNDS
    );
}

fn main() {
    if let Some(path) = arg_s("file") {
        let cols = arg("cols", 100);
        let rows = arg("rows", 40);
        bench_file(&path, cols, rows);
        return;
    }
    let cols = arg("cols", 61);
    let rows = arg("rows", 18);
    let frames = arg("frames", 500);
    let without_draw = std::env::args().any(|a| a == "--no-damage");

    let mut term = Term::new(cols, rows);
    term.scrollback_limit = 1000;

    // warm up (the first frame allocates rows, the wide-char table, etc.)
    let payload = frame(cols, rows);
    for _ in 0..20 {
        term.feed(&payload);
        term.clear_damage();
    }

    let cells_per_frame = cols * rows;
    let mut best = f64::MAX;
    let mut best_mbs = 0.0;
    let mut total_bytes = 0usize;
    let mut total_ns = 0u128;

    for _ in 0..frames {
        let t0 = Instant::now();
        term.feed(&payload);
        let ns = t0.elapsed().as_nanos();
        if !without_draw {
            let _ = term.has_damage();
            term.clear_damage();
        }
        total_ns += ns;
        total_bytes += payload.len();
        let us_cell = ns as f64 / 1000.0 / cells_per_frame as f64;
        if us_cell < best {
            best = us_cell;
            best_mbs = payload.len() as f64 / (ns as f64 / 1e9) / 1e6;
        }
    }

    let avg_ns = total_ns as f64 / frames as f64;
    let avg_us_cell = avg_ns / 1000.0 / cells_per_frame as f64;
    let avg_mbs = total_bytes as f64 / (total_ns as f64 / 1e9) / 1e6;
    let escapes = 1 + cells_per_frame * 2;
    let ns_escape = avg_ns / escapes as f64;

    println!("grid         {cols}x{rows} ({cells_per_frame} cells)");
    println!(
        "frame        {} B ({} B/cell)",
        payload.len(),
        payload.len() / cells_per_frame
    );
    println!("frames       {frames}");
    println!("mean         {avg_us_cell:.3} µs/cell  {avg_mbs:.1} MB/s  {ns_escape:.1} ns/escape");
    println!("best         {best:.3} µs/cell  {best_mbs:.1} MB/s");
    println!("reference    python post-P20 5,49 µs/cell · python pre-P20 35,0 · kitty ~1,0");
    if best > 0.0 {
        println!(
            "speedup      {:.1}x over python post-P20, {:.1}x over pre-P20",
            5.49 / best,
            35.0 / best
        );
    }
}
