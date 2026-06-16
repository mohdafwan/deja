//! Terminal screen model — vte se ANSI parse karke ek grid maintain karta hai.
//! Phase 1.1: common interactive shell cases (prompt, typing, output, colors,
//! clear, cursor moves) handle karta hai. Full TUI (vim/htop) baad me harden.

use egui::Color32;
use vte::{Params, Perform};

pub const FG: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
pub const BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);

const MAX_SCROLLBACK: usize = 5000;

// VS Code-ish ANSI palette.
const NORMAL: [Color32; 8] = [
    Color32::from_rgb(0x00, 0x00, 0x00),
    Color32::from_rgb(0xcd, 0x31, 0x31),
    Color32::from_rgb(0x0d, 0xbc, 0x79),
    Color32::from_rgb(0xe5, 0xe5, 0x10),
    Color32::from_rgb(0x24, 0x72, 0xc8),
    Color32::from_rgb(0xbc, 0x3f, 0xbc),
    Color32::from_rgb(0x11, 0xa8, 0xcd),
    Color32::from_rgb(0xe5, 0xe5, 0xe5),
];
const BRIGHT: [Color32; 8] = [
    Color32::from_rgb(0x66, 0x66, 0x66),
    Color32::from_rgb(0xf1, 0x4c, 0x4c),
    Color32::from_rgb(0x23, 0xd1, 0x8b),
    Color32::from_rgb(0xf5, 0xf5, 0x43),
    Color32::from_rgb(0x3b, 0x8e, 0xea),
    Color32::from_rgb(0xd6, 0x70, 0xd6),
    Color32::from_rgb(0x29, 0xb8, 0xdb),
    Color32::from_rgb(0xff, 0xff, 0xff),
];

fn ansi_color(idx: u16, bright: bool) -> Color32 {
    let table = if bright { &BRIGHT } else { &NORMAL };
    table[(idx as usize) % 8]
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Color32,
    pub bg: Color32,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', fg: FG, bg: BG }
    }
}

/// Shell integration (OSC 133 ; D) se aaya ek complete command event.
pub struct CmdEvent {
    pub exit: i64,
    pub command: String,
    pub cwd: String,
    pub block_id: u64,
}

/// Ek "block" boundary — OSC 133 ; A (prompt start) pe banta hai. Warp-style
/// block rendering ke liye: block content = [start .. next boundary).
pub struct Boundary {
    pub id: u64,
    /// global line index (scrollback.len() + cy us waqt) jaha block shuru hua.
    pub start: usize,
    /// global line jaha se output shuru hua (OSC 133 ; C). Prompt+command isse pehle.
    pub output_start: usize,
    pub command: Option<String>,
    pub exit: Option<i64>,
    pub when: i64,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    /// Command kitne ms me chali (C se D tak). None = abhi pata nahi.
    pub dur_ms: Option<u64>,
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    pub grid: Vec<Vec<Cell>>,
    pub scrollback: Vec<Vec<Cell>>,
    pub cx: usize,
    pub cy: usize,
    /// Completed command events — main loop inhe drain karke deja-core ko deta hai.
    pub events: Vec<CmdEvent>,
    /// Warp-style block boundaries (OSC 133 ; A se).
    pub boundaries: Vec<Boundary>,
    /// Current command ka start time (OSC 133 ; C pe set, D pe duration nikaalte).
    cmd_start: Option<std::time::Instant>,
    /// Alternate screen (vim/htop jaise full-screen apps) — fixed grid, no scrollback/blocks.
    pub alt_active: bool,
    pub alt: Vec<Vec<Cell>>,
    pub acx: usize,
    pub acy: usize,
    next_block_id: u64,
    cur_fg: Color32,
    cur_bg: Color32,
    bold: bool,
}

