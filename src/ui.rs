//! The main window. Closing or minimizing it sends it back to the tray.

use crate::config::{self, Config, Side};
use crate::devices::{self, Attached, Link};
use crate::net::{Cmd, NetHandle};
use crate::state::{NetView, PairingView, PeerView, Shared, Status};
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
/// Height of buttons and dropdowns that sit side by side.
const CONTROL_HEIGHT: f32 = 26.0;

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
    net: NetHandle,
    /// The network state for this frame.
    view: NetView,
    /// "Forget" was clicked once for this PC; waiting for the second click.
    confirm_forget: Option<String>,
    /// The "share my keyboard and mouse" value last told to the network side.
    sent_sharing: Option<bool>,
}

/// The "server" badge on a screen card: the PC whose keyboard and mouse drive.
#[derive(Clone, Copy, PartialEq)]
enum Badge {
    /// Shares its keyboard and mouse; nobody is controlling anything right now.
    Server,
    /// Its keyboard and mouse are controlling another PC right now.
    InUse,
}

/// One screen in the layout row.
enum Slot {
    /// This PC: name, badge, being used by another PC right now.
    Me(String, Option<Badge>, bool),
    /// A paired PC: name, side, online, badge, being used by this PC right now.
    Peer(String, &'static str, bool, Option<Badge>, bool),
    Empty(&'static str),
}

/// Space above the screen cards for the badge.
const BADGE_ROOM: f32 = 10.0;

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cfg: Config,
        shared: Arc<Shared>,
        tray: TrayHandle,
        net: NetHandle,
    ) -> Self {
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
            net,
            view: NetView::default(),
            confirm_forget: None,
            sent_sharing: None,
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

            // Who's the "server": while someone controls, only that PC; when idle, every
            // connected PC that shares its keyboard and mouse.
            let controlling = self.view.controlling_id.clone();
            let controlled_by = self.view.controlled_by_id.clone();
            let idle = controlling.is_none() && controlled_by.is_none();

            // The whole row: PCs on the left (outermost first), this PC, PCs on the right.
            // Only paired PCs are shown; the layout stores their IDs.
            let side = |ids: &[String], label: &'static str| -> Vec<Slot> {
                if demo_layout() {
                    // Dev-only preview of a long row (HOMEKVM_DEMO=1); never saved.
                    let names: &[&str] = if label == "left" { &["Work laptop", "Old MacBook"] } else { &["Mini PC", "Media PC"] };
                    return names.iter().map(|n| Slot::Peer(n.to_string(), label, true, None, false)).collect();
                }
                ids.iter()
                    .filter_map(|id| self.view.peer(id).filter(|p| p.paired))
                    .map(|p| {
                        let badge = if controlled_by.as_deref() == Some(p.id.as_str()) {
                            Some(Badge::InUse)
                        } else if idle && p.shares && p.connected {
                            Some(Badge::Server)
                        } else {
                            None
                        };
                        let used = controlling.as_deref() == Some(p.id.as_str());
                        Slot::Peer(p.name.clone(), label, p.connected, badge, used)
                    })
                    .collect()
            };
            let (left, right) = (side(&self.cfg.layout.left, "left"), side(&self.cfg.layout.right, "right"));

            let mut row: Vec<Slot> = Vec::new();
            if left.is_empty() {
                row.push(Slot::Empty("left"));
            }
            row.extend(left.into_iter().rev());
            let me = row.len();
            let my_badge = if controlling.is_some() {
                Some(Badge::InUse)
            } else if idle && (shares_input(&self.cfg, &self.attached) || demo_layout()) {
                Some(Badge::Server)
            } else {
                None
            };
            row.push(Slot::Me(self.cfg.device_name.clone(), my_badge, controlled_by.is_some()));
            let right_empty = right.is_empty();
            row.extend(right);
            if right_empty {
                row.push(Slot::Empty("right"));
            }

