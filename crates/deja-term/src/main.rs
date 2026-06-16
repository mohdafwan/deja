//! Déjà GUI terminal (egui). Phase 1.1 — PTY-backed working terminal.

mod pty;
mod term;

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_title("Déjà")
            .with_icon(make_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Déjà",
        options,
        Box::new(|cc| Ok(Box::new(DejaApp::new(cc)))),
    )
}

/// Programmatic app icon — dark square + accent "«" (rewind/Déjà feel). Koi asset file nahi.
fn make_icon() -> egui::IconData {
    let s = 64usize;
    let mut rgba = vec![0u8; s * s * 4];
    let bg = [0x1eu8, 0x1e, 0x1e, 0xff];
    let fg = [0xffu8, 0xcc, 0x66, 0xff];
    let sf = s as f32;
    let thick = sf * 0.08;
    // do chevrons "«" — har ek do segments ka
    let centers = [0.40f32, 0.62];
    for y in 0..s {
        for x in 0..s {
            let (px, py) = (x as f32, y as f32);
            let mut on = false;
            for &cxf in &centers {
                let cx = cxf * sf;
                let cy = 0.5 * sf;
                let w = 0.12 * sf;
                let h = 0.18 * sf;
                // "<" : top-right → mid-left → bottom-right
                let d1 = seg_dist(px, py, cx + w, cy - h, cx - w, cy);
                let d2 = seg_dist(px, py, cx - w, cy, cx + w, cy + h);
                if d1 < thick || d2 < thick {
                    on = true;
                }
            }
            let i = (y * s + x) * 4;
            rgba[i..i + 4].copy_from_slice(if on { &fg } else { &bg });
        }
    }
    egui::IconData {
        rgba,
        width: s as u32,
        height: s as u32,
    }
}

/// point se segment ki shortest distance.
fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Fail hui command ka diff jo block ke andar dikhta hai.
struct DiffView {
    when: i64,
    changes: Vec<deja_core::diff::Change>,
}

/// Worker thread se aaya diff result.
type DiffResult = (u64, i64, Vec<deja_core::diff::Change>);

/// Theme = default bg + fg. ANSI-colored cells waise hi rehte; sirf "default"
/// (term::BG/term::FG sentinel) cells theme ke hisaab se map hote hain.
#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    bg: egui::Color32,
    fg: egui::Color32,
}

const THEMES: [Theme; 3] = [
    Theme {
        name: "Dark",
        bg: egui::Color32::from_rgb(0x1e, 0x1e, 0x1e),
        fg: egui::Color32::from_rgb(0xcc, 0xcc, 0xcc),
    },
    Theme {
        name: "Light",
        bg: egui::Color32::from_rgb(0xfa, 0xf8, 0xf2),
        fg: egui::Color32::from_rgb(0x2b, 0x2b, 0x2b),
    },
    Theme {
        name: "Midnight",
        bg: egui::Color32::from_rgb(0x0f, 0x14, 0x1a),
        fg: egui::Color32::from_rgb(0xc8, 0xd3, 0xde),
    },
];

/// Cell ka stored "default" color (term::FG/BG sentinel) ko active theme pe map karo.
fn theme_fg(c: egui::Color32, theme: Theme) -> egui::Color32 {
    if c == term::FG {
        theme.fg
    } else {
        c
    }
}
fn theme_bg(c: egui::Color32, theme: Theme) -> egui::Color32 {
    if c == term::BG {
        theme.bg
    } else {
        c
    }
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let mut style = (*ctx.global_style()).clone();
    style.visuals.panel_fill = theme.bg;
    ctx.set_global_style(style);
}

/// Ek terminal session (ek tab).
struct Terminal {
    pty: pty::Pty,
    rx: Receiver<Vec<u8>>,
    parser: vte::Parser,
    screen: term::Screen,
    diffs: HashMap<u64, DiffView>,
    cmd_tx: Sender<term::CmdEvent>,
    diff_rx: Receiver<DiffResult>,
    id: usize,
}

