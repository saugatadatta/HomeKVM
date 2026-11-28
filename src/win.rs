//! Small Win32 helpers: single instance, and showing the hidden main window
//! from the tray thread (a hidden eframe window gets no frames, so it can't show itself).

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, POINT, RECT, WAIT_OBJECT_0,
};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetCursorPos, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow, ShowWindowAsync,
    SystemParametersInfoW, ASFW_ANY, GWL_EXSTYLE, LWA_ALPHA, SPI_GETCLIENTAREAANIMATION, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNORMAL, WS_EX_LAYERED,
};

const INSTANCE_MUTEX: &str = "Local\\HomeKVM.Instance";
const SHOW_EVENT: &str = "Local\\HomeKVM.Show";

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// The window is open but still invisible while it lays itself out.
static REVEAL_PENDING: AtomicBool = AtomicBool::new(false);
/// Just started: the window stays in the tray until eframe has shown it once (see
/// `settle_startup`).
static STARTING: AtomicBool = AtomicBool::new(false);
/// The middle of the tray icon on screen (physical pixels), once the tray has said.
static TRAY_X: AtomicI32 = AtomicI32::new(i32::MIN);
static TRAY_Y: AtomicI32 = AtomicI32::new(i32::MIN);
/// Counts window animations; a newer one stops an older one.
static ANIMATION: AtomicU64 = AtomicU64::new(0);
/// The window's current opacity (0 to 255).
static ALPHA: AtomicU8 = AtomicU8::new(255);

/// Opening: the window flies in from the tray, slowing down as it arrives, and fades in
/// during the first half. Closing: it flies back toward the tray, fading out.
const OPEN_MS: f32 = 380.0;
const CLOSE_MS: f32 = 300.0;
/// How far toward the tray icon the flight starts or ends (share of the way there).
const FLIGHT: f32 = 0.6;
static EGUI_CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Running from the Microsoft Store package (MSIX) rather than as the standalone exe.
/// The Store version updates through the Store and starts at sign-in through the
/// package's startup task.
pub fn is_packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;
        let mut len = 0u32;
        let result = unsafe {
            windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName(&mut len, std::ptr::null_mut())
        };
        result != APPMODEL_ERROR_NO_PACKAGE
    })
}

/// The "only one HomeKVM" lock this copy holds (0 = none).
static INSTANCE_LOCK: AtomicIsize = AtomicIsize::new(0);

/// True if this is the only running copy for this user session. Safe to call again
/// while waiting for another copy to exit.
pub fn acquire_single_instance() -> bool {
    let name = wide(&format!("{INSTANCE_MUTEX}{}", crate::profile::suffix()));
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if mutex.is_null() {
        return true;
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // Another copy has the lock. Close our handle to it, or it would keep the lock
        // alive even after that copy exits, and a retry could never succeed.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(mutex);
        }
        return false;
    }
    // Ours: kept open until this copy exits (or hands over to an update).
    INSTANCE_LOCK.store(mutex as isize, Ordering::SeqCst);
    true
}

/// Let go of the lock before starting a newer version, so it can take over at once.
pub fn release_single_instance() {
    let lock = INSTANCE_LOCK.swap(0, Ordering::SeqCst);
    if lock != 0 {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(lock as HANDLE);
        }
    }
}

/// Ask the copy that is already running to open its window.
pub fn signal_running_instance() {
    let name = wide(&format!("{SHOW_EVENT}{}", crate::profile::suffix()));
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
        let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
        if !event.is_null() {
            SetEvent(event);
            windows_sys::Win32::Foundation::CloseHandle(event);
        }
    }
}

/// Open the window whenever another copy of HomeKVM is started.
pub fn listen_for_show_requests() {
    let name = wide(&format!("{SHOW_EVENT}{}", crate::profile::suffix()));
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        return;
    }
    let event = event as isize;
    std::thread::Builder::new()
        .name("show-requests".into())
        .spawn(move || loop {
            if unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } != WAIT_OBJECT_0 {
                break;
            }
            show_main_window();
        })
        .ok();
}

pub fn register_main_window(cc: &eframe::CreationContext<'_>) {
    if let Ok(handle) = cc.window_handle() {
        if let RawWindowHandle::Win32(h) = handle.as_raw() {
            let hwnd = h.hwnd.get();
            MAIN_HWND.store(hwnd, Ordering::SeqCst);
            // A layered window can be faded in and out as a whole.
            unsafe {
                let style = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
            }
            set_alpha(hwnd, 255);
            // eframe makes its window visible after the first frame it draws, whatever
            // the start settings say. Cloaked, that shows nothing; settle_startup then
            // hides it, so HomeKVM really starts in the tray.
            set_cloak(hwnd, true);
            STARTING.store(true, Ordering::SeqCst);
        }
    }
    let _ = EGUI_CTX.set(cc.egui_ctx.clone());
}

