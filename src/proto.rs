//! Messages between two HomeKVM PCs on the control stream: a 4-byte length and
//! then JSON. Small and easy to read while the protocol grows.

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