impl Screen {
    pub fn new(rows: usize, cols: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Screen {
            cols,
            rows,
            grid: vec![vec![Cell::default(); cols]; rows],
            scrollback: Vec::new(),
            cx: 0,
            cy: 0,
            events: Vec::new(),
            boundaries: Vec::new(),
            cmd_start: None,
            alt_active: false,
            alt: vec![vec![Cell::default(); cols]; rows],
            acx: 0,
            acy: 0,
            next_block_id: 0,
            cur_fg: FG,
            cur_bg: BG,
            bold: false,
        }
    }

    /// Abhi cursor kis global line pe hai (scrollback + grid me).
    fn cur_global(&self) -> usize {
        self.scrollback.len() + self.cy
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let mut new_grid = vec![vec![Cell::default(); cols]; rows];
        for y in 0..rows.min(self.rows) {
            for x in 0..cols.min(self.cols) {
                new_grid[y][x] = self.grid[y][x];
            }
        }
        self.grid = new_grid;
        // alt buffer bhi resize
        let mut new_alt = vec![vec![Cell::default(); cols]; rows];
        for y in 0..rows.min(self.alt.len()) {
            for x in 0..cols.min(self.alt[y].len()) {
                new_alt[y][x] = self.alt[y][x];
            }
        }
        self.alt = new_alt;
        self.rows = rows;
        self.cols = cols;
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
        self.acx = self.acx.min(cols - 1);
        self.acy = self.acy.min(rows - 1);
    }

    fn blank(&self) -> Cell {
        Cell { ch: ' ', fg: self.cur_fg, bg: self.cur_bg }
    }

    fn newline(&mut self) {
        if self.cy + 1 >= self.rows {
            // scroll: top line scrollback me, neeche blank
            let line = std::mem::replace(&mut self.grid[0], vec![Cell::default(); self.cols]);
            self.scrollback.push(line);
            if self.scrollback.len() > MAX_SCROLLBACK {
                self.scrollback.remove(0);
                // global indices ek se khisak gaye → boundaries adjust karo
                for b in &mut self.boundaries {
                    b.start = b.start.saturating_sub(1);
                }
            }
            self.grid.remove(0);
            self.grid.push(vec![Cell::default(); self.cols]);
        } else {
            self.cy += 1;
        }
    }

    fn put(&mut self, ch: char) {
        if self.cx >= self.cols {
            self.cx = 0;
            self.newline();
        }
        let cell = Cell { ch, fg: self.cur_fg, bg: self.cur_bg };
        self.grid[self.cy][self.cx] = cell;
        self.cx += 1;
    }

    // ---- alternate screen (vim/htop) ----

    fn enter_alt(&mut self) {
        self.alt_active = true;
        self.alt = vec![vec![Cell::default(); self.cols]; self.rows];
        self.acx = 0;
        self.acy = 0;
    }

    fn exit_alt(&mut self) {
        self.alt_active = false;
    }

    fn alt_newline(&mut self) {
        if self.acy + 1 >= self.rows {
            self.alt.remove(0);
            self.alt.push(vec![Cell::default(); self.cols]);
        } else {
            self.acy += 1;
        }
    }

    fn alt_put(&mut self, ch: char) {
        if self.acx >= self.cols {
            self.acx = 0;
            self.alt_newline();
        }
        let cell = Cell { ch, fg: self.cur_fg, bg: self.cur_bg };
        self.alt[self.acy][self.acx] = cell;
        self.acx += 1;
    }

    fn sgr(&mut self, ps: &[u16]) {
        if ps.is_empty() {
            self.cur_fg = FG;
            self.cur_bg = BG;
            self.bold = false;
            return;
        }
        for &p in ps {
            match p {
                0 => {
                    self.cur_fg = FG;
                    self.cur_bg = BG;
                    self.bold = false;
                }
                1 => self.bold = true,
                22 => self.bold = false,
                30..=37 => self.cur_fg = ansi_color(p - 30, self.bold),
                39 => self.cur_fg = FG,
                40..=47 => self.cur_bg = ansi_color(p - 40, false),
                49 => self.cur_bg = BG,
                90..=97 => self.cur_fg = ansi_color(p - 90, true),
                100..=107 => self.cur_bg = ansi_color(p - 100, true),
                _ => {}
            }
        }
    }
}

