# HomeKVM

Use one keyboard and mouse across your home PCs. HomeKVM also shares the clipboard, and you
can copy files from one PC to another with copy and paste. It's one small Rust app, the same on
every PC. It runs in the tray without admin rights, and only PCs you pair can connect.

See [PLAN.md](PLAN.md) for the design and phases.

## Status
- Phase 1: tray icon with a status dot, main window, settings, start at sign-in, and
  detection of the keyboards and mice attached to this PC.
- Phase 2 (this version, 0.2.0): PCs find each other on the local network (mDNS), pair with
  a 6-digit code, and stay connected over encrypted QUIC (TLS 1.3). Place paired PCs left or
  right; Forget works both ways.

Mouse and keyboard sharing, the shared clipboard and file paste come in later phases.
For development, `HOMEKVM_PROFILE=b` runs a second, separate copy on the same PC (debug builds only).

## Build (Windows)
Needs Rust (GNU toolchain, `x86_64-pc-windows-gnu`) and MinGW-w64 binutils on `PATH`
(`winget install BrechtSanders.WinLibs.POSIX.MSVCRT`). Rust's bundled `dlltool` can't
build import libraries on its own.
```powershell
cargo build --release
```
The program is `target\release\HomeKVM.exe`. It starts hidden in the tray; click the tray
icon to open the window. Closing the window keeps it in the tray. Quit from the tray menu.

Development builds (`cargo run`) never register themselves to start at sign-in.
