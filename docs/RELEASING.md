# EMA desktop releases

EMA uses Tauri 2 + Rust + React. [release.yml](../.github/workflows/release.yml) builds installers and signed official Tauri updater artifacts. The updater uses this fixed HTTPS endpoint:

`https://github.com/tans/ema/releases/latest/download/latest.json`

## Version and platform policy

All four version manifests must agree: package.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, crates/core/Cargo.toml. Use a new numeric X.Y.Z version; never rebuild or move a published tag.

| Version | Installers | Updater platforms |
| --- | --- | --- |
| Patch X.Y.Z (Z > 0) | Apple Silicon ARM64 DMG | darwin-aarch64 |
| Minor/major X.Y.0 | macOS ARM64 + Intel DMGs, Linux x64 DEB + AppImage, Windows x64 NSIS + MSI | darwin-aarch64, darwin-x86_64, linux-x86_64-appimage, windows-x86_64-nsis, windows-x86_64-msi (generic windows-x86_64 selects NSIS) |

macOS updater payloads are signed `.app.tar.gz` archives, not DMGs. They are renamed to `EMA_VERSION_aarch64.app.tar.gz` and `EMA_VERSION_x64.app.tar.gz` so the two architectures cannot overwrite each other. Linux uses the raw signed AppImage; Windows uses the signed raw EXE/MSI, with bundle-specific keys so an MSI install keeps its installer type. Each updater payload has a `.sig` sidecar. `latest.json` contains the **contents** of those sidecars, not signature URLs. `SHA256SUMS` covers installers, updater payloads, signatures and latest.json.

A patch manifest intentionally omits Intel, Windows and Linux. Those clients cannot update to the patch and wait for the next full release. With GitHub's single latest endpoint they do not fall back to a previous full release; manual checking may report a missing-platform error. Linux DEB installations need a manual DEB upgrade; this release feed advertises only the bundle-specific AppImage target (no generic Linux fallback), so DEB clients cannot accidentally select an AppImage payload. No ARM Windows/Linux or universal macOS updater is advertised.

## Signing key provisioning (manual, before release)

The current public key is checked into `src-tauri/tauri.conf.json`. Its encrypted private key was generated locally outside the repository:

- Private key: `/Users/ke/.config/ema/updater/signing.key`
- Password: `/Users/ke/.config/ema/updater/signing.password`
- Public key copy: `/Users/ke/.config/ema/updater/signing.key.pub`

The directory is mode 0700; private key/password files are mode 0600. Never commit these files, put their contents in a chat/log, or upload them as build artifacts. Back them up securely. The following repository Actions secrets must be set by the maintainer:

| GitHub Actions secret | Value source |
| --- | --- |
| TAURI_SIGNING_PRIVATE_KEY | Entire contents of signing.key (the Tauri encoded encrypted private key, not its path) |
| TAURI_SIGNING_PRIVATE_KEY_PASSWORD | Contents of signing.password |