impl Terminal {
    fn new(ctx: &egui::Context, id: usize) -> Self {
        let (rows, cols) = (24u16, 80u16);
        let rctx = ctx.clone();
        let (pty, rx) =
            pty::Pty::spawn(rows, cols, move || rctx.request_repaint()).expect("shell spawn fail");

        // snapshot/diff/store ek worker thread pe — UI thread hitch na ho
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<term::CmdEvent>();
        let (diff_tx, diff_rx) = std::sync::mpsc::channel::<DiffResult>();
        let wctx = ctx.clone();
        std::thread::spawn(move || snapshot_worker(cmd_rx, diff_tx, wctx));

        Terminal {
            pty,
            rx,
            parser: vte::Parser::new(),
            screen: term::Screen::new(rows as usize, cols as usize),
            diffs: HashMap::new(),
            cmd_tx,
            diff_rx,
            id,
        }
    }

    /// Per-frame state update (UI nahi): shell output process + events + diffs.
    /// Background tabs bhi update hote rehte hain.
    fn pump(&mut self) {
        while let Ok(bytes) = self.rx.try_recv() {
            self.parser.advance(&mut self.screen, &bytes);
        }
        let events: Vec<term::CmdEvent> = std::mem::take(&mut self.screen.events);
        for ev in events {
            let _ = self.cmd_tx.send(ev);
        }
        while let Ok((block_id, when, changes)) = self.diff_rx.try_recv() {
            self.diffs.insert(block_id, DiffView { when, changes });
        }
    }

    fn resize_to(&mut self, rows: usize, cols: usize) {
        if cols != self.screen.cols || rows != self.screen.rows {
            self.screen.resize(rows, cols);
            self.pty.resize(rows as u16, cols as u16);
        }
    }

    /// Tab title — last command ka pehla word, ya "shell".
    fn title(&self) -> String {
        self.screen
            .boundaries
            .iter()
            .rev()
            .find_map(|b| b.command.as_deref())
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("shell")
            .to_string()
    }

    fn handle_input(&mut self, ui: &egui::Ui) {
        let mut out: Vec<u8> = Vec::new();
        ui.input(|i| {
            for ev in &i.events {
                match ev {
                    egui::Event::Text(t) => out.extend_from_slice(t.as_bytes()),
                    egui::Event::Paste(t) => out.extend_from_slice(t.as_bytes()),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        // Ctrl+letter → control byte (Ctrl-C = 0x03, etc.)
                        if modifiers.ctrl && !modifiers.shift {
                            let name = key.name();
                            let b = name.as_bytes();
                            if b.len() == 1 && b[0].is_ascii_alphabetic() {
                                out.push(b[0].to_ascii_lowercase() & 0x1f);
                                continue;
                            }
                        }
                        match key {
                            egui::Key::Enter => out.push(b'\r'),
                            egui::Key::Backspace => out.push(0x7f),
                            // Ctrl+Tab tab-switch ke liye reserved — shell ko mat bhejo
                            egui::Key::Tab if !modifiers.ctrl => out.push(b'\t'),
                            egui::Key::Escape => out.push(0x1b),
                            egui::Key::ArrowUp => out.extend_from_slice(b"\x1b[A"),
                            egui::Key::ArrowDown => out.extend_from_slice(b"\x1b[B"),
                            egui::Key::ArrowRight => out.extend_from_slice(b"\x1b[C"),
                            egui::Key::ArrowLeft => out.extend_from_slice(b"\x1b[D"),
                            egui::Key::Home => out.extend_from_slice(b"\x1b[H"),
                            egui::Key::End => out.extend_from_slice(b"\x1b[F"),
                            egui::Key::Delete => out.extend_from_slice(b"\x1b[3~"),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        });
        if !out.is_empty() {
            self.pty.write(&out);
        }
    }
}

struct DejaApp {
    tabs: Vec<Terminal>,
    active: usize,
    next_id: usize,
    font_size: f32,
    theme_idx: usize,
}

impl DejaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // bundled monospace font (JetBrains Mono) — har platform pe consistent look
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "jbmono".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/JetBrainsMono-Regular.ttf"
            ))),
        );
        if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            mono.insert(0, "jbmono".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);

        // default theme (Dark)
        apply_theme(&cc.egui_ctx, THEMES[0]);

        let first = Terminal::new(&cc.egui_ctx, 1);
        DejaApp {
            tabs: vec![first],
            active: 0,
            next_id: 2,
            font_size: 14.0,
            theme_idx: 0,
        }
    }

    fn theme(&self) -> Theme {
        THEMES[self.theme_idx % THEMES.len()]
    }

    fn add_tab(&mut self, ctx: &egui::Context) {
        let t = Terminal::new(ctx, self.next_id);
        self.next_id += 1;
        self.tabs.push(t);
        self.active = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, idx: usize, ctx: &egui::Context) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            // aakhri tab close → app band
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }

    /// Tab bar render karo + actions handle karo.
    fn tab_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut activate: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut want_new = false;
        ui.horizontal(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let title = format!("{}: {}", t.id, t.title());
                if ui.selectable_label(i == self.active, title).clicked() {
                    activate = Some(i);
                }
                if ui.small_button("×").on_hover_text("close tab").clicked() {
                    close = Some(i);
                }
                ui.separator();
            }
            if ui.button("+").on_hover_text("new tab (Ctrl+Shift+T)").clicked() {
                want_new = true;
            }
            // theme cycle button (right side)
            let mut cycle = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(format!("🎨 {}", self.theme().name))
                    .on_hover_text("theme badlo")
                    .clicked()
                {
                    cycle = true;
                }
            });
            if cycle {
                self.theme_idx = (self.theme_idx + 1) % THEMES.len();
                apply_theme(ctx, self.theme());
            }
        });
        if let Some(i) = activate {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_tab(i, ctx);
        }
        if want_new {
            self.add_tab(ctx);
        }
        ui.separator();
    }
}

