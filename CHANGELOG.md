# Changelog

What changed in each HomeKVM version. Every release on GitHub shows its section from
this file, and a release build stops if the new version has no section here yet.

## [0.10.2]

### Fixed
- Closing or minimizing the window sends it back to the tray every time. Before, it went
  to the tray only the first time; after opening it again from the tray, closing it did
  nothing and minimizing left it on the taskbar.
- HomeKVM really starts in the tray. Before, the window could open by itself soon after
  starting (for example when another PC was found on the network), away from the center.

### Changed
- The window opens in the center of the screen every time, on the screen with the mouse.
- **Start when I sign in** is softly highlighted in Settings. It stays off until you turn
  it on.

## [0.10.1]

### Fixed
- Keys reach the other PC from the start. Before, keys typed after moving the mouse to
  another PC could go nowhere until something was typed on the PC with the keyboard. Now
  HomeKVM's hidden window takes the keyboard while the mouse is on another PC, and the
  window used before gets it back when the mouse returns.

### Added
- Diagnostics says so when the window in front runs as administrator, which Windows
  won't let HomeKVM take keys from.

## [0.10.0]

### Added
- A new look: a Midnight dark theme and a matching light theme that follow the Windows
  setting, with Segoe UI and icons.
- Diagnostics in Advanced: hooks, keys, the last copy sent and received, file pastes and
  the link to other PCs, each with a status dot, and **Copy report**.
- A Microsoft Store package and submission in the release workflow (it runs once the
  Partner Center settings are added).

### Changed
- Two columns that always end on the same line: the shorter column's last card grows.
- PCs sit to the left and right only. Above and Below were tried and removed; the top and
  bottom screen edges no longer start the switch delay.

### Fixed
- The window could grow taller than the screen and hide its footer.
- The Other PCs card left an empty gap under its rows.

## [0.9.0]

### Fixed
- After an automatic update, the new version starts and the old file is removed. Before,
  the new version didn't start and the old file was left behind as `HomeKVM.old.exe`.

## [0.8.0]

### Changed
- **Start when I sign in** is off until you turn it on, so the first start never writes to
  the Run key.

### Added
- An application manifest (runs as the user, no admin rights).

## [0.7.0]

### Added
- Automatic updates from GitHub Releases, checked against the published SHA-256.

## [0.6.0]

### Added
- The Server badge on the PC whose keyboard and mouse are in use, an about footer, and
  preparation for code signing.

## [0.5.0]

### Added
- Copied files and folders paste on another PC through the shared clipboard: small copies
  as real files, bigger ones streamed straight into the folder.

## [0.4.0]

### Added
- Keyboard and mouse sharing across the screen edges, and the shared clipboard for text
  and images.

## [0.2.0]

### Added
- Finding PCs on the network, pairing with a 6-digit code, and encrypted links.

## [0.1.0]

### Added
- The tray app with a status dot, the main window and settings.
