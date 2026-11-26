//! The main window. Closing or minimizing it sends it back to the tray.

use crate::config::{self, Config};
use crate::devices::{self, Attached, Link};
use crate::state::{Shared, Status};
use crate::{autostart, tray::TrayHandle};
use eframe::egui::{
    self, Align, Color32, Context, Frame, Layout, Margin, Pos2, Rect, Response, RichText, Rounding, Sense, Shape,
    Stroke, Ui, Vec2, ViewportCommand,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESCAN: Duration = Duration::from_secs(3);
/// Screens shown at once in the layout row.
const VISIBLE: usize = 3;
/// Space around the window's content.
const MARGIN: f32 = 16.0;

pub struct App {
    cfg: Config,
    saved: Config,
    shared: Arc<Shared>,
    tray: TrayHandle,
    attached: Attached,
    last_scan: Instant,
    error: Option<String>,
    /// First screen shown in the layout row; `None` centers on this PC.
    row_start: Option<usize>,
    /// Frames spent fitting the height while the window opens invisibly.
    fit_frames: u32,
}

/// One screen in the layout row.
enum Slot {
    Me(String),
    Peer(String, &'static str),
    Empty(&'static str),
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, shared: Arc<Shared>, tray: TrayHandle) -> Self {
        // Same spacing in both themes, and brighter text than egui's default gray.
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            cc.egui_ctx.style_mut_of(theme, |style| {
                style.spacing.item_spacing = egui::vec2(8.0, 8.0);
                style.spacing.button_padding = egui::vec2(10.0, 4.0);
                let text = if theme == egui::Theme::Dark { Color32::from_gray(232) } else { Color32::from_gray(20) };
                let w = &mut style.visuals.widgets;
                w.noninteractive.fg_stroke.color = text;
                w.inactive.fg_stroke.color = text;
            });
        }
        Self {
            saved: cfg.clone(),
            cfg,
            shared,
            tray,
            attached: devices::scan(),
            last_scan: Instant::now(),
            error: None,
            row_start: None,
            fit_frames: 0,
        }
    }

    fn hide_instead_of_close(&self, ctx: &Context) {
        let (close, minimized) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        if close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        } else if minimized {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        let status = self.shared.status();
        ui.horizontal(|ui| {
            ui.label(RichText::new("HomeKVM").size(18.0).strong());
            pill(ui, status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = if self.shared.paused() { "Resume" } else { "Pause" };
                if ui.button(text).clicked() {
                    self.shared.set_paused(!self.shared.paused());
                    self.tray.refresh();
                }
            });
        });
    }

    fn layout_section(&mut self, ui: &mut Ui) {
        section(ui, "Screen layout", |ui| {
            ui.label(
                RichText::new("Place paired PCs left or right. The mouse crosses at the touching edge.").color(muted(ui)),
            );
            ui.add_space(4.0);

            // The whole row: PCs on the left (outermost first), this PC, PCs on the right.
            let demo;
            let layout = if demo_layout() {
                // Dev-only preview of a long row (HOMEKVM_DEMO=1); never saved.
                demo = config::Layout {
                    left: vec!["Work laptop".into(), "Old MacBook".into()],
                    right: vec!["Mini PC".into(), "Media PC".into()],
                };
                &demo
            } else {
                &self.cfg.layout
            };
            let mut row: Vec<Slot> = Vec::new();
            if layout.left.is_empty() {
                row.push(Slot::Empty("left"));
            }
            row.extend(layout.left.iter().rev().map(|n| Slot::Peer(n.clone(), "left")));
            let me = row.len();
            row.push(Slot::Me(self.cfg.device_name.clone()));
            row.extend(layout.right.iter().map(|n| Slot::Peer(n.clone(), "right")));
            if layout.right.is_empty() {
                row.push(Slot::Empty("right"));
            }

            // Show three screens at a time; ‹ and › page through longer rows.
            let max_start = row.len().saturating_sub(VISIBLE);
            let start = self.row_start.unwrap_or(me.saturating_sub(1)).min(max_start);
            let arrows = row.len() > VISIBLE;

            let (gap, arrow_w) = (8.0, 18.0);
            let w = ((ui.available_width() - 2.0 * arrow_w - 4.0 * gap) / VISIBLE as f32).min(124.0);
            let size = Vec2::new(w, 84.0);
            let total = VISIBLE as f32 * w + 2.0 * arrow_w + 4.0 * gap;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                ui.spacing_mut().item_spacing.x = gap;
                if arrow(ui, arrow_w, "‹", arrows && start > 0) {
                    self.row_start = Some(start - 1);
                }
                for slot in row.iter().skip(start).take(VISIBLE) {
                    screen_slot(ui, size, slot);
                }
                if arrow(ui, arrow_w, "›", arrows && start < max_start) {
                    self.row_start = Some(start + 1);
                }
            });
        });
    }

    fn this_pc_section(&mut self, ui: &mut Ui) {
        let suggested = self.attached.suggests_sharing();
        section(ui, "This PC", |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.cfg.device_name).desired_width(f32::INFINITY));
            });
            ui.label(RichText::new(describe(&self.attached)).color(muted(ui)));
            ui.add_space(2.0);

            let mut share = self.cfg.share_input.unwrap_or(suggested);
            let hint = match (self.cfg.share_input, suggested) {
                (None, true) => "Suggested: an external keyboard or mouse is plugged in".to_string(),
                (None, false) => "Suggested off: only the built-in keyboard and touchpad were found".to_string(),
                (Some(_), s) => format!("Set by you (suggested: {})", if s { "on" } else { "off" }),
            };
            if setting_row(ui, "Share my keyboard and mouse", &hint, &mut share) {
                self.cfg.share_input = Some(share);
            }
            if self.cfg.share_input.is_some() && ui.small_button("Use the suggestion").clicked() {
                self.cfg.share_input = None;
            }
        });
    }

    fn devices_section(&mut self, ui: &mut Ui) {
        section(ui, "Other PCs", |ui| {
            ui.label("No other PCs yet.");
            ui.label(
                RichText::new(
                    "Install HomeKVM on another PC on this network. Pairing with a 6-digit code comes in the next version.",
                )
                .color(muted(ui)),
            );
        });
    }

    fn settings_section(&mut self, ui: &mut Ui) {
        let c = &mut self.cfg;
        section(ui, "Settings", |ui| {
            setting_row(ui, "Share clipboard text", "Copy here, paste there", &mut c.clipboard_text);
            setting_row(ui, "Share images", "Screenshots and copied pictures", &mut c.clipboard_images);
            let files = format!("Up to {} GB, saved to Downloads\\HomeKVM", c.max_file_mb / 1024);
            setting_row(ui, "Send copied files", &files, &mut c.send_files);
            let login = if autostart::APPLIES {
                "Starts in the tray, no admin rights needed"
            } else {
                "Dev build: not applied on this PC"
            };
            setting_row(ui, "Start when I sign in", login, &mut c.start_at_login);
            setting_row(ui, "Pause at the screen edge", "Wait 0.2 s before switching PCs", &mut c.edge_delay);
        });
    }

    fn save_if_changed(&mut self) {
        if self.cfg == self.saved {
            return;
        }
        let mut errors = Vec::new();
        if self.cfg.start_at_login != self.saved.start_at_login {
            if let Err(e) = autostart::apply(self.cfg.start_at_login) {
                errors.push(format!("Couldn't change start at sign-in: {e}"));
            }
        }
        if let Err(e) = config::save(&self.cfg) {
            errors.push(format!("Couldn't save settings: {e}"));
        }
        self.error = (!errors.is_empty()).then(|| errors.join("\n"));
        self.saved = self.cfg.clone();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.hide_instead_of_close(ctx);

        if self.last_scan.elapsed() >= RESCAN {
            self.attached = devices::scan();
            self.last_scan = Instant::now();
        }
        // Keep the device list fresh while the window is open. A hidden window gets no frames.
        ctx.request_repaint_after(RESCAN);

        let mut needed_height = 0.0;
        egui::CentralPanel::default()
            .frame(Frame::central_panel(&ctx.style()).inner_margin(Margin::same(MARGIN)))
            // No scrolling: the window's height follows its content (see below).
            .show(ctx, |ui| {
                self.header(ui);
                ui.add_space(6.0);
                self.layout_section(ui);
                self.this_pc_section(ui);
                self.devices_section(ui);
                self.settings_section(ui);
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::from_rgb(218, 54, 51), e);
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Version {} · pairing, sharing and clipboard arrive in the next versions",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .size(12.0)
                    .color(muted(ui)),
                );
                // The cursor sits just below the last widget, whatever the panel's size.
                let bottom = ui.cursor().min.y - ui.spacing().item_spacing.y;
                needed_height = (bottom + MARGIN).clamp(300.0, 1000.0);
            });

        // Fit the fixed-width window to its content so nothing is ever cut off,
        // whatever the text wrapping or display scaling.
        // The window opens invisible; it appears once the height is right (or after a
        // few frames at most, so it can never stay hidden).
        let current = ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()));
        let fits = !current.is_some_and(|h| (h - needed_height).abs() > 1.0);
        if !fits {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(crate::WINDOW_WIDTH, needed_height)));
            ctx.request_repaint();
        }
        if crate::win::reveal_pending() {
            self.fit_frames += 1;
            if fits || self.fit_frames > 6 {
                crate::win::reveal();
                self.fit_frames = 0;
            } else {
                ctx.request_repaint();
            }
        }

        self.save_if_changed();
    }
}

