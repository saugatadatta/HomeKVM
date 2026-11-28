//! The main window: a fixed-width window in two equal columns, with no scrolling.
//! Left: screen layout, this PC, other PCs. Right: settings and advanced.
//! Both columns end on the same line: the last card in the shorter column grows to meet
//! the taller one, and the window's height follows. Closing or minimizing it sends it
//! back to the tray. The look ("Midnight" dark, and a matching light theme) follows the
//! Windows setting.

use crate::activity::{self, Health};
use crate::config::{self, Config, Side};
use crate::devices::{self, Attached, Link};
use crate::net::{Cmd, NetHandle};
use crate::state::{NetView, PairingView, PeerView, Shared, Status};
use crate::{autostart, tray::TrayHandle};
use eframe::egui::{
    self, text::LayoutJob, Align, Align2, Color32, Context, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    Response, RichText, Rounding, Sense, Stroke, TextFormat, Ui, UiBuilder, Vec2, ViewportCommand,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESCAN: Duration = Duration::from_secs(3);
/// Space around the window's content.
const MARGIN: f32 = 18.0;
/// Space between the two columns, and between cards in a column.
const COLUMN_GAP: f32 = 14.0;
const CARD_GAP: f32 = 12.0;
/// Inside a card.
const CARD_PADDING: f32 = 14.0;
/// Height of buttons and dropdowns that sit side by side.
const CONTROL_HEIGHT: f32 = 30.0;
/// One row in the Other PCs list, and the space between rows.
const ROW_HEIGHT: f32 = 50.0;
const ROW_GAP: f32 = 6.0;
/// Rows the Other PCs list asks room for before it scrolls.
const MIN_ROWS: usize = 3;
/// Room under the rows for the "more PCs show up here" note.
const FILLER: f32 = 96.0;
/// The hint line at the bottom of the Other PCs card.
const LIST_FOOTER: f32 = 30.0;
/// Screen cards in the layout.
const SCREEN_W: f32 = 118.0;
const SCREEN_H: f32 = 84.0;
const LINK_GAP: f32 = 14.0;
/// Space above the row of screens for the Server badge.
const BADGE_ROOM: f32 = 10.0;

// Icons from the Tabler icon font (assets/fonts, MIT license).
const I_KEYBOARD: &str = "\u{ebd6}";
const I_LAYOUT: &str = "\u{ead4}";
const I_LAPTOP: &str = "\u{eb64}";
const I_DESKTOP: &str = "\u{ea89}";
const I_DEVICES: &str = "\u{eb87}";
const I_ADJUST: &str = "\u{ec38}";
const I_TOOL: &str = "\u{eb40}";
const I_WIFI: &str = "\u{eb52}";
const I_DEVICES_PLUS: &str = "\u{f8f2}";
const I_REFRESH: &str = "\u{eb13}";
const I_COPY: &str = "\u{ea7a}";
const I_PAUSE: &str = "\u{ed45}";
const I_PLAY: &str = "\u{ed46}";
const I_GITHUB: &str = "\u{ec1c}";
const I_TRASH: &str = "\u{eb41}";
const I_LEFT: &str = "\u{ea60}";
const I_RIGHT: &str = "\u{ea61}";
const I_UP: &str = "\u{ea62}";
const I_DOWN: &str = "\u{ea5f}";

/// The colors of one theme.
#[derive(Clone, Copy)]
struct Pal {
    /// Window background.
    bg: Color32,
    /// Cards.
    card: Color32,
    /// Things on a card: screens, rows, panels.
    raise: Color32,
    /// Those under the mouse.
    hov: Color32,
    line: Color32,
    line2: Color32,
    /// Text: main, secondary, quiet.
    tx: Color32,
    tx2: Color32,
    tx3: Color32,
    /// The one accent, its text shade, and two tints of it on a card.
    ac: Color32,
    act: Color32,
    acbg: Color32,
    acbg2: Color32,
    ok: Color32,
    wa: Color32,
    bad: Color32,
    /// A switch that is off, the empty part of the slider.
    off: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const MIDNIGHT: Pal = Pal {
    bg: rgb(14, 16, 20),
    card: rgb(22, 26, 33),
    raise: rgb(29, 34, 43),
    hov: rgb(37, 43, 54),
    line: rgb(38, 44, 55),
    line2: rgb(53, 61, 75),
    tx: rgb(232, 236, 242),
    tx2: rgb(154, 164, 180),
    tx3: rgb(107, 117, 134),
    ac: rgb(79, 140, 255),
    act: rgb(156, 192, 255),
    acbg: rgb(29, 40, 60),
    acbg2: rgb(35, 51, 82),
    ok: rgb(63, 185, 80),
    wa: rgb(210, 153, 34),
    bad: rgb(248, 81, 73),
    off: rgb(43, 49, 60),
};

const LIGHT: Pal = Pal {
    bg: rgb(243, 245, 248),
    card: rgb(255, 255, 255),
    raise: rgb(244, 246, 249),
    hov: rgb(234, 238, 244),
    line: rgb(227, 231, 238),
    line2: rgb(207, 214, 224),
    tx: rgb(21, 26, 34),
    tx2: rgb(91, 101, 118),
    tx3: rgb(138, 147, 163),
    ac: rgb(47, 111, 232),
    act: rgb(31, 86, 194),
    acbg: rgb(236, 242, 253),
    acbg2: rgb(222, 233, 251),
    ok: rgb(26, 127, 55),
    wa: rgb(154, 103, 0),
    bad: rgb(207, 34, 46),
    off: rgb(213, 219, 228),
};

fn pal(ui: &Ui) -> Pal {
    if ui.visuals().dark_mode {
        MIDNIGHT
    } else {
        LIGHT
    }
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub struct App {
    cfg: Config,
    saved: Config,
    shared: Arc<Shared>,
    tray: TrayHandle,
    attached: Attached,
    last_scan: Instant,
    error: Option<String>,
    /// Frames spent fitting the height while the window opens invisibly.
    fit_frames: u32,
    net: NetHandle,
    /// The network state for this frame.
    view: NetView,
    /// "Forget" was clicked once for this PC; waiting for the second click.
    confirm_forget: Option<String>,
    /// The "share my keyboard and mouse" value last told to the network side.
    sent_sharing: Option<bool>,
    /// Each column's height without growing, measured last frame. The shorter column's
    /// last card grows by the difference.
    left_natural: f32,
    right_natural: f32,
    /// First row shown in the Other PCs list.
    first_peer: usize,
    /// Which PC each side of the row shows when it holds more than one.
    left_page: usize,
    right_page: usize,
}

/// The "server" badge on a screen card: the PC whose keyboard and mouse drive.
#[derive(Clone, Copy, PartialEq)]
enum Badge {
    /// Shares its keyboard and mouse; nobody is controlling anything right now.
    Server,
    /// Its keyboard and mouse are controlling another PC right now.
    InUse,
}

/// What a screen card shows.
struct Card {
    /// None: an empty spot.
    name: Option<String>,
    sub: String,
    me: bool,
    online: bool,
    badge: Option<Badge>,
    /// Drawn as a laptop rather than a monitor.
    laptop: bool,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cfg: Config,
        shared: Arc<Shared>,
        tray: TrayHandle,
        net: NetHandle,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);
        let mut cfg = cfg;
        let attached = if store_demo() {
            // Dev-only sample data for Store screenshots; never saved (`saved` matches).
            cfg.device_name = "Desk PC".into();
            cfg.share_input = None;
            cfg.start_at_login = false;
            cfg.edge_delay = false;
            cfg.clipboard_text = true;
            cfg.clipboard_images = true;
            cfg.send_files = true;
            cfg.layout.left = vec!["CREATORX".into()];
            cfg.layout.right = vec!["Office PC".into()];
            Attached { keyboards: vec![Link::Usb], mice: vec![Link::Usb], builtin_keyboard: false, builtin_pointer: false }
        } else {
            devices::scan()
        };
        Self {
            saved: cfg.clone(),
            cfg,
            shared,
            tray,
            attached,
            last_scan: Instant::now(),
            error: None,
            fit_frames: 0,
            net,
            view: NetView::default(),
            confirm_forget: None,
            sent_sharing: None,
            left_natural: 0.0,
            right_natural: 0.0,
            first_peer: 0,
            left_page: 0,
            right_page: 0,
        }
    }

    fn hide_instead_of_close(&self, ctx: &Context) {
        let (close, minimized) =
            ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        // Hidden with a direct Windows call (see win::hide_main_window), so it goes back
        // to the tray every time, not only the first time.
        if close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            crate::win::hide_main_window(true);
        } else if minimized {
            // Windows already animated it down to the taskbar.
            crate::win::hide_main_window(false);
        }
    }

    /// The app mark, the name, the status pill, and Pause on the right.
    fn header(&mut self, ui: &mut Ui) {
        let p = pal(ui);
        let status = if store_demo() { Status::Connected } else { self.shared.status() };
        let w = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(w, 30.0), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (mark, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            ui.painter().rect_filled(mark, 9.0, p.ac);
            ui.painter().text(mark.center(), Align2::CENTER_CENTER, I_KEYBOARD, icon_font(17.0), Color32::WHITE);
            ui.label(RichText::new("HomeKVM").font(semibold(18.0)).color(p.tx));
            status_pill(ui, status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let paused = self.shared.paused();
                let (icon, text) = if paused { (I_PLAY, "Resume") } else { (I_PAUSE, "Pause") };
                if icon_button(ui, icon, text, 12.5).clicked() {
                    self.shared.set_paused(!paused);
                    self.tray.refresh();
                }
            });
        });
    }

    // ---- Left column ------------------------------------------------------------------

    /// Draws the column; `extra` is how much the last card grows. Returns the column's
    /// natural height (without the growing).
    fn left_column(&mut self, ui: &mut Ui, extra: f32) -> f32 {
        let top = ui.min_rect().top();
        self.layout_card(ui);
        ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
        self.this_pc_card(ui);
        ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
        self.devices_card(ui, extra);
        ui.min_rect().bottom() - top - extra
    }

    /// Who is the "server": while someone controls, only that PC; when idle, every
    /// connected PC that shares its keyboard and mouse.
    fn peer_card(&self, p: &PeerView, sub: String) -> Card {
        let controlling = self.view.controlling_id.as_deref();
        let controlled_by = self.view.controlled_by_id.as_deref();
        let idle = controlling.is_none() && controlled_by.is_none();
        let badge = if controlled_by == Some(p.id.as_str()) {
            Some(Badge::InUse)
        } else if idle && p.shares && p.connected {
            Some(Badge::Server)
        } else {
            None
        };
        let in_use = controlling == Some(p.id.as_str());
        let sub = match (in_use, p.connected) {
            (true, _) => "in use".to_string(),
            (false, true) => sub,
            (false, false) => format!("{sub} · offline"),
        };
        Card { name: Some(p.name.clone()), sub, me: false, online: p.connected, badge, laptop: false }
    }

    fn layout_card(&mut self, ui: &mut Ui) {
        // The PCs on each side (paired only; the layout stores their IDs).
        let paired = |id: &String| self.view.peer(id).filter(|p| p.paired).cloned();
        let demo = demo_layout();
        let sample = |name: &str| PeerView { id: name.into(), name: name.into(), paired: true, connected: true, ..Default::default() };
        let (lefts, rights): (Vec<PeerView>, Vec<PeerView>) = if demo {
            // Dev-only preview, never saved. HOMEKVM_DEMO=full puts two PCs on the left.
            let full = std::env::var_os("HOMEKVM_DEMO").is_some_and(|v| v == "full");
            let mut lefts = vec![sample("CREATORX")];
            if full {
                lefts.push(sample("Old MacBook"));
            }
            (lefts, vec![sample("Office PC")])
        } else {
            let l = &self.cfg.layout;
            (l.left.iter().filter_map(paired).collect(), l.right.iter().filter_map(paired).collect())
        };
        self.left_page = self.left_page.min(lefts.len().saturating_sub(1));
        self.right_page = self.right_page.min(rights.len().saturating_sub(1));

        let controlling = self.view.controlling_id.is_some();
        let controlled = self.view.controlled_by_id.is_some();
        let my_badge = if controlling {
            Some(Badge::InUse)
        } else if !controlled && (shares_input(&self.cfg, &self.attached) || demo) {
            Some(Badge::Server)
        } else {
            None
        };
        let me = Card {
            name: Some(self.cfg.device_name.clone()),
            sub: if controlled { "in use".into() } else { "this PC".into() },
            me: true,
            online: true,
            badge: my_badge,
            laptop: self.attached.builtin_keyboard && self.attached.builtin_pointer,
        };
        let side_card = |list: &[PeerView], page: usize, side: &str| -> Card {
            match list.get(page) {
                Some(p) => {
                    let sub = if list.len() > 1 { format!("{side} · {} of {}", page + 1, list.len()) } else { side.to_string() };
                    self.peer_card(p, sub)
                }
                None => Card { name: None, sub: format!("{side} side empty"), me: false, online: false, badge: None, laptop: false },
            }
        };
        let left = side_card(&lefts, self.left_page, "left");
        let right = side_card(&rights, self.right_page, "right");
        let (more_left, more_right) = (lefts.len() > 1, rights.len() > 1);

        card(ui, I_LAYOUT, "Screen layout", |_| {}, |ui| {
            let p = pal(ui);
            ui.label(
                RichText::new("Place paired PCs left or right. The mouse crosses where they touch.").size(12.0).color(p.tx2),
            );
            ui.add_space(6.0);
            let full_w = ui.available_width();
            let w = ((full_w - 2.0 * LINK_GAP) / 3.0).min(SCREEN_W);
            let row_w = 3.0 * w + 2.0 * LINK_GAP;
            let (area, _) = ui.allocate_exact_size(Vec2::new(full_w, BADGE_ROOM + SCREEN_H + 2.0), Sense::hover());
            let x0 = area.center().x - row_w / 2.0;
            let y = area.top() + BADGE_ROOM;
            let at = |x: f32| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, SCREEN_H));
            let (left_r, me_r, right_r) = (at(x0), at(x0 + w + LINK_GAP), at(x0 + 2.0 * (w + LINK_GAP)));

            // Short blue lines where the mouse crosses between screens.
            let cy = me_r.center().y;
            let link = Stroke::new(2.0_f32, p.ac);
            if left.name.is_some() {
                ui.painter().line_segment([Pos2::new(left_r.right() + 1.0, cy), Pos2::new(me_r.left() - 1.0, cy)], link);
            }
            if right.name.is_some() {
                ui.painter().line_segment([Pos2::new(me_r.right() + 1.0, cy), Pos2::new(right_r.left() - 1.0, cy)], link);
            }
            screen_card(ui, left_r, &left, "left");
            screen_card(ui, right_r, &right, "right");
            screen_card(ui, me_r, &me, "me");
            // A side with more than one PC: a round button on its outer edge steps through them.
            if more_left && round_button(ui, Pos2::new(left_r.left(), cy), I_LEFT, "page-left") {
                self.left_page = (self.left_page + 1) % lefts.len().max(1);
            }
            if more_right && round_button(ui, Pos2::new(right_r.right(), cy), I_RIGHT, "page-right") {
                self.right_page = (self.right_page + 1) % rights.len().max(1);
            }
        });
    }

    fn this_pc_card(&mut self, ui: &mut Ui) {
        let suggested = self.attached.suggests_sharing();
        card(ui, I_LAPTOP, "This PC", |_| {}, |ui| {
            let p = pal(ui);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, CONTROL_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new("Name").color(p.tx));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cfg.device_name)
                        .margin(Margin::symmetric(10.0, 5.0))
                        .text_color(p.tx)
                        .desired_width(f32::INFINITY),
                );
            });
            ui.label(RichText::new(describe(&self.attached)).size(12.0).color(p.tx2));

            if !crate::input::available() && !store_demo() {
                // Built with --no-default-features: no global keyboard and mouse hooks.
                ui.label(RichText::new("Keyboard and mouse sharing isn't included in this build.").size(12.0).color(p.tx2));
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
            if self.cfg.share_input.is_some() && link_button(ui, "Use the suggestion").clicked() {
                self.cfg.share_input = None;
            }
            if let Some(name) = &self.view.controlled_by {
                ui.label(
                    RichText::new(format!("{name} is using this PC. Move this PC's mouse to take it back.")).size(12.0).color(p.tx2),
                );
            }
        });
    }

    /// The last card on the left: grows by `extra`, which goes to the space under the rows.
    fn devices_card(&mut self, ui: &mut Ui, extra: f32) {
        let view = self.view.clone();
        // Trusted PCs first, then PCs found on the network.
        let mut peers: Vec<&PeerView> = view.peers.iter().collect();
        peers.sort_by_key(|p| (!p.paired, !p.connected, p.name.to_lowercase()));
        let busy = view.pairing.is_some() || view.connecting.is_some();
        let count = peers.len();

        let right = |ui: &mut Ui| {
            let p = pal(ui);
            if count > 0 {
                count_chip(ui, count);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("Looking on this network").size(11.0).color(p.tx3));
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, p.ac);
            });
        };
        card(ui, I_DEVICES, "Other PCs", right, |ui| {
            let p = pal(ui);
            if let Some(pairing) = &view.pairing {
                self.pairing_box(ui, pairing);
            } else if let Some(id) = &view.connecting {
                let name = view.peer(id).map(|p| p.name.as_str()).unwrap_or("the other PC");
                ui.label(RichText::new(format!("Connecting to {name}…")).color(p.tx2));
            }
            if let Some(notice) = &view.notice {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(notice).size(12.0).color(p.wa));
                    if link_button(ui, "Dismiss").clicked() {
                        self.net.send(Cmd::ClearNotice);
                    }
                });
            }

            // The rows, then a note in the room that's left. With more PCs than fit, the
            // rows take all the room and round buttons scroll them.
            let step = ROW_HEIGHT + ROW_GAP;
            let wanted_rows = count.clamp(1, MIN_ROWS);
            let space = wanted_rows as f32 * step - ROW_GAP + FILLER + extra;
            let width = ui.available_width();
            let (area, _) = ui.allocate_exact_size(Vec2::new(width, space), Sense::hover());

            let with_note = ((space - FILLER + ROW_GAP) / step).floor().max(0.0) as usize;
            let (shown, scrolls) = if count <= with_note {
                (count, false)
            } else {
                let fit = (((space + ROW_GAP) / step).floor() as usize).max(1);
                (fit.min(count), count > fit)
            };
            if scrolls {
                self.first_peer = self.first_peer.min(count - shown);
                if ui.rect_contains_pointer(area) {
                    let wheel = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel < 0.0 {
                        self.first_peer = (self.first_peer + 1).min(count - shown);
                    } else if wheel > 0.0 {
                        self.first_peer = self.first_peer.saturating_sub(1);
                    }
                }
            } else {
                self.first_peer = 0;
            }
            let row_w = if scrolls { width - 32.0 } else { width };
            for (k, peer) in peers.iter().skip(self.first_peer).take(shown).enumerate() {
                let row = Rect::from_min_size(Pos2::new(area.left(), area.top() + k as f32 * step), Vec2::new(row_w, ROW_HEIGHT));
                self.peer_row(ui, row, peer, busy);
            }
            if scrolls {
                let x = area.right() - 12.0;
                if self.first_peer > 0 && round_button(ui, Pos2::new(x, area.top() + 12.0), I_UP, "scroll-up") {
                    self.first_peer -= 1;
                }
                if self.first_peer + shown < count && round_button(ui, Pos2::new(x, area.bottom() - 12.0), I_DOWN, "scroll-down") {
                    self.first_peer += 1;
                }
            } else {
                // The note sits in the middle of the room under the rows.
                let used = if shown == 0 { 0.0 } else { shown as f32 * step };
                let room = Rect::from_min_max(Pos2::new(area.left(), area.top() + used), area.max);
                if room.height() >= 60.0 {
                    let text = if count == 0 {
                        "No other PCs found yet"
                    } else {
                        "More PCs show up here as soon as HomeKVM starts on them."
                    };
                    let c = room.center();
                    ui.painter().text(Pos2::new(c.x, c.y - 12.0), Align2::CENTER_CENTER, I_DEVICES_PLUS, icon_font(26.0), p.line2);
                    ui.painter().text(Pos2::new(c.x, c.y + 14.0), Align2::CENTER_CENTER, text, FontId::proportional(12.0), p.tx3);
                }
            }

            // The hint at the bottom, under a hairline.
            let (foot, _) = ui.allocate_exact_size(Vec2::new(width, LIST_FOOTER - ui.spacing().item_spacing.y), Sense::hover());
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0_f32, p.line));
            let y = foot.center().y + 3.0;
            ui.painter().text(Pos2::new(foot.left() + 2.0, y), Align2::LEFT_CENTER, I_WIFI, icon_font(14.0), p.tx3);
            ui.painter().text(
                Pos2::new(foot.left() + 22.0, y),
                Align2::LEFT_CENTER,
                "Open HomeKVM on another PC on this network to add it here.",
                FontId::proportional(11.0),
                p.tx3,
            );
        });
    }

    fn pairing_box(&mut self, ui: &mut Ui, pairing: &PairingView) {
        let p = pal(ui);
        Frame::none()
            .fill(p.acbg)
            .stroke(Stroke::new(1.0_f32, p.ac))
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = if pairing.incoming {
                    format!("{} wants to pair with this PC", pairing.name)
                } else {
                    format!("Pairing with {}", pairing.name)
                };
                ui.label(RichText::new(title).font(semibold(13.0)).color(p.tx));
                ui.label(RichText::new("Check that both PCs show the same code.").size(12.0).color(p.tx2));
                ui.label(RichText::new(&pairing.code).size(28.0).monospace().color(p.tx));
                ui.horizontal(|ui| {
                    if pairing.confirmed {
                        ui.label(RichText::new(format!("Waiting for {} to confirm…", pairing.name)).size(12.0).color(p.tx2));
                    } else if primary_button(ui, "Codes match — trust", Vec2::new(0.0, CONTROL_HEIGHT)).clicked() {
                        self.net.send(Cmd::Confirm(pairing.id.clone(), true));
                    }
                    if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(76.0, CONTROL_HEIGHT))).clicked() {
                        self.net.send(Cmd::Confirm(pairing.id.clone(), false));
                    }
                });
            });
        ui.add_space(4.0);
    }

    /// One PC in the list: a device tile with a status dot, the name and status, and its
    /// controls on the right. Brighter under the mouse.
    fn peer_row(&mut self, ui: &mut Ui, row: Rect, peer: &PeerView, busy: bool) {
        let p = pal(ui);
        let (dot, status) = match (peer.paired, peer.connected) {
            (true, true) => (p.ok, "Connected".to_string()),
            (true, false) if peer.found => (p.wa, "Paired · reconnecting…".to_string()),
            (true, false) => (p.tx3, "Paired · offline".to_string()),
            (false, _) => (p.wa, "Found on the network".to_string()),
        };
        let status = match &peer.addr {
            Some(a) => format!("{status} · {a}"),
            None => status,
        };
        let hovered = ui.rect_contains_pointer(row);
        let painter = ui.painter().clone();
        let border = if hovered { Stroke::new(1.0_f32, p.line2) } else { Stroke::NONE };
        painter.rect(row, 10.0, if hovered { p.hov } else { p.raise }, border);

        let tile = Rect::from_min_size(Pos2::new(row.left() + 10.0, row.center().y - 17.0), Vec2::splat(34.0));
        painter.rect_filled(tile, 10.0, p.card);
        painter.text(tile.center(), Align2::CENTER_CENTER, I_DESKTOP, icon_font(18.0), if hovered { p.tx } else { p.tx2 });
        painter.circle(Pos2::new(tile.right() - 3.0, tile.bottom() - 3.0), 5.0, dot, Stroke::new(2.0_f32, if hovered { p.hov } else { p.raise }));

        let confirming = self.confirm_forget.as_deref() == Some(peer.id.as_str());
        let gap = 6.0;
        let actions_w = if !peer.paired {
            84.0
        } else if confirming {
            2.0 * 76.0 + gap
        } else {
            92.0 + gap + CONTROL_HEIGHT
        };
        let actions = Rect::from_min_max(
            Pos2::new(row.right() - 10.0 - actions_w, row.top()),
            Pos2::new(row.right() - 10.0, row.bottom()),
        );
        let text_x = tile.right() + 10.0;
        let text_w = (actions.left() - 8.0 - text_x).max(20.0);
        let (name, size) = fit(ui, &peer.name, text_w, 13.0, semibold);
        painter.text(Pos2::new(text_x, row.center().y - 8.0), Align2::LEFT_CENTER, name, semibold(size), p.tx);
        let (status, _) = fit(ui, &status, text_w, 11.0, FontId::proportional);
        painter.text(Pos2::new(text_x, row.center().y + 9.0), Align2::LEFT_CENTER, status, FontId::proportional(11.0), p.tx2);

        let mut actions_ui = ui.new_child(UiBuilder::new().max_rect(actions).layout(Layout::right_to_left(Align::Center)));
        let ui = &mut actions_ui;
        ui.spacing_mut().item_spacing.x = gap;
        ui.spacing_mut().interact_size.y = CONTROL_HEIGHT;
        let size = Vec2::new(76.0, CONTROL_HEIGHT);
        if !peer.paired {
            if ui.add_enabled_ui(!busy, |ui| primary_button(ui, "Pair", Vec2::new(84.0, CONTROL_HEIGHT))).inner.clicked() {
                self.net.send(Cmd::Pair(peer.id.clone()));
            }
        } else if confirming {
            // Second step: a solid red Forget next to a plain Keep.
            let forget = egui::Button::new(RichText::new("Forget").color(Color32::WHITE)).fill(p.bad).stroke(Stroke::NONE).min_size(size);
            if ui.add(forget).clicked() {
                self.net.send(Cmd::Forget(peer.id.clone()));
                self.cfg.layout.place(&peer.id, Side::NotPlaced);
                self.confirm_forget = None;
            }
            if ui.add(egui::Button::new("Keep").min_size(size)).clicked() {
                self.confirm_forget = None;
            }
        } else {
            if trash_button(ui, hovered) {
                self.confirm_forget = Some(peer.id.clone());
            }
            let mut side = self.cfg.layout.side_of(&peer.id);
            let before = side;
            egui::ComboBox::from_id_salt(("side", &peer.id))
                .width(92.0 - 2.0 * ui.spacing().button_padding.x)
                .selected_text(side.short())
                .show_ui(ui, |ui| {
                    for s in Side::ALL {
                        ui.selectable_value(&mut side, s, s.label());
                    }
                });
            if side != before {
                self.cfg.layout.place(&peer.id, side);
            }
        }
    }

    // ---- Right column -----------------------------------------------------------------

    /// Like `left_column`: `extra` goes into the Advanced card.
    fn right_column(&mut self, ui: &mut Ui, extra: f32) -> f32 {
        let top = ui.min_rect().top();
        self.settings_card(ui);
        ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
        self.advanced_card(ui, extra);
        ui.min_rect().bottom() - top - extra
    }

    fn settings_card(&mut self, ui: &mut Ui) {
        let login = if autostart::APPLIES || crate::win::is_packaged() || store_demo() {
            "Off until you turn it on"
        } else {
            "Dev build: not applied on this PC"
        };
        let c = &mut self.cfg;
        card(ui, I_ADJUST, "Settings", |_| {}, |ui| {
            let rows: [(&str, &str, &mut bool); 5] = [
                ("Share clipboard text", "Copy here, paste there", &mut c.clipboard_text),
                ("Share images", "Screenshots and copied pictures", &mut c.clipboard_images),
                ("Send copied files", "Files move only when you paste", &mut c.send_files),
                ("Start when I sign in", login, &mut c.start_at_login),
                ("Pause at the screen edge", "Wait 0.2 s before switching PCs", &mut c.edge_delay),
            ];
            for (k, (title, detail, on)) in rows.into_iter().enumerate() {
                // Start at sign-in is off until turned on: a soft blue panel points it out.
                let highlight = k == 3;
                let after_highlight = k == 4;
                if k > 0 && !highlight && !after_highlight {
                    let w = ui.available_width();
                    let (line, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0_f32, pal(ui).line));
                }
                if highlight {
                    let p = pal(ui);
                    Frame::none()
                        .fill(p.acbg)
                        .stroke(Stroke::new(1.0_f32, p.acbg2))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(10.0, 4.0))
                        .outer_margin(Margin::symmetric(-10.0, 0.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            setting_row_tinted(ui, title, detail, on);
                        });
                } else {
                    setting_row(ui, title, detail, on);
                }
            }
        });
    }

    /// The last card on the right: grows by `extra`, which goes above Updates.
    fn advanced_card(&mut self, ui: &mut Ui, extra: f32) {
        let update_status = self.shared.update_status();
        let rows = diagnostics(&self.view);
        let c = &mut self.cfg;
        card(ui, I_TOOL, "Advanced", |_| {}, |ui| {
            let p = pal(ui);
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, 24.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new("Paste as real files up to").color(p.tx));
                let value_w = 50.0;
                let slider_w = ui.available_width() - value_w - 10.0;
                slider(ui, &mut c.real_file_limit_mb, (10, 200), 10, slider_w);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} MB", c.real_file_limit_mb)).color(p.tx));
                });
            });
            ui.add_space(-4.0);
            ui.label(
                RichText::new("Smaller copies paste as real files in every app. Bigger copies stream straight into folders.")
                    .size(12.0)
                    .color(p.tx2),
            );
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(Vec2::new(w, 24.0), Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new("Diagnostics").font(semibold(12.0)).color(p.tx2));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let copy = icon_button(ui, I_COPY, "Copy report", 11.0).on_hover_text("Copies these lines, to paste into a bug report");
                    if copy.clicked() {
                        ui.ctx().copy_text(report(&rows));
                    }
                });
            });
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                for (health, label, value) in &rows {
                    diag_row(ui, *health, label, value);
                }
            });
            ui.add_space(6.0 + extra);

            // Updates, at the end in its own panel.
            let packaged = crate::win::is_packaged();
            let second = if packaged {
                "Updated by the Microsoft Store".to_string()
            } else if update_status.is_empty() || update_status.starts_with("Up to date") {
                "Installed automatically from GitHub Releases".to_string()
            } else {
                update_status
            };
            Frame::none()
                .fill(p.raise)
                .stroke(Stroke::new(1.0_f32, p.line))
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::symmetric(12.0, 9.0))
                .show(ui, |ui| {
                    let w = ui.available_width();
                    ui.allocate_ui_with_layout(Vec2::new(w, 40.0), Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.label(RichText::new(I_REFRESH).font(icon_font(18.0)).color(p.ac));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new("Updates").color(p.tx));
                            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).size(11.0).color(p.tx2));
                            ui.label(RichText::new(second).size(11.0).color(p.tx2));
                        });
                        // The Store version is updated by the Store.
                        if !packaged {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(egui::Button::new("Check now").min_size(Vec2::new(0.0, CONTROL_HEIGHT))).clicked() {
                                    crate::update::check_now();
                                }
                            });
                        }
                    });
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
        if crate::win::settle_startup() {
            ctx.request_repaint();
        }
        self.hide_instead_of_close(ctx);
        self.view = if store_demo() { demo_view() } else { self.shared.net() };
        apply_settings(&self.cfg, &self.attached);
        // Tell paired PCs when this PC starts or stops sharing its keyboard and mouse.
        let sharing = shares_input(&self.cfg, &self.attached);
        if self.sent_sharing != Some(sharing) {
            self.net.send(Cmd::SetSharing(sharing));
            self.sent_sharing = Some(sharing);
        }

        if self.last_scan.elapsed() >= RESCAN && !store_demo() {
            self.attached = devices::scan();
            self.last_scan = Instant::now();
        }
        // Keep the device list and diagnostics fresh while the window is open.
        ctx.request_repaint_after(Duration::from_secs(1));

        let mut needed_height = 0.0;
        egui::CentralPanel::default()
            .frame(Frame::central_panel(&ctx.style()).inner_margin(Margin::same(MARGIN)))
            // No scrolling: the window's height follows its content (see below).
            .show(ctx, |ui| {
                self.header(ui);
                ui.add_space(14.0 - ui.spacing().item_spacing.y);

                // Two equal columns that end on the same line: the shorter one's last
                // card grows by the difference measured last frame.
                let column_w = (ui.available_width() - COLUMN_GAP) / 2.0;
                let target = self.left_natural.max(self.right_natural);
                let (extra_l, extra_r) = ((target - self.left_natural).max(0.0), (target - self.right_natural).max(0.0));
                let (mut left, mut right) = (0.0, 0.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                    ui.allocate_ui_with_layout(Vec2::new(column_w, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(column_w);
                        left = self.left_column(ui, extra_l);
                    });
                    ui.allocate_ui_with_layout(Vec2::new(column_w, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(column_w);
                        right = self.right_column(ui, extra_r);
                    });
                });
                layout_log(&format!("columns: left {left:.1} right {right:.1} extra {extra_l:.1}/{extra_r:.1}"));
                if (left - self.left_natural).abs() > 0.5 || (right - self.right_natural).abs() > 0.5 {
                    self.left_natural = left;
                    self.right_natural = right;
                    ctx.request_repaint();
                }

                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).size(12.0).color(pal(ui).bad));
                }
                ui.add_space(14.0 - ui.spacing().item_spacing.y);
                footer(ui, &self.view.my_id);
                // The cursor sits just below the last widget, whatever the panel's size.
                let bottom = ui.cursor().min.y - ui.spacing().item_spacing.y;
                needed_height = (bottom + MARGIN).clamp(300.0, 1200.0);
                layout_log(&format!("window: needed {needed_height:.1}"));
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
            // Wait for a frame at the final size to be drawn, so it never appears empty.
            if (fits && self.fit_frames >= 2) || self.fit_frames > 8 {
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

/// Dev builds only: HOMEKVM_DEMO=1 fills the screen layout with sample PCs.
fn demo_layout() -> bool {
    cfg!(debug_assertions) && std::env::var_os("HOMEKVM_DEMO").is_some_and(|v| v == "1" || v == "full") || store_demo()
}

/// Dev builds only: HOMEKVM_DEMO=store shows sample PCs, settings and diagnostics
/// everywhere, for Microsoft Store screenshots (no real names or addresses);
/// HOMEKVM_DEMO=store-pair adds a pairing request.
fn store_demo() -> bool {
    cfg!(debug_assertions) && std::env::var("HOMEKVM_DEMO").is_ok_and(|v| v.starts_with("store"))
}

/// The sample network for Store screenshots.
fn demo_view() -> NetView {
    let pc = |id: &str, paired: bool, addr: &str| PeerView {
        id: id.into(),
        name: id.into(),
        paired,
        connected: paired,
        found: true,
        shares: false,
        addr: Some(addr.into()),
    };
    NetView {
        my_id: "7F3A…C21B".into(),
        peers: vec![pc("CREATORX", true, "192.168.1.24"), pc("Office PC", true, "192.168.1.31"), pc("Media PC", false, "192.168.1.40")],
        pairing: std::env::var("HOMEKVM_DEMO").is_ok_and(|v| v == "store-pair").then(|| PairingView {
            id: "Media PC".into(),
            name: "Media PC".into(),
            code: "482 913".into(),
            incoming: true,
            confirmed: false,
        }),
        ..Default::default()
    }
}

// ---- Theme ------------------------------------------------------------------------------

/// Windows' own Segoe UI (regular and semibold) when it's there, egui's fonts as the
/// fallback for other characters, and the small icon font built into the app.
fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("icons".into(), egui::FontData::from_static(include_bytes!("../assets/fonts/homekvm-icons.ttf")));
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let load = |file: &str| std::fs::read(format!(r"{windir}\Fonts\{file}")).ok();
    let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = load("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(bytes));
        proportional.insert(0, "segoe".into());
    }
    let mut strong = proportional.clone();
    if let Some(bytes) = load("seguisb.ttf") {
        fonts.font_data.insert("segoe-semibold".into(), egui::FontData::from_owned(bytes));
        strong.insert(0, "segoe-semibold".into());
    }
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name("semibold".into()), strong);
    fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".into()]);
    ctx.set_fonts(fonts);
}

