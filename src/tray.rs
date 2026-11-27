//! Tray icon with a status dot, and a right-click menu that HomeKVM draws itself
//! (owner-drawn Win32 menu) so it looks the same in light and dark mode:
//!
//!   HomeKVM                                  (bold; opens the window)
//!   Not connected · no paired PC online yet  (muted)
//!   ─────────────────────────────
//!   [icon] Open HomeKVM
//!   [icon] Pause sharing / Resume sharing
//!   [icon] Bring mouse back here    Ctrl+Alt+Home
//!   ─────────────────────────────
//!   [dot]  <paired PC>              <side>   (once pairing exists)
//!   ─────────────────────────────
//!   [icon] Quit
//!
//! It runs on its own thread with its own message loop, so it works while the
//! main window is hidden.

use crate::menu_icons::{self, Glyph};
use crate::config::Side;
use crate::state::{Shared, Status};
use crate::{icon, win};
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, SelectObject, SetBkMode, SetTextColor, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, HDC, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_SELECTED, ODT_MENU};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, GetCursorPos,
    GetMessageW, PeekMessageW, PostMessageW, PostThreadMessageW, RegisterClassW, SetForegroundWindow, SetMenuInfo,
    TrackPopupMenuEx, TranslateMessage, MENUINFO, MF_DISABLED, MF_OWNERDRAW, MIM_BACKGROUND, MSG, PM_NOREMOVE,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_DRAWITEM, WM_MEASUREITEM, WM_NULL, WM_USER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

const CMD_OPEN: u32 = 1;
const CMD_PAUSE: u32 = 2;
const CMD_HOME: u32 = 3;
const CMD_QUIT: u32 = 4;

/// Lets the window ask the tray to redraw after the status changes.
#[derive(Clone)]
pub struct TrayHandle {
    tx: Sender<()>,
    thread_id: u32,
}

impl TrayHandle {
    pub fn refresh(&self) {
        let _ = self.tx.send(());
        unsafe {
            PostThreadMessageW(self.thread_id, WM_APP, 0, 0);
        }
    }
}

pub fn spawn(shared: Arc<Shared>) -> TrayHandle {
    let (tx, rx) = channel();
    let (id_tx, id_rx) = channel();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(shared, rx, id_tx))
        .expect("start tray thread");
    let thread_id = id_rx.recv().unwrap_or(0);
    TrayHandle { tx, thread_id }
}

fn run(shared: Arc<Shared>, rx: Receiver<()>, id_tx: Sender<u32>) {
    // Sharp menu text and icons at every display scaling.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let owner = unsafe { create_owner_window() };

    let status = shared.status();
    let tray = TrayIconBuilder::new()
        .with_icon(tray_icon_for(status))
        .with_tooltip(tooltip(&shared))
        .build()
        .expect("create tray icon");

    // Make sure this thread has a message queue before handing out its id.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        let _ = id_tx.send(GetCurrentThreadId());
    }

    loop {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            if GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let mut changed = false;
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => win::show_main_window(),
                TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Up, .. } => {
                    match unsafe { show_menu(owner, &shared) } {
                        CMD_OPEN => win::show_main_window(),
                        CMD_PAUSE => {
                            shared.set_paused(!shared.paused());
                            changed = true;
                            win::request_repaint();
                        }
                        // Mouse sharing arrives in phase 3; until then the mouse is always here.
                        CMD_HOME => {}
                        CMD_QUIT => {
                            // Remove the icon now rather than leaving a ghost in the tray.
                            drop(tray);
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        while rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            let status = shared.status();
            let _ = tray.set_icon(Some(tray_icon_for(status)));
            let _ = tray.set_tooltip(Some(tooltip(&shared)));
        }
    }
}

/// "HomeKVM — connected to Work laptop, Mini PC" or "HomeKVM — not connected".
fn tooltip(shared: &Shared) -> String {
    let status = shared.status();
    let names: Vec<String> =
        shared.net().peers.into_iter().filter(|p| p.paired && p.connected).map(|p| p.name).collect();
    let text = if status == Status::Connected && !names.is_empty() {
        format!("HomeKVM — connected to {}", names.join(", "))
    } else {
        format!("HomeKVM — {}", status.title().to_lowercase())
    };
    // Windows cuts tray tooltips at 127 characters.
    text.chars().take(120).collect()
}

