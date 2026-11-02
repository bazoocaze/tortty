//! Estado do terminal: grade, cursor e parser VT.
//!
//! Sem GUI, sem pty, sem GPU — a mesma disciplina do `bzmux/term.py`. É o que
//! deixa o benchmark de parser rodar sem display e o que garante que o número
//! medido é do parser, não do toolkit.

use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

use crate::cell::{Cell, BOLD, INV, UL, WIDE_CONT};
use crate::charset::dec_graphics;
use crate::color::Color;
use crate::grid::Row;

const REPLACEMENT: char = '\u{FFFD}';

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    EscInter,
    Csi,
    Osc,
    OscEsc,
    /// DCS/APC/PM/SOS: consumidos até ST. Imagem do kitty cai aqui (fase 1
    /// ignora o conteúdo, mas não pode deixar lixo na grade).
    Str,
    StrEsc,
}

#[derive(Copy, Clone)]
struct SavedCursor {
    cx: usize,
    cy: usize,
    fg: Color,
    bg: Color,
    flags: u8,
    origin: bool,
    autowrap: bool,
}

impl Default for SavedCursor {
    fn default() -> Self {
        SavedCursor {
            cx: 0,
            cy: 0,
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
            origin: false,
            autowrap: true,
        }
    }
}

#[allow(clippy::struct_excessive_bools)] // estado do VT é booleano por natureza
pub struct Term {
    pub cols: usize,
    pub rows: usize,
    /// Grade viva (uma linha por `Row`, células contíguas).
    pub grid: Vec<Row>,
    /// Buffer principal guardado enquanto o app está em tela cheia.
    alt: Option<Vec<Row>>,
    pub scrollback: VecDeque<Row>,
    pub scrollback_limit: usize,
    pub cx: usize,
    pub cy: usize,
    pub fg: Color,
    pub bg: Color,
    pub flags: u8,
    /// Região de scroll (DECSTBM), inclusiva.
    pub top: usize,
    pub bottom: usize,
    pub wrap_pending: bool,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    /// `(forma 1..6, pisca)` do DECSCUSR; None = configuração.
    pub app_cursor_style: Option<(u8, bool)>,
    pub mouse_mode: u8,
    pub mouse_sgr: bool,
    pub focus_report: bool,
    pub bracketed_paste: bool,
    pub sync_output: bool,
    pub autowrap: bool,
    pub origin: bool,
    pub kbd_flags: u32,
    g0_dec: bool,
    g1_dec: bool,
    use_g1: bool,
    saved: SavedCursor,
    /// Respostas ao filho (DA, CPR, DECRQM...). O app drena e escreve no pty.
    pub child_out: Vec<u8>,
    pub title: String,
    pub cell_size: (u32, u32),
    pub bells: u32,
    pub respond_size: bool,
    tabstops: Vec<bool>,
    pub dirty: Vec<bool>,
    pub dirty_all: bool,
    /// Linhas que saíram pelo topo desde o início (índice absoluto).
    pub scrolled: u64,
    pub abs_gen: u64,
    pub hist_gen: u64,
    last_char: char,
    state: State,
    csi: Vec<u8>,
    osc: Vec<u8>,
    csi_prefix: u8,
    csi_inter: u8,
    esc_inter: u8,
    utf8: [u8; 4],
    utf8_len: u8,
    utf8_need: u8,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Term {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Term {
            cols,
            rows,
            grid: (0..rows).map(|_| Row::blank(cols)).collect(),
            alt: None,
            scrollback: VecDeque::new(),
            scrollback_limit: 1000,
            cx: 0,
            cy: 0,
            fg: Color::Default,
            bg: Color::Default,
            flags: 0,
            top: 0,
            bottom: rows - 1,
            wrap_pending: false,
            cursor_visible: true,
            app_cursor: false,
            app_cursor_style: None,
            mouse_mode: 0,
            mouse_sgr: false,
            focus_report: false,
            bracketed_paste: false,
            sync_output: false,
            autowrap: true,
            origin: false,
            kbd_flags: 0,
            g0_dec: false,
            g1_dec: false,
            use_g1: false,
            saved: SavedCursor::default(),
            child_out: Vec::new(),
            title: String::from("bzmux"),
            cell_size: (10, 19),
            bells: 0,
            respond_size: false,
            tabstops: (0..cols).map(|i| i % 8 == 0).collect(),
            dirty: vec![true; rows],
            dirty_all: true,
            scrolled: 0,
            abs_gen: 0,
            hist_gen: 0,
            last_char: ' ',
            state: State::Ground,
            csi: Vec::with_capacity(64),
            osc: Vec::with_capacity(128),
            csi_prefix: 0,
            csi_inter: 0,
            esc_inter: 0,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
        }
    }

    // ---------------------------------------------------------------- grade