/// Both themes, and following the Windows light/dark setting. HOMEKVM_THEME=light or
/// dark picks one in dev builds (for screenshots).
fn apply_theme(ctx: &Context) {
    for (theme, p) in [(egui::Theme::Dark, MIDNIGHT), (egui::Theme::Light, LIGHT)] {
        ctx.style_mut_of(theme, |s| {
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(12.0, 5.0);
            s.spacing.interact_size.y = CONTROL_HEIGHT;
            let v = &mut s.visuals;
            v.panel_fill = p.bg;
            v.window_fill = p.card;
            v.extreme_bg_color = p.bg;
            v.faint_bg_color = p.raise;
            v.window_stroke = Stroke::new(1.0_f32, p.line2);
            v.window_rounding = Rounding::same(10.0);
            v.menu_rounding = Rounding::same(10.0);
            v.selection.bg_fill = p.ac;
            v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
            v.hyperlink_color = p.ac;
            let w = &mut v.widgets;
            let look = |fill: Color32, stroke: Color32| egui::style::WidgetVisuals {
                bg_fill: fill,
                weak_bg_fill: fill,
                bg_stroke: Stroke::new(1.0_f32, stroke),
                rounding: Rounding::same(8.0),
                fg_stroke: Stroke::new(1.0_f32, p.tx),
                expansion: 0.0,
            };
            w.noninteractive = look(p.card, p.line);
            w.inactive = look(p.raise, p.line2);
            w.hovered = look(p.hov, p.tx3);
            w.active = look(p.hov, p.ac);
            w.open = look(p.hov, p.ac);
        });
    }
    let theme = match std::env::var("HOMEKVM_THEME").ok().filter(|_| cfg!(debug_assertions)).as_deref() {
        Some("light") => egui::ThemePreference::Light,
        Some("dark") => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    ctx.set_theme(theme);
}

// ---- Building blocks ----------------------------------------------------------------------

/// A card: an icon and a title on top (`right` adds to that line), then the content.
fn card<R>(ui: &mut Ui, icon: &str, title: &str, right: impl FnOnce(&mut Ui), body: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    Frame::none()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.line))
        .rounding(Rounding::same(14.0))
        .inner_margin(Margin::same(CARD_PADDING))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let w = ui.available_width();
            ui.allocate_ui_with_layout(Vec2::new(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(icon).font(icon_font(16.0)).color(p.ac));
                ui.label(RichText::new(title).font(semibold(13.0)).color(p.tx));
                right(ui);
            });
            ui.add_space(2.0);
            body(ui)
        })
        .inner
}