fn collect_params(params: &Params) -> Vec<u16> {
    params.iter().map(|p| p.first().copied().unwrap_or(0)).collect()
}

/// Kisi bhi grid pe erase-line (main ya alt buffer dono ke liye).
fn erase_line_on(grid: &mut [Vec<Cell>], cx: usize, cy: usize, cols: usize, blank: Cell, mode: u16) {
    if cy >= grid.len() {
        return;
    }
    let (start, end) = match mode {
        1 => (0, cx + 1),
        2 => (0, cols),
        _ => (cx, cols),
    };
    for x in start..end.min(cols) {
        grid[cy][x] = blank;
    }
}

/// Kisi bhi grid pe erase-display.
fn erase_display_on(grid: &mut [Vec<Cell>], cx: usize, cy: usize, rows: usize, cols: usize, blank: Cell, mode: u16) {
    match mode {
        2 | 3 => {
            for row in grid.iter_mut() {
                for c in row.iter_mut() {
                    *c = blank;
                }
            }
        }
        1 => {
            for y in 0..=cy.min(rows - 1) {
                let end = if y == cy { cx + 1 } else { cols };
                for x in 0..end.min(cols) {
                    grid[y][x] = blank;
                }
            }
        }
        _ => {
            for y in cy..rows {
                let start = if y == cy { cx } else { 0 };
                for x in start..cols {
                    grid[y][x] = blank;
                }
            }
        }
    }
}

/// move-amount: 0/missing => 1.
fn amount(ps: &[u16]) -> usize {
    ps.first().copied().filter(|v| *v != 0).unwrap_or(1) as usize
}

impl Perform for Screen {
    fn print(&mut self, c: char) {
        if self.alt_active {
            self.alt_put(c);
        } else {
            self.put(c);
        }
    }

