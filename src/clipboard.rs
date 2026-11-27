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
//!
//! Files: copying files or folders sends only the list (names and sizes). This PC
//! keeps the "offer" (the real paths) and serves file data when another PC pastes.
//! The receiving PC puts a data object on its clipboard (see vfiles.rs) that fetches
//! the data at paste time: as real files in a private cache for small copies, or as
//! virtual files streamed straight into place for big ones.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
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
use windows_sys::Win32::UI::Shell::DragQueryFileW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW, RegisterClassW, TranslateMessage,
    HWND_MESSAGE, MSG, WM_APP, WM_CLIPBOARDUPDATE, WNDCLASSW,
};

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
/// Progress lines in the console, development builds only.
macro_rules! dlog {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!("[clipboard] {}", format!($($arg)*));
        }
    };
}

/// Name of a marker format on everything HomeKVM puts on the clipboard, so no copy of
/// HomeKVM ever shares again what came from another PC.
pub const MARKER: &str = "HomeKVM.FromAnotherPC";
/// A copied folder tree with more entries than this isn't offered.
const MAX_ENTRIES: usize = 10_000;
/// Bigger images aren't shared (a 4K screenshot is about 33 MB).
pub const MAX_IMAGE: usize = 64 * 1024 * 1024;
pub const MAX_TEXT: usize = 16 * 1024 * 1024;
const WM_APPLY: u32 = WM_APP + 1;

/// One clipboard copy: text, an image (a Windows DIB), files, or a mix.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub text: Option<String>,
    pub image: Option<Vec<u8>>,
    pub files: Option<FileList>,
}

/// Copied files and folders: just the list. The data stays on the PC that copied them
/// until another PC pastes.
#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq)]
pub struct FileList {
    /// Identifies this copy, so a paste can't fetch from an older one.
    pub offer: u64,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash, PartialEq)]
pub struct FileEntry {
    /// Relative path such as `Photos\2026\beach.jpg`; top-level items have no `\`.
    pub path: String,
    pub size: u64,
    pub dir: bool,
}

impl FileList {
    pub fn total(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.image.is_none() && self.files.is_none()
    }

    fn fingerprint(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.image.hash(&mut h);
        self.files.hash(&mut h);
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
static SHARE_FILES: AtomicBool = AtomicBool::new(true);
/// Copies up to this many bytes paste as real files; bigger ones as virtual files.
static REAL_FILE_LIMIT: AtomicU64 = AtomicU64::new(50 * 1024 * 1024);
/// Files copied on this PC: the offer ID and each entry's real path (None for folders).
static OFFER: Mutex<Option<(u64, Vec<Option<PathBuf>>)>> = Mutex::new(None);
/// Clipboard sequence number of the change we made ourselves.
static OUR_CHANGE: AtomicU32 = AtomicU32::new(0);
/// Clipboard sequence number when a copy from another PC was accepted. If the local
/// "clipboard changed" note for that same state is handled afterwards, it isn't a new
/// copy made here (two copies of HomeKVM watching one clipboard see the same change).
static APPLIED_AT: AtomicU32 = AtomicU32::new(0);
/// Copies from other PCs waiting to be written, with the ID of the PC they came from.
static INCOMING: Mutex<VecDeque<(Clip, String)>> = Mutex::new(VecDeque::new());

/// Start watching the clipboard; copies made on this PC go to `tx`.
pub fn start(tx: UnboundedSender<Clip>) {
    let _ = TX.set(tx);
    crate::vfiles::clear_cache();
    std::thread::Builder::new().name("clipboard".into()).spawn(run).ok();
}

/// Which kinds to share (Settings). Applies to sending and receiving.
pub fn set_options(text: bool, images: bool, files: bool, real_file_limit_mb: u32) {
    SHARE_TEXT.store(text, SeqCst);
    SHARE_IMAGES.store(images, SeqCst);
    SHARE_FILES.store(files, SeqCst);
    REAL_FILE_LIMIT.store(real_file_limit_mb as u64 * 1024 * 1024, SeqCst);
}

/// The real path of an entry in the current file copy on this PC, if `offer` is still
/// the latest copy (last copy wins: older offers can't be fetched).
pub fn offered_file(offer: u64, index: usize) -> Option<PathBuf> {
    let guard = OFFER.lock().ok()?;
    let (id, paths) = guard.as_ref()?;
    if *id != offer {
        return None;
    }
    paths.get(index)?.clone()
}

/// A copy arrived from the PC with ID `from`: put it on this PC's clipboard.
pub fn apply(mut clip: Clip, from: String) {
    if !SHARE_TEXT.load(SeqCst) {
        clip.text = None;
    }
    if !SHARE_IMAGES.load(SeqCst) {
        clip.image = None;
    }
    if !SHARE_FILES.load(SeqCst) {
        clip.files = None;
    }
    // Two file copies crossing on the way (made at almost the same moment): the newer
    // one wins on both PCs, so exactly one PC keeps its files on offer.
    let ours_is_newer = match (&clip.files, OFFER.lock().ok().and_then(|o| o.as_ref().map(|(id, _)| *id))) {
        (Some(theirs), Some(ours)) => ours > theirs.offer,
        _ => false,
    };
    if ours_is_newer {
        dlog!("ignored files from {}: this PC's copy is newer", short(&from));
        return;
    }
    if clip.is_empty() || LAST.swap(clip.fingerprint(), SeqCst) == clip.fingerprint() {
        return;
    }
    dlog!("applying copy from {}: {}", short(&from), describe(&clip));
    crate::activity::clip_got(&clip);
    APPLIED_AT.store(unsafe { GetClipboardSequenceNumber() }, SeqCst);
    // Another PC's copy is now the latest: files copied here can no longer be pasted.
    if let Ok(mut offer) = OFFER.lock() {
        *offer = None;
    }
    if let Ok(mut q) = INCOMING.lock() {
        q.push_back((clip, from));
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
        // The clipboard thread is an OLE apartment: the data object for pasted files
        // lives here (vfiles.rs).
        crate::vfiles::init_thread();
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
                    Some((clip, from)) => write(hwnd, &clip, from),
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
    let seq = GetClipboardSequenceNumber();
    if seq == OUR_CHANGE.load(SeqCst) {
        return; // our own write of a copy from another PC
    }
    if seq == APPLIED_AT.load(SeqCst) {
        dlog!("not sharing: a copy from another PC already replaced this clipboard state");
        return;
    }
    if IsClipboardFormatAvailable(RegisterClipboardFormatW(wide(MARKER).as_ptr())) != 0 {
        return; // came from another PC through HomeKVM (possibly another copy of it)
    }
    let (text, images, files) = (SHARE_TEXT.load(SeqCst), SHARE_IMAGES.load(SeqCst), SHARE_FILES.load(SeqCst));
    if !open(hwnd) {
        return;
    }
    let (mut clip, paths) = if is_private() {
        (Clip::default(), Vec::new())
    } else {
        let paths = if files { read_file_paths() } else { Vec::new() };
        let clip = Clip {
            text: if text { read_text() } else { None },
            image: if images { read_bytes(CF_DIB, MAX_IMAGE) } else { None },
            files: None,
        };
        (clip, paths)
    };
    CloseClipboard();
    // Any new copy here replaces the previous file offer (last copy wins). The same
    // files announced again (some apps set the clipboard several times) keep their offer.
    let mut offer = if paths.is_empty() { None } else { list_files(&paths) };
    if let Ok(mut current) = OFFER.lock() {
        if let (Some((list, real)), Some((id, cur_real))) = (offer.as_mut(), current.as_ref()) {
            if real == cur_real {
                list.offer = *id;
            }
        }
        *current = offer.as_ref().map(|(list, real)| (list.offer, real.clone()));
    }
    clip.files = offer.map(|(list, _)| list);
    if !clip.is_empty() && LAST.swap(clip.fingerprint(), SeqCst) != clip.fingerprint() {
        dlog!("sharing a copy made here: {}", describe(&clip));
        if let Some(tx) = TX.get() {
            let _ = tx.send(clip);
        }
    }
}

fn short(id: &str) -> String {
    crate::identity::short(id)
}

fn describe(clip: &Clip) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &clip.text {
        parts.push(format!("text {} chars", t.chars().count()));
    }
    if let Some(i) = &clip.image {
        parts.push(format!("image {} bytes", i.len()));
    }
    if let Some(f) = &clip.files {
        parts.push(format!("{} files/folders, {} bytes, offer {}", f.entries.len(), f.total(), f.offer));
    }
    parts.join(", ")
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

/// The paths of files and folders copied in Explorer (the clipboard must be open).
unsafe fn read_file_paths() -> Vec<PathBuf> {
    if IsClipboardFormatAvailable(CF_HDROP) == 0 {
        return Vec::new();
    }
    let drop = GetClipboardData(CF_HDROP);
    if drop.is_null() {
        return Vec::new();
    }
    let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
    (0..count)
        .filter_map(|i| {
            let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            let got = DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32) as usize;
            (got > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..got])))
        })
        .collect()
}