/// The connection status in a pill: a colored dot and the status.
fn status_pill(ui: &mut Ui, status: Status) {
    let p = pal(ui);
    let dot = match status {
        Status::Connected => p.ok,
        Status::Away => p.ac,
        Status::NotConnected => p.tx3,
        Status::Paused => p.wa,
        Status::Attention => p.bad,
    };
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(status.title().to_owned(), font, p.tx2);
    let size = Vec2::new(10.0 + 7.0 + 6.0 + galley.size().x + 10.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, 11.0, p.raise, Stroke::new(1.0_f32, p.line));
    painter.circle_filled(Pos2::new(rect.left() + 13.5, rect.center().y), 3.5, dot);
    painter.galley(Pos2::new(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0), galley, p.tx2);
}

/// The number of PCs, in a small chip.
fn count_chip(ui: &mut Ui, count: usize) {
    let p = pal(ui);
    let galley = ui.painter().layout_no_wrap(count.to_string(), FontId::proportional(11.0), p.tx2);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 18.0), Sense::hover());
    ui.painter().rect(rect, 9.0, p.raise, Stroke::new(1.0_f32, p.line));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, p.tx2);
}

/// An icon followed by text, as one piece of text for a button.
fn icon_text(icon: &str, text: &str, size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let format = |font: FontId| TextFormat { font_id: font, color, valign: Align::Center, ..Default::default() };
    job.append(icon, 0.0, format(icon_font(size + 1.0)));
    job.append(text, 6.0, format(FontId::proportional(size)));
    job
}

