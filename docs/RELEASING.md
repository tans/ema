# EMA desktop release procedure

This project uses **Tauri 2 + Rust + React**. Release automation is defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Versioning

A tag `vX.Y.Z` triggers a release only when it refers to a commit already on `main`. For v0.1.1, merging the release-workflow update into `main` triggers a one-time public Alpha build; a GitHub-hosted job creates the version tag and drafts the release before building packages. Subsequent releases use explicit tags. The tag must match the version in:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- `crates/core/Cargo.toml`

Do not reuse or force-update version tags. For later versions, update all four files together in a reviewed change.

## Publish checklist

1. Ensure the code is merged to `main` and Build CI is green. Use a tag to create an unpublished **Draft Pre-release**; assets are made publicly downloadable only after all platform build jobs and checksums pass. A controlled mailbox/Codex end-to-end test is required **before publishing**.
2. Confirm the project license, security limitations, and the no-cost distribution policy: macOS ad-hoc signing (not notarized), Windows unsigned, Linux unsigned.
3. Create a release tag on the verified `main` commit (example for the current code version):

   ```bash
   git checkout main
   git pull --ff-only
   git tag v0.1.1
   git push origin v0.1.1
   ```

4. GitHub Actions validates versions, builds macOS ARM64 + Intel, Windows x64, and Linux x64, and uploads the installers to a **Draft / Pre-release**. A final job adds `SHA256SUMS` and **automatically publishes a public Pre-release only if all matrix builds succeed**. If any build fails the release remains a Draft (not publicly downloadable).
5. Review [GitHub Releases](https://github.com/tans/ema/releases) and the [Release workflow](https://github.com/tans/ema/actions/workflows/release.yml). Verify `SHA256SUMS`, macOS ad-hoc signature, launch/tray/settings, and the local mailbox/Codex workflow on controlled test machines **before enabling the gateway or using sensitive code**. Publication of the Alpha build is automatic once packaging succeeds; it does **not** imply functional/security acceptance.
6. Keep **Pre-release** checked for experimental versions. The workflow publishes `v0.1.1` as Alpha, never stable; platform installers remain unsigned/unnotarized as documented.

**Exception for the initial Alpha:** merging the one-time `v0.1.1` release bootstrap configuration to `main` starts a GitHub Actions run which creates the tag and, when successful, publishes the pre-release. For future versions only a matching pushed version tag starts the normal release workflow.

## Signing, notarization, and updater

**Zero-cost alpha policy (current):**
- **macOS**: Tauri `bundle.macOS.signingIdentity = "-"` produces a free **ad-hoc code signature**. The Release workflow verifies the app's signature with `codesign`. It is **not Developer ID signed and not notarized**. Gatekeeper may still block the first launch; see [Chinese installation guide](INSTALL.zh-CN.md) for the supported per-app approval procedure.
- **Windows**: installer is **not Authenticode signed**. SmartScreen/unknown publisher warnings are possible. Do not advise users to globally disable protection. A future option for qualifying open-source projects is SignPath Foundation.
- **Linux**: AppImage/DEB require no paid publisher certificate, but still need source and checksum verification.
- **Checksums**: the workflow appends `SHA256SUMS` to the draft release after all matrix jobs succeed. Hashes check file integrity but do not authenticate the developer independently.
- **Updater**: disabled; no Tauri updater signing keys, endpoint or silent updates are configured. No Apple/Windows code-signing Secrets are required for current packaging.

**Before enabling the gateway:** Verify live email-origin security boundaries, local Codex test flow and installer behavior. Public **Alpha** is explicitly for download/testing and must not be described as production-ready. Packaging success is not email/Agent security acceptance. SignPath, Apple Developer ID/notarization and signed Tauri Updater remain future, separately reviewed decisions. Do not commit signing keys, passwords, or API credentials.

Reference: [Tauri macOS ad-hoc signing](https://tauri.app/distribute/sign/macos/).

## Release acceptance

- Version is consistent and tag is reachable from `main`
- GitHub Actions release matrix succeeds for all supported platforms
- Installer files and `SHA256SUMS` are attached before automatic public **Pre-release** publication
- macOS package passes `codesign --verify`, and its signature is marked `Signature=adhoc`
- Download instructions clearly state no Apple notarization / Windows publisher certificate
- The desktop app launches, hides to tray, and reopens without losing settings
- IMAP/SMTP, email threading, local Codex, and read-only task execution are verified against test accounts
- Risk notes and remaining Issue #1 work are clearly documented in release notes
