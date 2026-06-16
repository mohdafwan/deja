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
            .with_icon(make_icon())
            .with_decorations(false) // custom title bar (tabs + window buttons)
            .with_resizable(true),
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

/// Auto-update state (tab bar me dikhta hai).
#[derive(Clone)]
enum UpdateState {
    Idle,
    Available(String), // newer version tag
    Updating,
    Done,
    Failed,
}

/// GitHub se latest release tag (curl + serde_json — koi heavy TLS dep nahi).
fn fetch_latest_tag() -> Option<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-sL",
            "-H",
            "User-Agent: deja-term",
            "https://api.github.com/repos/mohdafwan/deja/releases/latest",
        ])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("tag_name")?.as_str().map(|s| s.to_string())
}

/// "v0.2.2" vs "0.2.1" → newer hai?
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(s: &str) -> (u64, u64, u64) {
        let s = s.trim().trim_start_matches('v');
        let mut it = s.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0));
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    }
    parse(latest) > parse(current)
}

/// cargo-dist ka prebuilt `deja-term-update` chalao (installer ne ship kiya).
fn run_updater() -> bool {
    let mut candidates = vec![std::path::PathBuf::from("deja-term-update")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("deja-term-update"));
        }
    }
    for c in candidates {
        if let Ok(st) = std::process::Command::new(&c).status() {
            return st.success();
        }
    }
    false
}

/// Theme = default bg + fg + accent colors. ANSI-colored cells waise hi rehte;
/// sirf "default" (term::BG/term::FG sentinel) cells theme pe map hote hain.
#[derive(Clone, Copy)]
struct Theme {
    name: &'static str,
    bg: egui::Color32,     // window/card background
    fg: egui::Color32,     // bright output text
    muted: egui::Color32,  // metadata / borders
    accent: egui::Color32, // cursor + git branch
    path: egui::Color32,   // path text
}

const fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

