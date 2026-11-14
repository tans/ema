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

1. Ensure the code is merged to `main` and Build CI is green. Verify the application with a local Codex CLI and a controlled test mailbox before any public use.
2. Confirm copyright/license choices and security limitations, and decide whether unsigned packages are acceptable for the target audience.
3. Create a release tag on the verified `main` commit (example for the current code version):

   ```bash
   git checkout main
   git pull --ff-only
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. GitHub Actions validates versions, builds macOS ARM64 + Intel, Windows x64, and Linux x64, and uploads the installers to a **Draft / Pre-release**. A final job adds `SHA256SUMS`.
5. Review [GitHub Releases](https://github.com/tans/ema/releases) and the [Release workflow](https://github.com/tans/ema/actions/workflows/release.yml). Download installers and smoke-test every target platform before manually publishing the draft.
6. In the GitHub release editor, keep **Pre-release** checked for experimental versions. Never label an unverified build as stable.

A release is **not** created when merely merging a PR. Only a matching pushed version tag starts the desktop release workflow.

## Signing, notarization, and updater

**Not yet configured:** macOS Developer ID signing/notarization, Windows Authenticode signing, and signed Tauri updater bundles. As a result unsigned packages may be blocked or warned about by Gatekeeper/SmartScreen. Do not disable OS security policies to work around those warnings.

If public end-user distribution is required, configure platform-specific signing credentials as protected GitHub Actions secrets and integrate signing with the official Tauri procedures, then verify signed installations on clean machines. Only after establishing release signing should a Tauri auto-updater be added. Do not put signing keys or API credentials into the repository.

## Release acceptance

- Version is consistent and tag is reachable from `main`
- GitHub Actions release matrix succeeds for all supported platforms
- Installer files and `SHA256SUMS` are attached to the same draft release
- The desktop app launches, hides to tray, and reopens without losing settings
- IMAP/SMTP, email threading, local Codex, and read-only task execution are verified against test accounts
- Risk notes and remaining Issue #1 work are clearly documented in release notes