impl eframe::App for DejaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // --- tab shortcuts (Ctrl+Shift+T/W new/close, Ctrl+Tab / Ctrl+Shift+Tab switch) ---
        let mut want_new = false;
        let mut want_close = false;
        let mut nav = 0i32;
        ui.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    if modifiers.ctrl && modifiers.shift {
                        match key {
                            egui::Key::T => want_new = true,
                            egui::Key::W => want_close = true,
                            egui::Key::Tab => nav = -1,
                            _ => {}
                        }
                    } else if modifiers.ctrl && *key == egui::Key::Tab {
                        nav = 1;
                    }
                }
            }
        });
        if want_new {
            self.add_tab(&ctx);
        }
        if want_close {
            self.close_tab(self.active, &ctx);
        }
        if nav != 0 && !self.tabs.is_empty() {
            let n = self.tabs.len();
            self.active = ((self.active as i32 + nav).rem_euclid(n as i32)) as usize;
        }

        // saare tabs ka state update karo (background bhi chale)
        for t in &mut self.tabs {
            t.pump();
        }

        // tab bar
        self.tab_bar(ui, &ctx);

        // font cell size
        let font = egui::FontId::monospace(self.font_size);
        let galley = ui
            .painter()
            .layout_no_wrap("M".to_string(), font.clone(), term::FG);
        let char_w = galley.size().x.max(1.0);
        let row_h = galley.size().y.max(1.0);
        let avail = ui.available_size();
        let cols = ((avail.x / char_w).floor() as usize).max(20);
        let rows = ((avail.y / row_h).floor() as usize).max(5);

        // active terminal: resize + input + render
        let active = self.active;
        let theme = self.theme();
        if let Some(t) = self.tabs.get_mut(active) {
            t.resize_to(rows, cols);
            t.handle_input(ui);
            t.render_blocks(ui, &font, theme);
        }
    }
}