const THEMES: [Theme; 4] = [
    Theme {
        name: "Emerald",
        bg: rgb(0x0c, 0x23, 0x1e),
        fg: rgb(0xe6, 0xf0, 0xec),
        muted: rgb(0x6f, 0x8a, 0x83),
        accent: rgb(0x34, 0xd3, 0x99),
        path: rgb(0x8b, 0xd5, 0xc4),
    },
    Theme {
        name: "Dark",
        bg: rgb(0x1a, 0x1a, 0x1e),
        fg: rgb(0xe0, 0xe0, 0xe0),
        muted: rgb(0x80, 0x80, 0x88),
        accent: rgb(0x6c, 0xb6, 0xff),
        path: rgb(0xc8, 0xa8, 0xff),
    },
    Theme {
        name: "Light",
        bg: rgb(0xfa, 0xf8, 0xf2),
        fg: rgb(0x24, 0x28, 0x2c),
        muted: rgb(0x8a, 0x90, 0x96),
        accent: rgb(0x0e, 0x9f, 0x6e),
        path: rgb(0x2b, 0x6c, 0xb0),
    },
    Theme {
        name: "Midnight",
        bg: rgb(0x0d, 0x12, 0x1c),
        fg: rgb(0xc8, 0xd3, 0xde),
        muted: rgb(0x5e, 0x6b, 0x7e),
        accent: rgb(0x6c, 0xb6, 0xff),
        path: rgb(0x9d, 0xb8, 0xff),
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
    /// Bottom input field ka buffer (Warp-style line editor).
    input: String,
    /// Submit ki hui command jo abhi chal rahi (running card me dikhti, D pe clear).
    running: Option<String>,
}

impl Terminal {
    fn new(ctx: &egui::Context) -> Self {
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
            input: String::new(),
            running: None,
        }
    }

    /// Per-frame state update (UI nahi): shell output process + events + diffs.
    /// Background tabs bhi update hote rehte hain.
    fn pump(&mut self) {
        while let Ok(bytes) = self.rx.try_recv() {
            self.parser.advance(&mut self.screen, &bytes);
        }
        let events: Vec<term::CmdEvent> = std::mem::take(&mut self.screen.events);
        if !events.is_empty() {
            self.running = None; // command complete → running card hata do
        }
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

    /// Alt-screen (vim/htop) me raw keys seedha PTY ko bhejo.
    fn forward_raw(&mut self, ui: &egui::Ui) {
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
    font_size: f32,
    theme_idx: usize,
    update: UpdateState,
    update_tx: Sender<UpdateState>,
    update_rx: Receiver<UpdateState>,
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

        // background update check (curl GitHub API; receipt-installed builds ke liye)
        let (update_tx, update_rx) = std::sync::mpsc::channel::<UpdateState>();
        {
            let tx = update_tx.clone();
            let uctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Some(latest) = fetch_latest_tag() {
                    if is_newer(&latest, env!("CARGO_PKG_VERSION")) {
                        let _ = tx.send(UpdateState::Available(latest));
                        uctx.request_repaint();
                    }
                }
            });
        }

        let first = Terminal::new(&cc.egui_ctx);
        DejaApp {
            tabs: vec![first],
            active: 0,
            font_size: 14.0,
            theme_idx: 0,
            update: UpdateState::Idle,
            update_tx,
            update_rx,
        }
    }

    fn theme(&self) -> Theme {
        THEMES[self.theme_idx % THEMES.len()]
    }

    fn add_tab(&mut self, ctx: &egui::Context) {
        let t = Terminal::new(ctx);
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

    /// Modern tab bar — rounded chips, active highlighted, theme button right.
    fn tab_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, theme: Theme) {
        let mut activate: Option<usize> = None;
        let mut close: Option<usize> = None;
        let mut want_new = false;
        let mut cycle = false;
        let mut win_min = false;
        let mut win_max = false;
        let mut win_close = false;
        let mut start_update = false;

        // window drag + double-click maximize (background — buttons iske upar render hote)
        let title_resp = ui.interact(
            ui.max_rect(),
            egui::Id::new("deja_titlebar_drag"),
            egui::Sense::click_and_drag(),
        );
        if title_resp.double_clicked() {
            win_max = true;
        } else if title_resp.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0; // tabs flush (no gap)
            ui.spacing_mut().item_spacing.y = 0.0;
            let h = 34.0; // fixed bar height — highlight pura cell fill kare
            let plus_w = 30.0;
            let n = self.tabs.len().max(1);
            // right controls (theme + 3 window buttons) ke liye width reserve →
            // tabs unpe overflow na karein. Bache hue space me tabs exact-fit
            // (zyada tabs → shrink, Warp jaisa), max 200.
            let avail_for_tabs = (ui.available_width() - 230.0 - plus_w).max(60.0);
            let tab_w = (avail_for_tabs / n as f32).clamp(2.0, 200.0);
            let tab_font = egui::FontId::new(13.0, egui::FontFamily::Proportional);

            let x_w = 26.0;
            for (i, t) in self.tabs.iter().enumerate() {
                let selected = i == self.active;
                // layout space reserve (geometry ke liye), click body/× alag interacts se
                let (rect, _) = ui.allocate_exact_size(egui::vec2(tab_w, h), egui::Sense::hover());
                // geometric hover — widget hover-steal se bachne ke liye
                let hovered = ui
                    .input(|i| i.pointer.hover_pos())
                    .map_or(false, |p| rect.contains(p));

                // body (activate) — hover pe × area chhod do (overlap na ho)
                let body_rect = if hovered {
                    egui::Rect::from_min_max(
                        rect.left_top(),
                        egui::pos2(rect.right() - x_w, rect.bottom()),
                    )
                } else {
                    rect
                };
                let body = ui.interact(body_rect, egui::Id::new(("deja_tab", i)), egui::Sense::click());

                // flat bg (no border, no corner radius) — pura cell
                let bg = if selected {
                    theme.fg.gamma_multiply(0.13)
                } else if hovered {
                    theme.fg.gamma_multiply(0.05)
                } else {
                    egui::Color32::TRANSPARENT
                };
                // painter_at(rect) → bg + text cell me CLIP (narrow tab pe text overflow na ho)
                let tp = ui.painter_at(rect);
                tp.rect_filled(rect, 0.0, bg);
                let col = if selected { theme.fg } else { theme.muted };
                tp.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    t.title(),
                    tab_font.clone(),
                    col,
                );

                if body.clicked() {
                    activate = Some(i);
                }
                if body.middle_clicked() {
                    close = Some(i);
                }

                // close × sirf hover pe (right side) — non-overlapping interact
                if hovered {
                    let xr = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - x_w, rect.top()),
                        egui::vec2(x_w, h),
                    );
                    let xresp =
                        ui.interact(xr, egui::Id::new(("deja_tabx", i)), egui::Sense::click());
                    if xresp.hovered() {
                        ui.painter().rect_filled(
                            xr.shrink2(egui::vec2(4.0, 8.0)),
                            egui::CornerRadius::same(4),
                            egui::Color32::from_white_alpha(30),
                        );
                    }
                    let xcol = if xresp.hovered() {
                        egui::Color32::WHITE
                    } else {
                        theme.muted
                    };
                    let cc = xr.center();
                    let r = 4.5;
                    let st = egui::Stroke::new(1.5, xcol);
                    let p = ui.painter();
                    p.line_segment([egui::pos2(cc.x - r, cc.y - r), egui::pos2(cc.x + r, cc.y + r)], st);
                    p.line_segment([egui::pos2(cc.x - r, cc.y + r), egui::pos2(cc.x + r, cc.y - r)], st);
                    if xresp.clicked() {
                        close = Some(i);
                    }
                }
            }
            // new tab — bada "+"
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("+").color(theme.fg).size(22.0))
                        .min_size(egui::vec2(plus_w, h))
                        .frame(false),
                )
                .on_hover_text("new tab (Ctrl+Shift+T)")
                .clicked()
            {
                want_new = true;
            }
            // right side: window controls + theme (right_to_left → rightmost pehle)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if window_button(ui, theme, "close") {
                    win_close = true;
                }
                if window_button(ui, theme, "max") {
                    win_max = true;
                }
                if window_button(ui, theme, "min") {
                    win_min = true;
                }
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(format!("🎨 {}", self.theme().name))
                                .color(theme.muted)
                                .size(12.0),
                        )
                        .frame(false),
                    )
                    .on_hover_text("theme badlo")
                    .clicked()
                {
                    cycle = true;
                }
                // update status (theme ke left me)
                ui.add_space(6.0);
                match self.update.clone() {
                    UpdateState::Available(v) => {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("Update {v}"))
                                        .color(theme.accent)
                                        .size(12.0),
                                )
                                .frame(false),
                            )
                            .on_hover_text("naya version — click karke install karo")
                            .clicked()
                        {
                            start_update = true;
                        }
                    }
                    UpdateState::Updating => {
                        ui.label(egui::RichText::new("updating...").color(theme.muted).size(12.0));
                    }
                    UpdateState::Done => {
                        ui.label(
                            egui::RichText::new("restart to apply")
                                .color(theme.accent)
                                .size(12.0),
                        );
                    }
                    UpdateState::Failed => {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("update failed - retry")
                                        .color(C_RED)
                                        .size(12.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            start_update = true;
                        }
                    }
                    UpdateState::Idle => {}
                }
            });
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
        if cycle {
            self.theme_idx = (self.theme_idx + 1) % THEMES.len();
            apply_theme(ctx, self.theme());
        }
        // window controls
        if win_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if win_min {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if win_max {
            let m = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
        }
        // update install → background me deja-term-update chalao
        if start_update {
            self.update = UpdateState::Updating;
            let tx = self.update_tx.clone();
            let uctx = ctx.clone();
            std::thread::spawn(move || {
                let ok = run_updater();
                let _ = tx.send(if ok {
                    UpdateState::Done
                } else {
                    UpdateState::Failed
                });
                uctx.request_repaint();
            });
        }
    }
}