fn tray_icon_for(status: Status) -> Icon {
    Icon::from_rgba(icon::rgba(status), icon::SIZE, icon::SIZE).expect("valid icon")
}

// ---- The menu --------------------------------------------------------------

enum Row {
    Header,
    Status(String),
    Separator,
    Action { glyph: Glyph, text: &'static str, shortcut: &'static str },
    /// A paired PC with its status dot and side.
    Peer { dot: [u8; 3], name: String, side: String },
}

impl Row {
    /// Height at 100% scaling.
    fn height(&self) -> i32 {
        match self {
            Row::Header => 30,
            Row::Status(_) => 22,
            Row::Separator => 11,
            Row::Action { .. } => 34,
            Row::Peer { .. } => 32,
        }
    }

    fn clickable(&self) -> bool {
        matches!(self, Row::Header | Row::Action { .. } | Row::Peer { .. })
    }
}

/// Menu width at 100% scaling.
const WIDTH: i32 = 270;

#[derive(Clone, Copy)]
struct Theme {
    bg: [u8; 3],
    hover: [u8; 3],
    text: [u8; 3],
    muted: [u8; 3],
    line: [u8; 3],
}

const DARK: Theme = Theme {
    bg: [44, 44, 44],
    hover: [64, 64, 64],
    text: [243, 243, 243],
    muted: [160, 160, 160],
    line: [74, 74, 74],
};

const LIGHT: Theme = Theme {
    bg: [249, 249, 249],
    hover: [233, 233, 233],
    text: [26, 26, 26],
    muted: [110, 110, 110],
    line: [224, 224, 224],
};

thread_local! {
    static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    static THEME: Cell<Theme> = const { Cell::new(LIGHT) };
    static DPI: Cell<i32> = const { Cell::new(96) };
}

/// Scale a 100% size to the current display scaling.
fn px(v: i32) -> i32 {
    v * DPI.get() / 96
}

