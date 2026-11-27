//! The PCs this PC trusts, saved to peers.toml next to the settings.

use crate::profile;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Paired {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    peers: Vec<Paired>,
}

fn path() -> Option<std::path::PathBuf> {
    Some(profile::data_dir()?.join("peers.toml"))
}

pub fn load() -> Vec<Paired> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str::<File>(&s).ok())
        .map(|f| f.peers)
        .unwrap_or_default()
}

pub fn save(peers: &[Paired]) -> Result<(), String> {
    let p = path().ok_or("APPDATA is not set")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(&File { peers: peers.to_vec() }).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}
