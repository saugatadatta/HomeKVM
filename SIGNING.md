# Code signing with SignPath Foundation

HomeKVM releases are to be signed for free through [SignPath Foundation](https://signpath.org),
which signs open-source projects. The certificate stays in SignPath's hardware security
module: nobody, including the project, can sign anything outside the approved GitHub
Actions build. Signed releases show a verified publisher and stop the SmartScreen
"unknown publisher" warning. Windows builds trust for a signature over time, as downloads
add up.

Already in place in this repo:
- MIT license, public repository, releases built only by GitHub Actions
  ([`.github/workflows/build.yml`](.github/workflows/build.yml))
- The code signing policy and privacy statement in the [README](README.md)
- A signing step in the release workflow. It stays switched off until the repository
  variable `SIGNPATH_ORGANIZATION_ID` exists; until then, releases are published unsigned.

## 1. Apply (once)
Go to [signpath.org](https://signpath.org), open the application for free code signing for
open-source projects, and fill it in with these answers:

| Field | Answer |
|---|---|
| **Project Name** | HomeKVM |
| **Repository URL** | https://github.com/saugatadatta/HomeKVM |
| **Homepage URL** | https://github.com/saugatadatta/HomeKVM |
| **Download URL** | https://github.com/saugatadatta/HomeKVM#download (the README's Download section, right above the code signing policy, mentions SignPath. Release notes mention it from 0.7.0 on.) |
| **Privacy Policy URL** | https://github.com/saugatadatta/HomeKVM#privacy |
| **Wikipedia URL** | *(leave empty)* |
| **Tagline** | One keyboard and mouse across your Windows PCs, with a shared clipboard and copy-and-paste file transfer. |
| **Category** | Utilities / system tools (pick the closest one on the form) |
| **Description** | See below. |
| **Reputation** | See below. |
| **Maintainer Type** | Individual |
| **Build System** | GitHub Actions |
| **First Name** | Saugata |
| **Last Name** | Datta |
| **Email** | Your personal email. It isn't written here because this file is public. |
| **Company Name** | *(leave empty: personal project)* |
| **Primary Discovery Channel** | AI assistant (or "Other") |
| **Exact source** | Claude Code (Anthropic), while setting up code signing for this project |

**Description:**
> HomeKVM lets you use one keyboard and mouse across several Windows PCs on your home
> network, like a software KVM switch. Move the mouse past a screen edge and it continues on
> the PC placed there. The PCs share one clipboard for text, images and files: copy on one PC,
> paste on another, and files only transfer when you paste. It's a single small app that runs
> in the tray without admin rights. PCs pair with a 6-digit code and talk only to each other,
> over an encrypted connection. To share the keyboard and mouse, it uses Windows' low-level
> keyboard and mouse hooks on the PC you're using and replays the input on the paired PC with
> SendInput.

**Reputation** (be honest: it's a new project):
> HomeKVM is a new open-source project (MIT), started in November 2026 as a simpler
> alternative to PowerToys Mouse Without Borders. Development is public on GitHub, and every
> release is built by GitHub Actions from tagged source. Release downloads and GitHub insights
> are on the repository: https://github.com/saugatadatta/HomeKVM/releases and
> https://github.com/saugatadatta/HomeKVM/pulse

SignPath Foundation looks for some adoption, so a brand-new project may be asked to come
back later. Stars, downloads and issues on GitHub help. If they decline for now, apply again
once there's more use.

After submitting, wait for their review. SignPath may ask questions, for example about the
input hooks.

## 2. Set up SignPath (after approval)
Names in SignPath's web app may differ slightly from these.
1. **Project:** use the slug `HomeKVM`, or set the repository variable
   `SIGNPATH_PROJECT_SLUG` to whatever the slug is.
2. **Trusted build system:** link GitHub.com and this repository, so SignPath accepts
   builds from its GitHub Actions.
3. **Artifact configuration** (the default one, or set `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`).
   GitHub Actions uploads the exe as a zip with `HomeKVM.exe` inside:
   ```xml
   <?xml version="1.0" encoding="utf-8"?>
   <artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">
     <zip-file>
       <pe-file path="HomeKVM.exe">
         <authenticode-sign />
       </pe-file>
     </zip-file>
   </artifact-configuration>
   ```
4. **Signing policy:** use the slug `release-signing` (or set `SIGNPATH_SIGNING_POLICY_SLUG`),
   with the SignPath Foundation certificate. Saugata Datta is the approver, so every
   request needs a manual approval.
5. **API token:** create one for a CI user that's allowed to submit to that policy.

## 3. Connect GitHub (after approval)
In the repository, under **Settings → Secrets and variables → Actions**:
- **Secret** `SIGNPATH_API_TOKEN`: the API token from step 2.5
- **Variable** `SIGNPATH_ORGANIZATION_ID`: your SignPath organization ID. Setting this
  switches signing on.

## 4. Release
1. Bump `version` in `Cargo.toml` (for example to `0.6.0`), commit and push.
2. Tag and push: `git tag -a v0.6.0 -m "HomeKVM 0.6.0"`, then `git push origin v0.6.0`.
3. The workflow builds the exe and sends it to SignPath. **Approve the request on SignPath
   within an hour**, or the build times out.
4. The workflow checks the signature and publishes the release with the signed
   `HomeKVM.exe`.

After the first signed release, update the README: remove "Applied for" from the code
signing policy and the SmartScreen note from Download.