Optional maintainer commands, run locally without printing secret values:

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < /Users/ke/.config/ema/updater/signing.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < /Users/ke/.config/ema/updater/signing.password
```

These commands have **not** been executed as part of implementation. The release validation job fails clearly before draft creation when either secret is empty. An incorrect key/password fails bundling or the subsequent public-key verification, leaving the release unpublished. Do not generate a new key on each CI run. Changing the configured public key does not rotate the trust of already-installed apps: distribute a migration signed by the old key first, or require manual installation.

`requireSignedVersion` is enabled: Tauri CLI >= 2.12.4 embeds the release version in each signature's authenticated trusted comment; updater >= 2.14.1 and the release validator reject mismatches. Downgrades are not enabled. The manifest itself is transported over HTTPS rather than separately signed.

## Manual deployment and publication

1. Configure the two secrets above, review this change, and choose a **new** version. For the first updater-capable release across all platforms, use X.Y.0. Existing installed releases without this feature require one manual installation before they can update in-app.
2. Update all four version manifests together. Merge the reviewed change and bump to the current main. This repository automatically starts release.yml when package.json or release.yml changes on main; a matching vX.Y.Z tag on main or a manual dispatch from main also triggers it. This implementation did not bump versions, commit, push, tag or publish.
3. The workflow validates source/version/secrets and creates or reuses a draft Alpha release. It builds only the selected matrix with `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, producing updater artifacts via `bundle.createUpdaterArtifacts: true`. It verifies macOS ad-hoc signatures in each DMG.
4. The collection script requires exact nonempty installer/payload/signature files. After all builds succeed, the manifest job installs `minisign` and cryptographically verifies **every** updater artifact against the committed public key, checks its signed version, rejects missing/unexpected assets, then uploads latest.json and SHA256SUMS. Any failure leaves the release a draft.
5. Publication rechecks main has not moved, requires both manifests, and publishes with `--prerelease=false --latest`. This is deliberate: GitHub's `/releases/latest/download/` does **not** select prereleases. Product maturity remains Alpha even though the GitHub prerelease flag must be false for this endpoint. Old Alpha prereleases are not modified.
6. Inspect [Releases](https://github.com/tans/ema/releases), [Release Actions](https://github.com/tans/ema/actions/workflows/release.yml), and fetch the endpoint above. Confirm the version, architecture keys, exact asset URLs and SHA256SUMS. Test an older updater-capable app on each supported platform with controlled mailbox settings, then manually approve download and installation. A successful CI run alone is not this end-to-end acceptance.

For a patch expect one DMG, one ARM64 app archive, its signature, latest.json and SHA256SUMS (5 assets). For a full release expect six installers, two macOS updater archives, five signatures, latest.json and SHA256SUMS (15 assets). The exclusive release queue does not cancel earlier runs. A draft retry can replace selected assets; stray assets from another scope cause validation to fail and must be reviewed/removed manually.

## Application behavior and mail protection

EMA checks once in the background after startup/settings load. No-update and automatic-check errors are silent. An available version produces the existing bottom-right auto-dismiss Toast; the settings button and Updates tab retain an availability indicator. Manual checks, download, verification and installation failures use that same Toast system. Updates can be checked before connecting a mailbox.

Download requires a click, reports byte/percentage progress and verifies the official signature before enabling installation. A second explicit confirmation is required for installation/restart. Downloads and available-version state survive Settings navigation in the current process; bytes are kept in memory and must be downloaded again after quitting. Download failures can retry; failed installs can retry using verified bytes; a failed restart has its own retry and does not reinstall.

The backend uses a shared activity gate for scheduled/manual IMAP polling, local replies, queued Agent execution, SMTP delivery/test, connection test and Hook collection. Installation atomically acquires an exclusive guard only when these operations are idle. It holds that guard through installation and restart, preventing new work from starting after the idle check. Busy work makes installation fail immediately with a Toast; the user retries later. Failure releases the guard. Queued work remains in SQLite for the next launch. Mail receiving does not need to be permanently disabled or its settings rewritten. Unsaved settings block the install/restart buttons until saved or discarded.

On Windows the official updater launches the installer and exits the application; macOS/Linux explicitly relaunch via the official process plugin. EMA does not force-cancel running Agent children or automatically install on a timer.

## Verification and remaining limitations

Local checks: `npm test`, `npm run build`, `cargo check --workspace`, `cargo test -p ema-core --lib`, and `python3 -m unittest discover -s tests -p 'test_*.py'`. Release tests require `minisign` (or `EMA_TEST_MINISIGN` pointing to a compatible test verifier) and use disposable keys, never the release private key. They exercise exact asset/platform selection, signed manifest generation, tampering/wrong-key rejection, and version mismatch rejection. No real update installation or live mailbox/Agent work is required.

- **macOS:** free ad-hoc signing, not Apple Developer ID signing or notarization. Updater signatures authenticate payloads but do not remove Gatekeeper approval requirements. Install in a writable Applications location, not on the mounted DMG; macOS replacement/permissions still need real-device acceptance.
- **Windows:** installers lack Authenticode signing and may show SmartScreen warnings. UAC, installer behavior and relaunch need Windows acceptance.
- **Linux:** AppImage update needs a writable AppImage path; DEB upgrades are manual.
- **Network:** GitHub release/CDN access is required. Until the first latest release with latest.json exists, manual checks may report HTTP 404; startup failures remain silent.
- Main-branch Build CI requires both signing secrets, builds an ARM64 DMG and signed updater archive, and verifies the archive and signed version against the client's configured public key using `minisign`. It uploads the DMG, archive, `.sig`, `test-manifest.json` and `SHA256SUMS` only as Actions artifacts (7-day retention, no Release writes). The test manifest uses release-shaped URLs as a fixture; it is not a published update feed. Pull requests skip this credentialed packaging job. Production release.yml always requires signatures.
- Updater signing does not authenticate email senders. Keep existing Alpha mail safeguards and use controlled test accounts/read-only context. CI packaging is not live mailbox/Agent security acceptance.

See [中文安装指南](INSTALL.zh-CN.md) and [Tauri macOS signing](https://tauri.app/distribute/sign/macos/).
