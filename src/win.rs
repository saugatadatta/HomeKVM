//! Small Win32 helpers: single instance, and showing the hidden main window
//! from the tray thread (a hidden eframe window gets no frames, so it can't show itself).

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
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
    AllowSetForegroundWindow, GetCursorPos, GetWindowRect, IsWindowVisible, SetForegroundWindow, SetWindowPos,
    ShowWindow, ShowWindowAsync, ASFW_ANY, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNORMAL,
};

const INSTANCE_MUTEX: &str = "Local\\HomeKVM.Instance";
const SHOW_EVENT: &str = "Local\\HomeKVM.Show";

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// The window is open but still invisible while it lays itself out.
static REVEAL_PENDING: AtomicBool = AtomicBool::new(false);
/// Just started: the window stays in the tray until eframe has shown it once (see
/// `settle_startup`).
static STARTING: AtomicBool = AtomicBool::new(false);
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
            }
            ShowWindowAsync(hwnd as HWND, SW_SHOWNORMAL);
            SetForegroundWindow(hwnd as HWND);
        }
    }
    request_repaint();
}

/// Send the window back to the tray. This uses the same Windows call as
/// `show_main_window`: hiding it through egui instead left egui thinking it was still
/// hidden after the tray showed it again, so closing it a second time did nothing.
pub fn hide_main_window() {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            ShowWindowAsync(hwnd as HWND, SW_HIDE);
        }
    }
}

pub fn reveal_pending() -> bool {
    REVEAL_PENDING.load(Ordering::SeqCst)
}

/// Called by the window once its content fits: center it, then make it appear.
pub fn reveal() {
    if REVEAL_PENDING.swap(false, Ordering::SeqCst) {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        center_on_screen(hwnd);
        set_cloak(hwnd, false);
        unsafe {
            SetForegroundWindow(hwnd as HWND);
        }
    }
}

/// Center the window on the screen the mouse is on (the one with the tray icon that was
/// just clicked), inside the area the taskbar leaves free.
fn center_on_screen(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd as HWND, &mut rect) == 0 {
            return;
        }
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let x = work.left + ((work.right - work.left) - w) / 2;
        let y = (work.top + ((work.bottom - work.top) - h) / 2).max(work.top);
        SetWindowPos(hwnd as HWND, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
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