            // Show three screens at a time; ‹ and › page through longer rows.
            let max_start = row.len().saturating_sub(VISIBLE);
            let start = self.row_start.unwrap_or(me.saturating_sub(1)).min(max_start);
            let arrows = row.len() > VISIBLE;

            let (gap, arrow_w) = (8.0, 18.0);
            let w = ((ui.available_width() - 2.0 * arrow_w - 4.0 * gap) / VISIBLE as f32).min(124.0);
            let size = Vec2::new(w, 84.0 + BADGE_ROOM);
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

            if !crate::input::available() {
                // Built with --no-default-features: no global keyboard and mouse hooks.
                ui.label("Keyboard and mouse sharing isn't included in this build.");
                return;
            }
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
            if let Some(name) = &self.view.controlled_by {
                ui.label(RichText::new(format!("{name} is using this PC. Move this PC's mouse to take it back.")).color(muted(ui)));
            }
        });
    }

    fn devices_section(&mut self, ui: &mut Ui) {
        let view = self.view.clone();
        section(ui, "Other PCs", |ui| {
            if let Some(p) = &view.pairing {
                self.pairing_box(ui, p);
            } else if let Some(id) = &view.connecting {
                let name = view.peer(id).map(|p| p.name.as_str()).unwrap_or("the other PC");
                ui.label(format!("Connecting to {name}…"));
            }
            if let Some(notice) = &view.notice {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(notice).color(warning(ui)));
                    if ui.small_button("Dismiss").clicked() {
                        self.net.send(Cmd::ClearNotice);
                    }
                });
            }

            // Trusted PCs first, then PCs found on the network.
            let mut peers: Vec<&PeerView> = view.peers.iter().collect();
            peers.sort_by_key(|p| (!p.paired, !p.connected, p.name.to_lowercase()));
            if peers.is_empty() {
                ui.label("No other PCs found yet.");
                ui.label(
                    RichText::new("Open HomeKVM on another PC on this network. It shows up here on its own.")
                        .color(muted(ui)),
                );
            }
            let busy = view.pairing.is_some() || view.connecting.is_some();
            for (i, p) in peers.into_iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                self.peer_row(ui, p, busy);
            }
        });
    }

    fn pairing_box(&mut self, ui: &mut Ui, p: &PairingView) {
        Frame::none()
            .fill(ui.visuals().selection.bg_fill.gamma_multiply(0.22))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = if p.incoming {
                    format!("{} wants to pair with this PC", p.name)
                } else {
                    format!("Pairing with {}", p.name)
                };
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new("Check that both PCs show the same code.").size(12.0).color(muted(ui)));
                ui.label(RichText::new(&p.code).size(30.0).monospace().strong());
                ui.horizontal(|ui| {
                    if p.confirmed {
                        ui.label(RichText::new(format!("Waiting for {} to confirm…", p.name)).color(muted(ui)));
                    } else if ui.button("Codes match — trust").clicked() {
                        self.net.send(Cmd::Confirm(p.id.clone(), true));
                    }
                    if ui.button("Cancel").clicked() {
                        self.net.send(Cmd::Confirm(p.id.clone(), false));
                    }
                });
            });
        ui.add_space(4.0);
    }

    fn peer_row(&mut self, ui: &mut Ui, p: &PeerView, busy: bool) {
        let (dot, status) = match (p.paired, p.connected) {
            (true, true) => (Status::Connected.rgb(), "Connected".to_string()),
            (true, false) if p.found => (Status::NotConnected.rgb(), "Paired · reconnecting…".to_string()),
            (true, false) => (Status::NotConnected.rgb(), "Paired · offline".to_string()),
            (false, _) => (Status::Paused.rgb(), "Found on the network".to_string()),
        };
        let status = match &p.addr {
            Some(a) => format!("{status} · {a}"),
            None => status,
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.5, Color32::from_rgb(dot[0], dot[1], dot[2]));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Every control in the row is the same height.
                ui.spacing_mut().interact_size.y = CONTROL_HEIGHT;
                let size = Vec2::new(72.0, CONTROL_HEIGHT);
                if !p.paired {
                    if ui.add_enabled(!busy, egui::Button::new("Pair").min_size(size)).clicked() {
                        self.net.send(Cmd::Pair(p.id.clone()));
                    }
                } else if self.confirm_forget.as_deref() == Some(p.id.as_str()) {
                    // Second step: a solid red Forget next to a plain Keep.
                    if ui.add(danger_button(ui, "Forget", true).min_size(size)).clicked() {
                        self.net.send(Cmd::Forget(p.id.clone()));
                        self.cfg.layout.place(&p.id, Side::NotPlaced);
                        self.confirm_forget = None;
                    }
                    if ui.add(egui::Button::new("Keep").min_size(size)).clicked() {
                        self.confirm_forget = None;
                    }
                } else {
                    if ui.add(danger_button(ui, "Forget", false).min_size(size)).clicked() {
                        self.confirm_forget = Some(p.id.clone());
                    }
                    let mut side = self.cfg.layout.side_of(&p.id);
                    let before = side;
                    egui::ComboBox::from_id_salt(("side", &p.id))
                        .width(96.0)
                        .selected_text(side.label())
                        .show_ui(ui, |ui| {
                            for s in [Side::Left, Side::Right, Side::NotPlaced] {
                                ui.selectable_value(&mut side, s, s.label());
                            }
                        });
                    if side != before {
                        self.cfg.layout.place(&p.id, side);
                        self.row_start = None;
                    }
                }
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.label(&p.name);
                    ui.label(RichText::new(status).size(12.0).color(muted(ui)));
                });
            });
        });
    }

    fn settings_section(&mut self, ui: &mut Ui) {
        let update_status = self.shared.update_status();
        let c = &mut self.cfg;
        section(ui, "Settings", |ui| {
            setting_row(ui, "Share clipboard text", "Copy here, paste there", &mut c.clipboard_text);
            setting_row(ui, "Share images", "Screenshots and copied pictures", &mut c.clipboard_images);
            setting_row(ui, "Send copied files", "Copy here, paste there. Files move only when you paste.", &mut c.send_files);
            let login = if autostart::APPLIES {
                "Starts in the tray, no admin rights needed"
            } else {
                "Dev build: not applied on this PC"
            };
            setting_row(ui, "Start when I sign in", login, &mut c.start_at_login);
            setting_row(ui, "Pause at the screen edge", "Wait 0.2 s before switching PCs", &mut c.edge_delay);

            // Closed by default; opening it grows the window to fit.
            egui::CollapsingHeader::new("Advanced").default_open(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Paste as real files up to");
                    ui.add(
                        egui::Slider::new(&mut c.real_file_limit_mb, 10..=200)
                            .step_by(10.0)
                            .suffix(" MB"),
                    );
                });
                ui.label(
                    RichText::new(
                        "Smaller copies paste as real files in every app, including browsers and chat apps. \
                         Bigger copies stream straight into folders.",
                    )
                    .size(12.0)
                    .color(muted(ui)),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Updates");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Check now").clicked() {
                            crate::update::check_now();
                        }
                    });
                });
                let status = if update_status.is_empty() {
                    format!("Version {} · installed automatically from GitHub Releases", env!("CARGO_PKG_VERSION"))
                } else {
                    update_status
                };
                ui.label(RichText::new(status).size(12.0).color(muted(ui)));
            });
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
        if self.cfg.device_name != self.saved.device_name {
            self.net.send(Cmd::Rename(self.cfg.device_name.clone()));
        }
        if self.cfg.layout != self.saved.layout {
            self.shared.set_layout(&self.cfg.layout);
            self.tray.refresh();
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
        self.view = self.shared.net();
        apply_settings(&self.cfg, &self.attached);
        // Tell paired PCs when this PC starts or stops sharing its keyboard and mouse.
        let sharing = shares_input(&self.cfg, &self.attached);
        if self.sent_sharing != Some(sharing) {
            self.net.send(Cmd::SetSharing(sharing));
            self.sent_sharing = Some(sharing);
        }

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
                footer(ui, &self.view.my_id);
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

/// This PC shares its keyboard and mouse (and the build can).
pub fn shares_input(cfg: &Config, attached: &Attached) -> bool {
    crate::input::available() && cfg.share_input.unwrap_or(attached.suggests_sharing())
}

/// Apply the sharing settings: keyboard and mouse hooks on or off, and which
/// clipboard kinds are shared.
pub fn apply_settings(cfg: &Config, attached: &Attached) {
    crate::input::set_enabled(cfg.share_input.unwrap_or(attached.suggests_sharing()));
    crate::input::set_edge_delay(cfg.edge_delay);
    crate::clipboard::set_options(cfg.clipboard_text, cfg.clipboard_images, cfg.send_files, cfg.real_file_limit_mb);
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
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 84.0 + BADGE_ROOM), Sense::click());
    if !enabled {
        return false;
    }
    let color = if response.hovered() { ui.visuals().strong_text_color() } else { muted(ui) };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(24.0), color);
    response.clicked()
}

