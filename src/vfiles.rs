//! Pasting files copied on another PC.
//!
//! When another PC copies files, this PC's clipboard gets an OLE data object holding
//! only the list. Nothing is transferred until an app pastes:
//!
//! - **Small copies (real files):** the object offers `CF_HDROP`, ordinary file
//!   paths. When an app asks for them, the files are fetched into a private cache
//!   folder (`%LOCALAPPDATA%\HomeKVM\paste-cache`) and those paths are handed over.
//!   Works in every app, including browsers and chat apps that need a real path.
//! - **Big copies (virtual files):** the object offers `FileGroupDescriptorW` and
//!   `FileContents`, the way Remote Desktop pastes files. Explorer creates each file
//!   itself and reads its data from a stream that pulls it over the network, with
//!   Explorer's own progress and Cancel, and nothing stored in between.

use crate::clipboard::FileList;
use crate::net::{self, FetchMsg};
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::{implement, Error, Result, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, DV_E_FORMATETC, E_FAIL, E_NOTIMPL, OLE_E_ADVISENOTSUPPORTED, S_FALSE, S_OK, STG_E_ACCESSDENIED,
    STG_E_INVALIDFUNCTION,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, ISequentialStream_Impl, IStream,
    IStream_Impl, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, LOCKTYPE, STATFLAG, STATSTG, STGC, STGMEDIUM,
    STGMEDIUM_0, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET, TYMED_HGLOBAL,
    TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{OleInitialize, OleSetClipboard, CF_HDROP, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, FILEDESCRIPTORW};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FD_ATTRIBUTES: u32 = 0x4;
const FD_FILESIZE: u32 = 0x40;
const FD_PROGRESSUI: u32 = 0x4000;

/// Call once on the clipboard thread before anything else here.
pub fn init_thread() {
    unsafe {
        let _ = OleInitialize(None);
    }
}

/// %LOCALAPPDATA%\HomeKVM\paste-cache (per profile in dev builds).
fn cache_root() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(format!("HomeKVM{}", crate::profile::suffix())).join("paste-cache"))
}

/// Empty the paste cache (at start, and when a new file copy arrives).
pub fn clear_cache() {
    if let Some(root) = cache_root() {
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// Put files copied on PC `from` on this PC's clipboard. Returns false on failure.
pub fn put_on_clipboard(list: FileList, from: String, real_files: bool) -> bool {
    clear_cache();
    let object: IDataObject = FilesData::new(list, from, real_files).into();
    // Fails while another app has the clipboard open (often just for a moment, such as
    // Explorer finishing its copy): try for a little while.
    let mut last = Ok(());
    for _ in 0..20 {
        last = unsafe { OleSetClipboard(&object) };
        if last.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if cfg!(debug_assertions) {
        if let Err(e) = last {
            eprintln!("[paste] couldn't put files on the clipboard: {e}");
        }
    }
    false
}

fn clipboard_format(name: &str) -> u16 {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) as u16 }
}

fn format_etc(cf: u16, tymed: i32) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: tymed as u32 }
}

#[implement(IDataObject)]
struct FilesData {
    list: FileList,
    from: String,
    real_files: bool,
    cf_descriptor: u16,
    cf_contents: u16,
    cf_effect: u16,
    cf_marker: u16,
    /// Real-file mode: the top-level paths in the cache, once fetched.
    fetched: Mutex<Option<Vec<PathBuf>>>,
}

impl FilesData {
    fn new(list: FileList, from: String, real_files: bool) -> Self {
        Self {
            list,
            from,
            real_files,
            cf_descriptor: clipboard_format("FileGroupDescriptorW"),
            cf_contents: clipboard_format("FileContents"),
            cf_effect: clipboard_format("Preferred DropEffect"),
            cf_marker: clipboard_format(crate::clipboard::MARKER),
            fetched: Mutex::new(None),
        }
    }

    fn formats(&self) -> Vec<FORMATETC> {
        let mut formats = vec![format_etc(self.cf_effect, TYMED_HGLOBAL.0), format_etc(self.cf_marker, TYMED_HGLOBAL.0)];
        if self.real_files {
            formats.push(format_etc(CF_HDROP.0, TYMED_HGLOBAL.0));
        } else {
            formats.push(format_etc(self.cf_descriptor, TYMED_HGLOBAL.0));
            formats.push(format_etc(self.cf_contents, TYMED_ISTREAM.0));
        }
        formats
    }

    fn supports(&self, f: &FORMATETC) -> bool {
        self.formats().iter().any(|s| s.cfFormat == f.cfFormat && s.tymed & f.tymed != 0)
    }

