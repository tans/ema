# EMA desktop release procedure

This project uses **Tauri 2 + Rust + React**. Release automation is defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Versioning

A tag `vX.Y.Z` triggers a release only when it refers to a commit already on `main`. The tag must match the version in:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- `crates/core/Cargo.toml`

Do not reuse or force-update version tags. For later versions, update all four files together in a reviewed change.

## Publish checklist

1. Ensure the code is merged to `main` and Build CI is green. Use a tag to create an unpublished **Draft Pre-release** and verify installers before making them publicly downloadable. A controlled mailbox/Codex end-to-end test is required **before publishing**.
2. Confirm the project license, security limitations, and the no-cost distribution policy: macOS ad-hoc signing (not notarized), Windows unsigned, Linux unsigned.
3. Create a release tag on the verified `main` commit (example for the current code version):

   ```bash
   git checkout main
   git pull --ff-only
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. GitHub Actions validates versions, builds macOS ARM64 + Intel, Windows x64, and Linux x64, and uploads the installers to a **Draft / Pre-release**. A final job adds `SHA256SUMS`.
5. Review [GitHub Releases](https://github.com/tans/ema/releases) and the [Release workflow](https://github.com/tans/ema/actions/workflows/release.yml). Download installers and verify `SHA256SUMS`, macOS ad-hoc signature, launch/tray/settings, and the local mailbox/Codex workflow on controlled test machines before manually publishing the draft.
6. In the GitHub release editor, keep **Pre-release** checked for experimental versions. Never label an unverified build as stable.

A release is **not** created when merely merging a PR. Only a matching pushed version tag starts the desktop release workflow.

## Signing, notarization, and updater

**Zero-cost alpha policy (current):**
- **macOS**: Tauri `bundle.macOS.signingIdentity = "-"` produces a free **ad-hoc code signature**. The Release workflow verifies the app's signature with `codesign`. It is **not Developer ID signed and not notarized**. Gatekeeper may still block the first launch; see [Chinese installation guide](INSTALL.zh-CN.md) for the supported per-app approval procedure.
- **Windows**: installer is **not Authenticode signed**. SmartScreen/unknown publisher warnings are possible. Do not advise users to globally disable protection. A future option for qualifying open-source projects is SignPath Foundation.
- **Linux**: AppImage/DEB require no paid publisher certificate, but still need source and checksum verification.
- **Checksums**: the workflow appends `SHA256SUMS` to the draft release after all matrix jobs succeed. Hashes check file integrity but do not authenticate the developer independently.
- **Updater**: disabled; no Tauri updater signing keys, endpoint or silent updates are configured. No Apple/Windows code-signing Secrets are required for current packaging.

**Before a public preview:** Confirm live email-origin security boundaries, local Codex test flow and installer behavior. Keep the release as Draft until smoke testing has passed. SignPath, Apple Developer ID/notarization and signed Tauri Updater remain future, separately reviewed decisions. Do not commit signing keys, passwords, or API credentials.

Reference: [Tauri macOS ad-hoc signing](https://tauri.app/distribute/sign/macos/).

## Release acceptance

- Version is consistent and tag is reachable from `main`
- GitHub Actions release matrix succeeds for all supported platforms
- Installer files and `SHA256SUMS` are attached to the same draft release
- macOS package passes `codesign --verify`, and its signature is marked `Signature=adhoc`
- Download instructions clearly state no Apple notarization / Windows publisher certificate
- The desktop app launches, hides to tray, and reopens without losing settings
- IMAP/SMTP, email threading, local Codex, and read-only task execution are verified against test accounts
- Risk notes and remaining Issue #1 work are clearly documented in release notes
