# EMA desktop releases

EMA uses Tauri 2 + Rust + React. Release automation is in [release.yml](../.github/workflows/release.yml).

## Version policy

All four version manifests must agree: package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, crates/core/Cargo.toml.

| Version | Example | Installers published |
| --- | --- | --- |
| **Patch X.Y.Z (Z > 0)** | v0.1.4, v0.2.1 | **macOS Apple Silicon / M-series only** — ARM64 DMG |
| **Minor or major X.Y.0** | v0.2.0, v1.0.0 | **All platforms** — macOS ARM64 + Intel DMGs, Windows x64 NSIS + MSI, Linux x64 DEB + AppImage |

A patch release does not update Intel Mac, Windows, or Linux installers; they receive updated builds at the next minor or major release. This policy changes only downloadable **Release** packages, not normal pull request and main-branch Build CI.

The release workflow computes its platform matrix automatically from the patch component of the version string.

## Trigger and publication

1. Merge the change and matching version bump to main. The current repo automatically starts its release workflow when package.json or release.yml changes on main. A matching vX.Y.Z tag (on a main commit) or manual dispatch from main can also start it. Never reuse a published version tag.
2. Validate the numeric X.Y.Z version and all four manifests. An already-public version is not rebuilt.
3. Create or reuse a draft Alpha release; build **only** the selected version matrix. Verify macOS ad-hoc signatures in all produced DMGs.
4. Require exactly the expected assets for that scope, upload SHA256SUMS, and publish only when all selected builds and the checksum step succeed. Otherwise leave the release a draft.
5. Confirm the file list and checksums at [Releases](https://github.com/tans/ema/releases) and monitor [Release Actions](https://github.com/tans/ema/actions/workflows/release.yml).

For v0.1.4, expect EMA_0.1.4_aarch64.dmg plus SHA256SUMS. For v0.2.0, expect the six desktop installers plus SHA256SUMS. The exclusive release queue does not cancel existing runs, so a new version may queue behind an ongoing release.

## Signing and Alpha safeguards

- **macOS:** free ad-hoc signing, not Apple Developer ID signing or notarization. The user may need a per-app Gatekeeper approval.
- **Windows:** unsigned installers. **Linux:** unsigned packages.
- **SHA256SUMS** verifies download integrity but does not authenticate the publisher.
- **Updater:** not enabled. No signing secrets are required for this Alpha policy.
- CI packaging success is not live mailbox or Agent security acceptance. Before using a real workstation, verify IMAP/SMTP, thread resume, and read-only Codex tasks with a controlled test account. An allowed From address alone does not cryptographically prove email origin.

See [中文安装指南](INSTALL.zh-CN.md) and [Tauri macOS signing](https://tauri.app/distribute/sign/macos/).
