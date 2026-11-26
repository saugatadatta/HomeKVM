//! Which keyboards and mice are attached to this PC, from the Windows Raw Input
//! device list. This only lists devices; it does not read any input.

use std::collections::BTreeSet;
use windows_sys::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICENAME, RIM_TYPEKEYBOARD,
    RIM_TYPEMOUSE,
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Link {
    Usb,
    Bluetooth,
}

impl Link {
    pub fn label(self) -> &'static str {
        match self {
            Link::Usb => "USB",
            Link::Bluetooth => "Bluetooth",
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Attached {
    /// How each external keyboard is connected (one entry per physical device).
    pub keyboards: Vec<Link>,
    /// How each external mouse is connected.
    pub mice: Vec<Link>,
    pub builtin_keyboard: bool,
    pub builtin_pointer: bool,
}

impl Attached {
    /// A PC with an external keyboard or mouse is probably the one you sit at.
    pub fn suggests_sharing(&self) -> bool {
        !self.keyboards.is_empty() || !self.mice.is_empty()
    }
}

enum Kind {
    External(Link, String),
    BuiltIn,
}

/// Classify a raw-input device path such as
/// `\\?\HID#VID_046D&PID_C52B&MI_01&Col01#...` (USB) or
/// `\\?\HID#{00001124-0000-1000-8000-00805f9b34fb}_VID&0002046d_PID&b023...` (Bluetooth).
fn classify(path: &str) -> Option<Kind> {
    let u = path.to_ascii_uppercase();
    // Remote desktop and other virtual devices are not real hardware.
    if u.contains("RDP_") || u.contains("ROOT#") {
        return None;
    }
    let bluetooth = u.contains("{00001124-0000-1000-8000-00805F9B34FB}") || u.contains("BTHLE") || u.contains("_VID&0002");
    if bluetooth {
        return Some(Kind::External(Link::Bluetooth, device_key(&u, "_VID&")));
    }
    if u.contains("VID_") {
        return Some(Kind::External(Link::Usb, device_key(&u, "VID_")));
    }
    Some(Kind::BuiltIn)
}

/// "VID_046D&PID_C52B" style key, so one receiver with several HID collections counts once.
fn device_key(u: &str, marker: &str) -> String {
    match u.find(marker) {
        Some(i) => u[i..].chars().take(marker.len() + 13).collect(),
        None => u.to_string(),
    }
}

pub fn scan() -> Attached {
    let mut out = Attached::default();
    let mut kb = BTreeSet::new();
    let mut ms = BTreeSet::new();
    for (is_keyboard, path) in raw_devices() {
        match classify(&path) {
            Some(Kind::External(link, key)) => {
                if is_keyboard {
                    kb.insert((key, link));
                } else {
                    ms.insert((key, link));
                }
            }
            Some(Kind::BuiltIn) => {
                if is_keyboard {
                    out.builtin_keyboard = true;
                } else {
                    out.builtin_pointer = true;
                }
            }
            None => {}
        }
    }
    out.keyboards = kb.into_iter().map(|(_, l)| l).collect();
    out.mice = ms.into_iter().map(|(_, l)| l).collect();
    out
}

/// (is_keyboard, device path) for every keyboard and mouse Windows knows about.
fn raw_devices() -> Vec<(bool, String)> {
    let item = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    unsafe {
        if GetRawInputDeviceList(std::ptr::null_mut(), &mut count, item) != 0 || count == 0 {
            return Vec::new();
        }
        let mut list: Vec<RAWINPUTDEVICELIST> = vec![std::mem::zeroed(); count as usize];
        let got = GetRawInputDeviceList(list.as_mut_ptr(), &mut count, item);
        if got == u32::MAX {
            return Vec::new();
        }
        list.truncate(got as usize);

        let mut out = Vec::new();
        for dev in list {
            let is_keyboard = match dev.dwType {
                RIM_TYPEKEYBOARD => true,
                RIM_TYPEMOUSE => false,
                _ => continue,
            };
            let mut chars = 0u32;
            GetRawInputDeviceInfoW(dev.hDevice, RIDI_DEVICENAME, std::ptr::null_mut(), &mut chars);
            if chars == 0 {
                continue;
            }
            let mut buf = vec![0u16; chars as usize];
            let n = GetRawInputDeviceInfoW(dev.hDevice, RIDI_DEVICENAME, buf.as_mut_ptr().cast(), &mut chars);
            if n == u32::MAX || n == 0 {
                continue;
            }
            let path = String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]);
            out.push((is_keyboard, path.trim_end_matches('\0').to_string()));
        }
        out
    }
}
