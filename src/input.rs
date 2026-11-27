//! Keyboard and mouse sharing.
//!
//! On the PC you sit at (the controller), low-level keyboard and mouse hooks watch
//! the real input. When the cursor reaches a screen edge with a paired PC placed on
//! that side, the network side calls [`capture`]: from then on the hooks swallow the
//! input locally and report it (moves as deltas, buttons, wheel, keys by scan code)
//! to be sent to the other PC. The local cursor waits, hidden, in the middle of the
//! screen. [`release`] gives input back and puts the cursor at the edge it returns
//! through.
//!
//! On the PC being controlled (the target), the received input is replayed with
//! `SendInput`, with the cursor kept in absolute coordinates so pointer acceleration
//! isn't applied twice. Pushing past an edge is reported back so the controller can
//! take the mouse home or hand it to the next PC in the row.
//!
//! Safety nets: Ctrl+Alt+Home always brings the mouse back; a dropped connection
//! releases everything; real input on a controlled PC takes it back; keys held on the
//! target are released when control leaves; no edge switching while a mouse button
//! is held.
//!
//! All of this is the `input` build feature. Builds without it (for PCs where global
//! input hooks aren't allowed) get the no-op versions at the bottom.

use tokio::sync::mpsc::UnboundedSender;

/// What the local hooks report to the network thread.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(feature = "input"), allow(dead_code))]
pub enum Local {
    /// The cursor reached a screen edge; `y` is the position along it (0..1).
    EdgePush { edge: crate::proto::Edge, y: f32 },
    Move { dx: i32, dy: i32 },
    Button { b: u8, down: bool },
    Wheel { d: i32, h: bool },
    Key { vk: u16, sc: u16, ext: bool, down: bool },
    /// Ctrl+Alt+Home, "Bring mouse back here", or Pause.
    Release,
    /// This PC's own keyboard or mouse was used while another PC controlled it.
    Activity,
}

#[cfg(feature = "input")]
pub use real::*;
#[cfg(not(feature = "input"))]
pub use stub::*;

/// Ask the network side to bring the mouse back to this PC.
pub fn request_release() {
    send(Local::Release);
}

static TX: std::sync::OnceLock<UnboundedSender<Local>> = std::sync::OnceLock::new();

/// Counters for Settings > Advanced > Diagnostics: they show where keyboard input
/// stops (never captured, never sent, never received, or not typed).
pub mod diag {
    use crate::activity::Health;
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering::Relaxed};

    /// Hook status: 0 off, 1 on, negative = the Windows error code when installing failed.
    pub static KEYBOARD_HOOK: AtomicI64 = AtomicI64::new(0);
    pub static MOUSE_HOOK: AtomicI64 = AtomicI64::new(0);
    static CAPTURED: AtomicU64 = AtomicU64::new(0);
    static SENT: AtomicU64 = AtomicU64::new(0);
    static RECEIVED: AtomicU64 = AtomicU64::new(0);
    static IGNORED: AtomicU64 = AtomicU64::new(0);
    static TYPED: AtomicU64 = AtomicU64::new(0);
    static FAILED: AtomicU64 = AtomicU64::new(0);
    static LAST_ERROR: AtomicU64 = AtomicU64::new(0);

    #[cfg_attr(not(feature = "input"), allow(dead_code))]
    pub fn key_captured() {
        CAPTURED.fetch_add(1, Relaxed);
    }
    pub fn key_sent() {
        SENT.fetch_add(1, Relaxed);
    }
    pub fn key_received() {
        RECEIVED.fetch_add(1, Relaxed);
    }
    pub fn key_ignored() {
        IGNORED.fetch_add(1, Relaxed);
    }
    #[cfg_attr(not(feature = "input"), allow(dead_code))]
    pub fn key_typed(ok: bool, error: u32) {
        if ok {
            TYPED.fetch_add(1, Relaxed);
        } else {
            FAILED.fetch_add(1, Relaxed);
            LAST_ERROR.store(error as u64, Relaxed);
        }
    }

    fn hook(name: &str, value: i64) -> String {
        match value {
            1 => format!("{name} on"),
            0 => format!("{name} off"),
            e => format!("{name} failed ({})", -e),
        }
    }

    /// The Hooks line: both on is ok, a failed install is a failure, off is idle.
    pub fn hooks_row() -> (Health, String) {
        let (k, m) = (KEYBOARD_HOOK.load(Relaxed), MOUSE_HOOK.load(Relaxed));
        let health = if k < 0 || m < 0 {
            Health::Failed
        } else if k == 1 && m == 1 {
            Health::Ok
        } else {
            Health::Idle
        };
        (health, format!("{} · {}", hook("keyboard", k), hook("mouse", m)))
    }

    /// The Keys line: where keyboard input stops.
    pub fn keys_row() -> (Health, String) {
        let counts = [CAPTURED.load(Relaxed), SENT.load(Relaxed), RECEIVED.load(Relaxed), TYPED.load(Relaxed)];
        let mut keys = format!(
            "captured {} · sent {} · received {} · typed {}",
            CAPTURED.load(Relaxed),
            SENT.load(Relaxed),
            RECEIVED.load(Relaxed),
            TYPED.load(Relaxed)
        );
        let failed = FAILED.load(Relaxed);
        if failed > 0 {
            keys.push_str(&format!(" · failed {failed} (error {})", LAST_ERROR.load(Relaxed)));
        }
        let ignored = IGNORED.load(Relaxed);
        if ignored > 0 {
            keys.push_str(&format!(" · ignored {ignored}"));
        }
        let health = if failed > 0 {
            Health::Failed
        } else if counts.iter().all(|&c| c == 0) {
            Health::Idle
        } else {
            Health::Ok
        };
        (health, keys)
    }
}