/// Walk the copied files and folders: the list to share, and each entry's real path.
fn list_files(paths: &[PathBuf]) -> Option<(FileList, Vec<Option<PathBuf>>)> {
    fn walk(path: &Path, rel: String, entries: &mut Vec<FileEntry>, real: &mut Vec<Option<PathBuf>>) -> Option<()> {
        // Paths must fit Windows' 260-character limit for pasted file names.
        if entries.len() >= MAX_ENTRIES || rel.encode_utf16().count() >= 259 {
            return None;
        }
        let meta = std::fs::metadata(path).ok()?;
        if meta.is_dir() {
            entries.push(FileEntry { path: rel.clone(), size: 0, dir: true });
            real.push(None);
            let mut children: Vec<_> = std::fs::read_dir(path).ok()?.filter_map(|e| e.ok()).collect();
            children.sort_by_key(|e| e.file_name());
            for child in children {
                let name = child.file_name().to_string_lossy().into_owned();
                walk(&child.path(), format!("{rel}\\{name}"), entries, real)?;
            }
        } else {
            entries.push(FileEntry { path: rel, size: meta.len(), dir: false });
            real.push(Some(path.to_path_buf()));
        }
        Some(())
    }
    let (mut entries, mut real) = (Vec::new(), Vec::new());
    for p in paths {
        let name = p.file_name()?.to_string_lossy().into_owned();
        walk(p, name, &mut entries, &mut real)?;
    }
    let offer = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((FileList { offer, entries }, real))
}

/// Replace this PC's clipboard with a copy from another PC.
unsafe fn write(hwnd: HWND, clip: &Clip, from: String) {
    if let Some(files) = &clip.files {
        // Files: a data object that fetches the data when something pastes.
        let real_files = files.total() <= REAL_FILE_LIMIT.load(SeqCst);
        let ok = crate::vfiles::put_on_clipboard(files.clone(), from, real_files);
        dlog!("files on the clipboard as {}: {}", if real_files { "real files" } else { "virtual files" }, if ok { "ok" } else { "failed" });
        if ok {
            OUR_CHANGE.store(GetClipboardSequenceNumber(), SeqCst);
        }
        return;
    }
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
    put(RegisterClipboardFormatW(wide(MARKER).as_ptr()), &[1]);
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
