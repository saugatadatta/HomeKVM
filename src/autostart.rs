//! Start at sign-in, without admin rights or a service:
//! - the standalone exe uses the per-user Run key;
//! - the Microsoft Store version uses its package's startup task (the only way the
//!   Store allows), which Windows also lists under Settings, Apps, Startup.

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "HomeKVM";
/// Must match the TaskId in packaging/AppxManifest.xml.
const STARTUP_TASK_ID: &str = "HomeKVMStartup";

/// Development builds of the standalone exe never touch the Run key, so testing doesn't
/// register a dev build to start at sign-in.
pub const APPLIES: bool = !cfg!(debug_assertions);

/// HomeKVM is already set to start at sign-in (by any version). Read-only.
pub fn is_registered() -> bool {
    if crate::win::is_packaged() {
        return startup_task_enabled().unwrap_or(false);
    }
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|run| run.get_value::<String, _>(VALUE))
        .is_ok()
}

pub fn apply(enable: bool) -> Result<(), String> {
    if crate::win::is_packaged() {
        return apply_startup_task(enable);
    }
    if !APPLIES {
        return Ok(());
    }
    let run = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
        .map_err(|e| e.to_string())?;
    if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = format!("\"{}\"", exe.display());
        let current: Option<String> = run.get_value(VALUE).ok();
        if current.as_deref() != Some(command.as_str()) {
            run.set_value(VALUE, &command).map_err(|e| e.to_string())?;
        }
        Ok(())
    } else {
        match run.delete_value(VALUE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn startup_task() -> windows::core::Result<windows::ApplicationModel::StartupTask> {
    windows::ApplicationModel::StartupTask::GetAsync(&windows::core::HSTRING::from(STARTUP_TASK_ID))?.get()
}

fn startup_task_enabled() -> windows::core::Result<bool> {
    use windows::ApplicationModel::StartupTaskState;
    let state = startup_task()?.State()?;
    Ok(state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy)
}

fn apply_startup_task(enable: bool) -> Result<(), String> {
    use windows::ApplicationModel::StartupTaskState;
    let task = startup_task().map_err(|e| e.to_string())?;
    if !enable {
        return task.Disable().map_err(|e| e.to_string());
    }
    match task.RequestEnableAsync().and_then(|op| op.get()).map_err(|e| e.to_string())? {
        StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Ok(()),
        StartupTaskState::DisabledByUser => {
            Err("turned off in Windows Settings > Apps > Startup; turn HomeKVM on there".into())
        }
        StartupTaskState::DisabledByPolicy => Err("turned off by a policy on this PC".into()),
        _ => Err("Windows didn't allow it".into()),
    }
}