    /// Célula em branco herda o bg corrente (BCE), como terminal de verdade.
    #[inline]
    fn blank_cell(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: self.bg,
            flags: 0,
        }
    }

    #[inline]
    pub fn touch(&mut self, y: usize) {
        if y < self.dirty.len() {
            self.dirty[y] = true;
        }
    }

    pub fn touch_all(&mut self) {
        self.dirty_all = true;
    }

    pub fn has_damage(&self) -> bool {
        self.dirty_all || self.dirty.iter().any(|&d| d)
    }

    pub fn clear_damage(&mut self) {
        self.dirty_all = false;
        self.dirty.fill(false);
    }

    pub fn reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let limit = self.scrollback_limit;
        let hist = self.hist_gen + 1;
        let abs = self.abs_gen + 1;
        *self = Term::new(cols, rows);
        self.scrollback_limit = limit;
        self.hist_gen = hist;
        self.abs_gen = abs;
    }

    /// DECSTR (soft reset): não mexe em grade nem em histórico.
    fn soft_reset(&mut self) {
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.flags = 0;
        self.cursor_visible = true;
        self.app_cursor = false;
        self.origin = false;
        self.autowrap = true;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.wrap_pending = false;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.dirty_all = true;
        self.abs_gen += 1;
        let removed = resize_main(
            &mut self.grid,
            &mut self.scrollback,
            self.scrollback_limit,
            cols,
            rows,
        );
        if let Some(alt) = self.alt.as_mut() {
            // Alt-screen não tem histórico: quem sai pela tela morre.
            crop_rows(alt, cols, rows);
        }
        self.cols = cols;
        self.rows = rows;
        self.tabstops.resize(cols, false);
        for (i, t) in self.tabstops.iter_mut().enumerate() {
            if i % 8 == 0 {
                *t = true;
            }
        }
        self.top = 0;
        self.bottom = rows - 1;
        self.cy = self.cy.saturating_sub(removed).min(rows - 1);
        self.cx = self.cx.min(cols - 1);
        self.wrap_pending = false;
        self.dirty = vec![true; rows];
    }

    // --------------------------------------------------------------- escrita

    #[inline]
    fn dec_active(&self) -> bool {
        if self.use_g1 {
            self.g1_dec
        } else {
            self.g0_dec
        }
    }

    fn put_ascii_run(&mut self, run: &[u8]) {
        let mut i = 0usize;
        while i < run.len() {
            if self.wrap_pending {
                self.do_wrap();
            }
            let room = self.cols - self.cx;
            if room == 0 {
                self.cx = self.cols - 1;
                if self.autowrap {
                    self.wrap_pending = true;
                    continue;
                }
                break;
            }
            let n = (run.len() - i).min(room);
            let fg = self.fg;
            let bg = self.bg;
            let flags = self.flags;
            let dec = self.dec_active();
            let cx = self.cx;
            let cy = self.cy;
            let row = &mut self.grid[cy];
            if dec {
                for k in 0..n {
                    let ch = dec_graphics(run[i + k] as char);
                    row.cells[cx + k] = Cell { ch, fg, bg, flags };
                }
            } else {
                for k in 0..n {
                    let ch = run[i + k] as char;
                    row.cells[cx + k] = Cell { ch, fg, bg, flags };
                }
            }
            self.last_char = run[i + n - 1] as char;
            self.dirty[cy] = true;
            i += n;
            if cx + n >= self.cols {
                self.cx = self.cols - 1;
                if self.autowrap {
                    self.wrap_pending = true;
                }
            } else {
                self.cx = cx + n;
            }
        }
    }

    fn do_wrap(&mut self) {
        self.grid[self.cy].wrapped = true;
        self.cx = 0;
        self.newline();
        self.wrap_pending = false;
    }

    fn put_char(&mut self, ch: char) {
        let w = ch.width().unwrap_or(1);
        if w == 0 {
            // combinante: a célula do bzmux Python também não compõe (nota no
            // TASK). Ignorar é melhor que desenhar uma caixinha.
            return;
        }
        if self.wrap_pending {
            self.do_wrap();
        }
        if w == 1 {
            let ch = if self.dec_active() {
                dec_graphics(ch)
            } else {
                ch
            };
            let cell = Cell {
                ch,
                fg: self.fg,
                bg: self.bg,
                flags: self.flags,
            };
            self.grid[self.cy].cells[self.cx] = cell;
            self.dirty[self.cy] = true;
            self.last_char = ch;
            if self.cx == self.cols - 1 {
                if self.autowrap {
                    self.wrap_pending = true;
                }
            } else {
                self.cx += 1;
            }
            return;
        }
        // largo: precisa de duas colunas livres
        if self.cx + 1 >= self.cols {
            if self.autowrap {
                self.do_wrap();
            } else {
                self.cx = self.cols.saturating_sub(2);
            }
        }
        let ch = if self.dec_active() {
            dec_graphics(ch)
        } else {
            ch
        };
        let first = Cell {
            ch,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        };
        let cont = Cell {
            ch: WIDE_CONT,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
        };
        let cx = self.cx;
        let cy = self.cy;
        self.grid[cy].cells[cx] = first;
        if cx + 1 < self.cols {
            self.grid[cy].cells[cx + 1] = cont;
        }
        self.dirty[cy] = true;
        self.last_char = ch;
        if cx + 2 >= self.cols {
            self.cx = self.cols - 1;
            if self.autowrap {
                self.wrap_pending = true;
            }
        } else {
            self.cx = cx + 2;
        }
    }

    // ---------------------------------------------------------------- cursor

    fn newline(&mut self) {
        if self.cy == self.bottom {
            self.scroll_up(1);
        } else if self.cy + 1 < self.rows {
            self.cy += 1;
        }
    }

    /// IND (ESC D): desce sem voltar a coluna.
    fn index(&mut self) {
        self.newline();
    }

    /// RI (ESC M): sobe; no topo da região, rola para baixo.
    fn reverse_index(&mut self) {
        if self.cy == self.top {
            self.scroll_down(1);
        } else {
            self.cy = self.cy.saturating_sub(1);
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.dirty_all = true;
        let full = self.top == 0 && self.bottom + 1 == self.rows;
        let keep = full && self.alt.is_none();
        for _ in 0..n {
            let row = self.grid.remove(self.top);
            if keep {
                self.scrollback.push_back(row);
                while self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.pop_front();
                }
            }
            let blank = Row::blank(self.cols);
            let at = self.bottom.min(self.grid.len());
            self.grid.insert(at, blank);
            self.scrolled += 1;
        }
        // a região pode ter encolhido (linhas fora dela não se movem)
        while self.grid.len() > self.rows {
            self.grid.pop();
        }
    }

    fn scroll_down(&mut self, n: usize) {
        self.dirty_all = true;
        for _ in 0..n {
            let at = self.bottom.min(self.grid.len().saturating_sub(1));
            if self.grid.len() > at {
                self.grid.remove(at);
            }
            self.grid.insert(self.top, Row::blank(self.cols));
            while self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    fn cursor_to(&mut self, r: isize, c: isize) {
        let (rlo, rhi) = if self.origin {
            (self.top as isize, self.bottom as isize)
        } else {
            (0, self.rows as isize - 1)
        };
        let r = if self.origin {
            r + self.top as isize
        } else {
            r
        };
        self.cy = r.clamp(rlo, rhi) as usize;
        self.cx = c.clamp(0, self.cols as isize - 1) as usize;
        self.wrap_pending = false;
    }

    fn forward_tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cx + 1;
            while x < self.cols && !self.tabstops.get(x).copied().unwrap_or(false) {
                x += 1;
            }
            self.cx = x.min(self.cols - 1);
        }
        self.wrap_pending = false;
    }

    fn backward_tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cx;
            loop {
                if x == 0 {
                    break;
                }
                x -= 1;
                if self.tabstops.get(x).copied().unwrap_or(false) {
                    break;
                }
            }
            self.cx = x;
        }
        self.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            cx: self.cx,
            cy: self.cy,
            fg: self.fg,
            bg: self.bg,
            flags: self.flags,
            origin: self.origin,
            autowrap: self.autowrap,
        };
    }

    fn restore_cursor(&mut self) {
        let s = self.saved;
        self.cx = s.cx.min(self.cols - 1);
        self.cy = s.cy.min(self.rows - 1);
        self.fg = s.fg;
        self.bg = s.bg;
        self.flags = s.flags;
        self.origin = s.origin;
        self.autowrap = s.autowrap;
        self.wrap_pending = false;
    }

    // ---------------------------------------------------------------- apagar

    fn erase_display(&mut self, n: u32) {
        self.touch_all();
        match n {
            2 | 3 => {
                let cell = self.blank_cell();
                for y in 0..self.rows {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
                if n == 3 {
                    self.scrollback.clear();
                    self.abs_gen += 1;
                    self.hist_gen += 1;
                }
                self.cx = 0;
                self.cy = 0;
            },
            1 => {
                let cell = self.blank_cell();
                for x in 0..=self.cx.min(self.cols - 1) {
                    self.grid[self.cy].cells[x] = cell;
                }
                for y in 0..self.cy {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
            },
            _ => {
                let cell = self.blank_cell();
                for x in self.cx..self.cols {
                    self.grid[self.cy].cells[x] = cell;
                }
                for y in self.cy + 1..self.rows {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
            },
        }
    }

    fn erase_line(&mut self, n: u32) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        match n {
            2 => {
                let mut row = Row::blank(self.cols);
                row.cells.fill(cell);
                self.grid[self.cy] = row;
            },
            1 => {
                for x in 0..=self.cx.min(self.cols - 1) {
                    self.grid[self.cy].cells[x] = cell;
                }
            },
            _ => {
                for x in self.cx..self.cols {
                    self.grid[self.cy].cells[x] = cell;
                }
            },
        }
    }

    fn insert_lines(&mut self, n: usize) {
        self.touch_all();
        if !(self.top..=self.bottom).contains(&self.cy) {
            return;
        }
        for _ in 0..n {
            let at = self.bottom.min(self.grid.len().saturating_sub(1));
            if at >= self.cy && self.grid.len() > at {
                self.grid.remove(at);
            }
            self.grid.insert(self.cy, Row::blank(self.cols));
            if self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    fn delete_lines(&mut self, n: usize) {
        self.touch_all();
        if !(self.top..=self.bottom).contains(&self.cy) {
            return;
        }
        for _ in 0..n {
            self.grid.remove(self.cy);
            let at = self.bottom.min(self.grid.len());
            self.grid.insert(at, Row::blank(self.cols));
            if self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    fn insert_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let x = self.cx;
        let row = &mut self.grid[self.cy];
        for _ in 0..n.min(self.cols) {
            row.cells.insert(x, cell);
            row.cells.truncate(self.cols);
        }
    }

    fn delete_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let x = self.cx;
        let row = &mut self.grid[self.cy];
        let end = (x + n).min(self.cols);
        row.cells.drain(x..end);
        while row.cells.len() < self.cols {
            row.cells.push(cell);
        }
    }

    fn erase_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let end = (self.cx + n).min(self.cols);
        for x in self.cx..end {
            self.grid[self.cy].cells[x] = cell;
        }
    }

    // ------------------------------------------------------------------ SGR

    fn sgr(&mut self, body: &[u8]) {
        let mut toks = Toks::new(body);
        while let Some(tok) = toks.next() {
            if tok.contains(&b':') {
                self.sgr_sub(tok);
                continue;
            }
            let code = parse_u32(tok);
            match code {
                0 => {
                    self.fg = Color::Default;
                    self.bg = Color::Default;
                    self.flags = 0;
                },
                1 => self.flags |= BOLD,
                4 => self.flags |= UL,
                7 => self.flags |= INV,
                22 => self.flags &= !BOLD,
                24 => self.flags &= !UL,
                27 => self.flags &= !INV,
                30..=37 => self.fg = Color::Idx((code - 30) as u8),
                38 => self.extended(&mut toks, true),
                39 => self.fg = Color::Default,
                40..=47 => self.bg = Color::Idx((code - 40) as u8),
                48 => self.extended(&mut toks, false),
                49 => self.bg = Color::Default,
                90..=97 => self.fg = Color::Idx((code - 90 + 8) as u8),
                100..=107 => self.bg = Color::Idx((code - 100 + 8) as u8),
                _ => {},
            }
        }
    }

    fn extended(&mut self, toks: &mut Toks<'_>, is_fg: bool) {
        let Some(mode) = toks.next_num() else { return };
        let color = match mode {
            5 => match toks.next_num() {
                Some(i) if i <= 255 => Color::Idx(i as u8),
                _ => return,
            },
            2 => {
                let (Some(r), Some(g), Some(b)) =
                    (toks.next_num(), toks.next_num(), toks.next_num())
                else {
                    return;
                };
                Color::rgb(r, g, b)
            },
            _ => return,
        };
        if is_fg {
            self.fg = color;
        } else {
            self.bg = color;
        }
    }

    /// SGR com subparâmetros (`38:2::r:g:b`, `48:5:196`, `4:3`).
    fn sgr_sub(&mut self, tok: &[u8]) {
        let mut vals = [0u32; 8];
        let mut n = 0usize;
        for part in tok.split(|&b| b == b':') {
            if n >= vals.len() {
                break;
            }
            vals[n] = parse_u32(part);
            n += 1;
        }
        if n == 0 {
            return;
        }
        match vals[0] {
            38 | 48 => {
                let is_fg = vals[0] == 38;
                let color = if n >= 2 && vals[1] == 2 {
                    if n < 5 {
                        return;
                    }
                    // o campo vazio é o "espaço de cor" omitido: vale o último trio
                    Color::rgb(vals[n - 3], vals[n - 2], vals[n - 1])
                } else if n >= 3 && vals[1] == 5 {
                    Color::Idx(vals[2].min(255) as u8)
                } else {
                    return;
                };
                if is_fg {
                    self.fg = color;
                } else {
                    self.bg = color;
                }
            },
            4 => self.flags |= UL,
            0 => {
                self.fg = Color::Default;
                self.bg = Color::Default;
                self.flags = 0;
            },
            _ => {},
        }
    }

    // ----------------------------------------------------------------- modos

    fn set_mode(&mut self, body: &[u8], private: bool, on: bool) {
        for t in body.split(|&b| b == b';') {
            let p = parse_u32(t);
            self.set_one(p, private, on);
        }
    }

    fn set_one(&mut self, p: u32, private: bool, on: bool) {
        if private {
            match p {
                1 => self.app_cursor = on,
                6 => self.origin = on,
                7 => self.autowrap = on,
                25 => self.cursor_visible = on,
                47 | 1047 | 1049 => {
                    if on {
                        self.enter_alt(p == 1049);
                    } else {
                        self.leave_alt();
                    }
                },
                1000 | 1002 | 1003 => {
                    self.mouse_mode = if on {
                        match p {
                            1000 => 1,
                            1002 => 2,
                            _ => 3,
                        }
                    } else {
                        0
                    }
                },
                1006 => self.mouse_sgr = on,
                1004 => self.focus_report = on,
                2004 => self.bracketed_paste = on,
                2026 => self.sync_output = on,
                _ => {},
            }
        }
        // modos ANSI (IRM=4, LNM=20) não são implementados de propósito;
        // ficam anotados no TASK como pendência
        let _ = p;
    }

    fn enter_alt(&mut self, clear_scrollback: bool) {
        if self.alt.is_some() {
            return;
        }
        self.dirty_all = true;
        self.save_cursor();
        self.abs_gen += 1;
        let blank = (0..self.rows).map(|_| Row::blank(self.cols)).collect();
        self.alt = Some(std::mem::replace(&mut self.grid, blank));
        if clear_scrollback {
            self.scrollback.clear();
            self.hist_gen += 1;
        }
        self.cx = 0;
        self.cy = 0;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.wrap_pending = false;
    }

    fn leave_alt(&mut self) {
        let Some(saved_grid) = self.alt.take() else {
            return;
        };
        self.dirty_all = true;
        self.abs_gen += 1;
        self.grid = saved_grid;
        if self.grid.len() != self.rows
            || self.grid.first().map(super::grid::Row::len) != Some(self.cols)
        {
            crop_rows(&mut self.grid, self.cols, self.rows);
        }
        self.restore_cursor();
        self.top = 0;
        self.bottom = self.rows - 1;
    }

    fn mode_state(&self, p: u32, private: bool) -> u32 {
        if !private {
            return 0;
        }
        match p {
            1 => onoff(self.app_cursor),
            6 => onoff(self.origin),
            7 => onoff(self.autowrap),
            25 => onoff(self.cursor_visible),
            47 | 1047 | 1049 => onoff(self.alt.is_some()),
            1000 => onoff(self.mouse_mode == 1),
            1002 => onoff(self.mouse_mode == 2),
            1003 => onoff(self.mouse_mode == 3),
            1004 => onoff(self.focus_report),
            1006 => onoff(self.mouse_sgr),
            2004 => onoff(self.bracketed_paste),
            2026 => onoff(self.sync_output),
            _ => 0,
        }
    }

    // ------------------------------------------------------------ dispatch

    #[allow(clippy::too_many_lines)] // tabela de despacho: partir só esconde o mapa
    fn csi_dispatch(&mut self, final_: u8) {
        let body = std::mem::take(&mut self.csi);
        let prefix = self.csi_prefix;
        let inter = self.csi_inter;
        self.csi_prefix = 0;
        self.csi_inter = 0;
        let private = prefix == b'?';
        let p = |idx: usize, default: u32| param(&body, idx, default);

        match final_ {
            b'm' => self.sgr(&body),
            b'A' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy - k, cx);
            },
            b'B' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy + k, cx);
            },
            b'C' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy, cx + k);
            },
            b'D' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy, cx - k);
            },
            b'E' => {
                let k = (p(0, 1).max(1)) as isize;
                let cy = self.cy as isize;
                self.cursor_to(cy + k, 0);
            },
            b'F' => {
                let k = (p(0, 1).max(1)) as isize;
                let cy = self.cy as isize;
                self.cursor_to(cy - k, 0);
            },
            b'G' | b'`' => {
                let k = p(0, 1).max(1);
                let cy = self.cy as isize;
                self.cursor_to(cy, k as isize - 1);
            },
            b'H' | b'f' => {
                let r = p(0, 1).max(1);
                let c = p(1, 1).max(1);
                self.cursor_to(r as isize - 1, c as isize - 1);
            },
            b'I' => self.forward_tab(p(0, 1).max(1) as usize),
            b'Z' => self.backward_tab(p(0, 1).max(1) as usize),
            b'J' => self.erase_display(p(0, 0)),
            b'K' => self.erase_line(p(0, 0)),
            b'L' => self.insert_lines(p(0, 1).max(1) as usize),
            b'M' => self.delete_lines(p(0, 1).max(1) as usize),
            b'P' => self.delete_chars(p(0, 1).max(1) as usize),
            b'@' => self.insert_chars(p(0, 1).max(1) as usize),
            b'X' => self.erase_chars(p(0, 1).max(1) as usize),
            b'S' => self.scroll_up(p(0, 1).max(1) as usize),
            b'T' => self.scroll_down(p(0, 1).max(1) as usize),
            b'b' => {
                let n = p(0, 1).max(1) as usize;
                let ch = self.last_char;
                for _ in 0..n {
                    self.put_char(ch);
                }
            },
            b'c' => {
                if prefix == b'>' {
                    // DA2: a resposta TEM de começar com '>'
                    self.child_out.extend_from_slice(b"\x1b[>1;100;0c");
                } else if body.is_empty() || body == b"0" {
                    self.child_out.extend_from_slice(b"\x1b[?62c");
                }
            },
            b'd' => {
                let r = p(0, 1).max(1);
                let cx = self.cx as isize;
                self.cursor_to(r as isize - 1, cx);
            },
            b'g' => match p(0, 0) {
                0 => {
                    if self.cx < self.tabstops.len() {
                        self.tabstops[self.cx] = false;
                    }
                },
                3 => {
                    self.tabstops.fill(false);
                },
                _ => {},
            },
            b'h' => self.set_mode(&body, private, true),
            b'l' => self.set_mode(&body, private, false),
            b'n' => match p(0, 0) {
                5 => self.child_out.extend_from_slice(b"\x1b[0n"),
                6 => {
                    if private {
                        self.child_out.extend_from_slice(
                            format!("\x1b[?{};{}R", self.cy + 1, self.cx + 1).as_bytes(),
                        );
                    } else {
                        self.child_out.extend_from_slice(
                            format!("\x1b[{};{}R", self.cy + 1, self.cx + 1).as_bytes(),
                        );
                    }
                },
                _ => {},
            },
            b'p' => {
                if inter == b'$' {
                    let ps = p(0, 0);
                    let st = self.mode_state(ps, private);
                    let pre = if private { "?" } else { "" };
                    self.child_out
                        .extend_from_slice(format!("\x1b[{pre}{ps};{st}$y").as_bytes());
                } else if inter == b'!' {
                    self.soft_reset(); // DECSTR
                }
            },
            b'q' => {
                if inter == b' ' {
                    let ps = p(0, 0);
                    self.app_cursor_style = if (1..=6).contains(&ps) {
                        let forma = ((ps - 1) / 2) as u8;
                        Some((forma, ps % 2 == 1))
                    } else {
                        None
                    };
                }
            },
            b'r' => {
                let t = p(0, 1).max(1) as isize - 1;
                let b = p(1, self.rows as u32).max(1) as isize - 1;
                if t >= 0 && t < b && b < self.rows as isize {
                    self.top = t as usize;
                    self.bottom = b as usize;
                    self.cursor_to(0, 0);
                }
            },
            b's' => {
                if prefix == 0 {
                    self.save_cursor();
                }
            },
            b'u' => {
                if prefix == 0 {
                    self.restore_cursor();
                }
            },
            b't' => match p(0, 0) {
                16 => {
                    let (cw, ch) = self.cell_size;
                    self.child_out
                        .extend_from_slice(format!("\x1b[6;{ch};{cw}t").as_bytes());
                },
                18 => {
                    self.child_out.extend_from_slice(
                        format!("\x1b[8;{};{}t", self.rows, self.cols).as_bytes(),
                    );
                },
                _ => {},
            },
            _ => {},
        }
        // devolve o buffer (sem realocar no próximo escape)
        self.csi = body;
        self.csi.clear();
    }

    fn osc_dispatch(&mut self, data: &[u8]) {
        // "0;título", "2;título", "8;;url", "4;idx;rgb"...
        let mut it = data.splitn(2, |&b| b == b';');
        let cmd = it.next().unwrap_or_default();
        let arg = it.next().unwrap_or_default();
        match parse_u32(cmd) {
            0 | 2 if !arg.is_empty() => {
                self.title = String::from_utf8_lossy(arg).to_string();
            },
            _ => {},
        }
    }

    // ---------------------------------------------------------------- parser

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut i = 0usize;
        while i < bytes.len() {
            match self.state {
                State::Ground => {
                    let start = i;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c < 0x20 || c == 0x7f || c >= 0x80 {
                            break;
                        }
                        i += 1;
                    }
                    if i > start {
                        self.put_ascii_run(&bytes[start..i]);
                    }
                    if i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        if c >= 0x80 {
                            self.utf8_byte(c);
                        } else {
                            self.control(c);
                        }
                    }
                },
                State::Escape => {
                    let b = bytes[i];
                    i += 1;
                    self.escape(b);
                },
                State::EscInter => {
                    let b = bytes[i];
                    i += 1;
                    self.escape_inter(b);
                },
                State::Csi => {
                    let b = bytes[i];
                    i += 1;
                    self.csi_byte(b);
                },
                State::Osc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        0x07 => {
                            let data = std::mem::take(&mut self.osc);
                            self.osc_dispatch(&data);
                            self.osc = data;
                            self.osc.clear();
                            self.state = State::Ground;
                        },
                        0x1b => self.state = State::OscEsc,
                        _ => {
                            if self.osc.len() < 4096 {
                                self.osc.push(b);
                            }
                        },
                    }
                },
                State::OscEsc => {
                    let b = bytes[i];
                    i += 1;
                    if b == b'\\' {
                        let data = std::mem::take(&mut self.osc);
                        self.osc_dispatch(&data);
                        self.osc = data;
                        self.osc.clear();
                        self.state = State::Ground;
                    } else {
                        // ESC perdido dentro do OSC: volta para o corpo
                        if self.osc.len() < 4096 {
                            self.osc.push(0x1b);
                            self.osc.push(b);
                        }
                        self.state = State::Osc;
                    }
                },
                State::Str => {
                    let b = bytes[i];
                    i += 1;
                    if b == 0x1b {
                        self.state = State::StrEsc;
                    }
                },
                State::StrEsc => {
                    let b = bytes[i];
                    i += 1;
                    if b == b'\\' {
                        self.state = State::Ground;
                    } else {
                        self.state = State::Str;
                    }
                },
            }
        }
    }

    fn control(&mut self, b: u8) {
        match b {
            0x07 => self.bells = self.bells.wrapping_add(1),
            0x08 => {
                self.cx = self.cx.saturating_sub(1);
                self.wrap_pending = false;
            },
            0x09 => {
                self.wrap_pending = false;
                self.forward_tab(1);
            },
            0x0a..=0x0c => {
                self.wrap_pending = false;
                self.newline();
            },
            0x0d => {
                self.cx = 0;
                self.wrap_pending = false;
            },
            0x0e => self.use_g1 = true,
            0x0f => self.use_g1 = false,
            0x18 | 0x1a => {
                self.state = State::Ground;
                self.csi.clear();
            },
            0x1b => {
                self.state = State::Escape;
                self.esc_inter = 0;
            },
            _ => {},
        }
    }

    fn escape(&mut self, b: u8) {
        match b {
            b'[' => {
                self.state = State::Csi;
                self.csi.clear();
                self.csi_prefix = 0;
                self.csi_inter = 0;
            },
            b']' => {
                self.state = State::Osc;
                self.osc.clear();
            },
            b'P' | b'_' | b'^' | b'X' => {
                self.state = State::Str;
            },
            0x20..=0x2f => {
                self.esc_inter = b;
                self.state = State::EscInter;
            },
            b'7' => {
                self.save_cursor();
                self.state = State::Ground;
            },
            b'8' => {
                self.restore_cursor();
                self.state = State::Ground;
            },
            b'D' => {
                self.state = State::Ground;
                self.wrap_pending = false;
                self.index();
            },
            b'E' => {
                self.state = State::Ground;
                self.cx = 0;
                self.wrap_pending = false;
                self.index();
            },
            b'M' => {
                self.state = State::Ground;
                self.reverse_index();
            },
            b'H' => {
                self.state = State::Ground;
                if self.cx < self.tabstops.len() {
                    self.tabstops[self.cx] = true;
                }
            },
            b'c' => {
                self.state = State::Ground;
                self.reset();
            },
            _ => {
                // ESC = / ESC > (keypad), ESC \ (ST), e o resto: sem efeito
                self.state = State::Ground;
            },
        }
    }

    fn escape_inter(&mut self, b: u8) {
        match self.esc_inter {
            b'(' => {
                self.g0_dec = b == b'0';
                self.state = State::Ground;
            },
            b')' => {
                self.g1_dec = b == b'0';
                self.state = State::Ground;
            },
            b'#' if b == b'8' => {
                // DECALN: tela cheia de 'E' (usado por teste de alinhamento)
                let cell = Cell {
                    ch: 'E',
                    fg: Color::Default,
                    bg: Color::Default,
                    flags: 0,
                };
                for y in 0..self.rows {
                    self.grid[y].cells.fill(cell);
                }
                self.touch_all();
                self.state = State::Ground;
            },
            _ => self.state = State::Ground,
        }
    }

    fn csi_byte(&mut self, b: u8) {
        match b {
            0x1b => {
                self.state = State::Escape;
                self.esc_inter = 0;
            },
            0x00..=0x1f => {},
            0x20..=0x2f => self.csi_inter = b,
            0x40..=0x7e => {
                self.state = State::Ground;
                self.csi_dispatch(b);
            },
            _ => {
                if self.csi.is_empty() && matches!(b, b'?' | b'>' | b'=' | b'<') {
                    self.csi_prefix = b;
                } else if self.csi.len() < 128 {
                    self.csi.push(b);
                }
            },
        }
    }

    fn utf8_byte(&mut self, b: u8) {
        loop {
            if self.utf8_need == 0 {
                if b < 0x80 {
                    self.put_char(b as char);
                    return;
                }
                self.utf8_need = match b {
                    0xc2..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf4 => 3,
                    _ => {
                        self.put_char(REPLACEMENT);
                        return;
                    },
                };
                self.utf8[0] = b;
                self.utf8_len = 1;
                return;
            }
            if b & 0xc0 == 0x80 {
                self.utf8[self.utf8_len as usize] = b;
                self.utf8_len += 1;
                self.utf8_need -= 1;
                if self.utf8_need == 0 {
                    let len = self.utf8_len as usize;
                    match std::str::from_utf8(&self.utf8[..len]) {
                        Ok(s) => {
                            if let Some(ch) = s.chars().next() {
                                self.put_char(ch);
                            }
                        },
                        Err(_) => self.put_char(REPLACEMENT),
                    }
                }
                return;
            }
            // continuação inválida: emite U+FFFD e reprocessa este byte
            self.put_char(REPLACEMENT);
            self.utf8_need = 0;
            self.utf8_len = 0;
        }
    }

    // ------------------------------------------------------------ leitura

    pub fn row_text(&self, y: usize) -> String {
        match self.grid.get(y) {
            Some(row) => row.text(),
            None => String::new(),
        }
    }

    /// Linhas visíveis, com `off` linhas de histórico acima do topo do vivo.
    pub fn visible_rows(&self, off: usize) -> Vec<&Row> {
        let sb = self.scrollback.len();
        let off = off.min(sb);
        let start = sb - off;
        let mut out = Vec::with_capacity(self.rows);
        for i in start..start + self.rows {
            if i < sb {
                out.push(&self.scrollback[i]);
            } else if let Some(r) = self.grid.get(i - sb) {
                out.push(r);
            }
        }
        out
    }

    pub fn history_text(&self, start: usize, count: usize) -> Vec<String> {
        self.scrollback
            .iter()
            .skip(start)
            .take(count)
            .map(super::grid::Row::text)
            .collect()
    }

    pub fn take_child_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.child_out)
    }

    pub fn in_alt_screen(&self) -> bool {
        self.alt.is_some()
    }
}