impl eframe::App for DejaApp {
    #[allow(deprecated)] // egui::TopBottomPanel alias — kaam karta hai
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
        // update-check thread se messages
        while let Ok(st) = self.update_rx.try_recv() {
            self.update = st;
        }

        let theme = self.theme();
        let font = egui::FontId::monospace(self.font_size);
        let galley = ui
            .painter()
            .layout_no_wrap("M".to_string(), font.clone(), theme.fg);
        let char_w = galley.size().x.max(1.0);
        let row_h = galley.size().y.max(1.0);
        let active = self.active;
        let alt = self.tabs.get(active).map_or(false, |t| t.screen.alt_active);

        // Ctrl+C / Ctrl+D (non-alt) → seedha PTY (interrupt / EOF)
        if !alt {
            let (cc, cd) = ui.input(|i| {
                (
                    i.modifiers.ctrl && i.key_pressed(egui::Key::C),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::D),
                )
            });
            if let Some(t) = self.tabs.get_mut(active) {
                if cc {
                    t.input.clear();
                    t.pty.write(&[0x03]);
                }
                if cd && t.input.is_empty() {
                    t.pty.write(&[0x04]);
                }
            }
        }

        // TOP — tab bar
        egui::TopBottomPanel::top("deja_tabs")
            .frame(egui::Frame::default().fill(theme.bg)) // no margin → tabs edge-to-edge
            .show_inside(ui, |ui| self.tab_bar(ui, &ctx, theme));

        // BOTTOM — fixed input field (Warp style). Alt-screen apps me chhupa do.
        if !alt {
            egui::TopBottomPanel::bottom("deja_input")
                .frame(
                    egui::Frame::default()
                        .fill(theme.bg)
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(14)))
                        .inner_margin(egui::Margin::symmetric(16, 10)),
                )
                .show_inside(ui, |ui| {
                    if let Some(t) = self.tabs.get_mut(active) {
                        t.render_input(ui, &font, theme);
                    }
                });
        }

        // CENTER — command history (scrolls). Alt-screen apps yahi poora grid use karte.
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme.bg))
            .show_inside(ui, |ui| {
                let avail = ui.available_size();
                let cols = ((avail.x / char_w).floor() as usize).max(20);
                let rows = ((avail.y / row_h).floor() as usize).max(5);
                if let Some(t) = self.tabs.get_mut(active) {
                    t.resize_to(rows, cols);
                    if t.screen.alt_active {
                        t.forward_raw(ui); // vim/htop ke raw keys
                    }
                    t.render_history(ui, &font, theme);
                }
            });
    }
}