/// A plain button with an icon before its text.
fn icon_button(ui: &mut Ui, icon: &str, text: &str, size: f32) -> Response {
    let p = pal(ui);
    let height = if size < 12.0 { 24.0 } else { CONTROL_HEIGHT };
    ui.scope(|ui| {
        if size < 12.0 {
            ui.spacing_mut().button_padding = egui::vec2(10.0, 3.0);
            ui.spacing_mut().interact_size.y = height;
        }
        ui.add(egui::Button::new(icon_text(icon, text, size, p.tx)).min_size(Vec2::new(0.0, height)))
    })
    .inner
}

/// The main action: solid accent with white text.
fn primary_button(ui: &mut Ui, text: &str, size: Vec2) -> Response {
    let p = pal(ui);
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(p.ac).stroke(Stroke::NONE).min_size(size))
}

/// Text in the accent color that acts as a button.
fn link_button(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui);
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.ac)).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A title, a quiet line under it, and a switch on the right. Returns true when toggled.
fn setting_row(ui: &mut Ui, title: &str, detail: &str, on: &mut bool) -> bool {
    let p = pal(ui);
    let mut changed = false;
    let w = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(w, 36.0), Layout::left_to_right(Align::Center), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).color(p.tx));
            ui.label(RichText::new(detail).size(12.0).color(p.tx2));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = toggle(ui, on).changed();
        });
    });
    changed
}

