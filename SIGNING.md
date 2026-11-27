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
1. Go to [signpath.org](https://signpath.org) and open the application for free code signing
   for open-source projects.
2. Describe the project:
   - **Name:** HomeKVM
   - **Repository:** https://github.com/saugatadatta/HomeKVM
   - **License:** MIT
   - **What it does:** shares one keyboard and mouse across Windows PCs on a home network,
     with a shared clipboard and file paste. It uses low-level keyboard and mouse hooks
     (on the PC you're using) and `SendInput` (on the PC being controlled) for that, only
     between PCs the user pairs with a 6-digit code, over encrypted QUIC.
   - **Releases:** https://github.com/saugatadatta/HomeKVM/releases
   - **Maintainer:** Saugata Datta
3. Wait for their review. SignPath may ask questions, for example about the input hooks.

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
