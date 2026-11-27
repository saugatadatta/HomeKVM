//! The shared clipboard: one clipboard across all paired PCs, last copy wins.
//!
//! A hidden window listens for clipboard changes (`AddClipboardFormatListener`).
//! When you copy text or an image, it is handed to the network side, which sends it
//! to every connected paired PC. A PC that receives a copy puts it on its own
//! clipboard, replacing what was there, and remembers that change as its own so it
//! isn't sent back (no ping-pong).
//!
//! Never sent: anything a password manager marks as private, i.e. the clipboard
//! history opt-outs `ExcludeClipboardContentFromMonitorProcessing`,
//! `CanIncludeInClipboardHistory = 0` and `CanUploadToCloudClipboard = 0`.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, RegisterClassW, TranslateMessage,
    HWND_MESSAGE, MSG, WM_APP, WM_CLIPBOARDUPDATE, WNDCLASSW,
};

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
/// Bigger images aren't shared (a 4K screenshot is about 33 MB).
pub const MAX_IMAGE: usize = 64 * 1024 * 1024;
pub const MAX_TEXT: usize = 16 * 1024 * 1024;
const WM_APPLY: u32 = WM_APP + 1;

/// One clipboard copy: text, an image (a Windows DIB), or both.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub text: Option<String>,
    pub image: Option<Vec<u8>>,
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.image.is_none()
    }

    fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.image.hash(&mut h);
        h.finish()
    }
}

/// The last copy this PC sent or received. The same content is never sent or
/// written twice, which also stops two copies of HomeKVM that share one clipboard
/// (the dev setup) from echoing each other.
static LAST: AtomicU64 = AtomicU64::new(0);

static TX: std::sync::OnceLock<UnboundedSender<Clip>> = std::sync::OnceLock::new();
static WINDOW: AtomicUsize = AtomicUsize::new(0);
static SHARE_TEXT: AtomicBool = AtomicBool::new(true);
static SHARE_IMAGES: AtomicBool = AtomicBool::new(true);
/// Clipboard sequence number of the change we made ourselves.
static OUR_CHANGE: AtomicU32 = AtomicU32::new(0);
static INCOMING: Mutex<VecDeque<Clip>> = Mutex::new(VecDeque::new());

/// Start watching the clipboard; copies made on this PC go to `tx`.
pub fn start(tx: UnboundedSender<Clip>) {
    let _ = TX.set(tx);
    std::thread::Builder::new().name("clipboard".into()).spawn(run).ok();
}

/// Which kinds to share (Settings). Applies to sending and receiving.
pub fn set_options(text: bool, images: bool) {
    SHARE_TEXT.store(text, SeqCst);
    SHARE_IMAGES.store(images, SeqCst);
}

/// A copy arrived from another PC: put it on this PC's clipboard.
pub fn apply(mut clip: Clip) {
    if !SHARE_TEXT.load(SeqCst) {
        clip.text = None;
    }
    if !SHARE_IMAGES.load(SeqCst) {
        clip.image = None;
    }
    if clip.is_empty() || LAST.swap(clip.fingerprint(), SeqCst) == clip.fingerprint() {
        return;
    }
    if let Ok(mut q) = INCOMING.lock() {
        q.push_back(clip);
    }
    let hwnd = WINDOW.load(SeqCst);
    if hwnd != 0 {
        unsafe {
            PostMessageW(hwnd as HWND, WM_APPLY, 0, 0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn run() {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let class = wide("HomeKVM.Clipboard");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        // A message-only window: never shown, just receives clipboard notifications.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            module,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        WINDOW.store(hwnd as usize, SeqCst);
        AddClipboardFormatListener(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            on_local_copy(hwnd);
            0
        }
        WM_APPLY => {
            loop {
                let next = INCOMING.lock().ok().and_then(|mut q| q.pop_front());
                match next {
                    Some(clip) => write(hwnd, &clip),
                    None => break,
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Someone copied something on this PC.
unsafe fn on_local_copy(hwnd: HWND) {
    if GetClipboardSequenceNumber() == OUR_CHANGE.load(SeqCst) {
        return; // our own write of a copy from another PC
    }
    let (text, images) = (SHARE_TEXT.load(SeqCst), SHARE_IMAGES.load(SeqCst));
    if !text && !images {
        return;
    }
    if !open(hwnd) {
        return;
    }
    let clip = if is_private() {
        Clip::default()
    } else {
        Clip {
            text: if text { read_text() } else { None },
            image: if images { read_bytes(CF_DIB, MAX_IMAGE) } else { None },
        }
    };
    CloseClipboard();
    if !clip.is_empty() && LAST.swap(clip.fingerprint(), SeqCst) != clip.fingerprint() {
        if let Some(tx) = TX.get() {
            let _ = tx.send(clip);
        }
    }
}

/// Password managers mark secrets so clipboard tools leave them alone.
/// The clipboard must be open.
unsafe fn is_private() -> bool {
    let format = |name: &str| RegisterClipboardFormatW(wide(name).as_ptr());
    if IsClipboardFormatAvailable(format("ExcludeClipboardContentFromMonitorProcessing")) != 0 {
        return true;
    }
    // These hold a DWORD; 0 means "keep this out of clipboard history and the cloud".
    ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"].iter().any(|name| {
        read_bytes(format(name), 16)
            .is_some_and(|b| b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0)
    })
}

/// OpenClipboard fails while another app has it open; try for a moment.
unsafe fn open(hwnd: HWND) -> bool {
    for _ in 0..10 {
        if OpenClipboard(hwnd) != 0 {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

unsafe fn read_text() -> Option<String> {
    let bytes = read_bytes(CF_UNICODETEXT, MAX_TEXT)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let text = String::from_utf16_lossy(&units[..end]);
    (!text.is_empty()).then_some(text)
}

/// The raw bytes of one clipboard format (the clipboard must be open).
unsafe fn read_bytes(format: u32, max: usize) -> Option<Vec<u8>> {
    if IsClipboardFormatAvailable(format) == 0 {
        return None;
    }
    let handle = GetClipboardData(format);
    if handle.is_null() {
        return None;
    }
    let size = GlobalSize(handle);
    if size == 0 || size > max {
        return None;
    }
    let ptr = GlobalLock(handle) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
    GlobalUnlock(handle);
    Some(bytes)
}

/// Replace this PC's clipboard with a copy from another PC.
unsafe fn write(hwnd: HWND, clip: &Clip) {
    if !open(hwnd) {
        return;
    }
    EmptyClipboard();
    if let Some(text) = &clip.text {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        put(CF_UNICODETEXT, &bytes);
    }
    if let Some(image) = &clip.image {
        put(CF_DIB, image);
    }
    CloseClipboard();
    // Remember this change as ours so it isn't sent back.
    OUR_CHANGE.store(GetClipboardSequenceNumber(), SeqCst);
}

unsafe fn put(format: u32, bytes: &[u8]) {
    let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if mem.is_null() {
        return;
    }
    let ptr = GlobalLock(mem) as *mut u8;
    if ptr.is_null() {
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    GlobalUnlock(mem);
    // The clipboard owns the memory from here.
    SetClipboardData(format, mem);
}