    fn execute(&mut self, byte: u8) {
        let cols = self.cols;
        let alt = self.alt_active;
        match byte {
            0x0a | 0x0b | 0x0c => {
                if alt {
                    self.alt_newline()
                } else {
                    self.newline()
                }
            }
            0x0d => {
                if alt {
                    self.acx = 0
                } else {
                    self.cx = 0
                }
            }
            0x08 => {
                if alt {
                    self.acx = self.acx.saturating_sub(1)
                } else {
                    self.cx = self.cx.saturating_sub(1)
                }
            }
            0x09 => {
                let cur = if alt { &mut self.acx } else { &mut self.cx };
                *cur = (((*cur / 8) + 1) * 8).min(cols - 1);
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, inter: &[u8], _ignore: bool, action: char) {
        let ps = collect_params(params);

        // private modes: CSI ? <n> h/l — alt screen (1049/47/1047) toggle
        if inter.first() == Some(&b'?') {
            if action == 'h' || action == 'l' {
                let set = action == 'h';
                for &p in &ps {
                    if matches!(p, 1049 | 47 | 1047) {
                        if set {
                            self.enter_alt();
                        } else {
                            self.exit_alt();
                        }
                    }
                }
            }
            return;
        }

        // SGR pehle (grid borrow se pehle, kyunki ye cur_fg/cur_bg badalta hai)
        if action == 'm' {
            self.sgr(&ps);
            return;
        }

        let (rows, cols) = (self.rows, self.cols);
        let blank = self.blank();
        let (grid, cx, cy) = if self.alt_active {
            (&mut self.alt, &mut self.acx, &mut self.acy)
        } else {
            (&mut self.grid, &mut self.cx, &mut self.cy)
        };

        match action {
            'H' | 'f' => {
                let row = ps.first().copied().filter(|v| *v != 0).unwrap_or(1) as usize;
                let col = ps.get(1).copied().filter(|v| *v != 0).unwrap_or(1) as usize;
                *cy = (row - 1).min(rows - 1);
                *cx = (col - 1).min(cols - 1);
            }
            'A' => *cy = cy.saturating_sub(amount(&ps)),
            'B' => *cy = (*cy + amount(&ps)).min(rows - 1),
            'C' => *cx = (*cx + amount(&ps)).min(cols - 1),
            'D' => *cx = cx.saturating_sub(amount(&ps)),
            'G' => *cx = (ps.first().copied().unwrap_or(1).max(1) as usize - 1).min(cols - 1),
            'd' => *cy = (ps.first().copied().unwrap_or(1).max(1) as usize - 1).min(rows - 1),
            'J' => erase_display_on(grid, *cx, *cy, rows, cols, blank, ps.first().copied().unwrap_or(0)),
            'K' => erase_line_on(grid, *cx, *cy, cols, blank, ps.first().copied().unwrap_or(0)),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // (Hamari shell-integration emit karti hai; user setup nahi karta.)
        let code = params.first().copied().unwrap_or(b"");
        if code != b"133" {
            return;
        }
        let kind = params.get(1).copied().unwrap_or(b"");

        // OSC 133 ; A ; <cwd_b64> ; <branch_b64>  → prompt start = naya block boundary
        if kind == b"A" {
            let g = self.cur_global();
            let cwd = params.get(2).map(|p| b64(p)).filter(|s| !s.is_empty());
            let branch = params.get(3).map(|p| b64(p)).filter(|s| !s.is_empty());
            // consecutive empty prompts (khaali enter) pe naya block mat banao
            if self.boundaries.last().map_or(true, |b| b.start != g) {
                self.next_block_id += 1;
                self.boundaries.push(Boundary {
                    id: self.next_block_id,
                    start: g,
                    output_start: g,
                    command: None,
                    exit: None,
                    when: 0,
                    cwd,
                    branch,
                    dur_ms: None,
                });
            }
            return;
        }

        // OSC 133 ; C  → output start (prompt + command isse pehle hai)
        if kind == b"C" {
            let g = self.cur_global();
            self.cmd_start = Some(std::time::Instant::now());
            if let Some(b) = self.boundaries.last_mut() {
                b.output_start = g;
            }
            return;
        }

        // OSC 133 ; D ; <exit> ; <cmd_b64> ; <cwd_b64>  → command complete
        if kind == b"D" && params.len() >= 5 {
            let exit = std::str::from_utf8(params[2])
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let command = b64(params[3]);
            let cwd = b64(params[4]);
            if command.is_empty() {
                return;
            }
            // command kitne der chali
            let dur = self.cmd_start.take().map(|s| s.elapsed().as_millis() as u64);
            // current block ko finalize karo
            let block_id = if let Some(b) = self.boundaries.last_mut() {
                b.command = Some(command.clone());
                b.exit = Some(exit);
                b.when = now_unix();
                b.dur_ms = dur;
                b.id
            } else {
                0
            };
            self.events.push(CmdEvent {
                exit,
                command,
                cwd,
                block_id,
            });
        }
    }
}

fn b64(p: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(p)
        .ok()
        .and_then(|v| String::from_utf8(v).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vte::Parser;

    fn feed(s: &mut Screen, bytes: &[u8]) {
        let mut p = Parser::new();
        p.advance(s, bytes);
    }

    #[test]
    fn basic_print() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"hi");
        assert_eq!(s.grid[0][0].ch, 'h');
        assert_eq!(s.grid[0][1].ch, 'i');
        assert_eq!((s.cy, s.cx), (0, 2));
    }

    #[test]
    fn crlf_moves_to_next_line() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"a\r\nb");
        assert_eq!(s.grid[0][0].ch, 'a');
        assert_eq!(s.grid[1][0].ch, 'b');
    }