/// Like `setting_row`, for the highlighted row: the quiet line is in the accent's text shade.
fn setting_row_tinted(ui: &mut Ui, title: &str, detail: &str, on: &mut bool) -> bool {
    let p = pal(ui);
    let mut changed = false;
    let w = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(w, 36.0), Layout::left_to_right(Align::Center), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(title).color(p.tx));
            ui.label(RichText::new(detail).size(12.0).color(p.act));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = toggle(ui, on).changed();
        });
    });
    changed
}

fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = pal(ui);
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let off = if response.hovered() { p.line2 } else { p.off };
    let track = if *on { p.ac } else { off };
    ui.painter().rect_filled(rect, 10.0, track);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 7.0, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A slider in the accent color: filled up to the value, a white knob with a blue ring.
fn slider(ui: &mut Ui, value: &mut u32, (min, max): (u32, u32), step: u32, width: f32) -> Response {
    let p = pal(ui);
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width.max(40.0), 20.0), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 16.0, 4.0));
    if let Some(pos) = response.interact_pointer_pos() {
        let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
        let raw = min as f32 + t * (max - min) as f32;
        let snapped = ((raw / step as f32).round() as u32 * step).clamp(min, max);
        if snapped != *value {
            *value = snapped;
            response.mark_changed();
        }
    }
    let t = (*value).saturating_sub(min) as f32 / (max - min) as f32;
    let x = track.left() + t * track.width();
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, p.off);
    painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(x, track.max.y)), 2.0, p.ac);
    painter.circle(Pos2::new(x, rect.center().y), 8.0, Color32::WHITE, Stroke::new(2.0_f32, p.ac));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A round button with an icon (paging the screens, scrolling the list).
