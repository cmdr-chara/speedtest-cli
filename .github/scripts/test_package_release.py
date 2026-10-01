#!/usr/bin/env python3
"""Exercise release packaging in temporary directories, never publish artifacts."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

SCRIPT = Path(__file__).with_name("package_release.py").resolve()
MANIFEST_SCRIPT = Path(__file__).with_name("generate_package_manifests.py").resolve()


class PackagingTests(unittest.TestCase):
    def run_packager(self, root, *args):
        return subprocess.run([sys.executable, str(SCRIPT), "--binary", "speedtest", *args],
                              cwd=root, capture_output=True, text=True, timeout=10)

    def test_archive_contents_and_checksums(self):
        for suffix in (".zip", ".tar.gz"):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                (root / "speedtest").write_bytes(b"fixture binary")
                (root / "README.md").write_text("fixture docs", encoding="utf-8")
                (root / "LICENSE").write_text("fixture license", encoding="utf-8")
                archive_name = "test-platform" + suffix
                result = self.run_packager(root, "--artifact", "test-platform", "--archive", archive_name)
                self.assertEqual(result.returncode, 0, result.stderr)
                archive = root / "dist" / archive_name
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual(archive.with_name(archive.name + ".sha256").read_text().strip(), f"{digest}  {archive_name}")
                if suffix == ".zip":
                    with zipfile.ZipFile(archive) as handle:
                        names = handle.namelist()
                        self.assertEqual(handle.read("test-platform/speedtest"), b"fixture binary")
                else:
                    with tarfile.open(archive) as handle:
                        names = handle.getnames()
                        self.assertEqual(handle.extractfile("test-platform/speedtest").read(), b"fixture binary")
                        self.assertEqual(handle.getmember("test-platform/speedtest").mode & 0o777, 0o755)
                self.assertIn("test-platform/README.md", names)
                self.assertIn("test-platform/LICENSE", names)

    def test_rejects_unsafe_paths_before_touching_files(self):
        for flag, value in (("--artifact", "../keep"), ("--archive", "../keep.zip"),
                            ("--artifact", "..\\keep"), ("--archive", "invalid.ext")):
            with self.subTest(flag=flag, value=value), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                (root / "speedtest").write_bytes(b"fixture")
                marker = root / "keep"
                marker.mkdir()
                (marker / "unchanged").write_text("keep", encoding="utf-8")
                arguments = ["--artifact", "safe", "--archive", "safe.zip", flag, value]
                self.assertNotEqual(self.run_packager(root, *arguments).returncode, 0)
                self.assertTrue((marker / "unchanged").exists())
                self.assertFalse((root / "dist").exists())

    def test_generates_homebrew_and_winget_metadata_from_release_checksums(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            dist = root / "dist"
            dist.mkdir()
            assets = (
                "speedtest-linux-x86_64.tar.gz",
                "speedtest-macos-aarch64.tar.gz",
                "speedtest-macos-x86_64.tar.gz",
                "speedtest-windows-x86_64.zip",
            )
            for index, asset in enumerate(assets, start=1):
                (dist / asset).write_bytes(f"asset-{index}".encode())
                digest = hashlib.sha256(f"asset-{index}".encode()).hexdigest()
                (dist / f"{asset}.sha256").write_text(f"{digest}  {asset}\n", encoding="utf-8")
            result = subprocess.run(
                [
                    sys.executable,
                    str(MANIFEST_SCRIPT),
                    "--version",
                    "1.2.3",
                    "--repository",
                    "example/speedtest-cli",
                    "--dist",
                    str(dist),
                ],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            formula = (dist / "speedtest-homebrew.rb").read_text(encoding="utf-8")
            self.assertIn('version "1.2.3"', formula)
            self.assertIn(
                hashlib.sha256(b"asset-1").hexdigest(),
                formula,
            )
            installer = (dist / "speedtest-winget-installer.yaml").read_text(encoding="utf-8")
            self.assertIn("PackageIdentifier: CmdrChara.SpeedtestCli", installer)
            self.assertIn("speedtest-windows-x86_64.zip", installer)

    def test_manifest_generator_rejects_unsafe_version_before_writing(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            dist = root / "dist"
            dist.mkdir()
            result = subprocess.run(
                [
                    sys.executable,
                    str(MANIFEST_SCRIPT),
                    "--version",
                    "../unsafe",
                    "--repository",
                    "example/speedtest-cli",
                    "--dist",
                    str(dist),
                ],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(list(dist.iterdir()), [])

    def test_manifest_generator_rejects_a_stale_checksum(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            dist = root / "dist"
            dist.mkdir()
            assets = (
                "speedtest-linux-x86_64.tar.gz",
                "speedtest-macos-aarch64.tar.gz",
                "speedtest-macos-x86_64.tar.gz",
                "speedtest-windows-x86_64.zip",
            )
            for asset in assets:
                (dist / asset).write_bytes(b"actual")
                (dist / f"{asset}.sha256").write_text(
                    f"{'0' * 64}  {asset}\n", encoding="utf-8"
                )
            result = subprocess.run(
                [
                    sys.executable,
                    str(MANIFEST_SCRIPT),
                    "--version",
                    "1.2.3",
                    "--repository",
                    "example/speedtest-cli",
                    "--dist",
                    str(dist),
                ],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((dist / "speedtest-homebrew.rb").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
