use std::sync::atomic::{AtomicBool, Ordering};

/// What the tray dot and the status pill show.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // Connected, Away and Attention arrive with pairing (phase 2).
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

/// State shared by the window and the tray thread.
#[derive(Default)]
pub struct Shared {
    paused: AtomicBool,
}

impl Shared {
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn status(&self) -> Status {
        if self.paused() {
            Status::Paused
        } else {
            // Phase 1 has no network yet, so nothing can be connected.
            Status::NotConnected
        }
    }
}