impl Terminal {
    /// Output ko OSC-133 boundaries ke hisaab se blocks me render karo.
    fn render_blocks(&self, ui: &mut egui::Ui, font: &egui::FontId, theme: Theme) {
        // alt-screen (vim/htop) → poora grid, blocks nahi
        if self.screen.alt_active {
            render_alt(ui, font, &self.screen, theme);
            return;
        }
        let sb = self.screen.scrollback.len();
        let total = sb + self.screen.grid.len();
        let cursor_global = sb + self.screen.cy;
        let b = &self.screen.boundaries;

        // segments banao: (start, end, boundary_index?)
        let mut segs: Vec<(usize, usize, Option<usize>)> = Vec::new();
        let first = b.first().map(|x| x.start.min(total)).unwrap_or(total);
        if first > 0 {
            segs.push((0, first, None)); // pehle prompt se pehle ka output
        }
        for i in 0..b.len() {
            let start = b[i].start.min(total);
            let end = if i + 1 < b.len() {
                b[i + 1].start.min(total)
            } else {
                total
            };
            segs.push((start, end, Some(i)));
        }

        let get = |g: usize| -> &[term::Cell] {
            if g < sb {
                &self.screen.scrollback[g]
            } else {
                &self.screen.grid[g - sb]
            }
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (start, end, bidx) in segs {
                    if start >= end && bidx.is_none() {
                        continue;
                    }
                    let cursor_in = if cursor_global >= start && cursor_global < end {
                        Some((cursor_global - start, self.screen.cx))
                    } else {
                        None
                    };

                    let mut lines: Vec<&[term::Cell]> = (start..end).map(get).collect();
                    // trailing blank lines trim karo (cursor line chhod ke)
                    while let Some(last) = lines.last() {
                        let idx = lines.len() - 1;
                        let blank = last.iter().all(|c| c.ch == ' ');
                        let is_cursor = cursor_in.map(|(l, _)| l) == Some(idx);
                        if blank && !is_cursor {
                            lines.pop();
                        } else {
                            break;
                        }
                    }

                    let header = bidx.map(|i| &self.screen.boundaries[i]);
                    render_one_block(ui, font, &lines, header, cursor_in, &self.diffs, theme);
                }
            });
    }
}

/// Alternate screen (vim/htop) — poora grid, ek galley, blocks nahi.
fn render_alt(ui: &mut egui::Ui, font: &egui::FontId, screen: &term::Screen, theme: Theme) {
    let lines: Vec<&[term::Cell]> = screen.alt.iter().map(|r| r.as_slice()).collect();
    let cursor = Some((screen.acy, screen.acx));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(block_job(&lines, font, cursor, theme))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
}

const C_GREEN: egui::Color32 = egui::Color32::from_rgb(0x23, 0xd1, 0x8b);
const C_RED: egui::Color32 = egui::Color32::from_rgb(0xf1, 0x4c, 0x4c);
const C_GRAY: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);

/// Ek block ek bordered frame me — header (status + command + time) + content + diff.
fn render_one_block(
    ui: &mut egui::Ui,
    font: &egui::FontId,
    lines: &[&[term::Cell]],
    header: Option<&term::Boundary>,
    cursor: Option<(usize, usize)>,
    diffs: &HashMap<u64, DiffView>,
    theme: Theme,
) {
    egui::Frame::group(ui.style())
        .fill(theme.bg)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // header
            if let Some(bnd) = header {
                ui.horizontal(|ui| {
                    let (badge, color) = match bnd.exit {
                        Some(0) => ("✓".to_string(), C_GREEN),
                        Some(code) => (format!("✗ {code}"), C_RED),
                        None => ("▶".to_string(), C_GRAY),
                    };
                    ui.label(egui::RichText::new(badge).color(color).monospace());
                    if let Some(cmd) = &bnd.command {
                        ui.label(egui::RichText::new(cmd).strong().monospace());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("⧉").on_hover_text("copy block").clicked() {
                            ui.ctx().copy_text(block_text(lines));
                        }
                        if bnd.when > 0 {
                            ui.label(
                                egui::RichText::new(deja_core::diff::humanize_since(bnd.when))
                                    .weak()
                                    .small(),
                            );
                        }
                    });
                });
            }
            // content
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(block_job(lines, font, cursor, theme))
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true), // mouse-drag selection
            );
            // diff (failed block ke andar)
            if let Some(bnd) = header {
                if let Some(diff) = diffs.get(&bnd.id) {
                    render_diff(ui, diff);
                }
            }
        });
}