/// Dev builds only: HOMEKVM_DEMO=1 fills the screen row with sample PCs.
fn demo_layout() -> bool {
    cfg!(debug_assertions) && std::env::var_os("HOMEKVM_DEMO").is_some_and(|v| v == "1")
}

fn describe(a: &Attached) -> String {
    fn list(one: &str, many: &str, links: &[Link]) -> Option<String> {
        if links.is_empty() {
            return None;
        }
        let mut names: Vec<&str> = links.iter().map(|l| l.label()).collect();
        names.dedup();
        let count = if links.len() > 1 { format!("{} external {many}", links.len()) } else { format!("External {one}") };
        Some(format!("{count} ({})", names.join(", ")))
    }
    let mut parts: Vec<String> = [list("keyboard", "keyboards", &a.keyboards), list("mouse", "mice", &a.mice)]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        parts.push("No external keyboard or mouse found".into());
    }
    if a.builtin_keyboard || a.builtin_pointer {
        parts.push("built-in keyboard or touchpad".into());
    }
    parts.join(" · ")
}

fn pill(ui: &mut Ui, status: Status) {
    let [r, g, b] = status.rgb();
    let color = Color32::from_rgb(r, g, b);
    Frame::none()
        .fill(color.gamma_multiply(0.18))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, color);
                ui.label(RichText::new(status.title()).color(color).size(12.0));
            });
        });
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).size(12.0).strong().color(muted(ui)));
    Frame::none()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// A label, a muted line under it, and a switch on the right. Returns true when toggled.