fn send(ev: Local) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(ev);
    }
}

#[cfg(feature = "input")]
mod real {
    use super::{send, Local, TX};
    use crate::proto::Edge;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::UnboundedSender;
    use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
        MOUSEINPUT, VK_CONTROL, VK_HOME, VK_LBUTTON, VK_LCONTROL, VK_LMENU, VK_MBUTTON, VK_MENU, VK_RBUTTON,
        VK_RCONTROL, VK_RMENU, VK_XBUTTON1, VK_XBUTTON2,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CreateCursor, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW,
        GetSystemMetrics, PeekMessageW, PostThreadMessageW, RegisterClassW, SetCursorPos, SetLayeredWindowAttributes,
        SetWindowPos, SetWindowsHookExW, ShowWindow, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK,
        HWND_TOPMOST, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, LWA_ALPHA, MSG,
        MSLLHOOKSTRUCT, PM_NOREMOVE, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WH_KEYBOARD_LL,
        WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
        WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_USER, WM_XBUTTONDOWN,
        WM_XBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    const WM_INSTALL: u32 = WM_APP + 1;
    const WM_UNINSTALL: u32 = WM_APP + 2;
    const WM_CAPTURE: u32 = WM_APP + 3;
    const WM_RELEASE: u32 = WM_APP + 4;

    /// Ignore the top and bottom few pixels of an edge, so corner buttons stay usable.
    const CORNER: i32 = 6;
    /// After a switch, ignore the edge briefly so the mouse doesn't bounce back.
    const COOLDOWN: Duration = Duration::from_millis(250);

    static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
    /// "Share my keyboard and mouse" is on: hooks installed.
    static ENABLED: AtomicBool = AtomicBool::new(false);
    /// This PC's input goes to another PC right now.
    static CAPTURED: AtomicBool = AtomicBool::new(false);
    /// Another PC controls this one right now.
    static CONTROLLED: AtomicBool = AtomicBool::new(false);
    static EDGE_DELAY: AtomicBool = AtomicBool::new(false);

    pub fn available() -> bool {
        true
    }

    pub fn start(tx: UnboundedSender<Local>) {
        let _ = TX.set(tx);
        let (id_tx, id_rx) = std::sync::mpsc::channel();
        if std::thread::Builder::new().name("input".into()).spawn(move || hook_thread(id_tx)).is_err() {
            return;
        }
        if let Ok(id) = id_rx.recv() {
            HOOK_THREAD.store(id, SeqCst);
        }
        if ENABLED.load(SeqCst) {
            post(WM_INSTALL, 0, 0);
        }
    }

    /// Install or remove the hooks ("Share my keyboard and mouse").
    pub fn set_enabled(on: bool) {
        if ENABLED.swap(on, SeqCst) != on {
            post(if on { WM_INSTALL } else { WM_UNINSTALL }, 0, 0);
        }
    }