/// Called every frame. Returns true while the start isn't settled yet (the window
/// should draw another frame). Once eframe has shown the window, it is hidden again,
/// unless it was asked to open meanwhile; then it appears the usual way (`reveal`).
pub fn settle_startup() -> bool {
    if !STARTING.load(Ordering::SeqCst) {
        return false;
    }
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if unsafe { IsWindowVisible(hwnd as HWND) } == 0 {
        return true;
    }
    STARTING.store(false, Ordering::SeqCst);
    if !REVEAL_PENDING.load(Ordering::SeqCst) {
        unsafe {
            ShowWindow(hwnd as HWND, SW_HIDE);
        }
        set_cloak(hwnd, false);
    }
    false
}

/// Open the window. A hidden window is opened "cloaked" (invisible on screen but
/// drawing), so it can fit its height to the content before it appears.
pub fn show_main_window() {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            if IsWindowVisible(hwnd as HWND) == 0 {
                set_cloak(hwnd, true);
                REVEAL_PENDING.store(true, Ordering::SeqCst);
            } else if ALPHA.load(Ordering::SeqCst) < 255 && IsIconic(hwnd as HWND) == 0 {
                // Clicked again while it was flying back to the tray: it comes back.
                let generation = ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
                if let (Some(from), Some(place)) = (position(hwnd), centered(hwnd)) {
                    animate(hwnd, generation, from, place.at, ALPHA.load(Ordering::SeqCst), 255, OPEN_MS * 0.7, true, false);
                }
            }
            ShowWindowAsync(hwnd as HWND, SW_SHOWNORMAL);
            SetForegroundWindow(hwnd as HWND);
        }
    }
    request_repaint();
}

/// Send the window back to the tray: with `fly`, it flies toward the tray icon first.
/// This uses Windows calls like `show_main_window`: hiding it through egui instead left
/// egui thinking it was still hidden after the tray showed it again, so closing it a
/// second time did nothing.
pub fn hide_main_window(fly: bool) {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd == 0 {
        return;
    }
    let generation = ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
    unsafe {
        let shown = IsWindowVisible(hwnd as HWND) != 0 && IsIconic(hwnd as HWND) == 0;
        if fly && shown && animations_on() {
            if let (Some(from), Some(place)) = (position(hwnd), centered(hwnd)) {
                let tray = tray_point(place.work);
                let center = (from.0 + place.size.0 / 2, from.1 + place.size.1 / 2);
                let to = (
                    from.0 + ((tray.0 - center.0) as f32 * FLIGHT) as i32,
                    from.1 + ((tray.1 - center.1) as f32 * FLIGHT) as i32,
                );
                animate(hwnd, generation, from, to, ALPHA.load(Ordering::SeqCst), 0, CLOSE_MS, false, true);
                return;
            }
        }
        ShowWindowAsync(hwnd as HWND, SW_HIDE);
    }
}

/// Where the tray icon is, from the tray's own events.
pub fn set_tray_point(x: i32, y: i32) {
    TRAY_X.store(x, Ordering::SeqCst);
    TRAY_Y.store(y, Ordering::SeqCst);
}

/// The main window's handle (0 before it exists), for Windows APIs that show their own
/// dialogs over it.
pub fn main_hwnd() -> isize {
    MAIN_HWND.load(Ordering::SeqCst)
}

pub fn reveal_pending() -> bool {
    REVEAL_PENDING.load(Ordering::SeqCst)
}

/// Called by the window once its content fits and has been drawn: it flies in from the
/// tray to the center of the screen. Fully transparent until then, so it never shows an
/// empty white frame.
pub fn reveal() {
    if !REVEAL_PENDING.swap(false, Ordering::SeqCst) {
        return;
    }
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    let generation = ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
    let Some(place) = centered(hwnd) else {
        set_alpha(hwnd, 255);
        set_cloak(hwnd, false);
        return;
    };
    if animations_on() {
        let tray = tray_point(place.work);
        let center = (place.at.0 + place.size.0 / 2, place.at.1 + place.size.1 / 2);
        let from = (
            place.at.0 + ((tray.0 - center.0) as f32 * FLIGHT) as i32,
            place.at.1 + ((tray.1 - center.1) as f32 * FLIGHT) as i32,
        );
        set_alpha(hwnd, 0);
        move_to(hwnd, from, false);
        set_cloak(hwnd, false);
        unsafe {
            SetForegroundWindow(hwnd as HWND);
        }
        animate(hwnd, generation, from, place.at, 0, 255, OPEN_MS, true, false);
    } else {
        // Windows' "Animation effects" is off: it just appears.
        move_to(hwnd, place.at, false);
        set_alpha(hwnd, 255);
        set_cloak(hwnd, false);
        unsafe {
            SetForegroundWindow(hwnd as HWND);
        }
    }
}