impl Terminal {
    fn line_at<'a>(&'a self, g: usize) -> &'a [term::Cell] {
        let sb = self.screen.scrollback.len();
        if g < sb {
            &self.screen.scrollback[g]
        } else {
            &self.screen.grid[g - sb]
        }
    }

    /// Sab segments (start, end, boundary_index?). Aakhri segment = active prompt.
    fn segments(&self) -> Vec<(usize, usize, Option<usize>)> {
        let total = self.screen.scrollback.len() + self.screen.grid.len();
        let b = &self.screen.boundaries;
        let mut segs = Vec::new();
        let first = b.first().map(|x| x.start.min(total)).unwrap_or(total);
        if first > 0 {
            segs.push((0, first, None));
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
        segs
    }

    /// Command history (central, scrolls). Top-down, clean. stick_to_bottom se
    /// newest hamesha visible rehta jab content overflow ho.
    fn render_history(&self, ui: &mut egui::Ui, font: &egui::FontId, theme: Theme) {
        // alt-screen (vim/htop) → poora grid central me
        if self.screen.alt_active {
            render_alt(ui, font, &self.screen, theme);
            return;
        }
        let segs = self.segments();
        let last = segs.len().saturating_sub(1);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // bada top spacer + stick_to_bottom = content bottom pe chipakti
                // (Warp jaisa: kam commands neeche, upar khaali). No flicker.
                ui.add_space(ui.available_height());
                for (idx, seg) in segs.iter().enumerate() {
                    let (start, end, bidx) = *seg;
                    let boundary = bidx.map(|i| &self.screen.boundaries[i]);

                    if idx == last {
                        // active: command chal rahi → running card, warna idle
                        // prompt bottom input bar me hai → skip
                        if let (Some(cmd), Some(b)) = (&self.running, boundary) {
                            let ostart = b.output_start.clamp(start, end);
                            let mut out: Vec<&[term::Cell]> =
                                (ostart..end).map(|g| self.line_at(g)).collect();
                            trim_trailing_blank(&mut out, None);
                            render_card(ui, font, b, Some(cmd), &out, theme, &self.diffs);
                            ui.add_space(8.0);
                        }
                        continue;
                    }

                    if start >= end && bidx.is_none() {
                        continue;
                    }
                    if boundary.map_or(false, |b| b.exit.is_some()) {
                        let b = boundary.unwrap();
                        let ostart = b.output_start.clamp(start, end);
                        let mut out: Vec<&[term::Cell]> =
                            (ostart..end).map(|g| self.line_at(g)).collect();
                        trim_trailing_blank(&mut out, None);
                        render_card(ui, font, b, b.command.as_deref(), &out, theme, &self.diffs);
                    } else {
                        let mut raw: Vec<&[term::Cell]> =
                            (start..end).map(|g| self.line_at(g)).collect();
                        trim_trailing_blank(&mut raw, None);
                        render_raw(ui, font, boundary, &raw, None, theme);
                    }
                    ui.add_space(8.0);
                }
            });
    }

    /// Active boundary ka cwd (input chip ke liye).
    fn active_cwd(&self) -> Option<String> {
        self.screen
            .boundaries
            .last()
            .and_then(|b| b.cwd.as_deref())
            .map(short_path)
    }

    /// Fixed bottom input field (Warp-style line editor). Enter pe command submit.
    fn render_input(&mut self, ui: &mut egui::Ui, font: &egui::FontId, theme: Theme) {
        if self.screen.alt_active {
            return;
        }
        let big = egui::FontId::monospace(font.size + 2.0);
        ui.horizontal(|ui| {
            // path chip (folder)
            let chip = self.active_cwd().unwrap_or_else(|| "~".to_string());
            egui::Frame::default()
                .fill(theme.accent.gamma_multiply(0.14))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(7, 3))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    // folder icon (manually drawn)
                    let (ir, _) =
                        ui.allocate_exact_size(egui::vec2(13.0, 14.0), egui::Sense::hover());
                    folder_icon(&ui.painter_at(ir), ir.center(), theme.path);
                    ui.label(egui::RichText::new(chip).color(theme.path).size(12.0));
                });

            // input field
            let te = egui::TextEdit::singleline(&mut self.input)
                .frame(egui::Frame::default())
                .font(big.clone())
                .text_color(theme.fg)
                .hint_text(egui::RichText::new("type a command…").color(theme.muted))
                .desired_width(f32::INFINITY);
            let resp = ui.add(te);

            // hamesha focused rakho (terminal feel)
            if !resp.has_focus() {
                resp.request_focus();
            }
            // Enter → submit (input field hamesha focused hai)
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let line = std::mem::take(&mut self.input);
                self.pty.write(line.as_bytes());
                self.pty.write(b"\n");
                if !line.is_empty() {
                    self.running = Some(line);
                }
                resp.request_focus();
            }
        });
    }
}