/// One screen in the layout row: this PC, a placed PC, or an empty side.
fn screen_slot(ui: &mut Ui, size: Vec2, slot: &Slot) {
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(area);
    // The card sits below a strip of room for the badge, which overlaps its top edge.
    let rect = Rect::from_min_max(Pos2::new(area.left(), area.top() + BADGE_ROOM), area.max);
    let v = ui.visuals();
    let accent = v.selection.bg_fill;
    let weak = muted(ui);

    let (name, me, side, online, badge, used) = match slot {
        Slot::Me(n, badge, used) => (Some(n.as_str()), true, "", true, *badge, *used),
        Slot::Peer(n, side, online, badge, used) => (Some(n.as_str()), false, *side, *online, *badge, *used),
        Slot::Empty(side) => (None, false, *side, false, None, false),
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
            let sub = match (me, online, used) {
                (_, _, true) => "in use".to_string(),
                (true, _, _) => "this PC".to_string(),
                (false, true, _) => format!("{side} side"),
                (false, false, _) => format!("{side} · offline"),
            };
            painter.text(
                Pos2::new(rect.center().x, rect.top() + 67.0),
                egui::Align2::CENTER_CENTER,
                sub,
                egui::FontId::proportional(12.0),
                weak,
            );
            if let Some(badge) = badge {
                draw_badge(&painter, Pos2::new(rect.center().x, rect.top() + 1.0), badge);
            }
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

const REPO_URL: &str = "https://github.com/saugatadatta/HomeKVM";

/// Two lines on the left (version, this PC's ID) and the about badge on the right,
/// the same height as the two lines.
fn footer(ui: &mut Ui, my_id: &str) {
    let muted = muted(ui);
    let line = 15.0;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(format!("HomeKVM {} · encrypted, paired PCs only", env!("CARGO_PKG_VERSION")))
                    .size(12.0)
                    .color(muted),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("This PC's ID:").size(12.0).color(muted));
                ui.label(RichText::new(my_id).size(12.0).monospace().color(muted));
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            about_badge(ui, 2.0 * line + 4.0);
        });
    });
}