    #[test]
    fn carriage_return_overwrites() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"abc\rX");
        assert_eq!(s.grid[0][0].ch, 'X');
        assert_eq!(s.grid[0][1].ch, 'b');
    }

    #[test]
    fn wrap_at_edge() {
        let mut s = Screen::new(10, 3);
        feed(&mut s, b"abcd");
        assert_eq!(s.grid[0][0].ch, 'a');
        assert_eq!(s.grid[0][2].ch, 'c');
        assert_eq!(s.grid[1][0].ch, 'd');
    }

    #[test]
    fn sgr_sets_color() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"\x1b[31mR");
        assert_eq!(s.grid[0][0].ch, 'R');
        assert_eq!(s.grid[0][0].fg, ansi_color(1, false)); // red
    }

    #[test]
    fn cursor_position_and_clear() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"junk");
        feed(&mut s, b"\x1b[2J\x1b[H"); // clear + home
        assert_eq!((s.cy, s.cx), (0, 0));
        assert_eq!(s.grid[0][0].ch, ' ');
        // absolute cursor move (row 3, col 5 → 0-indexed 2,4)
        feed(&mut s, b"\x1b[3;5H");
        assert_eq!((s.cy, s.cx), (2, 4));
    }

    #[test]
    fn osc133_emits_command_event() {
        let mut s = Screen::new(10, 20);
        // OSC 133 ; D ; exit=1 ; base64("ls") ; base64("/tmp")
        feed(&mut s, b"\x1b]133;D;1;bHM=;L3RtcA==\x07");
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].exit, 1);
        assert_eq!(s.events[0].command, "ls");
        assert_eq!(s.events[0].cwd, "/tmp");
    }

    #[test]
    fn osc133_ignores_partial() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"\x1b]133;A\x07"); // prompt-start marker, no command
        assert_eq!(s.events.len(), 0);
    }

    #[test]
    fn osc133_a_creates_block_boundary() {
        let mut s = Screen::new(10, 40);
        // prompt start (A), output, command complete (D)
        feed(&mut s, b"\x1b]133;A\x07");
        feed(&mut s, b"$ ls\r\nfile1\r\n");
        feed(&mut s, b"\x1b]133;D;0;bHM=;L3RtcA==\x07");
        assert_eq!(s.boundaries.len(), 1);
        assert_eq!(s.boundaries[0].command.as_deref(), Some("ls"));
        assert_eq!(s.boundaries[0].exit, Some(0));
        // event bhi block_id ke saath aaya
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].block_id, s.boundaries[0].id);
    }

    #[test]
    fn osc133_a_dedups_empty_prompts() {
        let mut s = Screen::new(10, 40);
        // do consecutive A bina kisi output ke (khaali enter) → ek hi boundary
        feed(&mut s, b"\x1b]133;A\x07");
        feed(&mut s, b"\x1b]133;A\x07");
        assert_eq!(s.boundaries.len(), 1);
    }

    #[test]
    fn alt_screen_enter_exit() {
        let mut s = Screen::new(10, 20);
        feed(&mut s, b"main");
        // enter alt screen (CSI ?1049h)
        feed(&mut s, b"\x1b[?1049h");
        assert!(s.alt_active);
        feed(&mut s, b"VIM");
        assert_eq!(s.alt[0][0].ch, 'V');
        // main screen abhi bhi safe hai
        assert_eq!(s.grid[0][0].ch, 'm');
        // exit alt (CSI ?1049l)
        feed(&mut s, b"\x1b[?1049l");
        assert!(!s.alt_active);
        // main content waisa ka waisa
        assert_eq!(s.grid[0][0].ch, 'm');
        assert_eq!(s.grid[0][3].ch, 'n');
    }

    #[test]
    fn scroll_pushes_to_scrollback() {
        let mut s = Screen::new(2, 10);
        feed(&mut s, b"l1\r\nl2\r\nl3"); // 3 lines in 2-row screen → 1 scrolled off
        assert_eq!(s.scrollback.len(), 1);
        assert_eq!(s.scrollback[0][0].ch, 'l');
        assert_eq!(s.grid[1][0].ch, 'l');
        assert_eq!(s.grid[1][1].ch, '3');
    }
}
