# Security

HomeKVM shares a keyboard, mouse, clipboard and files between PCs, so security problems
matter. Thank you for reporting them privately.

## Reporting a problem
Use GitHub's private vulnerability reporting: on the repository, open **Security**, then
**Report a vulnerability**
([direct link](https://github.com/saugatadatta/HomeKVM/security/advisories/new)).
Only the maintainer sees the report.

Please don't open a public issue for security problems.

A useful report includes:
- what the problem is and what an attacker could do with it
- the steps to reproduce it, and the HomeKVM version (shown at the bottom of the window)
- whether an attacker needs to be on the same network, needs a paired PC, or needs access to
  the PC itself

## What to expect
HomeKVM is maintained by one person, so responses are best effort. You'll get a reply
acknowledging the report, and you'll be told when it's fixed. Fixes ship as a new release,
and HomeKVM installs it automatically within a day.

## Supported versions
Only the latest release. HomeKVM updates itself, so it's the version everyone runs.

## In scope
- Pairing and trust: connecting without being paired, or faking the 6-digit code
- The encrypted connection between PCs (QUIC with TLS 1.3, pinned certificates)
- Keyboard and mouse sharing: input reaching a PC it shouldn't, or getting stuck elsewhere
  (see [how the input hooks are used](README.md#how-the-keyboard-and-mouse-hooks-are-used))
- The shared clipboard and file paste: reading files that weren't copied, or writing
  outside the folder being pasted into
- The updater: getting HomeKVM to install something that isn't a genuine release

## Out of scope
- Someone who already controls one of your paired PCs, or has admin rights on your PC
- Antivirus programs flagging the unsigned exe (code signing is in progress; see the README)
