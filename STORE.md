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
1. Sign up at [Partner Center](https://partner.microsoft.com/dashboard/registration) as an
   **individual** developer. Microsoft has made this free for individuals; check the price
   shown when you sign up.
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

## Store listing text

**Short description:**
> One keyboard and mouse across your Windows PCs, with a shared clipboard and file paste.

**Description:**
> HomeKVM lets you use one keyboard and mouse across several Windows PCs on your home
> network, like a software KVM switch. Move the mouse past a screen edge and it continues on
> the PC placed there.
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

**Notes for certification:**
> HomeKVM shares one keyboard and mouse between the user's own PCs, like PowerToys Mouse
> Without Borders. As a full-trust desktop app (runFullTrust) it uses low-level keyboard and
> mouse hooks (WH_KEYBOARD_LL, WH_MOUSE_LL), only while the user turns on "Share my keyboard
> and mouse", and SendInput on the PC being controlled. Input is sent only to a PC the user
> paired with a 6-digit code, over an encrypted local-network connection, and is never
> recorded. To test: install on two PCs on the same network, pair them, place the second PC
> on the right in Screen layout, and move the mouse off the right edge. Full details:
> https://github.com/saugatadatta/HomeKVM#how-the-keyboard-and-mouse-hooks-are-used.
> The Store version doesn't update itself; the Store updates it.
