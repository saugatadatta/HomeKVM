//! Keeping HomeKVM up to date: from GitHub Releases for the standalone exe, and through
//! the Microsoft Store for the Store version. Both go through the same steps, shown in
//! the Updates panel:
//!
//! 1. **Checking**: a minute after start, then once a day, and on "Check now".
//! 2. **Downloading**, with progress. GitHub: the release's `HomeKVM.exe`, which must
//!    match the `HomeKVM.exe.sha256` published with the release (and, if this copy is
//!    code-signed, be validly signed too). Store: the Store downloads the new package; if
//!    it can't do that silently, the panel asks first ("Download") and the Store shows its
//!    own prompt.
//! 3. **Ready**: nothing is installed until the user clicks "Restart to update".
//! 4. **Installing**. GitHub: rename the running exe to `HomeKVM.old.exe` (Windows allows
//!    renaming a running exe), put the new one in its place, start it with
//!    `--after-update` and quit; the new version deletes `HomeKVM.old.exe`. Store: the
//!    Store installs the package, and Windows starts HomeKVM again (it's registered for
//!    restart).
//!
//! Development builds go through the same steps but never install.

use crate::state::{Shared, UpdateState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LATEST: &str = "https://api.github.com/repos/saugatadatta/HomeKVM/releases/latest";
const ASSET: &str = "HomeKVM.exe";
const CHECKSUM: &str = "HomeKVM.exe.sha256";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The exe is about 7 MB; anything far bigger isn't ours.
const MAX_EXE: usize = 64 * 1024 * 1024;
pub const AFTER_UPDATE: &str = "--after-update";

/// Only release builds replace themselves.
const INSTALLS: bool = !cfg!(debug_assertions);

static CHECK_NOW: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_NOW: AtomicBool = AtomicBool::new(false);
static INSTALL_NOW: AtomicBool = AtomicBool::new(false);

/// "Check now" (and "Try again").
pub fn check_now() {
    CHECK_NOW.store(true, SeqCst);
}

/// "Download": the Store version, when the Store needs the user's OK to download.
pub fn download_now() {
    DOWNLOAD_NOW.store(true, SeqCst);
}

/// "Restart to update": install the downloaded update and restart.
pub fn install_now() {
    INSTALL_NOW.store(true, SeqCst);
}

type Set = Arc<dyn Fn(UpdateState) + Send + Sync>;

/// Start the background updater. `redraw` refreshes the window when the state changes.
pub fn start(shared: Arc<Shared>, redraw: Arc<dyn Fn() + Send + Sync>) {
    let set: Set = Arc::new(move |state| {
        shared.set_update(state);
        redraw();
    });
    let packaged = crate::win::is_packaged();
    let simulate = cfg!(debug_assertions) && std::env::var_os("HOMEKVM_UPDATE_DEMO").is_some();
    std::thread::Builder::new()
        .name("update".into())
        .spawn(move || {
            if simulate {
                simulate_steps(set)
            } else if packaged {
                store::run(set)
            } else {
                run_github(set)
            }
        })
        .ok();
}

/// Dev builds only (HOMEKVM_UPDATE_DEMO=1): walks the Updates panel through every step
/// with a pretend version, without the network, to check how each step looks.
fn simulate_steps(set: Set) {
    let version = "0.10.9".to_string();
    eprintln!("[update] simulating the update steps");
    // Time to open the window first.
    std::thread::sleep(Duration::from_secs(9));
    loop {
        std::thread::sleep(Duration::from_secs(3));
        set(UpdateState::Checking);
        std::thread::sleep(Duration::from_millis(2500));
        for percent in (0..=100).step_by(2) {
            set(UpdateState::Downloading { version: version.clone(), percent });
            std::thread::sleep(Duration::from_millis(80));
        }
        set(UpdateState::Ready(version.clone()));
        CHECK_NOW.store(false, SeqCst);
        INSTALL_NOW.store(false, SeqCst);
        while !INSTALL_NOW.swap(false, SeqCst) && !CHECK_NOW.swap(false, SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        set(UpdateState::Installing(version.clone()));
        std::thread::sleep(Duration::from_secs(3));
        set(UpdateState::Failed("dev build: updates aren't installed".into()));
        while !CHECK_NOW.swap(false, SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// When the next check is due: a minute after start, then once a day.
struct Schedule {
    started: Instant,
    last: Option<Instant>,
}

impl Schedule {
    fn new() -> Self {
        Self { started: Instant::now(), last: None }
    }

    /// True when a check should run now (also on "Check now").
    fn due(&mut self) -> bool {
        let due = match self.last {
            None => self.started.elapsed() >= FIRST_CHECK,
            Some(t) => t.elapsed() >= CHECK_EVERY,
        };
        if due || CHECK_NOW.swap(false, SeqCst) {
            self.last = Some(Instant::now());
            return true;
        }
        false
    }
}

// ---- GitHub Releases --------------------------------------------------------------------

fn run_github(set: Set) {
    remove_old_exe();
    let mut schedule = Schedule::new();
    // The downloaded, verified update waiting for "Restart to update".
    let mut ready: Option<(String, PathBuf)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        DOWNLOAD_NOW.store(false, SeqCst); // only the Store version asks first
        if INSTALL_NOW.swap(false, SeqCst) {
            if let Some((version, exe)) = ready.take() {
                if INSTALLS {
                    set(UpdateState::Installing(version.clone()));
                    if let Err(e) = install(&exe) {
                        set(UpdateState::Failed(format!("couldn't install {version}: {e}")));
                    }
                } else {
                    eprintln!("[update] dev build: {version} verified at {}, not installed", exe.display());
                    set(UpdateState::Failed("dev build: updates aren't installed".into()));
                }
            }
            continue;
        }
        if !schedule.due() {
            continue;
        }
        set(UpdateState::Checking);
        let staged = ready.as_ref().map(|(v, _)| v.as_str());
        match check_and_download(&set, staged) {
            Ok(Found::UpToDate) => set(UpdateState::UpToDate),
            Ok(Found::AlreadyReady) => set(UpdateState::Ready(staged.unwrap_or_default().to_string())),
            Ok(Found::Downloaded(version, path)) => {
                if let Some((_, old)) = ready.replace((version.clone(), path)) {
                    let _ = std::fs::remove_file(old);
                }
                set(UpdateState::Ready(version));
            }
            // A failed re-check doesn't lose an update that's already waiting.
            Err(e) => match &ready {
                Some((version, _)) => set(UpdateState::Ready(version.clone())),
                None => set(UpdateState::Failed(e)),
            },
        }
    }
}

enum Found {
    UpToDate,
    /// The latest version is the one already downloaded.
    AlreadyReady,
    Downloaded(String, PathBuf),
}

/// "0.6.0" or "v0.6.0" -> (0, 6, 0).
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Downloads and verifies a newer release, reporting progress.
fn check_and_download(set: &Set, staged: Option<&str>) -> Result<Found, String> {
    let body = https_get(LATEST, 1024 * 1024, None)?;
    let release: Release = serde_json::from_slice(&body).map_err(|_| "unexpected reply from GitHub".to_string())?;
    let latest = parse_version(&release.tag_name).ok_or("unexpected version on GitHub")?;
    let current = parse_version(env!("CARGO_PKG_VERSION")).ok_or("bad own version")?;
    if latest <= current {
        return Ok(Found::UpToDate);
    }
    let version = release.tag_name.trim_start_matches('v').to_string();
    if staged == Some(version.as_str()) {
        return Ok(Found::AlreadyReady);
    }
    let url = |name: &str| release.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    let exe_url = url(ASSET).ok_or("the release has no HomeKVM.exe")?;
    let sum_url = url(CHECKSUM).ok_or("the release has no checksum")?;

    set(UpdateState::Downloading { version: version.clone(), percent: 0 });
    let expected = String::from_utf8_lossy(&https_get(&sum_url, 1024, None)?).trim().to_lowercase();
    let mut shown = 0u8;
    let mut progress = |done: usize, total: usize| {
        if total > 0 {
            let percent = (done * 100 / total).min(100) as u8;
            if percent != shown {
                shown = percent;
                set(UpdateState::Downloading { version: version.clone(), percent });
            }
        }
    };
    let exe = https_get(&exe_url, MAX_EXE, Some(&mut progress))?;
    let actual: String =
        ring::digest::digest(&ring::digest::SHA256, &exe).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    if !expected.starts_with(&actual) {
        return Err("the download didn't match its checksum".into());
    }

    let dir = update_dir().ok_or("no local app data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("HomeKVM-{version}.exe"));
    std::fs::write(&path, &exe).map_err(|e| e.to_string())?;

    // A signed HomeKVM only ever replaces itself with a signed HomeKVM.
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if signature_valid(&me) && !signature_valid(&path) {
        let _ = std::fs::remove_file(&path);
        return Err("the new version isn't properly signed".into());
    }
    Ok(Found::Downloaded(version, path))
}

fn update_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(format!("HomeKVM{}", crate::profile::suffix())).join("update"))
}

fn old_exe_path(exe: &Path) -> PathBuf {
    exe.with_file_name("HomeKVM.old.exe")
}

/// Swap the running exe for the new one, start it and quit.
fn install(new_exe: &Path) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = old_exe_path(&me);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&me, &old).map_err(|e| format!("can't rename the running app ({e})"))?;
    if let Err(e) = std::fs::copy(new_exe, &me) {
        // Put the old one back so HomeKVM still starts next time.
        let _ = std::fs::rename(&old, &me);
        return Err(format!("can't write the new version ({e})"));
    }
    let _ = std::fs::remove_file(new_exe);
    // Hand over: release the single-instance lock first, so the new version doesn't have
    // to wait for this one to finish exiting.
    crate::win::release_single_instance();
    if let Err(e) = std::process::Command::new(&me).arg(AFTER_UPDATE).spawn() {
        // Couldn't start it: keep running this version rather than leave nothing running.
        crate::win::acquire_single_instance();
        return Err(e.to_string());
    }
    crate::tray::quit();
    Ok(())
}

/// After an update, delete the previous version (it may take a moment to be released),
/// and any download left from before.
fn remove_old_exe() {
    let Ok(me) = std::env::current_exe() else { return };
    let old = old_exe_path(&me);
    for _ in 0..30 {
        if !old.exists() || std::fs::remove_file(&old).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if let Some(dir) = update_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Windows' own check of a file's Authenticode signature.
fn signature_valid(path: &Path) -> bool {
    use windows_sys::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
        WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut file: WINTRUST_FILE_INFO = std::mem::zeroed();
        file.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
        file.pcwszFilePath = wide.as_ptr();
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut file;
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let result = WinVerifyTrust(std::ptr::null_mut(), &mut action, (&mut data as *mut WINTRUST_DATA).cast());
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(std::ptr::null_mut(), &mut action, (&mut data as *mut WINTRUST_DATA).cast());
        result == 0
    }
}

/// A small HTTPS GET with Windows' own HTTP stack (system proxy settings and
/// certificate store). Follows GitHub's download redirects. `progress` gets the bytes
/// read so far and the total size (0 when the server doesn't say).
fn https_get(url: &str, max: usize, mut progress: Option<&mut dyn FnMut(usize, usize)>) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
        WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    };
    const ACCESS_AUTOMATIC_PROXY: u32 = 4;
    const FLAG_SECURE: u32 = 0x0080_0000;
    const QUERY_CONTENT_LENGTH: u32 = 5;
    const QUERY_STATUS_CODE: u32 = 19;
    const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let rest = url.strip_prefix("https://").ok_or("only https is allowed")?;
    let (host, path) = match rest.split_once('/') {
        Some((h, p)) => (h.to_string(), format!("/{p}")),
        None => (rest.to_string(), "/".to_string()),
    };
    let failed = |what: &str| format!("couldn't reach GitHub ({what})");
    unsafe {
        let session = Handle(WinHttpOpen(
            wide("HomeKVM-updater").as_ptr(),
            ACCESS_AUTOMATIC_PROXY as _,
            std::ptr::null(),
            std::ptr::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(failed("open"));
        }
        let connection = Handle(WinHttpConnect(session.0, wide(&host).as_ptr(), 443, 0));
        if connection.0.is_null() {
            return Err(failed("connect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(&path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            FLAG_SECURE as _,
        ));
        if request.0.is_null() {
            return Err(failed("request"));
        }
        let headers = wide("Accept: application/vnd.github+json, application/octet-stream\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0
        {
            return Err(failed("send"));
        }
        let number = |query: u32| -> u32 {
            let (mut value, mut size) = (0u32, 4u32);
            WinHttpQueryHeaders(
                request.0,
                query | QUERY_FLAG_NUMBER,
                std::ptr::null(),
                (&mut value as *mut u32).cast(),
                &mut size,
                std::ptr::null_mut(),
            );
            value
        };
        let status = number(QUERY_STATUS_CODE);
        if status != 200 {
            return Err(format!("GitHub answered {status}"));
        }
        let total = number(QUERY_CONTENT_LENGTH) as usize;
        if total > max {
            return Err("the download is too large".into());
        }
        let mut body = Vec::with_capacity(total.min(max));
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(failed("read"));
            }
            if available == 0 {
                break;
            }
            let start = body.len();
            if start + available as usize > max {
                return Err("the download is too large".into());
            }
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(request.0, body[start..].as_mut_ptr().cast(), available, &mut read) == 0 {
                return Err(failed("read"));
            }
            body.truncate(start + read as usize);
            if let Some(report) = progress.as_mut() {
                report(body.len(), total);
            }
        }
        Ok(body)
    }
}

// ---- Microsoft Store --------------------------------------------------------------------

/// The Store version: the Store finds, downloads and installs updates (the only way Store
/// rules allow), through Windows' StoreContext.
mod store {
    use super::{Schedule, Set, DOWNLOAD_NOW, INSTALL_NOW};
    use crate::state::UpdateState;
    use std::sync::atomic::Ordering::SeqCst;
    use std::time::Duration;
    use windows::core::Interface;
    use windows::Foundation::Collections::IVectorView;
    use windows::Foundation::{AsyncOperationProgressHandler, IAsyncOperationWithProgress};
    use windows::Services::Store::{
        StoreContext, StorePackageUpdate, StorePackageUpdateResult, StorePackageUpdateState, StorePackageUpdateStatus,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Shell::IInitializeWithWindow;

    /// A Store update this PC can get.
    struct Found {
        context: StoreContext,
        updates: IVectorView<StorePackageUpdate>,
        version: String,
        downloaded: bool,
    }

    type Operation = IAsyncOperationWithProgress<StorePackageUpdateResult, StorePackageUpdateStatus>;

    pub fn run(set: Set) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut schedule = Schedule::new();
        let mut found: Option<Found> = None;
        loop {
            std::thread::sleep(Duration::from_millis(300));
            if DOWNLOAD_NOW.swap(false, SeqCst) {
                if let Some(f) = found.as_mut().filter(|f| !f.downloaded) {
                    match download(f, false, &set) {
                        Ok(()) => {
                            f.downloaded = true;
                            set(UpdateState::Ready(f.version.clone()));
                        }
                        Err(e) => set(UpdateState::Failed(e)),
                    }
                }
                continue;
            }
            if INSTALL_NOW.swap(false, SeqCst) {
                if let Some(f) = found.as_ref().filter(|f| f.downloaded) {
                    set(UpdateState::Installing(f.version.clone()));
                    restart_after_update();
                    // On success the Store closes HomeKVM to install, and Windows starts it again.
                    if let Err(e) = install(f, &set) {
                        set(UpdateState::Failed(e));
                    }
                }
                continue;
            }
            if !schedule.due() {
                continue;
            }
            set(UpdateState::Checking);
            match check() {
                Ok(None) => {
                    found = None;
                    set(UpdateState::UpToDate);
                }
                Ok(Some(mut f)) => {
                    if found.as_ref().is_some_and(|old| old.downloaded && old.version == f.version) {
                        set(UpdateState::Ready(f.version.clone()));
                        continue;
                    }
                    // Downloads by itself when the Store allows it; otherwise it asks first.
                    if f.context.CanSilentlyDownloadStorePackageUpdates().unwrap_or(false) {
                        match download(&f, true, &set) {
                            Ok(()) => {
                                f.downloaded = true;
                                set(UpdateState::Ready(f.version.clone()));
                            }
                            Err(e) => set(UpdateState::Failed(e)),
                        }
                    } else {
                        set(UpdateState::Available(f.version.clone()));
                    }
                    found = Some(f);
                }
                Err(e) => set(UpdateState::Failed(e)),
            }
        }
    }

    fn message(e: windows::core::Error) -> String {
        format!("the Microsoft Store said: {}", e.message())
    }

    /// A Store context tied to the main window, so the Store's prompts appear over it.
    fn context() -> windows::core::Result<StoreContext> {
        let context = StoreContext::GetDefault()?;
        let hwnd = crate::win::main_hwnd();
        if hwnd != 0 {
            let init: IInitializeWithWindow = context.cast()?;
            unsafe {
                init.Initialize(HWND(hwnd as _))?;
            }
        }
        Ok(context)
    }

    fn check() -> Result<Option<Found>, String> {
        let context = context().map_err(message)?;
        let updates = context.GetAppAndOptionalStorePackageUpdatesAsync().and_then(|op| op.get()).map_err(message)?;
        if updates.Size().map_err(message)? == 0 {
            return Ok(None);
        }
        let version = updates
            .GetAt(0)
            .and_then(|u| u.Package())
            .and_then(|p| p.Id())
            .and_then(|id| id.Version())
            .map(|v| format!("{}.{}.{}", v.Major, v.Minor, v.Build))
            .unwrap_or_else(|_| "a new version".into());
        Ok(Some(Found { context, updates, version, downloaded: false }))
    }

    /// The Store reports download progress from 0 to 0.8, then installing to 1.
    fn watch(op: &Operation, version: &str, installing: bool, set: &Set) {
        let (version, set) = (version.to_string(), set.clone());
        let mut shown = u8::MAX;
        let _ = op.SetProgress(&AsyncOperationProgressHandler::new(move |_, status: &StorePackageUpdateStatus| {
            if installing {
                set(UpdateState::Installing(version.clone()));
            } else {
                let percent = (status.TotalDownloadProgress / 0.8 * 100.0).clamp(0.0, 100.0) as u8;
                if percent != shown {
                    shown = percent;
                    set(UpdateState::Downloading { version: version.clone(), percent });
                }
            }
            Ok(())
        }));
    }

    fn finished(op: Operation) -> Result<(), String> {
        let result = op.get().map_err(message)?;
        match result.OverallState().map_err(message)? {
            StorePackageUpdateState::Canceled => Err("the update was canceled".into()),
            StorePackageUpdateState::ErrorLowBattery => Err("the battery is too low; plug in and try again".into()),
            StorePackageUpdateState::ErrorWiFiRequired | StorePackageUpdateState::ErrorWiFiRecommended => {
                Err("the Store wants Wi-Fi for this download".into())
            }
            StorePackageUpdateState::OtherError => Err("the Microsoft Store couldn't download it".into()),
            _ => Ok(()),
        }
    }

    /// Download only; `silent` when the Store allows it without asking.
    fn download(f: &Found, silent: bool, set: &Set) -> Result<(), String> {
        set(UpdateState::Downloading { version: f.version.clone(), percent: 0 });
        let op = if silent {
            f.context.TrySilentDownloadStorePackageUpdatesAsync(&f.updates)
        } else {
            f.context.RequestDownloadStorePackageUpdatesAsync(&f.updates)
        }
        .map_err(message)?;
        watch(&op, &f.version, false, set);
        finished(op)
    }

    /// Install what was downloaded (the download part is already done, so this is quick).
    fn install(f: &Found, set: &Set) -> Result<(), String> {
        let op = if f.context.CanSilentlyDownloadStorePackageUpdates().unwrap_or(false) {
            f.context.TrySilentDownloadAndInstallStorePackageUpdatesAsync(&f.updates)
        } else {
            f.context.RequestDownloadAndInstallStorePackageUpdatesAsync(&f.updates)
        }
        .map_err(message)?;
        watch(&op, &f.version, true, set);
        finished(op)
    }

    /// Ask Windows to start HomeKVM again after the Store closes it to install.
    fn restart_after_update() {
        let args: Vec<u16> = super::AFTER_UPDATE.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            windows_sys::Win32::System::Recovery::RegisterApplicationRestart(args.as_ptr(), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.6.0"), Some((0, 6, 0)));
        assert_eq!(parse_version("0.10.2"), Some((0, 10, 2)));
        assert!(parse_version("v0.7.0") > parse_version("0.6.9"));
        assert_eq!(parse_version("latest"), None);
    }

    /// Talks to GitHub: run with `cargo test live_release -- --ignored`.
    #[test]
    #[ignore]
    fn live_release() {
        let body = super::https_get(super::LATEST, 1024 * 1024, None).expect("GitHub reachable");
        let release: super::Release = serde_json::from_slice(&body).expect("release JSON");
        println!("latest: {} with {:?}", release.tag_name, release.assets.iter().map(|a| &a.name).collect::<Vec<_>>());
        assert!(parse_version(&release.tag_name).is_some());
    }
}