    pub fn set_edge_delay(on: bool) {
        EDGE_DELAY.store(on, SeqCst);
    }

    /// Controller: send this PC's input to another PC from now on.
    pub fn capture() {
        CAPTURED.store(true, SeqCst);
        post(WM_CAPTURE, 0, 0);
    }

    /// Controller: input stays here again. With `place`, the cursor appears at that edge.
    pub fn release(place: Option<(Edge, f32)>) {
        CAPTURED.store(false, SeqCst);
        let (side, y) = match place {
            Some((Edge::Left, y)) => (1, y),
            Some((Edge::Right, y)) => (2, y),
            Some((Edge::Top, y)) => (3, y),
            Some((Edge::Bottom, y)) => (4, y),
            None => (0, 0.0),
        };
        post(WM_RELEASE, side, (y.clamp(0.0, 1.0) * 1_000_000.0) as isize);
    }

    /// Target: another PC controls this one (real local input then takes it back).
    pub fn set_controlled(on: bool) {
        CONTROLLED.store(on, SeqCst);
    }

    fn post(msg: u32, wparam: usize, lparam: isize) {
        let id = HOOK_THREAD.load(SeqCst);
        if id != 0 {
            unsafe {
                PostThreadMessageW(id, msg, wparam, lparam);
            }
        }
    }

