# HomeKVM — plan (draft, awaiting approval)

One small Rust app, the same on every computer. Use one keyboard and mouse across
computers on your home network, share the clipboard, and copy files by copy and paste.

## Principles
- Simple: one window, one list of devices, one row of screens. No server, no account, no cloud.
- Runs as you, never as admin: no service and no driver. Start-at-login uses
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (or the equivalent on macOS and Linux).
- Secure by default: only devices you pair can connect. Everything is encrypted.
- Fails safe: if a connection drops, the keyboard and mouse come straight back to the local PC.
  A hotkey (Ctrl+Alt+Home) always returns them too.

## Any PC can be the "server"
There is no fixed server. Every install can both share its own keyboard and mouse and be
controlled by another PC, so you choose the server by choosing where you sit.
- The app checks which keyboards and mice are physically attached (Windows Raw Input device
  list; USB and Bluetooth). It separates external devices from a laptop's built-in keyboard
  and touchpad.
- A PC with an external keyboard or mouse is suggested as the sharing PC ("Share my keyboard
  and mouse": on). A bare laptop starts as "Can be controlled" only.
- You can override this either way in the window. Unplug the external mouse and the app
  shows a notice; plug it into another laptop and that laptop offers to take over.
- Only one PC controls at a time. Whichever PC's real keyboard or mouse is being used
  right now is in charge, so two people can't fight over a cursor.

## UI (references: Mouse Without Borders, Input Leap / Deskflow, lan-mouse)
- Screen row: this PC in the middle; drag other devices to its left or right.
  The touching edge is where the mouse crosses over.
- Device list: connected, found nearby (Pair), and paired but offline.
- Pairing: both screens show the same 6-digit code, and you confirm it on each (like Bluetooth).
- Settings: clipboard text, images, files (with a size limit), start at login, edge delay.
- Tray first: the app always starts hidden in the tray, both at sign-in and when launched by hand.
  The window opens only from the tray icon (click it or choose Open). Closing or minimizing the
  window sends it back to the tray. Only Quit in the tray menu exits.
- Tray icon has a colored status dot:
  | Dot | Meaning |
  |---|---|
  | Green | Connected to at least one paired PC; mouse is here |
  | Blue | Mouse and keyboard are on another PC right now |
  | Gray | No paired PC online |
  | Amber | Sharing paused |
  | Red | Needs attention (pairing failed, firewall blocked, etc.) |
  Hovering shows text such as "HomeKVM — connected to Work laptop, Mini PC".
  Colors are never the only signal: the hover text and the menu name the state too.
- Tray menu: status line, Open HomeKVM, Pause/Resume sharing, Bring mouse back here
  (Ctrl+Alt+Home), each paired PC with its own dot and side, Quit.

## Tech stack
| Part | Choice |
|---|---|
| Language | Rust (stable) |
| UI | `egui` / `eframe`: one small native exe with no web runtime; `tray-icon` for the tray |
| Network | QUIC (`quinn`) with TLS 1.3 (`rustls`). Mouse moves use unreliable datagrams for low lag; keys, clipboard and files use reliable streams |
| Identity | Each install makes its own key and self-signed certificate. A device is paired by pinning its certificate fingerprint |
| Discovery | mDNS on the local network (`mdns-sd`); IP can also be typed by hand |
| Input (Windows) | Low-level keyboard and mouse hooks to capture; `SendInput` to replay |
| Input (macOS / Linux) | CGEventTap with the Accessibility permission / X11 or libei (later phase) |
| Clipboard | `arboard` for text and images; Windows file lists (CF_HDROP) for files |
| Config | TOML in `%APPDATA%\HomeKVM` |

## How the mouse crosses
1. This PC watches the cursor. When it hits the edge with a device next to it, input is "captured":
   the local cursor is held still and moves are sent to the other device instead.
2. The other device moves its own cursor with `SendInput`, starting at the matching edge
   and at the same height (as a percentage).
3. When the cursor reaches the edge on the other side that faces back, control returns.
4. Clipboard sync happens at the moment control switches, so you paste what you last copied.

## File copy via clipboard
Copy files in Explorer on PC A, move to PC B, then paste. Version 1 copies the files to
`Downloads\HomeKVM` on B and puts them on B's clipboard, so a paste in Explorer drops them
where you want. Large or many files show progress in the window. The size limit is a setting.

## Phases
1. Skeleton: window, tray, config, start at login (no input or network yet).
2. Pairing and an encrypted connection between two PCs, plus the device list.
3. Mouse and keyboard sharing on Windows, with left/right edges and the return hotkey.
4. Clipboard: text and images.
5. Files through the clipboard, with progress.
6. macOS and Linux.
7. Installer or portable zip, plus code signing.

## Risk: antivirus and EDR
Global keyboard hooks, input injection, start-at-login and network traffic in one unsigned
exe look like a keylogger to Defender and corporate EDR. Do not build or run phase 3 and later
on a corporate-managed PC. Develop and test on personal machines. For wider use, sign the exe.