    /// Real-file mode: fetch every file into the cache (once) and return the top-level paths.
    fn fetch_all(&self) -> Result<Vec<PathBuf>> {
        let mut fetched = self.fetched.lock().map_err(|_| Error::from(E_FAIL))?;
        if let Some(paths) = fetched.as_ref() {
            return Ok(paths.clone());
        }
        let root = cache_root().ok_or(Error::from(E_FAIL))?.join(self.list.offer.to_string());
        for (index, entry) in self.list.entries.iter().enumerate() {
            let target = root.join(&entry.path);
            if entry.dir {
                std::fs::create_dir_all(&target).map_err(|_| Error::from(E_FAIL))?;
            } else {
                download(&self.from, self.list.offer, index, &target).map_err(|e| {
                    if cfg!(debug_assertions) {
                        eprintln!("[paste] {}: {e}", entry.path);
                    }
                    Error::from(E_FAIL)
                })?;
            }
        }
        let top: Vec<PathBuf> =
            self.list.entries.iter().filter(|e| !e.path.contains('\\')).map(|e| root.join(&e.path)).collect();
        *fetched = Some(top.clone());
        Ok(top)
    }
}

/// Fetch one file from the PC that copied it into `target`.
fn download(from: &str, offer: u64, index: usize, target: &Path) -> std::result::Result<(), String> {
    use std::io::Write;
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut rx = net::fetch(from, offer, index);
    let (mut expected, mut written) = (None, 0u64);
    loop {
        match rx.blocking_recv() {
            Some(FetchMsg::Size(size)) => expected = Some(size),
            Some(FetchMsg::Data(bytes)) => {
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                written += bytes.len() as u64;
            }
            // Only a complete file counts.
            Some(FetchMsg::Done) if expected == Some(written) => return Ok(()),
            Some(FetchMsg::Done) => return Err("the file arrived incomplete".into()),
            Some(FetchMsg::Failed(e)) => return Err(e),
            None => return Err("connection closed".into()),
        }
    }
}

/// Bytes in a movable global memory block, as the clipboard expects.
fn hglobal(bytes: &[u8]) -> Result<STGMEDIUM> {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            return Err(E_FAIL.into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: mem },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

/// DROPFILES header followed by double-NUL-terminated wide paths.
fn drop_files(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(20u32.to_le_bytes()); // offset of the path list
    out.extend([0u8; 8]); // drop point
    out.extend(0u32.to_le_bytes()); // fNC
    out.extend(1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in p.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)) {
            out.extend(u.to_le_bytes());
        }
    }
    out.extend(0u16.to_le_bytes());
    out
}

/// FILEGROUPDESCRIPTORW: a count, then one FILEDESCRIPTORW per file or folder.
fn descriptors(list: &FileList) -> Vec<u8> {
    let mut out = (list.entries.len() as u32).to_le_bytes().to_vec();
    for entry in &list.entries {
        let mut fd = FILEDESCRIPTORW::default();
        fd.dwFlags = FD_ATTRIBUTES | FD_FILESIZE | FD_PROGRESSUI;
        fd.dwFileAttributes = if entry.dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        fd.nFileSizeHigh = (entry.size >> 32) as u32;
        fd.nFileSizeLow = entry.size as u32;
        for (i, u) in entry.path.encode_utf16().take(259).enumerate() {
            fd.cFileName[i] = u;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(&fd as *const FILEDESCRIPTORW as *const u8, std::mem::size_of::<FILEDESCRIPTORW>())
        };
        out.extend_from_slice(bytes);
    }
    out
}

