use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Settings saved to %APPDATA%\HomeKVM\config.toml.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    /// Name other PCs see.
    pub device_name: String,
    /// Share this PC's keyboard and mouse with others.
    /// `None` follows the suggestion from the attached devices.
    pub share_input: Option<bool>,
    pub clipboard_text: bool,
    pub clipboard_images: bool,
    pub send_files: bool,
    pub max_file_mb: u32,
    pub start_at_login: bool,
    /// Wait briefly at the screen edge before switching PCs.
    pub edge_delay: bool,
    pub layout: Layout,
}

/// The row of screens. Each list starts with the PC next to this one and goes outward.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Layout {
    pub left: Vec<String>,
    pub right: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into()),
            share_input: None,
            clipboard_text: true,
            clipboard_images: true,
            send_files: true,
            max_file_mb: 2048,
            start_at_login: true,
            edge_delay: false,
            layout: Layout::default(),
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("HomeKVM").join("config.toml"))
}

pub fn load() -> Config {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let p = path().ok_or("APPDATA is not set")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    // Write a temp file first so a crash never leaves a half-written config.
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}
