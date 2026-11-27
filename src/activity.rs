//! What happened lately with copies and file pastes, for Settings > Advanced >
//! Diagnostics. Only kinds, sizes and times are kept: never the text, image or file names.

use crate::clipboard::Clip;
use std::sync::Mutex;
use std::time::Instant;

/// The dot in front of each diagnostics line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Health {
    /// Working.
    Ok,
    /// Waiting for something (a PC to connect, the first key).
    Waiting,
    /// Something went wrong; the line says what.
    Failed,
    /// Not used yet.
    Idle,
}

impl Health {
    pub fn word(self) -> &'static str {
        match self {
            Health::Ok => "ok",
            Health::Waiting => "waiting",
            Health::Failed => "failed",
            Health::Idle => "idle",
        }
    }
}

struct Event {
    what: String,
    at: Instant,
}

struct State {
    clip_sent: Option<Event>,
    clip_got: Option<Event>,
    files_got: u32,
    files_got_bytes: u64,
    files_sent: u32,
    files_sent_bytes: u64,
    last_file_ok: Option<Instant>,
    file_error: Option<Event>,
}

static STATE: Mutex<State> = Mutex::new(State {
    clip_sent: None,
    clip_got: None,
    files_got: 0,
    files_got_bytes: 0,
    files_sent: 0,
    files_sent_bytes: 0,
    last_file_ok: None,
    file_error: None,
});

fn with(f: impl FnOnce(&mut State)) {
    if let Ok(mut s) = STATE.lock() {
        f(&mut s);
    }
}

/// A copy made here went to `peers` connected PCs.
pub fn clip_sent(clip: &Clip, peers: usize) {
    let mut what = kinds(clip);
    if peers == 0 {
        what.push_str(" (no PC connected)");
    }
    with(|s| s.clip_sent = Some(Event { what, at: Instant::now() }));
}

/// A copy from another PC was put on this PC's clipboard.
pub fn clip_got(clip: &Clip) {
    let what = kinds(clip);
    with(|s| s.clip_got = Some(Event { what, at: Instant::now() }));
}

/// A file pasted here arrived in full.
pub fn file_got(bytes: u64) {
    with(|s| {
        s.files_got += 1;
        s.files_got_bytes += bytes;
        s.last_file_ok = Some(Instant::now());
    });
}

/// Another PC pasted a file copied here, and it was sent in full.
pub fn file_sent(bytes: u64) {
    with(|s| {
        s.files_sent += 1;
        s.files_sent_bytes += bytes;
        s.last_file_ok = Some(Instant::now());
    });
}

/// Pasting a file here failed.
pub fn file_failed(reason: &str) {
    let what = reason.to_string();
    with(|s| s.file_error = Some(Event { what, at: Instant::now() }));
}

/// The "Copy sent" line: the last copy made here.
pub fn copy_sent_row() -> (Health, String) {
    let Ok(s) = STATE.lock() else { return (Health::Idle, String::new()) };
    match &s.clip_sent {
        None => (Health::Idle, "nothing copied here yet".into()),
        Some(e) if e.what.ends_with("(no PC connected)") => (Health::Waiting, format!("{} · {}", e.what, ago(e.at))),
        Some(e) => (Health::Ok, format!("{} · {}", e.what, ago(e.at))),
    }
}

/// The "Copy got" line: the last copy that arrived from another PC.
pub fn copy_got_row() -> (Health, String) {
    let Ok(s) = STATE.lock() else { return (Health::Idle, String::new()) };
    match &s.clip_got {
        None => (Health::Idle, "nothing from other PCs yet".into()),
        Some(e) => (Health::Ok, format!("{} · {}", e.what, ago(e.at))),
    }
}

/// The Files line.
pub fn files_row() -> (Health, String) {
    let Ok(s) = STATE.lock() else { return (Health::Idle, String::new()) };
    if let Some(e) = &s.file_error {
        if s.last_file_ok.is_none_or(|ok| ok < e.at) {
            return (Health::Failed, format!("last paste failed: {} · {}", e.what, ago(e.at)));
        }
    }
    if s.files_got == 0 && s.files_sent == 0 {
        return (Health::Idle, "no files pasted yet".into());
    }
    let mut line = format!(
        "pasted {} ({}) · sent {} ({})",
        s.files_got,
        size(s.files_got_bytes),
        s.files_sent,
        size(s.files_sent_bytes)
    );
    if let Some(at) = s.last_file_ok {
        line.push_str(&format!(" · {}", ago(at)));
    }
    (Health::Ok, line)
}

/// "text 1.2 KB", "image 640×480", "2 files (84.0 MB)", joined with " + ".
fn kinds(clip: &Clip) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &clip.text {
        parts.push(format!("text {}", size(t.len() as u64)));
    }
    if let Some(i) = &clip.image {
        // A DIB starts with a BITMAPINFOHEADER: width and height at bytes 4 and 8.
        let dims = (i.len() >= 12).then(|| {
            let w = i32::from_le_bytes([i[4], i[5], i[6], i[7]]);
            let h = i32::from_le_bytes([i[8], i[9], i[10], i[11]]);
            format!("{}×{}", w.unsigned_abs(), h.unsigned_abs())
        });
        parts.push(format!("image {}", dims.unwrap_or_else(|| size(i.len() as u64))));
    }
    if let Some(f) = &clip.files {
        let n = f.entries.iter().filter(|e| !e.dir).count();
        let noun = if n == 1 { "file" } else { "files" };
        parts.push(format!("{n} {noun} ({})", size(f.total())));
    }
    parts.join(" + ")
}

pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

fn ago(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(18), "18 B");
        assert_eq!(size(1229), "1.2 KB");
        assert_eq!(size(60 * 1024 * 1024), "60.0 MB");
    }
}