fn colorref(c: [u8; 3]) -> u32 {
    c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Show the menu at the cursor and return the chosen command (0 if dismissed).
unsafe fn show_menu(owner: HWND, shared: &Shared) -> u32 {
    let status = shared.status();
    let paused = status == Status::Paused;

    // Paired PCs, each with its dot and where it sits. Clicking one opens the window.
    let layout = shared.layout();
    let mut paired: Vec<_> = shared.net().peers.into_iter().filter(|p| p.paired).collect();
    paired.sort_by_key(|p| (!p.connected, p.name.to_lowercase()));
    let connected = paired.iter().filter(|p| p.connected).count();
    let detail = if status == Status::Connected {
        format!("{connected} PC{} linked, mouse is here", if connected == 1 { "" } else { "s" })
    } else {
        status.detail().to_string()
    };
    let peer_rows: Vec<(Row, u32)> = paired
        .into_iter()
        .map(|p| {
            let side = match layout.side_of(&p.id) {
                Side::Left => "left",
                Side::Right => "right",
                Side::NotPlaced => "not placed",
            };
            let (dot, side) = if p.connected {
                (Status::Connected.rgb(), side.to_string())
            } else {
                (Status::NotConnected.rgb(), "offline".to_string())
            };
            (Row::Peer { dot, name: p.name, side }, CMD_OPEN)
        })
        .collect();

    let mut rows: Vec<(Row, u32)> = vec![
        (Row::Header, CMD_OPEN),
        (Row::Status(format!("{} · {}", status.title(), detail)), 0),
        (Row::Separator, 0),
        (Row::Action { glyph: Glyph::Open, text: "Open HomeKVM", shortcut: "" }, CMD_OPEN),
        (
            Row::Action {
                glyph: if paused { Glyph::Resume } else { Glyph::Pause },
                text: if paused { "Resume sharing" } else { "Pause sharing" },
                shortcut: "",
            },
            CMD_PAUSE,
        ),
        (Row::Action { glyph: Glyph::Home, text: "Bring mouse back here", shortcut: "Ctrl+Alt+Home" }, CMD_HOME),
        (Row::Separator, 0),
    ];
    if !peer_rows.is_empty() {
        rows.extend(peer_rows);
        rows.push((Row::Separator, 0));
    }
    rows.push((Row::Action { glyph: Glyph::Power, text: "Quit", shortcut: "" }, CMD_QUIT));

    DPI.set(GetDpiForWindow(owner).max(96) as i32);
    let theme = if menu_icons::dark_mode() { DARK } else { LIGHT };
    THEME.set(theme);

    let menu = CreatePopupMenu();
    for (i, (row, cmd)) in rows.iter().enumerate() {
        let flags = MF_OWNERDRAW | if row.clickable() { 0 } else { MF_DISABLED };
        // For owner-drawn items the last argument carries our row index.
        AppendMenuW(menu, flags, *cmd as usize, i as *const u16);
    }
    ROWS.with(|r| *r.borrow_mut() = rows.into_iter().map(|(row, _)| row).collect());

    // Paint the menu's own padding in our background color too.
    let brush = CreateSolidBrush(colorref(theme.bg));
    let mut info: MENUINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MENUINFO>() as u32;
    info.fMask = MIM_BACKGROUND;
    info.hbrBack = brush;
    SetMenuInfo(menu, &info);

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    // Needed so the menu closes when you click elsewhere.
    SetForegroundWindow(owner);
    let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_NONOTIFY;
    let cmd = TrackPopupMenuEx(menu, flags, pt.x, pt.y, owner, std::ptr::null());
    PostMessageW(owner, WM_NULL, 0, 0);

    DestroyMenu(menu);
    DeleteObject(brush);
    cmd as u32
}

/// A hidden window that owns the menu and draws its rows.
unsafe fn create_owner_window() -> HWND {
    let class = wide("HomeKVM.TrayMenu");
    let instance = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(owner_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
    };
    RegisterClassW(&wc);
    CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class.as_ptr(),
        class.as_ptr(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    )
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MEASUREITEM => {
            let m = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            if m.CtlType == ODT_MENU {
                let h = ROWS.with(|r| r.borrow().get(m.itemData).map(Row::height)).unwrap_or(24);
                m.itemWidth = px(WIDTH) as u32;
                m.itemHeight = px(h) as u32;
                return 1;
            }
        }
        WM_DRAWITEM => {
            let d = &*(lparam as *const DRAWITEMSTRUCT);
            if d.CtlType == ODT_MENU {
                ROWS.with(|r| {
                    if let Some(row) = r.borrow().get(d.itemData) {
                        draw_row(d, row);
                    }
                });
                return 1;
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn draw_row(d: &DRAWITEMSTRUCT, row: &Row) {
    let t = THEME.get();
    let r = d.rcItem;
    let (w, h) = ((r.right - r.left).max(1) as usize, (r.bottom - r.top).max(1) as usize);
    let selected = row.clickable() && d.itemState & ODS_SELECTED != 0;

    // Background, hover highlight, separator line and icon as one image.
    let mut img = Image::new(w, h, t.bg);
    if selected {
        img.rounded(px(5) as f32, px(1) as f32, px(6) as f32, t.hover);
    }
    let icon_x = px(14) as usize;
    let text_x = px(44);
    match row {
        Row::Separator => img.hline(h / 2, px(1).max(1) as usize, t.line),
        Row::Action { glyph, .. } => img.icon(*glyph, icon_x, t.text),
        Row::Peer { dot, .. } => img.icon(Glyph::Dot(*dot), icon_x, t.text),
        _ => {}
    }
    img.blit(d.hDC, r.left, r.top);

    // Text on top.
    let left = |x: i32| RECT { left: r.left + x, top: r.top, right: r.right - px(14), bottom: r.bottom };
    match row {
        Row::Header => text(d.hDC, "HomeKVM", left(px(14)), t.text, px(14), 600, DT_LEFT),
        Row::Status(s) => {
            // Sit a little higher, close under the title.
            let mut rc = left(px(14));
            rc.bottom -= px(4);
            text(d.hDC, s, rc, t.muted, px(12), 400, DT_LEFT)
        }
        Row::Action { text: label, shortcut, .. } => {
            text(d.hDC, label, left(text_x), t.text, px(13), 400, DT_LEFT);
            if !shortcut.is_empty() {
                text(d.hDC, shortcut, left(text_x), t.muted, px(12), 400, DT_RIGHT);
            }
        }
        Row::Peer { name, side, .. } => {
            text(d.hDC, name, left(text_x), t.text, px(13), 400, DT_LEFT);
            text(d.hDC, side, left(text_x), t.muted, px(12), 400, DT_RIGHT);
        }
        Row::Separator => {}
    }
}

unsafe fn text(hdc: HDC, s: &str, mut rc: RECT, color: [u8; 3], size: i32, weight: i32, align: u32) {
    let face = wide("Segoe UI");
    // Default charset, ClearType quality.
    let font = CreateFontW(-size, 0, 0, 0, weight, 0, 0, 0, 1 as _, 0 as _, 0 as _, 5 as _, 0 as _, face.as_ptr());
    let old = SelectObject(hdc, font as _);
    SetBkMode(hdc, TRANSPARENT as _);
    SetTextColor(hdc, colorref(color));
    let w: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(
        hdc,
        w.as_ptr() as _,
        w.len() as i32,
        &mut rc,
        align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
    );
    SelectObject(hdc, old);
    DeleteObject(font as _);
}

/// A small BGRA image for one menu row.
struct Image {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Image {
    fn new(w: usize, h: usize, bg: [u8; 3]) -> Self {
        let mut px = vec![0u8; w * h * 4];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[bg[2], bg[1], bg[0], 255]);
        }
        Self { w, h, px }
    }

    fn blend(&mut self, x: usize, y: usize, c: [u8; 3], a: f32) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let i = (y * self.w + x) * 4;
        for (k, v) in [c[2], c[1], c[0]].into_iter().enumerate() {
            self.px[i + k] = (v as f32 * a + self.px[i + k] as f32 * (1.0 - a)).round() as u8;
        }
    }

    /// Anti-aliased rounded rectangle, inset from the row edges.
    fn rounded(&mut self, inset_x: f32, inset_y: f32, radius: f32, c: [u8; 3]) {
        let (a, b) = ((inset_x, inset_y), (self.w as f32 - inset_x, self.h as f32 - inset_y));
        for y in 0..self.h {
            for x in 0..self.w {
                let d = menu_icons::rrect(x as f32 + 0.5, y as f32 + 0.5, a, b, radius);
                self.blend(x, y, c, (0.5 - d).clamp(0.0, 1.0));
            }
        }
    }

    fn hline(&mut self, y: usize, thickness: usize, c: [u8; 3]) {
        for yy in y..y + thickness {
            for x in 0..self.w {
                self.blend(x, yy, c, 1.0);
            }
        }
    }

    /// An icon 16 px tall at 100% scaling, vertically centered.
    fn icon(&mut self, glyph: Glyph, x0: usize, ink: [u8; 3]) {
        let n = px(16) as usize;
        let y0 = self.h.saturating_sub(n) / 2;
        let rgba = menu_icons::rgba(glyph, n, ink);
        for y in 0..n {
            for x in 0..n {
                let i = (y * n + x) * 4;
                let a = rgba[i + 3] as f32 / 255.0;
                self.blend(x0 + x, y0 + y, [rgba[i], rgba[i + 1], rgba[i + 2]], a);
            }
        }
    }

    unsafe fn blit(&self, hdc: HDC, x: i32, y: i32) {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: self.w as i32,
            biHeight: -(self.h as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB as _,
            ..std::mem::zeroed()
        };
        StretchDIBits(
            hdc,
            x,
            y,
            self.w as i32,
            self.h as i32,
            0,
            0,
            self.w as i32,
            self.h as i32,
            self.px.as_ptr().cast(),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}
