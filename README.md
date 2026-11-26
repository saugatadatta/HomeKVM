# HomeKVM

Use one keyboard and mouse across your home PCs. HomeKVM also shares the clipboard, and you
can copy files from one PC to another with copy and paste. It's one small Rust app, the same on
every PC. It runs in the tray without admin rights, and only PCs you pair can connect.

See [PLAN.md](PLAN.md) for the design and phases.

## Status
Phase 1 (this version) has the tray icon with a status dot, the main window, settings,
start at sign-in, and detection of the keyboards and mice attached to this PC.
Pairing, mouse and keyboard sharing, and the clipboard come in later phases.

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