fn round_button(ui: &mut Ui, center: Pos2, icon: &str, id: &str) -> bool {
    let p = pal(ui);
    let rect = Rect::from_center_size(center, Vec2::splat(22.0));
    let response = ui.interact(rect, ui.id().with(("round", id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    ui.painter().circle(center, 11.0, if hovered { p.hov } else { p.card }, Stroke::new(1.0_f32, if hovered { p.tx3 } else { p.line2 }));
    ui.painter().text(center, Align2::CENTER_CENTER, icon, icon_font(13.0), if hovered { p.tx } else { p.tx2 });
    response.clicked()
}

/// One screen in the layout: this PC, a placed PC, or an empty spot. Brighter under the
/// mouse; this PC is outlined in blue.
fn screen_card(ui: &mut Ui, rect: Rect, card: &Card, id: &str) {
    let p = pal(ui);
    let hovered = ui.interact(rect, ui.id().with(("screen", id)), Sense::hover()).hovered();
    let painter = ui.painter().clone();
    let c = rect.center();

    let Some(name) = &card.name else {
        dashed_rect(&painter, rect.shrink(1.0), if hovered { p.tx3 } else { p.line2 });
        painter.text(Pos2::new(c.x, c.y - 9.0), Align2::CENTER_CENTER, "+", FontId::proportional(18.0), p.tx3);
        painter.text(Pos2::new(c.x, c.y + 12.0), Align2::CENTER_CENTER, &card.sub, FontId::proportional(11.0), p.tx3);
        return;
    };

    let (fill, stroke) = if card.me {
        (p.acbg, Stroke::new(1.5_f32, if hovered { p.act } else { p.ac }))
    } else {
        (if hovered { p.hov } else { p.raise }, Stroke::new(1.0_f32, if hovered { p.tx3 } else { p.line2 }))
    };
    painter.rect(rect.shrink(0.5), 12.0, fill, stroke);

    // The device in a small tile.
    let tile = Rect::from_center_size(Pos2::new(c.x, rect.top() + 24.0), Vec2::new(38.0, 30.0));
    let (tile_fill, glyph) = if card.me { (p.acbg2, p.act) } else { (p.card, p.tx) };
    painter.rect_filled(tile, 9.0, tile_fill);
    painter.text(tile.center(), Align2::CENTER_CENTER, if card.laptop { I_LAPTOP } else { I_DESKTOP }, icon_font(22.0), glyph);

    // Long names shrink a little, then get shortened, so they never spill over the card.
    let (label, size) = fit(ui, name, rect.width() - 14.0, 13.0, semibold);
    painter.text(Pos2::new(c.x, rect.top() + 52.0), Align2::CENTER_CENTER, label, semibold(size), p.tx);
    painter.text(
        Pos2::new(c.x, rect.top() + 69.0),
        Align2::CENTER_CENTER,
        &card.sub,
        FontId::proportional(11.0),
        if card.me { p.act } else { p.tx2 },
    );
    if !card.me {
        painter.circle_filled(Pos2::new(rect.right() - 10.0, rect.top() + 10.0), 3.5, if card.online { p.ok } else { p.tx3 });
    }
    if let Some(badge) = card.badge {
        draw_badge(&painter, Pos2::new(c.x, rect.top()), badge, p);
    }
}

/// "Server" on a blue pill, centered on a screen card's top edge.
fn draw_badge(painter: &egui::Painter, center: Pos2, badge: Badge, p: Pal) {
    let text = match badge {
        Badge::Server => "Server",
        Badge::InUse => "Server · in use",
    };
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), Color32::WHITE);
    let (icon_w, pad, gap, h) = (12.0, 10.0, 4.0, 19.0);
    let w = pad + icon_w + gap + galley.size().x + pad;
    let pill = Rect::from_center_size(center, Vec2::new(w, h));
    painter.rect_filled(pill, h / 2.0, p.ac);
    painter.text(Pos2::new(pill.left() + pad + icon_w / 2.0, pill.center().y), Align2::CENTER_CENTER, I_KEYBOARD, icon_font(12.0), Color32::WHITE);
    painter.galley(Pos2::new(pill.left() + pad + icon_w + gap, pill.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
}

fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for pair in corners.windows(2) {
        p.extend(egui::Shape::dashed_line(pair, Stroke::new(1.0_f32, color), 5.0, 4.0));
    }
}

/// A trash can that shows when the row is under the mouse; red when pointed at.
fn trash_button(ui: &mut Ui, row_hovered: bool) -> bool {
    let p = pal(ui);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CONTROL_HEIGHT), Sense::click());
    if !row_hovered && !response.hovered() {
        return false;
    }
    let response = response.on_hover_text("Forget this PC").on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if response.hovered() {
        ui.painter().rect(rect, 8.0, p.bad.gamma_multiply(0.15), Stroke::new(1.0_f32, p.bad.gamma_multiply(0.7)));
        p.bad
    } else {
        p.tx2
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, I_TRASH, icon_font(16.0), color);
    response.clicked()
}

