import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_updater", ROOT / "scripts/release_updater.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseAssets(unittest.TestCase):
    def test_platform_policy_and_architecture_specific_names(self):
        self.assertEqual(release.updater_targets("1.2.3", "patch"), {"darwin-aarch64": "EMA_1.2.3_aarch64.app.tar.gz"})
        targets = release.updater_targets("1.2.0", "full")
        self.assertEqual(set(targets), {"darwin-aarch64", "darwin-x86_64", "linux-x86_64-appimage", "windows-x86_64-nsis", "windows-x86_64-msi"})
        self.assertEqual(len(release.expected_assets("1.2.3", "patch")), 3)
        self.assertEqual(len(release.expected_assets("1.2.0", "full")), 13)

    def test_collect_renames_mac_archive_and_requires_signature(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            bundle = root / "release/bundle/macos"
            bundle.mkdir(parents=True)
            (bundle / "EMA.app.tar.gz").write_bytes(b"archive")
            dmg = root / "release/bundle/dmg"
            dmg.mkdir()
            (dmg / "EMA_1.2.3_aarch64.dmg").write_bytes(b"installer")
            out = root / "assets"
            with self.assertRaisesRegex(ValueError, "sig"):
                release.collect(root, out, "1.2.3", "aarch64-apple-darwin")
            (bundle / "EMA.app.tar.gz.sig").write_bytes(b"signature")
            release.collect(root, out, "1.2.3", "aarch64-apple-darwin")
            self.assertEqual({p.name for p in out.iterdir()}, release.expected_assets("1.2.3", "patch"))


class SignedManifest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.verifier = os.environ.get("EMA_TEST_MINISIGN") or shutil.which("minisign")
        if not cls.verifier:
            raise unittest.SkipTest("Install minisign to run cryptographic release tests")
        cls.temp = tempfile.TemporaryDirectory(prefix="ema-updater-tests-")
        cls.root = Path(cls.temp.name)
        cls.key = cls.root / "test.key"
        cls.cli = ROOT / "node_modules/.bin/tauri"
        result = subprocess.run([str(cls.cli), "signer", "generate", "--ci", "-p", "disposable-test-password", "-w", str(cls.key)], capture_output=True)
        if result.returncode:
            cls.temp.cleanup()
            raise RuntimeError("Could not generate disposable test signing key")
        cls.public = Path(str(cls.key) + ".pub").read_text().strip()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def fixture(self, scope, version):
        folder = self.root / self.id().split(".")[-1]
        folder.mkdir()
        for name in release.expected_assets(version, scope):
            if not name.endswith(".sig"):
                (folder / name).write_bytes(("test artifact " + name).encode())
        for name in release.updater_targets(version, scope).values():
            result = subprocess.run([str(self.cli), "signer", "sign", "-f", str(self.key), "-p", "disposable-test-password", "--app-version", version, str(folder / name)], capture_output=True)
            self.assertEqual(result.returncode, 0, "Could not sign disposable fixture")
        return folder

    def generate(self, folder, scope="patch", version="1.2.3", public=None):
        return release.manifest(folder, version, scope, "tans/ema", public or self.public, self.verifier)

    def test_full_manifest_contains_exact_https_architecture_assets_and_checksums(self):
        folder = self.fixture("full", "1.2.0")
        manifest = self.generate(folder, "full", "1.2.0")
        self.assertEqual(len(manifest["platforms"]), 6)
        for platform, asset in release.updater_targets("1.2.0", "full").items():
            self.assertEqual(manifest["platforms"][platform]["url"], f"https://github.com/tans/ema/releases/download/v1.2.0/{asset}")
            self.assertEqual(manifest["platforms"][platform]["signature"], (folder / (asset + ".sig")).read_text().strip())
        self.assertEqual(len((folder / "SHA256SUMS").read_text().splitlines()), 14)
        self.assertEqual(json.loads((folder / "latest.json").read_text()), manifest)

    def test_patch_manifest_never_advertises_other_architectures(self):
        folder = self.fixture("patch", "1.2.3")
        manifest = self.generate(folder)
        self.assertEqual(set(manifest["platforms"]), {"darwin-aarch64"})
        self.assertEqual(len(list(folder.iterdir())), 5)

    def test_tampered_payload_never_produces_manifest(self):
        folder = self.fixture("patch", "1.2.3")
        (folder / "EMA_1.2.3_aarch64.app.tar.gz").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "verification failed"):
            self.generate(folder)
        self.assertFalse((folder / "latest.json").exists())

    def test_wrong_key_never_produces_manifest(self):
        folder = self.fixture("patch", "1.2.3")
        actual = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]
        with self.assertRaisesRegex(ValueError, "verification failed"):
            self.generate(folder, public=actual)
        self.assertFalse((folder / "latest.json").exists())

    def test_tampered_signed_version_is_rejected(self):
        folder = self.fixture("patch", "1.2.3")
        sig = folder / "EMA_1.2.3_aarch64.app.tar.gz.sig"
        content = base64.b64decode(sig.read_text()).replace(b"version:1.2.3", b"version:9.9.9")
        sig.write_text(base64.b64encode(content).decode())
        with self.assertRaisesRegex(ValueError, "bind release version"):
            self.generate(folder)
        self.assertFalse((folder / "latest.json").exists())

    def test_missing_signature_and_stale_platform_asset_fail_closed(self):
        folder = self.fixture("patch", "1.2.3")
        extra = folder / "EMA_1.2.3_x64.dmg"
        extra.write_bytes(b"stale")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.generate(folder)
        extra.unlink()
        (folder / "EMA_1.2.3_aarch64.app.tar.gz.sig").unlink()
        with self.assertRaisesRegex(ValueError, "missing"):
            self.generate(folder)
        self.assertFalse((folder / "latest.json").exists())


if __name__ == "__main__":
    unittest.main()
