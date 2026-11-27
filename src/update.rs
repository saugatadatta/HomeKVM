//! Keeping HomeKVM up to date from GitHub Releases.
//!
//! Once a day (and on "Check now") HomeKVM asks GitHub for the latest release. When it's
//! newer, it downloads that release's `HomeKVM.exe` and checks it: the SHA-256 must
//! match the `HomeKVM.exe.sha256` published with the release, and if this copy is
//! code-signed, the new one must be validly signed too. Then, at a quiet moment (the
//! keyboard and mouse aren't being shared right now), it swaps the exe and restarts:
//!
//! 1. rename the running exe to `HomeKVM.old.exe` (Windows allows renaming a running exe),
//! 2. put the new exe in its place,
//! 3. start it with `--after-update` and quit,
//! 4. the new version deletes `HomeKVM.old.exe` when it starts.
//!
//! Updates are always installed; there's no setting to skip them. Development builds
//! only check and log, and never install.

use crate::state::Shared;
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

/// Ask for an update check right away (Settings, "Check now").
pub fn check_now() {
    CHECK_NOW.store(true, SeqCst);
}

/// Start the background updater. `redraw` refreshes the window when the status changes.
pub fn start(shared: Arc<Shared>, redraw: Arc<dyn Fn() + Send + Sync>) {
    std::thread::Builder::new()
        .name("update".into())
        .spawn(move || run(shared, redraw))
        .ok();
}

fn run(shared: Arc<Shared>, redraw: Arc<dyn Fn() + Send + Sync>) {
    remove_old_exe();
    let set = |text: String| {
        shared.set_update_status(text);
        redraw();
    };
    let started = Instant::now();
    let mut last_check: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let due = match last_check {
            None => started.elapsed() >= FIRST_CHECK,
            Some(t) => t.elapsed() >= CHECK_EVERY,
        };
        if !(due || CHECK_NOW.swap(false, SeqCst)) {
            continue;
        }
        last_check = Some(Instant::now());
        set("Checking for updates…".into());
        match check_and_download(&set) {
            Ok(None) => set(format!("Up to date ({})", env!("CARGO_PKG_VERSION"))),
            Ok(Some((version, exe))) if INSTALLS => {
                set(format!("Installing {version}…"));
                wait_for_quiet_moment(&shared);
                if let Err(e) = install(&exe) {
                    set(format!("Couldn't install {version}: {e}"));
                }
            }
            Ok(Some((version, exe))) => {
                set(format!("{version} downloaded and verified (dev build: not installed)"));
                eprintln!("[update] {version} verified at {}", exe.display());
            }
            Err(e) => set(format!("Update check failed: {e}")),
        }
    }
}

/// Install only while nobody is using the shared keyboard and mouse.
fn wait_for_quiet_moment(shared: &Shared) {
    loop {
        let net = shared.net();
        if net.controlling.is_none() && net.controlled_by.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_secs(30));
    }
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

/// Returns the new version and the path of its verified exe, or None when up to date.
fn check_and_download(set: &dyn Fn(String)) -> Result<Option<(String, PathBuf)>, String> {
    let body = https_get(LATEST, 1024 * 1024)?;
    let release: Release = serde_json::from_slice(&body).map_err(|_| "unexpected reply from GitHub".to_string())?;
    let latest = parse_version(&release.tag_name).ok_or("unexpected version on GitHub")?;
    let current = parse_version(env!("CARGO_PKG_VERSION")).ok_or("bad own version")?;
    if latest <= current {
        return Ok(None);
    }
    let version = release.tag_name.trim_start_matches('v').to_string();
    let url = |name: &str| release.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
    let exe_url = url(ASSET).ok_or("the release has no HomeKVM.exe")?;
    let sum_url = url(CHECKSUM).ok_or("the release has no checksum")?;

    set(format!("Downloading {version}…"));
    let expected = String::from_utf8_lossy(&https_get(&sum_url, 1024)?).trim().to_lowercase();
    let exe = https_get(&exe_url, MAX_EXE)?;
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
    Ok(Some((version, path)))
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
    std::process::Command::new(&me).arg(AFTER_UPDATE).spawn().map_err(|e| e.to_string())?;
    crate::tray::quit();
    Ok(())
}

/// After an update, delete the previous version (it may take a moment to be released).
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
/// certificate store). Follows GitHub's download redirects.
fn https_get(url: &str, max: usize) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
        WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    };
    const ACCESS_AUTOMATIC_PROXY: u32 = 4;
    const FLAG_SECURE: u32 = 0x0080_0000;
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
        let (mut status, mut size) = (0u32, 4u32);
        WinHttpQueryHeaders(
            request.0,
            QUERY_STATUS_CODE | QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
        );
        if status != 200 {
            return Err(format!("GitHub answered {status}"));
        }
        let mut body = Vec::new();
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
        }
        Ok(body)
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
        let body = super::https_get(super::LATEST, 1024 * 1024).expect("GitHub reachable");
        let release: super::Release = serde_json::from_slice(&body).expect("release JSON");
        println!("latest: {} with {:?}", release.tag_name, release.assets.iter().map(|a| &a.name).collect::<Vec<_>>());
        assert!(parse_version(&release.tag_name).is_some());
    }
}