#[inline]
fn onoff(v: bool) -> u32 {
    if v {
        1
    } else {
        2
    }
}

/// Números do corpo do CSI, sem alocação: `/` é separador e `:` é subparâmetro.
struct Toks<'a> {
    body: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Toks<'a> {
    fn new(body: &'a [u8]) -> Self {
        Toks {
            body,
            pos: 0,
            done: false,
        }
    }
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.done {
            return None;
        }
        let start = self.pos;
        let mut i = start;
        while i < self.body.len() && self.body[i] != b';' {
            i += 1;
        }
        if i >= self.body.len() {
            self.done = true;
        } else {
            self.pos = i + 1;
        }
        Some(&self.body[start..i])
    }
    fn next_num(&mut self) -> Option<u32> {
        self.next().map(parse_u32)
    }
}

#[inline]
fn parse_u32(t: &[u8]) -> u32 {
    let mut v: u32 = 0;
    for &c in t {
        if c.is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add(u32::from(c - b'0'));
        } else {
            break;
        }
    }
    v
}

/// Parâmetro `idx` do corpo: token vazio vale 0; token inexistente, `default`.
fn param(body: &[u8], idx: usize, default: u32) -> u32 {
    let mut n = 0usize;
    let mut start = 0usize;
    loop {
        let mut j = start;
        while j < body.len() && body[j] != b';' {
            j += 1;
        }
        if n == idx {
            return parse_u32(&body[start..j]);
        }
        if j >= body.len() {
            return default;
        }
        n += 1;
        start = j + 1;
    }
}