/// "by Saugata Datta" over "View on GitHub ↗" in a soft blue box; opens the repo.
fn about_badge(ui: &mut Ui, height: f32) {
    let blue = Color32::from_rgb(47, 129, 247);
    let text = if ui.visuals().dark_mode { Color32::from_rgb(207, 226, 255) } else { Color32::from_rgb(20, 70, 150) };
    let font = egui::FontId::proportional(11.5);
    let top = ui.painter().layout_no_wrap("by Saugata Datta".to_owned(), font.clone(), text);
    let bottom = ui.painter().layout_no_wrap("View on GitHub ↗".to_owned(), font, text);
    let (icon, pad, gap) = (12.0, 9.0, 7.0);
    let width = pad + icon + gap + top.size().x.max(bottom.size().x) + pad;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let response = response.on_hover_text(REPO_URL).on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(REPO_URL));
    }
    let p = ui.painter();
    let fill = if response.hovered() { blue.gamma_multiply(0.30) } else { blue.gamma_multiply(0.16) };
    p.rect(rect, 8.0, fill, Stroke::new(1.0_f32, blue.gamma_multiply(0.6)));
    // A small "code" glyph: </>
    let c = Pos2::new(rect.left() + pad + icon / 2.0, rect.center().y);
    let s = Stroke::new(1.3_f32, text);
    p.line_segment([Pos2::new(c.x - 2.5, c.y - 4.0), Pos2::new(c.x - 6.0, c.y)], s);
    p.line_segment([Pos2::new(c.x - 6.0, c.y), Pos2::new(c.x - 2.5, c.y + 4.0)], s);
    p.line_segment([Pos2::new(c.x + 2.5, c.y - 4.0), Pos2::new(c.x + 6.0, c.y)], s);
    p.line_segment([Pos2::new(c.x + 6.0, c.y), Pos2::new(c.x + 2.5, c.y + 4.0)], s);
    p.line_segment([Pos2::new(c.x + 1.0, c.y - 5.0), Pos2::new(c.x - 1.0, c.y + 5.0)], s);
    let x = rect.left() + pad + icon + gap;
    let (h1, h2) = (top.size().y, bottom.size().y);
    let y0 = rect.center().y - (h1 + h2 + 1.0) / 2.0;
    p.galley(Pos2::new(x, y0), top, text);
    p.galley(Pos2::new(x, y0 + h1 + 1.0), bottom, text);
}