fn render_diff(ui: &mut egui::Ui, diff: &DiffView) {
    egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(0x33, 0x26, 0x26))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "⏪ Déjà — ye command last {} chali thi. Tab se ye badla:",
                    deja_core::diff::humanize_since(diff.when)
                ))
                .color(egui::Color32::from_rgb(0xff, 0xcc, 0x66)),
            );
            for c in diff.changes.iter().take(5) {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{:<5} {:<14}", c.category, c.key));
                    ui.monospace(format!("{}  →  {}", c.before, c.after));
                    if c.score >= 80 {
                        ui.label(egui::RichText::new("⚠ likely cause").color(C_RED));
                    }
                });
            }
            if diff.changes.len() > 5 {
                ui.label(format!("… +{} aur changes", diff.changes.len() - 5));
            }
        });
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Worker thread: har command pe snapshot capture + store, fail pe diff bheje.
/// (UI thread pe ye nahi chalta — terminal smooth rehta hai.)
fn snapshot_worker(
    rx: Receiver<term::CmdEvent>,
    tx: Sender<DiffResult>,
    ctx: egui::Context,
) {
    while let Ok(ev) = rx.recv() {
        let snap = deja_core::snapshot::capture(&ev.cwd, &ev.command);

        // failure pe last-good run se diff (store se PEHLE)
        if ev.exit != 0 {
            if let Ok(conn) = deja_core::db::open() {
                if let Ok(Some((good_run, good_snap))) =
                    deja_core::db::last_good_snapshot(&conn, &ev.command, &ev.cwd)
                {
                    let changes = deja_core::diff::diff_snapshots(&good_snap, &snap);
                    if !changes.is_empty() {
                        let _ = tx.send((ev.block_id, good_run.started_at, changes));
                        ctx.request_repaint();
                    }
                }
            }
        }

        // run store karo
        if let Ok(conn) = deja_core::db::open() {
            if let Ok(sid) = deja_core::db::insert_snapshot(&conn, &snap) {
                let _ = deja_core::db::insert_run(
                    &conn,
                    &deja_core::db::Run {
                        command: ev.command,
                        cwd: ev.cwd,
                        exit_code: ev.exit,
                        duration_ms: -1,
                        started_at: now_unix(),
                    },
                    Some(sid),
                );
            }
        }
    }
}

/// Ek block ki saari lines ek LayoutJob me (performance: ek galley per block).
/// cursor = (line_index_within_block, col).
fn block_job(
    lines: &[&[term::Cell]],
    font: &egui::FontId,
    cursor: Option<(usize, usize)>,
    theme: Theme,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (i, line) in lines.iter().enumerate() {
        let cur = cursor.and_then(|(l, c)| if l == i { Some(c) } else { None });
        append_line(&mut job, line, font, cur, theme);
        if i + 1 < lines.len() {
            job.append("\n", 0.0, plain(font));
        }
    }
    if lines.is_empty() {
        job.append(" ", 0.0, plain(font));
    }
    job
}

/// Block ki lines ko plain text me (copy ke liye), trailing spaces hata ke.
fn block_text(lines: &[&[term::Cell]]) -> String {
    lines
        .iter()
        .map(|l| {
            let s: String = l.iter().map(|c| c.ch).collect();
            s.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn plain(font: &egui::FontId) -> TextFormat {
    TextFormat {
        font_id: font.clone(),
        color: term::FG,
        ..Default::default()
    }
}

/// Ek line ko colored runs me append karo (same-format cells ek run me).
/// "default" colors (term::FG/BG sentinel) ko active theme pe map karta hai.
fn append_line(
    job: &mut LayoutJob,
    line: &[term::Cell],
    font: &egui::FontId,
    cursor: Option<usize>,
    theme: Theme,
) {
    let mut i = 0;
    while i < line.len() {
        let is_cur = cursor == Some(i);
        // default colors → theme; fir cursor pe invert
        let rfg = theme_fg(line[i].fg, theme);
        let rbg = theme_bg(line[i].bg, theme);
        let (fg, bg) = if is_cur { (rbg, rfg) } else { (rfg, rbg) };
        let mut text = String::new();
        text.push(line[i].ch);
        let mut j = i + 1;
        if !is_cur {
            while j < line.len()
                && cursor != Some(j)
                && line[j].fg == line[i].fg
                && line[j].bg == line[i].bg
            {
                text.push(line[j].ch);
                j += 1;
            }
        }
        job.append(
            &text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: fg,
                background: bg,
                ..Default::default()
            },
        );
        i = j;
    }
    if line.is_empty() {
        job.append(" ", 0.0, plain(font));
    }
}
