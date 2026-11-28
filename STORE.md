# Microsoft Store

HomeKVM is published two ways from the same code:

| | GitHub Releases | Microsoft Store |
|---|---|---|
| File | `HomeKVM.exe` (single portable file) | MSIX package |
| Signed by | SignPath Foundation (once approved) | Microsoft, when published |
| Updates | HomeKVM's own updater, from GitHub Releases | The Store |
| Start at sign-in | Run key | The package's startup task |

It's the same `HomeKVM.exe` inside the package. HomeKVM notices when it runs from the Store
package: then the Store handles updates (HomeKVM's updater stays off, as Store rules
require), and "Start when I sign in" uses the package's startup task. Everything else is
identical, and Store and GitHub copies pair and work together.

## What's automated
On every push, GitHub Actions builds the Store package (`HomeKVM-<version>.msixupload`, a run
artifact) from [`packaging/AppxManifest.xml`](packaging/AppxManifest.xml), with Store images
drawn from the app icon. On every version tag (`v1.0.0`), once the one-time setup below is
done, it also **submits the package to the Store and sends it to certification**. Nothing to
upload by hand.

## One-time setup (only you can do this)
Microsoft requires these steps in Partner Center once. After that, releases go to the Store
by themselves.

### 1. Developer account and app name
1. Sign up at [storedeveloper.microsoft.com](https://storedeveloper.microsoft.com), the only
   entry point for Microsoft's current (free) sign-up: **Get started for free → Individual
   developer**, sign in with your **personal** Microsoft account, verify your identity with a
   government ID and a selfie, and complete the profile. Then **Go to Partner Center
   dashboard** and pick the same account. If the Apps and games tile isn't there yet, wait
   about five minutes and refresh, or open https://aka.ms/submitwindowsapp. (Signing up
   through Partner Center's own registration page shows the old flow, and an account that
   hasn't joined shows "Access restricted".)
2. Under **Apps and games**, choose **New product → MSIX or PWA app** and **reserve the name
   "HomeKVM"** (or an alternative if it's taken).

### 2. Package identity: three repository variables
In the app's **Product management → Product identity** page, copy these into GitHub
(repository **Settings → Secrets and variables → Actions → Variables**):

| GitHub variable | Partner Center value |
|---|---|
| `MSSTORE_IDENTITY_NAME` | Package/Identity/Name |
| `MSSTORE_PUBLISHER` | Package/Identity/Publisher (starts with `CN=`) |
| `MSSTORE_PUBLISHER_DISPLAY_NAME` | Package/Properties/PublisherDisplayName |

Also add **`MSSTORE_PRODUCT_ID`**: the app's Store ID (12 characters, like `9NBLGGH4XXXX`),
shown on the same page. Setting this one switches on automatic submission.

### 3. API access for GitHub Actions: four repository secrets
1. In Partner Center, open **Account settings → User management → Microsoft Entra
   applications**, and associate a Microsoft Entra tenant if asked (you can create one there
   for free).
2. **Create a new Microsoft Entra application** with the **Manager** role, then
   **add a new key** (client secret). Copy it right away: it's shown only once.
3. Add these GitHub **secrets** (repository **Settings → Secrets and variables → Actions →
   Secrets**):

| GitHub secret | Where to find it |
|---|---|
| `PARTNER_CENTER_TENANT_ID` | Tenant ID, on the Entra applications page |
| `PARTNER_CENTER_CLIENT_ID` | The application's Client ID |
| `PARTNER_CENTER_CLIENT_SECRET` | The key from step 2 |
| `PARTNER_CENTER_SELLER_ID` | Seller ID, under Account settings → Legal info |

### 4. The first submission (once, by hand)
The Store needs the listing details once before automation can take over. Create the first
submission in Partner Center:
- **Pricing and availability:** Free, all markets you want.
- **Properties:** category **Utilities & tools**. Privacy policy URL:
  `https://github.com/saugatadatta/HomeKVM#privacy`. Support:
  `https://github.com/saugatadatta/HomeKVM/issues`.
- **Age ratings:** fill in the questionnaire (no user content, no purchases).
- **Packages:** upload `HomeKVM-<version>.msixupload` from the latest GitHub Actions build
  (Actions → Build → Artifacts → HomeKVM-store-…).
- **Store listing:** description and screenshots (the text below is ready to paste).
- **Submission options → Notes for certification:** paste the notes below. They explain the
  keyboard and mouse hooks and the `runFullTrust` capability before reviewers ask.

After it passes certification, every new version tag submits automatically.

## The Store listing
HomeKVM is reserved as **HomeKVM** (Store ID `9P4H0H9835CN`, publisher **iAmSaugata**). The
first submission, 0.10.3, went to certification on 2026-11-28. These are the values it was
submitted with, so later listing updates stay consistent.

**Pricing and availability:** Free (base price 0), all worldwide markets, public and
discoverable, released as soon as it passes certification.

**Properties:** category Utilities & tools. Personal information: yes (keystrokes and clipboard
contents travel between the user's own paired PCs), privacy policy
`https://github.com/saugatadatta/HomeKVM#privacy`. Website
`https://github.com/saugatadatta/HomeKVM`, support contact
`https://github.com/saugatadatta/HomeKVM/issues`. Recommended hardware: keyboard and mouse.

**Age ratings:** IARC questionnaire, "All Other App Types", every answer No. Result: 3+
(ESRB Everyone, PEGI 3, USK 0).

**Packages:** the `.msixupload` from the GitHub Actions build, Windows 10/11 Desktop only.

**Short description:**
> One keyboard and mouse across your Windows PCs, with a shared clipboard and file paste.

**Description:**
> HomeKVM lets you use one keyboard and mouse across several Windows PCs on your home
> network, like a software KVM switch. Move the mouse past the left or right edge of the
> screen and it continues on the PC placed there.
>
> - One shared clipboard: copy text, screenshots or files on one PC and paste on another.
>   Files only transfer when you paste.
> - Simple pairing: PCs on your network show up on their own; pair them by checking a
>   6-digit code.
> - Private and encrypted: PCs talk only to each other over an encrypted connection. No
>   account, no cloud, no telemetry.
> - Small and quiet: runs in the tray without admin rights.
>
> Free and open source (MIT): https://github.com/saugatadatta/HomeKVM

**Product features:**
- Share one keyboard and mouse across Windows PCs
- Shared clipboard for text, images and files
- Pair PCs with a 6-digit code
- Encrypted local-network connection, no cloud
- Runs in the tray, no admin rights needed

**Keywords:** KVM, keyboard mouse sharing, mouse without borders, clipboard sharing, multiple
PCs, software KVM, share mouse.

**Copyright:** © 2026 Saugata Datta. **Developed by:** Saugata Datta. **Additional license
terms:** MIT License, https://github.com/saugatadatta/HomeKVM/blob/main/LICENSE.

**Notes for certification** (Submission options; the field takes at most 500 characters):
> Shares one keyboard and mouse between the user's own PCs (like PowerToys Mouse Without
> Borders). runFullTrust is needed for low-level keyboard/mouse hooks, active only when the
> user turns on sharing, and SendInput on the controlled PC. Input goes only to PCs paired
> with a 6-digit code, over encrypted LAN; nothing is recorded. Test: install on 2 PCs on one
> network, pair, set the 2nd PC to Right, move the mouse off the right edge. No self-update.
> Details: github.com/saugatadatta/HomeKVM

**Additional Testing Information** (Supplemental info; kept for every submission):
> HomeKVM shares one keyboard and mouse between the user's own PCs, like PowerToys Mouse
> Without Borders. As a full-trust desktop app (runFullTrust) it uses low-level keyboard and
> mouse hooks (WH_KEYBOARD_LL, WH_MOUSE_LL), only while the user turns on "Share my keyboard
> and mouse", and SendInput on the PC being controlled. Input is sent only to a PC the user
> paired with a 6-digit code, over an encrypted local-network connection, and is never
> recorded.
>
> To test: install on two PCs on the same network, open HomeKVM from the tray on both, click
> Pair on one and confirm that both show the same code, then set the second PC to "Right" in
> Other PCs and move the mouse off the right edge of the screen. Copy text on one PC and paste
> it on the other to test the shared clipboard.
>
> No account or sign-in is needed. The Store version doesn't update itself; the Store updates
> it. Details: https://github.com/saugatadatta/HomeKVM#how-the-keyboard-and-mouse-hooks-are-used

No credentials: HomeKVM has no account or sign-in.

## Store images
The listing images are in [`docs/store`](docs/store):

| File | Where it goes in the listing |
|---|---|
| `1-screenshot-dark.png`, `2-screenshot-pairing.png`, `3-screenshot-light.png` | Screenshots, Desktop tab, in this order (1920 x 1080) |
| `poster-9x16-1440x2160.png` | Store logos: 9:16 Poster art |
| `box-art-1x1-2160x2160.png` | Store logos: 1:1 Box art |
| `app-tile-icon-300x300.png` | Store display images: 1:1 App tile icon |

The screenshots come from a dev build in its Store screenshot mode (`HOMEKVM_DEMO=store`, or
`store-pair` for the pairing request): sample PCs, made-up addresses and ID, sharing shown on.
It exists only in dev builds, so release builds never show it. To make new ones after a
change to the window:

```powershell
cargo build --no-default-features
pwsh tools/store-screenshot.ps1 -Theme dark  -Out target/store-art/window-dark.png
pwsh tools/store-screenshot.ps1 -Theme light -Out target/store-art/window-light.png
pwsh tools/store-screenshot.ps1 -Theme dark  -Mode store-pair -Out target/store-art/window-pair.png
cargo run --example icon_png -- 1024 target/store-art/icon-1024.png
python tools/store-images.py target/store-art docs/store
```

`tools/store-images.py` needs Pillow (`pip install pillow`). Close any running HomeKVM first:
only one copy runs at a time.