    /// The whole desktop across all monitors, in physical pixels: (left, top, width, height).
    /// Callers' threads are per-monitor DPI aware.
    fn desktop() -> (i32, i32, i32, i32) {
        unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2),
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2),
            )
        }
    }

    fn any_button_down() -> bool {
        [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
            .iter()
            .any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0)
    }

    // ---- Controller side: the hook thread --------------------------------------

    #[derive(Default)]
    struct Hooks {
        mouse: usize,
        keys: usize,
        /// A tiny transparent window that hides the parked cursor.
        overlay: usize,
        edge_since: Option<(Edge, Instant)>,
        cooldown: Option<Instant>,
        ctrl: bool,
        alt: bool,
        /// Keys pressed here before control moved away; their release must reach this PC.
        local_keys: HashSet<u32>,
    }

    thread_local! {
        static HOOKS: RefCell<Hooks> = RefCell::new(Hooks::default());
    }

    fn hook_thread(id_tx: std::sync::mpsc::Sender<u32>) {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let mut msg: MSG = std::mem::zeroed();
            PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
            let _ = id_tx.send(GetCurrentThreadId());
            let overlay = create_overlay() as usize;
            HOOKS.with(|h| h.borrow_mut().overlay = overlay);

            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                match msg.message {
                    WM_INSTALL => install(),
                    WM_UNINSTALL => uninstall(),
                    WM_CAPTURE => park(),
                    WM_RELEASE => unpark(msg.wParam, msg.lParam),
                    _ => {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
    }

    unsafe fn install() {
        let module = GetModuleHandleW(std::ptr::null());
        HOOKS.with(|h| {
            let mut h = h.borrow_mut();
            if h.mouse == 0 {
                h.mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) as usize;
                let status = if h.mouse != 0 { 1 } else { -(GetLastError() as i64).max(1) };
                super::diag::MOUSE_HOOK.store(status, SeqCst);
            }
            if h.keys == 0 {
                h.keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), module, 0) as usize;
                let status = if h.keys != 0 { 1 } else { -(GetLastError() as i64).max(1) };
                super::diag::KEYBOARD_HOOK.store(status, SeqCst);
            }
        });
    }

    unsafe fn uninstall() {
        HOOKS.with(|h| {
            let mut guard = h.borrow_mut();
            let h = &mut *guard;
            for hook in [&mut h.mouse, &mut h.keys] {
                if *hook != 0 {
                    UnhookWindowsHookEx(*hook as HHOOK);
                    *hook = 0;
                }
            }
        });
        super::diag::KEYBOARD_HOOK.store(0, SeqCst);
        super::diag::MOUSE_HOOK.store(0, SeqCst);
        if CAPTURED.load(SeqCst) {
            send(Local::Release);
        }
    }

    /// Park the cursor in the middle of the main screen, hidden, while input goes elsewhere.
    unsafe fn park() {
        let (cx, cy) = (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2);
        let overlay = HOOKS.with(|h| h.borrow().overlay) as HWND;
        if !overlay.is_null() {
            SetWindowPos(overlay, HWND_TOPMOST, cx - 32, cy - 32, 64, 64, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        SetCursorPos(cx, cy);
    }

    unsafe fn unpark(side: usize, y: isize) {
        let overlay = HOOKS.with(|h| h.borrow().overlay) as HWND;
        if !overlay.is_null() {
            ShowWindow(overlay, SW_HIDE);
        }
        if side != 0 {
            let (l, t, w, h) = desktop();
            let along = y as f32 / 1_000_000.0;
            let (x, y) = match side {
                1 => (l + 1, t + (along * (h - 1) as f32) as i32),
                2 => (l + w - 2, t + (along * (h - 1) as f32) as i32),
                3 => (l + (along * (w - 1) as f32) as i32, t + 1),
                _ => (l + (along * (w - 1) as f32) as i32, t + h - 2),
            };
            SetCursorPos(x, y);
        }
        HOOKS.with(|h| h.borrow_mut().cooldown = Some(Instant::now() + COOLDOWN));
    }

    /// A 64 x 64, almost fully transparent, always-on-top window with a blank cursor.
    /// Sitting under the parked cursor, it makes the cursor invisible.
    unsafe fn create_overlay() -> HWND {
        let module = GetModuleHandleW(std::ptr::null());
        let and_mask = [0xFFu8; 128];
        let xor_mask = [0u8; 128];
        let blank = CreateCursor(module, 0, 0, 32, 32, and_mask.as_ptr().cast(), xor_mask.as_ptr().cast());
        let class: Vec<u16> = "HomeKVM.HiddenCursor\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module,
            hIcon: std::ptr::null_mut(),
            hCursor: blank,
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            64,
            64,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        );
        if !hwnd.is_null() {
            SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
        }
        hwnd
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            // Our own injected input (on a controlled PC) is never forwarded again.
            if info.flags & LLMHF_INJECTED == 0 && on_mouse(wparam as u32, info) {
                return 1; // swallowed: it went to the other PC
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    /// Returns true to swallow the event here. Must stay quick: Windows drops slow hooks.
    fn on_mouse(msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
        if CONTROLLED.swap(false, SeqCst) {
            send(Local::Activity);
        }
        if CAPTURED.load(SeqCst) {
            let high = (info.mouseData >> 16) as u16;
            let wheel = high as i16 as i32;
            let xbutton = if high == 1 { 4 } else { 5 };
            let event = match msg {
                WM_MOUSEMOVE => {
                    let mut cur = POINT { x: 0, y: 0 };
                    unsafe { GetCursorPos(&mut cur) };
                    let (dx, dy) = (info.pt.x - cur.x, info.pt.y - cur.y);
                    (dx != 0 || dy != 0).then_some(Local::Move { dx, dy })
                }
                WM_LBUTTONDOWN => Some(Local::Button { b: 1, down: true }),
                WM_LBUTTONUP => Some(Local::Button { b: 1, down: false }),
                WM_RBUTTONDOWN => Some(Local::Button { b: 2, down: true }),
                WM_RBUTTONUP => Some(Local::Button { b: 2, down: false }),
                WM_MBUTTONDOWN => Some(Local::Button { b: 3, down: true }),
                WM_MBUTTONUP => Some(Local::Button { b: 3, down: false }),
                WM_XBUTTONDOWN => Some(Local::Button { b: xbutton, down: true }),
                WM_XBUTTONUP => Some(Local::Button { b: xbutton, down: false }),
                WM_MOUSEWHEEL => Some(Local::Wheel { d: wheel, h: false }),
                WM_MOUSEHWHEEL => Some(Local::Wheel { d: wheel, h: true }),
                _ => None,
            };
            if let Some(event) = event {
                send(event);
            }
            return true;
        }
        if msg == WM_MOUSEMOVE {
            HOOKS.with(|h| h.borrow_mut().check_edge(info.pt));
        }
        false
    }

    impl Hooks {
        fn check_edge(&mut self, pt: POINT) {
            let (l, t, w, h) = desktop();
            let near_y_end = pt.y <= t + CORNER || pt.y >= t + h - 1 - CORNER;
            // Which side edge, if any (PCs sit left and right only). Corners never
            // switch, so their buttons stay usable.
            let edge = if pt.x <= l && !near_y_end {
                Some(Edge::Left)
            } else if pt.x >= l + w - 1 && !near_y_end {
                Some(Edge::Right)
            } else {
                None
            };
            let now = Instant::now();
            let cooling = self.cooldown.is_some_and(|c| now < c);
            // No switching right after a switch, or while dragging.
            let Some(edge) = edge.filter(|_| !cooling && !any_button_down()) else {
                self.edge_since = None;
                return;
            };
            let since = match self.edge_since {
                Some((e, t0)) if e == edge => t0,
                _ => {
                    self.edge_since = Some((edge, now));
                    now
                }
            };
            let delay = if EDGE_DELAY.load(SeqCst) { Duration::from_millis(200) } else { Duration::ZERO };
            if now - since >= delay {
                // The position along that edge, 0..1.
                let along = match edge {
                    Edge::Left | Edge::Right => (pt.y - t) as f32 / (h - 1) as f32,
                    Edge::Top | Edge::Bottom => (pt.x - l) as f32 / (w - 1) as f32,
                };
                send(Local::EdgePush { edge, y: along.clamp(0.0, 1.0) });
                self.edge_since = None;
                self.cooldown = Some(now + COOLDOWN);
            }
        }
    }

    unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let k = &*(lparam as *const KBDLLHOOKSTRUCT);
            if k.flags & LLKHF_INJECTED == 0 && on_key(wparam as u32, k) {
                return 1;
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    fn on_key(msg: u32, k: &KBDLLHOOKSTRUCT) -> bool {
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk = k.vkCode;
        if CONTROLLED.swap(false, SeqCst) {
            send(Local::Activity);
        }
        HOOKS.with(|h| {
            let mut h = h.borrow_mut();
            if [VK_CONTROL, VK_LCONTROL, VK_RCONTROL].iter().any(|&c| c as u32 == vk) {
                h.ctrl = down;
            }
            if [VK_MENU, VK_LMENU, VK_RMENU].iter().any(|&a| a as u32 == vk) {
                h.alt = down;
            }
            let captured = CAPTURED.load(SeqCst);
            // Ctrl+Alt+Home: always bring the mouse back.
            if down && vk == VK_HOME as u32 && h.ctrl && h.alt {
                send(Local::Release);
                return captured;
            }
            if captured {
                if !down && h.local_keys.remove(&vk) {
                    return false;
                }
                send(Local::Key { vk: vk as u16, sc: k.scanCode as u16, ext: k.flags & LLKHF_EXTENDED != 0, down });
                super::diag::key_captured();
                true
            } else {
                if down {
                    h.local_keys.insert(vk);
                } else {
                    h.local_keys.remove(&vk);
                }
                false
            }
        })
    }

    // ---- Target side: replaying input ------------------------------------------

    struct Target {
        x: f64,
        y: f64,
        /// Keys and buttons pressed by us and not yet released.
        keys: HashSet<(u16, u16, bool)>,
        buttons: HashSet<u8>,
    }

    static TARGET: Mutex<Option<Target>> = Mutex::new(None);

    fn target() -> std::sync::MutexGuard<'static, Option<Target>> {
        TARGET.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cursor arrives at `edge`, at `along` (0..1) along that edge.
    pub fn inject_enter(edge: Edge, along: f32) {
        let (l, t, w, h) = desktop();
        let along = along.clamp(0.0, 1.0) as f64;
        let (x, y) = match edge {
            Edge::Left => (l as f64 + 1.0, t as f64 + along * (h - 1) as f64),
            Edge::Right => ((l + w) as f64 - 2.0, t as f64 + along * (h - 1) as f64),
            Edge::Top => (l as f64 + along * (w - 1) as f64, t as f64 + 1.0),
            Edge::Bottom => (l as f64 + along * (w - 1) as f64, (t + h) as f64 - 2.0),
        };
        *target() = Some(Target { x, y, keys: HashSet::new(), buttons: HashSet::new() });
        move_to(x, y);
    }

    /// Move by a delta. Returns the edge pushed past, if any, and the position along it.
    pub fn inject_move(dx: i32, dy: i32) -> Option<(Edge, f32)> {
        let (l, t, w, h) = desktop();
        let mut guard = target();
        let tg = guard.as_mut()?;
        let (min_x, max_x, min_y, max_y) = (l as f64, (l + w - 1) as f64, t as f64, (t + h - 1) as f64);
        tg.x += dx as f64;
        tg.y += dy as f64;
        let over = if tg.x < min_x {
            Some(Edge::Left)
        } else if tg.x > max_x {
            Some(Edge::Right)
        } else if tg.y < min_y {
            Some(Edge::Top)
        } else if tg.y > max_y {
            Some(Edge::Bottom)
        } else {
            None
        };
        tg.x = tg.x.clamp(min_x, max_x);
        tg.y = tg.y.clamp(min_y, max_y);
        let (x, y) = (tg.x, tg.y);
        drop(guard);
        move_to(x, y);
        over.map(|edge| {
            let along = match edge {
                Edge::Left | Edge::Right => (y - min_y) / (h - 1) as f64,
                Edge::Top | Edge::Bottom => (x - min_x) / (w - 1) as f64,
            };
            (edge, along as f32)
        })
    }

    pub fn inject_button(b: u8, down: bool) {
        let (flag, data) = match (b, down) {
            (1, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (1, false) => (MOUSEEVENTF_LEFTUP, 0),
            (2, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (2, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (3, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (3, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (4, true) => (MOUSEEVENTF_XDOWN, 1),
            (4, false) => (MOUSEEVENTF_XUP, 1),
            (5, true) => (MOUSEEVENTF_XDOWN, 2),
            (5, false) => (MOUSEEVENTF_XUP, 2),
            _ => return,
        };
        if let Some(tg) = target().as_mut() {
            if down {
                tg.buttons.insert(b);
            } else {
                tg.buttons.remove(&b);
            }
        }
        mouse(0, 0, data, flag);
    }

    pub fn inject_wheel(d: i32, h: bool) {
        mouse(0, 0, d, if h { MOUSEEVENTF_HWHEEL } else { MOUSEEVENTF_WHEEL });
    }

    pub fn inject_key(vk: u16, sc: u16, ext: bool, down: bool) {
        if let Some(tg) = target().as_mut() {
            if down {
                tg.keys.insert((vk, sc, ext));
            } else {
                tg.keys.remove(&(vk, sc, ext));
            }
        }
        key(vk, sc, ext, down);
    }

    /// Control left: release every key and button we still hold down.
    pub fn inject_release_all() {
        let Some(tg) = target().take() else { return };
        for (vk, sc, ext) in tg.keys {
            key(vk, sc, ext, false);
        }
        for b in tg.buttons {
            inject_button(b, false);
        }
    }

    fn move_to(x: f64, y: f64) {
        let (l, t, w, h) = desktop();
        let nx = ((x - l as f64) * 65535.0 / (w - 1) as f64).round() as i32;
        let ny = ((y - t as f64) * 65535.0 / (h - 1) as f64).round() as i32;
        mouse(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
    }

    fn mouse(dx: i32, dy: i32, data: i32, flags: u32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: data as _, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        unsafe {
            SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn key(vk: u16, sc: u16, ext: bool, down: bool) {
        let mut flags = 0;
        if sc != 0 {
            flags |= KEYEVENTF_SCANCODE;
        }
        if ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: if sc != 0 { 0 } else { vk },
                    wScan: sc,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        let error = if sent == 1 { 0 } else { unsafe { GetLastError() } };
        super::diag::key_typed(sent == 1, error);
    }
}

/// Builds without the `input` feature: no hooks, no injection.
#[cfg(not(feature = "input"))]
mod stub {
    use super::{Local, TX};
    use crate::proto::Edge;
    use tokio::sync::mpsc::UnboundedSender;

    pub fn available() -> bool {
        false
    }
    pub fn start(tx: UnboundedSender<Local>) {
        let _ = TX.set(tx);
    }
    pub fn set_enabled(_on: bool) {}
    pub fn set_edge_delay(_on: bool) {}
    pub fn capture() {}
    pub fn release(_place: Option<(Edge, f32)>) {}
    pub fn set_controlled(_on: bool) {}
    pub fn inject_enter(_edge: Edge, _y: f32) {}
    pub fn inject_move(_dx: i32, _dy: i32) -> Option<(Edge, f32)> {
        None
    }
    pub fn inject_button(_b: u8, _down: bool) {}
    pub fn inject_wheel(_d: i32, _h: bool) {}
    pub fn inject_key(_vk: u16, _sc: u16, _ext: bool, _down: bool) {}
    pub fn inject_release_all() {}
}
