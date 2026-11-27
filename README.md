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
- Phase 5: files and folders through the same shared clipboard. Copying
  sends only the list; data moves when you paste. Small copies (under 50 MB by default,
  adjustable up to 200 MB under Settings, Advanced) paste as real files in any app; bigger
  ones stream straight into the folder with Explorer's own progress.

Since 0.6.0 the screen cards show a **Server** badge on the PC whose keyboard and mouse
are in control (when idle, on every PC that shares them), and the window's footer links
back to this repository.

Next: free code signing (see [SIGNING.md](SIGNING.md)).

## Download
Two ways, same app: a single portable `HomeKVM.exe` from GitHub Releases (below), or the
Microsoft Store (coming; see [STORE.md](STORE.md)). The Store version is updated by the Store;
the GitHub version updates itself from GitHub Releases.

Get `HomeKVM.exe` from [Releases](https://github.com/saugatadatta/HomeKVM/releases/latest).
It's a single file: run it on each PC. Every release is built from this repo by GitHub
Actions. To start HomeKVM with Windows, turn on **Start when I sign in** in its Settings; it's
off until you do.

HomeKVM uses [SignPath Foundation](https://signpath.org) for free code signing (applied for;
see the code signing policy below). Until then, releases aren't signed, so Windows SmartScreen
shows a warning the first time (More info, then Run anyway). Keyboard and mouse sharing uses
global input hooks and input injection, which antivirus heuristics can mistake for a keylogger
until the app is signed.

From 0.7.0 on, HomeKVM updates itself from these releases: it checks once a day, verifies the
download, swaps in the new version and restarts.

## Code signing policy
Free code signing provided by [SignPath.io](https://signpath.io), certificate by
[SignPath Foundation](https://signpath.org). *(Applied for; releases are unsigned until it's
approved.)*

- Only release builds made by this repository's GitHub Actions workflow from a version tag
  are signed, never builds made on someone's own PC.
- Every signing request is approved by hand.
- Committers and reviewers: [Saugata Datta](https://github.com/saugatadatta)
- Approvers: [Saugata Datta](https://github.com/saugatadatta)

## Privacy
HomeKVM has no accounts, no cloud service and no telemetry. The only internet request is the
update check: once a day it asks GitHub (`api.github.com`) for the latest HomeKVM release and,
when there's a newer one, downloads it from GitHub. Nothing about you or your PCs is sent. On
your local network it does the following:
- It announces this PC's name and HomeKVM ID with mDNS, so your other PCs can find it.
- It talks only to PCs you've paired with a 6-digit code, over an encrypted connection
  (QUIC with TLS 1.3).
- It sends those PCs only what you share: keyboard and mouse input while you control them,
  your clipboard (text, images, file lists), and files when you paste them there.

Settings, the paired-PC list and this PC's key stay on this PC (`%APPDATA%\HomeKVM`).

## How the keyboard and mouse hooks are used
Sharing a keyboard and mouse needs the same Windows features a keylogger would use, so here
is exactly what HomeKVM does with them. The code is in [`src/input.rs`](src/input.rs).

**On the PC you're using** (low-level keyboard and mouse hooks, `WH_KEYBOARD_LL` and
`WH_MOUSE_LL`):
- The hooks are installed only while **Share my keyboard and mouse** is on, and removed
  when you turn it off.
- While the mouse is on this PC, input goes to Windows as normal. The hooks only notice when
  the cursor reaches a screen edge that has a paired PC next to it, and watch for the
  Ctrl+Alt+Home shortcut. To avoid stuck keys, they remember in memory which keys are held
  down right now. Nothing is written to disk or sent anywhere.
- Only after the mouse crosses onto another paired PC does input leave this PC. It goes only
  to that PC, over the encrypted connection, until the mouse comes back: by the screen edge,
  Ctrl+Alt+Home, the tray menu, Pause, or a dropped connection.

**On the PC being controlled** (`SendInput`):
- Input is replayed only for a paired PC that has moved its mouse onto this PC, never for
  anything else.
- Touching this PC's own keyboard or mouse takes control back at once. When control leaves,
  any keys or buttons still held down are released.
- Input that HomeKVM replays is marked by Windows as injected, and HomeKVM's hooks ignore it,
  so it never travels further.

Nothing is recorded: there's no keystroke history, log or file. Builds made with
`--no-default-features` leave out the hooks and `SendInput` entirely.

To report a security problem, see [SECURITY.md](SECURITY.md).

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

The window's icons come from [Tabler Icons](https://tabler.io/icons) (MIT, see
[assets/fonts/TABLER-ICONS-LICENSE.txt](assets/fonts/TABLER-ICONS-LICENSE.txt)); the app
carries only the few it uses.