/// Trailing all-blank lines hatao (cursor line chhod ke).
fn trim_trailing_blank(lines: &mut Vec<&[term::Cell]>, keep: Option<usize>) {
    while let Some(last) = lines.last() {
        let idx = lines.len() - 1;
        let blank = last.iter().all(|c| c.ch == ' ');
        if blank && keep != Some(idx) {
            lines.pop();
        } else {
            break;
        }
    }
}

const C_BLUE: egui::Color32 = rgb(0x4a, 0xa3, 0xff);

/// $HOME ko ~ se chhota karo.
fn short_path(cwd: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if cwd == home {
            return "~".to_string();
        }
        if let Some(rest) = cwd.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    cwd.to_string()
}

/// Window control button — icon manually draw (font glyphs pe depend nahi).
/// kind: "min" | "max" | "close". Returns clicked.
fn window_button(ui: &mut egui::Ui, theme: Theme, kind: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 24.0), egui::Sense::click());
    let hovered = resp.hovered();
    if hovered {
        let bg = if kind == "close" {
            C_RED
        } else {
            egui::Color32::from_white_alpha(30)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(5), bg);
    }
    let col = if hovered {
        egui::Color32::WHITE
    } else {
        theme.fg.gamma_multiply(0.75)
    };
    let stroke = egui::Stroke::new(1.4, col);
    let c = rect.center();
    let r = 5.0;
    let p = ui.painter();
    match kind {
        "min" => {
            p.line_segment([egui::pos2(c.x - r, c.y), egui::pos2(c.x + r, c.y)], stroke);
        }
        "max" => {
            let sq = egui::Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r));
            p.line_segment([sq.left_top(), sq.right_top()], stroke);
            p.line_segment([sq.right_top(), sq.right_bottom()], stroke);
            p.line_segment([sq.right_bottom(), sq.left_bottom()], stroke);
            p.line_segment([sq.left_bottom(), sq.left_top()], stroke);
        }
        "close" => {
            p.line_segment([egui::pos2(c.x - r, c.y - r), egui::pos2(c.x + r, c.y + r)], stroke);
            p.line_segment([egui::pos2(c.x - r, c.y + r), egui::pos2(c.x + r, c.y - r)], stroke);
        }
        _ => {}
    }
    resp.clicked()
}