/// Corta/preenche as linhas para o tamanho novo (sem reflow nesta fase).
fn crop_rows(grid: &mut Vec<Row>, cols: usize, rows: usize) {
    for r in grid.iter_mut() {
        r.cells.resize(cols, Cell::BLANK);
    }
    if grid.len() > rows {
        let excess = grid.len() - rows;
        grid.drain(0..excess);
    } else {
        while grid.len() < rows {
            grid.push(Row::blank(cols));
        }
    }
}

/// Resize do buffer principal: o que sai pelo topo vai para o histórico.
/// Devolve quantas linhas saíram do topo (para corrigir o cursor).
fn resize_main(
    grid: &mut Vec<Row>,
    scrollback: &mut VecDeque<Row>,
    limit: usize,
    cols: usize,
    rows: usize,
) -> usize {
    for r in grid.iter_mut() {
        r.cells.resize(cols, Cell::BLANK);
    }
    let mut removed = 0usize;
    if grid.len() > rows {
        removed = grid.len() - rows;
        for _ in 0..removed {
            let row = grid.remove(0);
            scrollback.push_back(row);
        }
        while scrollback.len() > limit {
            scrollback.pop_front();
        }
    } else {
        while grid.len() < rows {
            grid.push(Row::blank(cols));
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Term {
        Term::new(10, 4)
    }

    fn screen(t: &Term) -> Vec<String> {
        (0..t.rows).map(|y| t.row_text(y)).collect()
    }

    #[test]
    fn texto_simples_e_quebra() {
        let mut t = term();
        t.feed(b"abc");
        assert_eq!(t.row_text(0), "abc");
        assert_eq!((t.cy, t.cx), (0, 3));
        // '\n' desce sem voltar a coluna
        t.feed(b"\nxy");
        assert_eq!(t.row_text(1), "   xy");
    }

    #[test]
    fn autowrap_marca_quebra_mole() {
        let mut t = term();
        t.feed(b"0123456789A");
        assert_eq!(t.row_text(0), "0123456789");
        assert_eq!(t.row_text(1), "A");
        assert!(t.grid[0].wrapped);
        assert!(!t.grid[1].wrapped);
    }

    #[test]
    fn crlf_depois_de_linha_cheia_nao_pula_duas() {
        // o bug do P15: preencher a linha e depois \r\n avançava duas
        let mut t = term();
        t.feed(b"0123456789\r\nx");
        assert_eq!(t.row_text(1), "x");
        assert_eq!(t.row_text(2), "");
    }

    #[test]
    fn sgr_truecolor_por_celula() {
        let mut t = term();
        t.feed(b"\x1b[38;2;1;2;3m\x1b[48;2;4;5;6mA");
        let c = t.grid[0].cells[0];
        assert_eq!(c.ch, 'A');
        assert_eq!(c.fg, Color::Rgb(1, 2, 3));
        assert_eq!(c.bg, Color::Rgb(4, 5, 6));
    }

    #[test]
    fn sgr_256_e_basico() {
        let mut t = term();
        t.feed(b"\x1b[38;5;196m\x1b[1;4mA");
        let c = t.grid[0].cells[0];
        assert_eq!(c.fg, Color::Idx(196));
        assert_eq!(c.flags & BOLD, BOLD);
        assert_eq!(c.flags & UL, UL);
        t.feed(b"\x1b[0m");
        assert_eq!(t.fg, Color::Default);
        assert_eq!(t.flags, 0);
    }

    #[test]
    fn sgr_subparametro_com_espaco_de_cor() {
        let mut t = term();
        t.feed(b"\x1b[38:2::10:20:30mX");
        assert_eq!(t.grid[0].cells[0].fg, Color::Rgb(10, 20, 30));
        let mut t = term();
        t.feed(b"\x1b[48:5:196mX");
        assert_eq!(t.grid[0].cells[0].bg, Color::Idx(196));
    }

    #[test]
    fn erase_e_cursor() {
        let mut t = term();
        t.feed(b"abc\r\nxyz\x1b[2J");
        assert_eq!(screen(&t), vec!["", "", "", ""]);
        assert_eq!((t.cy, t.cx), (0, 0));
        t.feed(b"abc\x1b[1K");
        assert_eq!(t.row_text(0), "");
    }

    #[test]
    fn alt_screen_guarda_o_principal() {
        let mut t = term();
        t.feed(b"principal\r\n");
        t.feed(b"\x1b[?1049h");
        assert!(t.in_alt_screen());
        t.feed(b"dentro");
        assert_eq!(t.row_text(0), "dentro");
        t.feed(b"\x1b[?1049l");
        assert!(!t.in_alt_screen());
        assert_eq!(t.row_text(0), "principal");
        assert_eq!((t.cy, t.cx), (1, 0));
    }

    #[test]
    fn charset_dec_desenha_moldura() {
        let mut t = term();
        t.feed(b"\x1b(0lqk\x1b(B");
        assert_eq!(t.row_text(0), "┌─┐");
    }

    #[test]
    fn decstbm_rola_dentro_da_regiao() {
        let mut t = term();
        t.feed(b"1\r\n2\r\n3\r\n4");
        t.feed(b"\x1b[2;3r"); // linhas 2..3 (1-based)
        t.feed(b"\x1b[3;1H\n");
        assert_eq!(t.row_text(0), "1");
        assert_eq!(t.row_text(1), "3");
        assert_eq!(t.row_text(2), "");
        assert_eq!(t.row_text(3), "4");
    }

    #[test]
    fn il_dl_ich_dch() {
        let mut t = term();
        t.feed(b"abc\r\n");
        t.feed(b"\x1b[1;1H\x1b[1@");
        assert_eq!(t.row_text(0), " abc");
        t.feed(b"\x1b[1P");
        assert_eq!(t.row_text(0), "abc");
        t.feed(b"\x1b[L");
        assert_eq!(t.row_text(0), "");
        assert_eq!(t.row_text(1), "abc");
        t.feed(b"\x1b[M");
        assert_eq!(t.row_text(0), "abc");
    }

    #[test]
    fn salva_e_restaura_cursor() {
        let mut t = term();
        t.feed(b"\x1b[2;3H\x1b7\x1b[4;1H\x1b8X");
        assert_eq!(t.grid[1].cells[2].ch, 'X');
    }

    #[test]
    fn titulo_osc() {
        let mut t = term();
        t.feed(b"\x1b]0;meu titulo\x07");
        assert_eq!(t.title, "meu titulo");
        t.feed(b"\x1b]2;outro\x1b\\");
        assert_eq!(t.title, "outro");
    }

    #[test]
    fn respostas_ao_filho() {
        let mut t = term();
        t.feed(b"\x1b[c");
        assert_eq!(t.take_child_out(), b"\x1b[?62c".to_vec());
        t.feed(b"\x1b[>c");
        assert_eq!(t.take_child_out(), b"\x1b[>1;100;0c".to_vec());
        t.feed(b"\x1b[2;3H\x1b[6n");
        assert_eq!(t.take_child_out(), b"\x1b[2;3R".to_vec());
        t.feed(b"\x1b[?2026$p");
        assert_eq!(t.take_child_out(), b"\x1b[?2026;2$y".to_vec());
    }

    #[test]
    fn modo_autowrap_desligado_nao_quebra() {
        let mut t = term();
        t.feed(b"\x1b[?7l0123456789AB");
        assert_eq!(t.row_text(0), "012345678B");
        assert_eq!(t.row_text(1), "");
    }

    #[test]
    fn scrollback_guarda_o_que_saiu() {
        let mut t = term();
        for i in 0..8 {
            t.feed(format!("linha{i}\r\n").as_bytes());
        }
        assert!(!t.scrollback.is_empty());
        let hist = t.history_text(0, 10);
        assert_eq!(hist[0], "linha0");
    }

    #[test]
    fn resize_preserva_roda_para_o_historico() {
        let mut t = term();
        for i in 0..6 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        t.resize(20, 3);
        assert_eq!(t.rows, 3);
        assert!(t.scrollback.len() >= 3);
        // o que saiu pelo topo foi para o histórico; o vivo deslizou
        assert_eq!(t.row_text(0), "l4");
        assert_eq!(t.row_text(1), "l5");
        assert_eq!(t.row_text(2), "");
    }

    #[test]
    fn utf8_incremental_atravessa_chunks() {
        let mut t = term();
        let bytes = "▀".as_bytes();
        t.feed(&bytes[..1]);
        t.feed(&bytes[1..2]);
        t.feed(&bytes[2..]);
        assert_eq!(t.grid[0].cells[0].ch, '▀');
    }

    #[test]
    fn largo_ocupa_duas_celulas() {
        let mut t = term();
        t.feed("漢".as_bytes());
        assert_eq!(t.grid[0].cells[0].ch, '漢');
        assert!(t.grid[0].cells[1].is_cont());
        assert_eq!(t.cx, 2);
    }

    #[test]
    fn rep_repete_o_ultimo_caractere() {
        let mut t = term();
        t.feed(b"a\x1b[3b");
        assert_eq!(t.row_text(0), "aaaa");
    }
}