/// The version and this PC's ID on the left, and the GitHub badge on the right.
fn footer(ui: &mut Ui, my_id: &str) {
    let p = pal(ui);
    let w = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(w, 40.0), Layout::left_to_right(Align::Center), |ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(format!("HomeKVM {} · encrypted, paired PCs only", env!("CARGO_PKG_VERSION"))).size(12.0).color(p.tx3),
            );
            let mut job = LayoutJob::default();
            job.append("This PC's ID: ", 0.0, TextFormat { font_id: FontId::proportional(12.0), color: p.tx3, ..Default::default() });
            job.append(my_id, 0.0, TextFormat { font_id: FontId::monospace(11.5), color: p.tx2, ..Default::default() });
            ui.label(job);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| about_badge(ui));
    });
}

const REPO_URL: &str = "https://github.com/saugatadatta/HomeKVM";

/// The GitHub mark, "by Saugata Datta" over "View on GitHub ↗"; opens the repo.
fn about_badge(ui: &mut Ui) {
    let p = pal(ui);
    let font = FontId::proportional(11.5);
    let top = ui.painter().layout_no_wrap("by Saugata Datta".to_owned(), font.clone(), p.tx2);
    let bottom = ui.painter().layout_no_wrap("View on GitHub ↗".to_owned(), font, p.ac);
    let (pad, icon, gap) = (12.0, 18.0, 8.0);
    let width = pad + icon + gap + top.size().x.max(bottom.size().x) + pad;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
    let response = response.on_hover_text(REPO_URL).on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(REPO_URL));
    }
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect(rect, 10.0, p.card, Stroke::new(1.0_f32, if hovered { p.ac } else { p.line2 }));
    painter.text(Pos2::new(rect.left() + pad + icon / 2.0, rect.center().y), Align2::CENTER_CENTER, I_GITHUB, icon_font(18.0), p.tx);
    let x = rect.left() + pad + icon + gap;
    let line_h = top.size().y;
    painter.galley(Pos2::new(x, rect.center().y - line_h), top, p.tx2);
    painter.galley(Pos2::new(x, rect.center().y), bottom, p.ac);
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