/// Rect ka outline 4 lines se (rect_stroke API se bachne ke liye).
fn stroke_rect(p: &egui::Painter, r: egui::Rect, s: egui::Stroke) {
    p.line_segment([r.left_top(), r.right_top()], s);
    p.line_segment([r.right_top(), r.right_bottom()], s);
    p.line_segment([r.right_bottom(), r.left_bottom()], s);
    p.line_segment([r.left_bottom(), r.left_top()], s);
}

/// Copy icon button — do overlapping rects (clipboard). Returns clicked.
fn copy_button(ui: &mut egui::Ui, theme: Theme) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 18.0), egui::Sense::click());
    let resp = resp.on_hover_text("copy output");
    let col = if resp.hovered() { theme.fg } else { theme.muted };
    let st = egui::Stroke::new(1.2, col);
    let c = rect.center();
    let p = ui.painter();
    let back = egui::Rect::from_min_size(egui::pos2(c.x - 1.0, c.y - 5.0), egui::vec2(7.0, 7.0));
    let front = egui::Rect::from_min_size(egui::pos2(c.x - 5.0, c.y - 1.0), egui::vec2(7.0, 7.0));
    stroke_rect(&p, back, st);
    p.rect_filled(front, egui::CornerRadius::same(1), theme.bg); // back ko mask
    stroke_rect(&p, front, st);
    resp.clicked()
}

/// Folder icon (filled silhouette) — input chip ke liye.
fn folder_icon(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    let w = 11.0;
    let body = egui::Rect::from_min_size(
        egui::pos2(c.x - w / 2.0, c.y - 3.0),
        egui::vec2(w, 7.0),
    );
    p.rect_filled(body, egui::CornerRadius::same(1), col);
    let tab = egui::Rect::from_min_size(
        egui::pos2(c.x - w / 2.0, c.y - 5.0),
        egui::vec2(w * 0.5, 3.0),
    );
    p.rect_filled(tab, egui::CornerRadius::same(1), col);
}

/// Left accent bar (status color) frame ke left edge pe paint karo.
fn paint_accent(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 1.0, rect.top() + 6.0),
        egui::pos2(rect.left() + 4.0, rect.bottom() - 6.0),
    );
    ui.painter()
        .rect_filled(bar, egui::CornerRadius::same(2), color);
}

/// Command duration → "(0.18s)".
fn fmt_dur(ms: u64) -> String {
    format!("({:.2}s)", ms as f64 / 1000.0)
}

/// Header: `~/path (0.18s) git:(branch)` — path + duration + branch color-coded.
fn header_line(ui: &mut egui::Ui, b: &term::Boundary, theme: Theme) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if let Some(cwd) = &b.cwd {
            ui.label(
                egui::RichText::new(short_path(cwd))
                    .color(theme.path)
                    .monospace()
                    .size(12.0),
            );
        }
        if let Some(br) = &b.branch {
            ui.label(egui::RichText::new("  git:(").color(theme.muted).monospace().size(12.0));
            ui.label(egui::RichText::new(br).color(theme.accent).monospace().size(12.0));
            ui.label(egui::RichText::new(")").color(theme.muted).monospace().size(12.0));
        }
    });
}