/// A small blue pill with a keyboard glyph: "Server" or "Server · in use".
fn draw_badge(p: &egui::Painter, center: Pos2, badge: Badge) {
    let text = match badge {
        Badge::Server => "Server",
        Badge::InUse => "Server · in use",
    };
    let white = Color32::WHITE;
    let galley = p.layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), white);
    let (icon_w, pad, gap, h) = (11.0, 8.0, 4.0, 18.0);
    let w = pad + icon_w + gap + galley.size().x + pad;
    let pill = Rect::from_center_size(center, Vec2::new(w, h));
    p.rect_filled(pill, h / 2.0, Color32::from_rgb(47, 129, 247));
    // Keyboard glyph: an outline with a space bar.
    let key = Rect::from_min_size(Pos2::new(pill.left() + pad, pill.center().y - 3.5), Vec2::new(icon_w, 7.0));
    p.rect_stroke(key, 1.5, Stroke::new(1.0_f32, white));
    p.line_segment(
        [Pos2::new(key.left() + 3.0, key.bottom() - 2.0), Pos2::new(key.right() - 3.0, key.bottom() - 2.0)],
        Stroke::new(1.0_f32, white),
    );
    p.galley(Pos2::new(key.right() + gap, pill.center().y - galley.size().y / 2.0), galley, white);
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

/// A red button: tinted, or solid when `solid` (for the final confirmation).
fn danger_button(ui: &Ui, text: &str, solid: bool) -> egui::Button<'static> {
    let red = danger(ui);
    if solid {
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
            .fill(Color32::from_rgb(196, 43, 28))
            .stroke(Stroke::NONE)
    } else {
        egui::Button::new(RichText::new(text).color(red))
            .fill(red.gamma_multiply(0.14))
            .stroke(Stroke::new(1.0_f32, red.gamma_multiply(0.55)))
    }
}

fn warning(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(232, 180, 80)
    } else {
        Color32::from_rgb(150, 95, 10)
    }
}

fn danger(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(240, 110, 105)
    } else {
        Color32::from_rgb(190, 40, 35)
    }
}

/// Secondary text: clearly readable, just quieter than the main text.
fn muted(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_gray(175)
    } else {
        Color32::from_gray(85)
    }
}
