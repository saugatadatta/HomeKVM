//! Small Win32 helpers: single instance, and showing the hidden main window
//! from the tray thread (a hidden eframe window gets no frames, so it can't show itself).

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, WAIT_OBJECT_0};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsWindowVisible, SetForegroundWindow, ShowWindowAsync, ASFW_ANY, SW_SHOWNORMAL,
};

const INSTANCE_MUTEX: &str = "Local\\HomeKVM.Instance";
const SHOW_EVENT: &str = "Local\\HomeKVM.Show";

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// The window is open but still invisible while it lays itself out.
static REVEAL_PENDING: AtomicBool = AtomicBool::new(false);
static EGUI_CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
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
            MAIN_HWND.store(h.hwnd.get(), Ordering::SeqCst);
        }
    }
    let _ = EGUI_CTX.set(cc.egui_ctx.clone());
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

pub fn reveal_pending() -> bool {
    REVEAL_PENDING.load(Ordering::SeqCst)
}

/// Called by the window once its content fits: make it appear.
pub fn reveal() {
    if REVEAL_PENDING.swap(false, Ordering::SeqCst) {
        let hwnd = MAIN_HWND.load(Ordering::SeqCst);
        set_cloak(hwnd, false);
        unsafe {
            SetForegroundWindow(hwnd as HWND);
        }
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