fn deja_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style())
        .fill(egui::Color32::TRANSPARENT) // card = panel bg; sirf border + accent + spacing
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(12)))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin {
            left: 16,
            right: 14,
            top: 10,
            bottom: 12,
        })
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

const C_RED: egui::Color32 = egui::Color32::from_rgb(0xf1, 0x4c, 0x4c);

/// Finished command card — Warp style: header (path+branch) + bada command + output + diff.
fn render_card(
    ui: &mut egui::Ui,
    font: &egui::FontId,
    b: &term::Boundary,
    command: Option<&str>,
    output: &[&[term::Cell]],
    theme: Theme,
    diffs: &HashMap<u64, DiffView>,
) {
    let accent = match b.exit {
        Some(0) => theme.accent,
        Some(_) => C_RED,
        None => C_BLUE, // running
    };
    let big = egui::FontId::monospace(font.size + 3.0);
    let inner = deja_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // header: path + git:(branch) ..... time + copy
        ui.horizontal(|ui| {
            header_line(ui, b, theme);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if copy_button(ui, theme) {
                    ui.ctx().copy_text(block_text(output));
                }
                if let Some(ms) = b.dur_ms {
                    ui.label(
                        egui::RichText::new(fmt_dur(ms))
                            .color(theme.muted)
                            .monospace()
                            .size(11.0),
                    );
                }
            });
        });
        // command — bada, bright
        if let Some(cmd) = command {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(cmd).font(big.clone()).color(theme.fg));
        }
        // output
        if !output.is_empty() {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(
                egui::Label::new(block_job(output, font, None, theme))
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selectable(true),
            );
        }
        // Déjà diff (failed block ke andar)
        if let Some(diff) = diffs.get(&b.id) {
            ui.add_space(6.0);
            render_diff(ui, diff, theme);
        }
    });
    paint_accent(ui, inner.response.rect, accent);
}

/// Active prompt / preamble — raw shell prompt + typing + glowing cursor.
fn render_raw(
    ui: &mut egui::Ui,
    font: &egui::FontId,
    boundary: Option<&term::Boundary>,
    lines: &[&[term::Cell]],
    cursor: Option<(usize, usize)>,
    theme: Theme,
) {
    let accent = if boundary.is_some() { C_BLUE } else { theme.muted };
    let inner = deja_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(
            egui::Label::new(block_job(lines, font, cursor, theme))
                .wrap_mode(egui::TextWrapMode::Extend)
                .selectable(true),
        );
    });
    paint_accent(ui, inner.response.rect, accent);
}

fn render_diff(ui: &mut egui::Ui, diff: &DiffView, theme: Theme) {
    egui::Frame::group(ui.style())
        .fill(C_RED.gamma_multiply(0.12))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "⏪ Déjà — ye command last {} chali thi. Tab se ye badla:",
                    deja_core::diff::humanize_since(diff.when)
                ))
                .color(theme.accent)
                .strong(),
            );
            for c in diff.changes.iter().take(5) {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:<5} {:<14}", c.category, c.key))
                            .color(theme.muted)
                            .monospace(),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}  →  {}", c.before, c.after))
                            .color(theme.fg)
                            .monospace(),
                    );
                    if c.score >= 80 {
                        ui.label(egui::RichText::new("⚠ likely cause").color(C_RED));
                    }
                });
            }
            if diff.changes.len() > 5 {
                ui.label(
                    egui::RichText::new(format!("… +{} aur changes", diff.changes.len() - 5))
                        .color(theme.muted),
                );
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
        // default colors → theme; cursor = teal glowing block
        let rfg = theme_fg(line[i].fg, theme);
        let rbg = theme_bg(line[i].bg, theme);
        let (fg, bg) = if is_cur {
            (theme.bg, theme.accent)
        } else {
            (rfg, rbg)
        };
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
