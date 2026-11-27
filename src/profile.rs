//! Where HomeKVM keeps its files, and a dev-only second profile.
//!
//! In development builds, `HOMEKVM_PROFILE=b` runs a second, fully separate copy on
//! the same PC (its own settings, identity, paired PCs and tray icon), so pairing can
//! be tested without a second computer. Release builds ignore it.

use std::path::PathBuf;

pub fn name() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("HOMEKVM_PROFILE")
        .ok()
        .filter(|s| !s.is_empty() && s.len() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// "" normally, "-b" for profile b.
pub fn suffix() -> String {
    name().map(|n| format!("-{n}")).unwrap_or_default()
}

/// %APPDATA%\HomeKVM (or HomeKVM-b for profile b).
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join(format!("HomeKVM{}", suffix())))
}

/// Window title: "HomeKVM", or "HomeKVM (b)" for profile b.
pub fn title() -> String {
    match name() {
        Some(n) => format!("HomeKVM ({n})"),
        None => "HomeKVM".into(),
    }
}