/// Where the window goes: centered on the screen the mouse is on (the one with the tray
/// icon that was just clicked), inside the area the taskbar leaves free.
struct Place {
    at: (i32, i32),
    size: (i32, i32),
    work: RECT,
}

fn centered(hwnd: isize) -> Option<Place> {
    if hwnd == 0 {
        return None;
    }
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd as HWND, &mut rect) == 0 {
            return None;
        }
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let x = work.left + ((work.right - work.left) - w) / 2;
        let y = (work.top + ((work.bottom - work.top) - h) / 2).max(work.top);
        Some(Place { at: (x, y), size: (w, h), work })
    }
}

/// The tray icon's middle; before the tray has said, the corner where it usually sits.
fn tray_point(work: RECT) -> (i32, i32) {
    let (x, y) = (TRAY_X.load(Ordering::SeqCst), TRAY_Y.load(Ordering::SeqCst));
    if x == i32::MIN {
        (work.right - 60, work.bottom + 20)
    } else {
        (x, y)
    }
}

fn position(hwnd: isize) -> Option<(i32, i32)> {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    (unsafe { GetWindowRect(hwnd as HWND, &mut rect) } != 0).then_some((rect.left, rect.top))
}

fn move_to(hwnd: isize, (x, y): (i32, i32), from_other_thread: bool) {
    let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | if from_other_thread { SWP_ASYNCWINDOWPOS } else { 0 };
    unsafe {
        SetWindowPos(hwnd as HWND, std::ptr::null_mut(), x, y, 0, 0, flags);
    }
}

fn set_alpha(hwnd: isize, alpha: u8) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        SetLayeredWindowAttributes(hwnd as HWND, 0, alpha, LWA_ALPHA);
    }
    ALPHA.store(alpha, Ordering::SeqCst);
}

/// Windows' "Animation effects" setting (Settings > Accessibility > Visual effects).
fn animations_on() -> bool {
    let mut on: BOOL = 1;
    unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut BOOL).cast(), 0);
    }
    on != 0
}

/// Move and fade the window over `ms`, on its own thread so the window keeps drawing.
/// `opening` uses an ease-out (fast, then settling) with the fade done by halfway;
/// closing eases in. With `hide`, the window is hidden at the end.
#[allow(clippy::too_many_arguments)]
fn animate(hwnd: isize, generation: u64, from: (i32, i32), to: (i32, i32), a0: u8, a1: u8, ms: f32, opening: bool, hide: bool) {
    std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            if ANIMATION.load(Ordering::SeqCst) != generation {
                return; // a newer animation took over
            }
            let t = (start.elapsed().as_secs_f32() * 1000.0 / ms).min(1.0);
            let (moved, faded) = if opening {
                let out_expo = if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) };
                let fade = (t / 0.5).min(1.0);
                (out_expo, 1.0 - (1.0 - fade).powi(3))
            } else {
                (t * t * t, t)
            };
            let x = from.0 + ((to.0 - from.0) as f32 * moved).round() as i32;
            let y = from.1 + ((to.1 - from.1) as f32 * moved).round() as i32;
            let alpha = (a0 as f32 + (a1 as f32 - a0 as f32) * faded).round().clamp(0.0, 255.0) as u8;
            move_to(hwnd, (x, y), true);
            set_alpha(hwnd, alpha);
            if t >= 1.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(6));
        }
        if hide && ANIMATION.load(Ordering::SeqCst) == generation {
            unsafe {
                ShowWindowAsync(hwnd as HWND, SW_HIDE);
            }
        }
    });
}

fn set_cloak(hwnd: isize, on: bool) {
    if hwnd == 0 {
        return;
    }
    let value: BOOL = on as BOOL;
    unsafe {
        DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_CLOAK as _,
            (&value as *const BOOL).cast(),
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

pub fn request_repaint() {
    if let Some(ctx) = EGUI_CTX.get() {
        ctx.request_repaint();
    }
}