/// The largest size (from `max` down to 11) at which `s` fits in `max_width`. If it
/// doesn't fit even at 11, it is shortened with "…".
fn fit(ui: &Ui, s: &str, max_width: f32, max: f32, font: fn(f32) -> FontId) -> (String, f32) {
    let width = |t: &str, size: f32| ui.fonts(|f| f.layout_no_wrap(t.to_owned(), font(size), Color32::WHITE).size().x);
    let mut size = max;
    while size >= 11.0 {
        if width(s, size) <= max_width {
            return (s.to_owned(), size);
        }
        size -= 0.5;
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

// ---- Diagnostics --------------------------------------------------------------------------

/// The Diagnostics lines: health, label and what happened.
fn diagnostics(view: &NetView) -> Vec<(Health, &'static str, String)> {
    if store_demo() {
        return vec![
            (Health::Ok, "Hooks", "keyboard on · mouse on".into()),
            (Health::Ok, "Keys", "captured 1,284 · sent 1,284 · received 0 · typed 0".into()),
            (Health::Ok, "Copy sent", "text 2.1 KB · 3 min ago".into()),
            (Health::Ok, "Copy got", "image 1920×1080 · just now".into()),
            (Health::Ok, "Files", "pasted 4 (86.2 MB) · sent 2 (12.4 MB) · 9 min ago".into()),
            (Health::Ok, "Link", "CREATORX, Office PC · encrypted".into()),
        ];
    }
    let (hooks_h, hooks) = crate::input::diag::hooks_row();
    let (keys_h, keys) = crate::input::diag::keys_row();
    let (sent_h, sent) = activity::copy_sent_row();
    let (got_h, got) = activity::copy_got_row();
    let (files_h, files) = activity::files_row();
    let (link_h, link) = link_row(view);
    vec![
        (hooks_h, "Hooks", hooks),
        (keys_h, "Keys", keys),
        (sent_h, "Copy sent", sent),
        (got_h, "Copy got", got),
        (files_h, "Files", files),
        (link_h, "Link", link),
    ]
}

/// Which paired PCs are connected right now.
fn link_row(view: &NetView) -> (Health, String) {
    let paired: Vec<&PeerView> = view.peers.iter().filter(|p| p.paired).collect();
    let connected: Vec<String> = paired
        .iter()
        .filter(|p| p.connected)
        .map(|p| match &p.addr {
            Some(a) => format!("{} · {a}", p.name),
            None => p.name.clone(),
        })
        .collect();
    if paired.is_empty() {
        (Health::Idle, "no paired PC yet".into())
    } else if !connected.is_empty() {
        (Health::Ok, format!("{} · encrypted", connected.join(", ")))
    } else if paired.iter().any(|p| p.found) {
        (Health::Waiting, "reconnecting…".into())
    } else {
        (Health::Waiting, format!("{} paired · offline", paired.len()))
    }
}

/// Plain text for "Copy report".
fn report(rows: &[(Health, &'static str, String)]) -> String {
    let mut out = format!("HomeKVM {} diagnostics\n", env!("CARGO_PKG_VERSION"));
    for (health, label, value) in rows {
        out.push_str(&format!("{label}: {} · {value}\n", health.word()));
    }
    out
}

/// One Diagnostics line: a colored dot, the label, and the value (shortened to fit; the
/// whole value shows when pointed at).
fn diag_row(ui: &mut Ui, health: Health, label: &str, value: &str) {
    let p = pal(ui);
    let w = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(w, 17.0), Sense::hover());
    let dot = match health {
        Health::Ok => p.ok,
        Health::Waiting => p.wa,
        Health::Failed => p.bad,
        Health::Idle => p.tx3,
    };
    let label_w = 74.0;
    let x = rect.left() + 14.0 + label_w;
    let (shown, size) = fit(ui, value, rect.right() - x, 12.0, FontId::proportional);
    let painter = ui.painter();
    let y = rect.center().y;
    painter.circle_filled(Pos2::new(rect.left() + 3.5, y), 3.5, dot);
    painter.text(Pos2::new(rect.left() + 14.0, y), Align2::LEFT_CENTER, label, FontId::proportional(12.0), p.tx2);
    let color = if health == Health::Failed { p.bad } else { p.tx };
    painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, &shown, FontId::proportional(size), color);
    if shown != value {
        response.on_hover_text(value);
    }
}

/// Dev only: with HOMEKVM_LAYOUT_LOG=1, prints layout measurements when they change.
fn layout_log(line: &str) {
    use std::sync::Mutex;
    static LAST: Mutex<Vec<String>> = Mutex::new(Vec::new());
    if !cfg!(debug_assertions) || std::env::var_os("HOMEKVM_LAYOUT_LOG").is_none() {
        return;
    }
    let key = line.split(':').next().unwrap_or("").to_string();
    let mut last = LAST.lock().unwrap();
    if let Some(old) = last.iter_mut().find(|l| l.split(':').next() == Some(key.as_str())) {
        if old == line {
            return;
        }
        *old = line.to_string();
    } else {
        last.push(line.to_string());
    }
    eprintln!("[layout] {line}");
}
