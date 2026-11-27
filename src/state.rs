use crate::config::Layout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// What the tray dot and the status pill show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Attention: for problems such as a blocked firewall (later).
pub enum Status {
    /// At least one paired PC is connected; the mouse is here.
    Connected,
    /// The mouse and keyboard are on another PC right now.
    Away,
    /// No paired PC is online.
    NotConnected,
    /// Sharing is paused.
    Paused,
    /// Something needs the user (pairing failed, firewall, ...).
    Attention,
}

impl Status {
    pub fn title(self) -> &'static str {
        match self {
            Status::Connected => "Connected",
            Status::Away => "Using another PC",
            Status::NotConnected => "Not connected",
            Status::Paused => "Paused",
            Status::Attention => "Needs attention",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Status::Connected => "mouse is here",
            Status::Away => "mouse and keys are on another PC",
            Status::NotConnected => "no paired PC online yet",
            Status::Paused => "sharing is off until you resume",
            Status::Attention => "open HomeKVM to see what's wrong",
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Status::Connected => [46, 160, 67],
            Status::Away => [31, 111, 235],
            Status::NotConnected => [140, 149, 159],
            Status::Paused => [210, 153, 34],
            Status::Attention => [218, 54, 51],
        }
    }
}

/// Another PC as the window and tray show it.
#[derive(Clone, Debug, Default)]
pub struct PeerView {
    pub id: String,
    pub name: String,
    /// Trusted (paired) PC.
    pub paired: bool,
    /// Connected right now.
    pub connected: bool,
    /// Seen on the local network.
    pub found: bool,
    pub addr: Option<String>,
}

/// A pairing waiting for the user to compare codes.
#[derive(Clone, Debug)]
pub struct PairingView {
    pub id: String,
    pub name: String,
    /// "482 913"
    pub code: String,
    /// The other PC asked.
    pub incoming: bool,
    /// This PC already confirmed; waiting for the other one.
    pub confirmed: bool,
}

/// What the network side reports.
#[derive(Clone, Debug, Default)]
pub struct NetView {
    /// This PC's ID, shortened for display.
    pub my_id: String,
    pub peers: Vec<PeerView>,
    pub pairing: Option<PairingView>,
    /// Connecting to this PC to pair with it.
    pub connecting: Option<String>,
    /// Something to tell the user (pairing declined, PC unreachable, ...).
    pub notice: Option<String>,
    /// This PC's keyboard and mouse are on this PC (by name) right now.
    pub controlling: Option<String>,
    /// This PC is controlled by that PC (by name) right now.
    pub controlled_by: Option<String>,
}

impl NetView {
    pub fn peer(&self, id: &str) -> Option<&PeerView> {
        self.peers.iter().find(|p| p.id == id)
    }
}

/// State shared by the window, the tray thread and the network thread.
#[derive(Default)]
pub struct Shared {
    paused: AtomicBool,
    net: Mutex<NetView>,
    layout: Mutex<Layout>,
}

impl Shared {
    pub fn net(&self) -> NetView {
        self.net.lock().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn set_net(&self, view: NetView) {
        if let Ok(mut v) = self.net.lock() {
            *v = view;
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn set_layout(&self, layout: &Layout) {
        if let Ok(mut l) = self.layout.lock() {
            *l = layout.clone();
        }
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        if paused {
            // Pausing always brings the keyboard and mouse back here.
            crate::input::request_release();
        }
    }

    pub fn status(&self) -> Status {
        let net = self.net();
        if self.paused() {
            Status::Paused
        } else if net.controlling.is_some() {
            Status::Away
        } else if net.peers.iter().any(|p| p.paired && p.connected) {
            Status::Connected
        } else {
            Status::NotConnected
        }
    }
}