impl IDataObject_Impl for FilesData_Impl {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        let f = unsafe { format.as_ref() }.ok_or(Error::from(DV_E_FORMATETC))?;
        if !self.supports(f) {
            return Err(DV_E_FORMATETC.into());
        }
        if f.cfFormat == self.cf_effect {
            return hglobal(&(DROPEFFECT_COPY.0 as u32).to_le_bytes());
        }
        if f.cfFormat == self.cf_marker {
            return hglobal(&[1]);
        }
        if f.cfFormat == CF_HDROP.0 {
            let paths = self.fetch_all();
            if cfg!(debug_assertions) {
                match &paths {
                    Ok(p) => eprintln!("[paste] real files ready: {} items", p.len()),
                    Err(e) => eprintln!("[paste] fetching files failed: {e}"),
                }
            }
            return hglobal(&drop_files(&paths?));
        }
        if f.cfFormat == self.cf_descriptor {
            return hglobal(&descriptors(&self.list));
        }
        if f.cfFormat == self.cf_contents {
            let index = usize::try_from(f.lindex).map_err(|_| Error::from(DV_E_FORMATETC))?;
            let entry = self.list.entries.get(index).filter(|e| !e.dir).ok_or(Error::from(DV_E_FORMATETC))?;
            let stream: IStream = NetStream::new(self.from.clone(), self.list.offer, index, entry.size).into();
            return Ok(STGMEDIUM {
                tymed: TYMED_ISTREAM.0 as u32,
                u: STGMEDIUM_0 { pstm: ManuallyDrop::new(Some(stream)) },
                pUnkForRelease: ManuallyDrop::new(None),
            });
        }
        Err(DV_E_FORMATETC.into())
    }

    fn GetDataHere(&self, _format: *const FORMATETC, _medium: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        match unsafe { format.as_ref() } {
            Some(f) if self.supports(f) => S_OK,
            _ => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _in: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if let Some(out) = unsafe { out.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    fn SetData(&self, _format: *const FORMATETC, _medium: *const STGMEDIUM, _release: BOOL) -> Result<()> {
        // Explorer reports things like "paste succeeded" here; nothing to do.
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }

    fn DAdvise(&self, _format: *const FORMATETC, _advf: u32, _sink: Option<&IAdviseSink>) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _connection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

/// One virtual file's data, read from the network as Explorer asks for it.
#[implement(IStream)]
struct NetStream {
    from: String,
    offer: u64,
    index: usize,
    size: u64,
    state: Mutex<StreamState>,
}

#[derive(Default)]
struct StreamState {
    rx: Option<tokio::sync::mpsc::Receiver<FetchMsg>>,
    buffer: Vec<u8>,
    taken: usize,
    pos: u64,
    started: bool,
    done: bool,
}

impl NetStream {
    fn new(from: String, offer: u64, index: usize, size: u64) -> Self {
        Self { from, offer, index, size, state: Mutex::new(StreamState::default()) }
    }
}

impl ISequentialStream_Impl for NetStream_Impl {
    fn Read(&self, pv: *mut core::ffi::c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        let Ok(mut st) = self.state.lock() else { return E_FAIL };
        if !st.started {
            if st.pos != 0 {
                return STG_E_INVALIDFUNCTION;
            }
            st.started = true;
            st.rx = Some(net::fetch(&self.from, self.offer, self.index));
        }
        let out = unsafe { std::slice::from_raw_parts_mut(pv as *mut u8, cb as usize) };
        let mut filled = 0;
        while filled < out.len() {
            if st.taken < st.buffer.len() {
                let n = (st.buffer.len() - st.taken).min(out.len() - filled);
                out[filled..filled + n].copy_from_slice(&st.buffer[st.taken..st.taken + n]);
                st.taken += n;
                filled += n;
                continue;
            }
            if st.done {
                break;
            }
            let next = st.rx.as_mut().and_then(|rx| rx.blocking_recv());
            match next {
                Some(FetchMsg::Size(_)) => {}
                Some(FetchMsg::Data(bytes)) => {
                    st.buffer = bytes;
                    st.taken = 0;
                }
                Some(FetchMsg::Done) => st.done = true,
                Some(FetchMsg::Failed(_)) | None => return E_FAIL,
            }
        }
        st.pos += filled as u64;
        if let Some(read) = unsafe { pcbread.as_mut() } {
            *read = filled as u32;
        }
        if filled < out.len() {
            S_FALSE
        } else {
            S_OK
        }
    }

    fn Write(&self, _pv: *const core::ffi::c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for NetStream_Impl {
    fn Seek(&self, offset: i64, origin: STREAM_SEEK, new_position: *mut u64) -> Result<()> {
        let mut st = self.state.lock().map_err(|_| Error::from(E_FAIL))?;
        let target = match origin {
            STREAM_SEEK_SET => offset,
            STREAM_SEEK_CUR => st.pos as i64 + offset,
            STREAM_SEEK_END => self.size as i64 + offset,
            _ => return Err(STG_E_INVALIDFUNCTION.into()),
        };
        // The data comes in order over the network: before reading starts any position
        // can be set (it must be back at 0 to read); after that, only "where am I".
        if target < 0 || (st.started && target as u64 != st.pos) {
            return Err(STG_E_INVALIDFUNCTION.into());
        }
        st.pos = target as u64;
        if let Some(p) = unsafe { new_position.as_mut() } {
            *p = st.pos;
        }
        Ok(())
    }

    fn SetSize(&self, _size: u64) -> Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }

    fn CopyTo(&self, _to: Option<&IStream>, _cb: u64, _read: *mut u64, _written: *mut u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Commit(&self, _flags: &STGC) -> Result<()> {
        Ok(())
    }

    fn Revert(&self) -> Result<()> {
        Ok(())
    }

    fn LockRegion(&self, _offset: u64, _cb: u64, _kind: &LOCKTYPE) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn UnlockRegion(&self, _offset: u64, _cb: u64, _kind: u32) -> Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn Stat(&self, stat: *mut STATSTG, _flag: &STATFLAG) -> Result<()> {
        let stat = unsafe { stat.as_mut() }.ok_or(Error::from(E_FAIL))?;
        *stat = STATSTG::default();
        stat.r#type = STGTY_STREAM.0 as u32;
        stat.cbSize = self.size;
        Ok(())
    }

    fn Clone(&self) -> Result<IStream> {
        Err(E_NOTIMPL.into())
    }
}
