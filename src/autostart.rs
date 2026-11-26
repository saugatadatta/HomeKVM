//! Start at sign-in through the per-user Run key: no admin rights, no service.

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "HomeKVM";

/// Development builds never touch the Run key, so testing doesn't register
/// a dev build to start at sign-in.
pub const APPLIES: bool = !cfg!(debug_assertions);

pub fn apply(enable: bool) -> Result<(), String> {
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