fn setting_row(ui: &mut Ui, title: &str, detail: &str, on: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = toggle(ui, on).changed();
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.label(title);
                ui.label(RichText::new(detail).size(12.0).color(muted(ui)));
            });
        });
    });
    changed
}

fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let visuals = ui.style().interact_selectable(&response, *on);
    let radius = rect.height() / 2.0;
    let track = if *on { ui.visuals().selection.bg_fill } else { ui.visuals().widgets.inactive.bg_fill };
    ui.painter().rect(rect, radius, track, visuals.bg_stroke);
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), radius - 3.0, Color32::WHITE);
    response
}

/// A ‹ or › button that pages the layout row. Its space is kept even when hidden,
/// so the row never shifts.
fn arrow(ui: &mut Ui, width: f32, glyph: &str, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 84.0), Sense::click());
    if !enabled {
        return false;
    }
    let color = if response.hovered() { ui.visuals().strong_text_color() } else { muted(ui) };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(24.0), color);
    response.clicked()
}

/// One screen in the layout row: this PC, a placed PC, or an empty side.
fn screen_slot(ui: &mut Ui, size: Vec2, slot: &Slot) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let v = ui.visuals();
    let accent = v.selection.bg_fill;
    let weak = muted(ui);

    let (name, me, side) = match slot {
        Slot::Me(n) => (Some(n.as_str()), true, ""),
        Slot::Peer(n, side) => (Some(n.as_str()), false, *side),
        Slot::Empty(side) => (None, false, *side),
    };
    match name {
        Some(name) => {
            let stroke = if me { Stroke::new(2.0_f32, accent) } else { v.widgets.noninteractive.fg_stroke };
            painter.rect(rect.shrink(1.0), 8.0, v.extreme_bg_color, stroke);
            draw_monitor(&painter, Pos2::new(rect.center().x, rect.top() + 24.0), if me { accent } else { weak });
            // Long names shrink a little, then get shortened, so they never spill over the card.
            let (label, font_size) = fit_text(ui, name, rect.width() - 14.0);
            painter.text(
                Pos2::new(rect.center().x, rect.top() + 50.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(font_size),
                v.strong_text_color(),
            );
            let sub = if me { "this PC".to_string() } else { format!("{side} side") };
            painter.text(
                Pos2::new(rect.center().x, rect.top() + 67.0),
                egui::Align2::CENTER_CENTER,
                sub,
                egui::FontId::proportional(12.0),
                weak,
            );
        }
        None => {
            dashed_rect(&painter, rect.shrink(1.5), weak);
            painter.text(
                Pos2::new(rect.center().x, rect.center().y - 8.0),
                egui::Align2::CENTER_CENTER,
                "+",
                egui::FontId::proportional(20.0),
                weak,
            );
            painter.text(
                Pos2::new(rect.center().x, rect.center().y + 14.0),
                egui::Align2::CENTER_CENTER,
                format!("{side} side empty"),
                egui::FontId::proportional(12.0),
                weak,
            );
        }
    }
}

