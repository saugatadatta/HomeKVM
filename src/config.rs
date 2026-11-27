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
    /// Copies up to this total size paste as real files (every app); bigger ones as
    /// virtual files that stream straight into a folder. 10 to 200 MB.
    pub real_file_limit_mb: u32,
    pub start_at_login: bool,
    /// Wait briefly at the screen edge before switching PCs.
    pub edge_delay: bool,
    pub layout: Layout,
}

/// The row of screens, as paired-PC IDs. Each list starts with the PC next to this
/// one and goes outward.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Layout {
    pub left: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    NotPlaced,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Left => "Left side",
            Side::Right => "Right side",
            Side::NotPlaced => "Not placed",
        }
    }
}

impl Layout {
    pub fn side_of(&self, id: &str) -> Side {
        if self.left.iter().any(|x| x == id) {
            Side::Left
        } else if self.right.iter().any(|x| x == id) {
            Side::Right
        } else {
            Side::NotPlaced
        }
    }

    /// Move a PC to a side (at the outer end), or take it out of the row.
    pub fn place(&mut self, id: &str, side: Side) {
        self.left.retain(|x| x != id);
        self.right.retain(|x| x != id);
        match side {
            Side::Left => self.left.push(id.to_string()),
            Side::Right => self.right.push(id.to_string()),
            Side::NotPlaced => {}
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let pc = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into());
        Self {
            device_name: match crate::profile::name() {
                Some(p) => format!("{pc} ({p})"),
                None => pc,
            },
            share_input: None,
            clipboard_text: true,
            clipboard_images: true,
            send_files: true,
            real_file_limit_mb: 50,
            // Off until the user turns it on (see load()).
            start_at_login: false,
            edge_delay: false,
            layout: Layout::default(),
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(crate::profile::data_dir()?.join("config.toml"))
}

pub fn load() -> Config {
    let saved: Option<Config> =
        path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| toml::from_str(&s).ok());
    let mut cfg = match saved {
        Some(cfg) => cfg,
        None => Config {
            // Nothing saved yet: start at sign-in stays off for a new install, so the first
            // start never writes to the Run key. A PC that an older version (where it was
            // on by default) already registered keeps starting at sign-in.
            start_at_login: crate::autostart::is_registered(),
            ..Config::default()
        },
    };
    cfg.real_file_limit_mb = (cfg.real_file_limit_mb / 10 * 10).clamp(10, 200);
    cfg
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
