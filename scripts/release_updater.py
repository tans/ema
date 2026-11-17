#!/usr/bin/env python3
"""Collect exact release assets and verify updater signatures before publishing."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone


def installers(version, scope):
    files = [f"EMA_{version}_aarch64.dmg"]
    if scope == "full":
        files += [f"EMA_{version}_x64.dmg", f"EMA_{version}_amd64.deb", f"EMA_{version}_amd64.AppImage",
                  f"EMA_{version}_x64-setup.exe", f"EMA_{version}_x64_en-US.msi"]
    return files


def updater_targets(version, scope):
    targets = {"darwin-aarch64": f"EMA_{version}_aarch64.app.tar.gz"}
    if scope == "full":
        targets.update({"darwin-x86_64": f"EMA_{version}_x64.app.tar.gz",
                        "linux-x86_64-appimage": f"EMA_{version}_amd64.AppImage",
                        "windows-x86_64-nsis": f"EMA_{version}_x64-setup.exe",
                        "windows-x86_64-msi": f"EMA_{version}_x64_en-US.msi"})
    return targets


def expected_assets(version, scope):
    targets = updater_targets(version, scope)
    return set(installers(version, scope)) | set(targets.values()) | {name + ".sig" for name in targets.values()}


def collect(root, output, version, platform):
    """macOS archives have identical bundle names, so give them architecture names."""
    output.mkdir(parents=True, exist_ok=True)
    if platform in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
        arch = "aarch64" if platform.startswith("aarch64") else "x64"
        names = {f"EMA_{version}_{arch}.dmg": f"EMA_{version}_{arch}.dmg",
                 "EMA.app.tar.gz": f"EMA_{version}_{arch}.app.tar.gz",
                 "EMA.app.tar.gz.sig": f"EMA_{version}_{arch}.app.tar.gz.sig"}
    elif platform == "linux":
        names = {name: name for name in [f"EMA_{version}_amd64.deb", f"EMA_{version}_amd64.AppImage", f"EMA_{version}_amd64.AppImage.sig"]}
    elif platform == "windows":
        names = {name: name for name in [f"EMA_{version}_x64-setup.exe", f"EMA_{version}_x64-setup.exe.sig", f"EMA_{version}_x64_en-US.msi", f"EMA_{version}_x64_en-US.msi.sig"]}
    else:
        raise ValueError(f"Unsupported build platform: {platform}")
    for source, destination in names.items():
        matches = list(root.glob(f"release/bundle/*/{source}"))
        if len(matches) != 1 or matches[0].stat().st_size == 0:
            raise ValueError(f"Expected exactly one nonempty build artifact: {source}")
        shutil.copyfile(matches[0], output / destination)


def manifest(folder, version, scope, repository, pubkey, verifier="minisign"):
    expected = expected_assets(version, scope)
    actual = {p.name for p in folder.iterdir() if p.is_file()} - {"SHA256SUMS", "latest.json"}
    if actual != expected:
        raise ValueError(f"Release asset mismatch: missing={sorted(expected-actual)}, unexpected={sorted(actual-expected)}")
    for name in expected:
        if not (folder / name).stat().st_size:
            raise ValueError(f"Empty release asset: {name}")
    platforms = {}
    with tempfile.TemporaryDirectory(prefix="ema-verify-") as temp:
        public = Path(temp) / "updater.pub"
        public.write_bytes(base64.b64decode(pubkey, validate=True))
        for target, name in updater_targets(version, scope).items():
            signature = (folder / (name + ".sig")).read_text().strip()
            decoded = Path(temp) / "updater.sig"
            decoded.write_bytes(base64.b64decode(signature, validate=True))
            lines = decoded.read_text().splitlines()
            if len(lines) != 4 or f"version:{version}" not in lines[2].removeprefix("trusted comment: ").split("\t"):
                raise ValueError(f"Updater signature does not bind release version: {name}")
            result = subprocess.run([verifier, "-Vm", str(folder / name), "-p", str(public), "-x", str(decoded)], capture_output=True, text=True)
            if result.returncode:
                raise ValueError(f"Updater signature verification failed: {name}")
            platforms[target] = {"signature": signature, "url": f"https://github.com/{repository}/releases/download/v{version}/{name}"}
    # Older updater clients use the generic Windows key. Choose NSIS explicitly.
    if "windows-x86_64-nsis" in platforms:
        platforms["windows-x86_64"] = platforms["windows-x86_64-nsis"]
    result = {"version": version, "notes": "EMA " + version, "pub_date": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"), "platforms": platforms}
    (folder / "latest.json").write_text(json.dumps(result, indent=2) + "\n")
    with (folder / "SHA256SUMS").open("w") as out:
        for name in sorted(expected | {"latest.json"}):
            with (folder / name).open("rb") as artifact:
                digest = hashlib.file_digest(artifact, "sha256").hexdigest()
            out.write(f"{digest}  {name}\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["collect", "manifest"])
    parser.add_argument("--version", required=True)
    parser.add_argument("--folder", type=Path, required=True)
    parser.add_argument("--root", type=Path)
    parser.add_argument("--platform")
    parser.add_argument("--scope", choices=["patch", "full"])
    parser.add_argument("--repository", default="tans/ema")
    args = parser.parse_args()
    if args.mode == "collect":
        collect(args.root, args.folder, args.version, args.platform)
    else:
        config = json.loads(Path("src-tauri/tauri.conf.json").read_text())
        manifest(args.folder, args.version, args.scope, args.repository, config["plugins"]["updater"]["pubkey"])
    print(f"Verified {args.mode} assets for {args.version}")


if __name__ == "__main__":
    main()