fn draw_monitor(p: &egui::Painter, c: Pos2, color: Color32) {
    let screen = Rect::from_center_size(c, Vec2::new(30.0, 19.0));
    p.rect_stroke(screen, 2.0, Stroke::new(1.6_f32, color));
    p.line_segment([Pos2::new(c.x, screen.bottom()), Pos2::new(c.x, screen.bottom() + 4.0)], Stroke::new(1.6_f32, color));
    p.line_segment(
        [Pos2::new(c.x - 7.0, screen.bottom() + 4.5), Pos2::new(c.x + 7.0, screen.bottom() + 4.5)],
        Stroke::new(1.6_f32, color),
    );
}

fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for pair in corners.windows(2) {
        p.extend(Shape::dashed_line(pair, Stroke::new(1.0_f32, color), 5.0, 4.0));
    }
}

/// The largest font size (13 down to 11) at which `s` fits in `max_width`.
/// If it doesn't fit even at 11, it is shortened with "…".
fn fit_text(ui: &Ui, s: &str, max_width: f32) -> (String, f32) {
    let width = |t: &str, size: f32| {
        ui.fonts(|f| f.layout_no_wrap(t.to_owned(), egui::FontId::proportional(size), Color32::WHITE).size().x)
    };
    for size in [13.0, 12.0, 11.0] {
        if width(s, size) <= max_width {
            return (s.to_owned(), size);
        }
    }
    let mut chars: Vec<char> = s.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let t = format!("{}…", chars.iter().collect::<String>().trim_end());
        if width(&t, 11.0) <= max_width {
            return (t, 11.0);
        }
    }
    (s.to_owned(), 11.0)
}

/// Secondary text: clearly readable, just quieter than the main text.
fn muted(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_gray(175)
    } else {
        Color32::from_gray(85)
    }
}
