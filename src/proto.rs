//! Messages between two HomeKVM PCs on the control stream: a 4-byte length and
//! then JSON. Small and easy to read while the protocol grows.

use crate::clipboard::{Clip, MAX_IMAGE, MAX_TEXT};
use serde::{Deserialize, Serialize};

const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    /// Sent first by both sides, and again when the name changes.
    /// `paired` says whether the sender trusts the receiver.
    Hello { name: String, paired: bool },
    /// "Let's pair": the receiver shows the code and asks its user.
    PairRequest,
    /// The sender's user confirmed the codes match.
    PairAccept,
    /// The sender's user cancelled (or pairing timed out).
    PairReject,
    /// The sender no longer trusts the receiver.
    Unpair,

    // ---- Keyboard and mouse sharing (only between paired PCs) ----
    /// Controller -> target: input now goes to you; show the cursor at `edge`, `y` (0..1) down.
    Enter { edge: Edge, y: f32 },
    /// Controller -> target: input goes elsewhere now; release any held keys and buttons.
    Leave,
    /// Mouse movement in the controller's pixels.
    Move { dx: i32, dy: i32 },
    /// 1 left, 2 right, 3 middle, 4 and 5 the side buttons.
    Button { b: u8, down: bool },
    /// Wheel turn (120 per notch); `h` for horizontal.
    Wheel { d: i32, h: bool },
    /// A key by scan code (layout-independent), with the virtual key as a fallback.
    Key { vk: u16, sc: u16, ext: bool, down: bool },
    /// Target -> controller: the cursor was pushed past my left (-1) or right (+1) edge.
    Overflow { dir: i8, y: f32 },
    /// Target -> controller: someone is using my own keyboard or mouse; stop sending.
    TakeOver,
}

/// A screen's left or right edge.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    Right,
}

/// A clipboard copy travels on its own one-way stream, so a large image never holds
/// up mouse and keyboard messages: a 4-byte length, this header as JSON, then the
/// text (UTF-8) and image (Windows DIB) bytes.
#[derive(Serialize, Deserialize, Debug)]
struct ClipHeader {
    text: u64,
    image: u64,
}

pub async fn send_clip(conn: &quinn::Connection, clip: &Clip) -> Result<(), String> {
    let text = clip.text.as_deref().unwrap_or("").as_bytes();
    let image = clip.image.as_deref().unwrap_or(&[]);
    let header = serde_json::to_vec(&ClipHeader { text: text.len() as u64, image: image.len() as u64 })
        .map_err(|e| e.to_string())?;
    let mut s = conn.open_uni().await.map_err(|e| e.to_string())?;
    s.write_all(&(header.len() as u32).to_be_bytes()).await.map_err(|e| e.to_string())?;
    s.write_all(&header).await.map_err(|e| e.to_string())?;
    s.write_all(text).await.map_err(|e| e.to_string())?;
    s.write_all(image).await.map_err(|e| e.to_string())?;
    s.finish().map_err(|e| e.to_string())?;
    // Wait until the other PC has read it all (or gives up).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(60), s.stopped()).await;
    Ok(())
}

pub async fn read_clip(stream: &mut quinn::RecvStream) -> Result<Clip, String> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err("header too large".into());
    }
    let mut header = vec![0u8; len];
    stream.read_exact(&mut header).await.map_err(|e| e.to_string())?;
    let header: ClipHeader = serde_json::from_slice(&header).map_err(|e| e.to_string())?;
    if header.text as usize > MAX_TEXT || header.image as usize > MAX_IMAGE {
        return Err("copy too large".into());
    }
    let mut text = vec![0u8; header.text as usize];
    stream.read_exact(&mut text).await.map_err(|e| e.to_string())?;
    let mut image = vec![0u8; header.image as usize];
    stream.read_exact(&mut image).await.map_err(|e| e.to_string())?;
    Ok(Clip {
        text: (!text.is_empty()).then(|| String::from_utf8_lossy(&text).into_owned()),
        image: (!image.is_empty()).then_some(image),
    })
}

pub async fn write(stream: &mut quinn::SendStream, msg: &Msg) -> Result<(), String> {
    let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    stream.write_all(&(body.len() as u32).to_be_bytes()).await.map_err(|e| e.to_string())?;
    stream.write_all(&body).await.map_err(|e| e.to_string())
}

pub async fn read(stream: &mut quinn::RecvStream) -> Result<Msg, String> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err("message too large".into());
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}
