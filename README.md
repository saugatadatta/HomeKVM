# HomeKVM

Use one keyboard and mouse across your home PCs. HomeKVM also shares the clipboard, and you
can copy files from one PC to another with copy and paste. It's one small Rust app, the same on
every PC. It runs in the tray without admin rights, and only PCs you pair can connect.

## Why HomeKVM
HomeKVM started out of frustration with PowerToys Mouse Without Borders. It aims at the same
idea with less to set up and less to go wrong:
- **One small app.** It runs in the tray, starts with Windows, and needs no admin rights.
- **Simple pairing.** PCs on your network show up on their own, and you pair them by checking
  a 6-digit code.
- **A clear screen layout.** Each paired PC goes to the left or right of this one.
- **One shared clipboard.** Text, screenshots and files, last copy wins. Files only move when
  you paste.

See [PLAN.md](PLAN.md) for the design and phases.

## Status
- Phase 1: tray icon with a status dot, main window, settings, start at sign-in, and
  detection of the keyboards and mice attached to this PC.
- Phase 2: PCs find each other on the local network (mDNS), pair with a 6-digit code, and
  stay connected over encrypted QUIC (TLS 1.3). Place paired PCs left or right; Forget works
  both ways.
- Phase 3: keyboard and mouse sharing. Move the mouse past a screen edge onto the PC placed
  there, along a row of several PCs, and back. Ctrl+Alt+Home (or the tray menu) always brings
  it back; a dropped connection releases everything; using a controlled PC's own mouse takes
  it back. Not yet tested on real hardware.
- Phase 4: one shared clipboard for text and images (screenshots too), last copy wins, both
  ways. Password-manager copies marked private are never sent.
- Phase 5 (this version, 0.5.0): files and folders through the same shared clipboard. Copying
  sends only the list; data moves when you paste. Small copies (under 50 MB by default,
  adjustable up to 200 MB under Settings, Advanced) paste as real files in any app; bigger
  ones stream straight into the folder with Explorer's own progress.

Next: releases and free code signing (see PLAN.md).

## Download
Every push to `main` is built by GitHub Actions (Actions tab, latest "Build" run,
Artifacts). That build includes keyboard and mouse sharing.

The exe isn't code-signed yet, so Windows SmartScreen shows a warning the first time
(More info, then Run anyway). Keyboard and mouse sharing uses global input hooks and input
injection, which antivirus heuristics can mistake for a keylogger until the app is signed.

## Build (Windows)
Needs Rust. With the GNU toolchain (`x86_64-pc-windows-gnu`), MinGW-w64 binutils must also be
on `PATH` (`winget install BrechtSanders.WinLibs.POSIX.MSVCRT`), because Rust's bundled
`dlltool` can't build import libraries on its own.
```powershell
cargo build --release
```
Without keyboard and mouse sharing (no input hooks), for PCs where they aren't allowed:
```powershell
cargo build --release --no-default-features
```
The program is `target\release\HomeKVM.exe`. It starts hidden in the tray; click the tray
icon to open the window. Closing the window keeps it in the tray. Quit from the tray menu.

Development builds (`cargo run`) never register themselves to start at sign-in.
For development, `HOMEKVM_PROFILE=b` runs a second, separate copy on the same PC (debug
builds only).

## License
MIT. See [LICENSE](LICENSE).

