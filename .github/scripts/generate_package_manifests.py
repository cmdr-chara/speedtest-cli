#!/usr/bin/env python3
"""Generate package-manager metadata from the already verified release assets.

The repository does not publish directly to Homebrew or WinGet. Release jobs
attach these generated files so a package maintainer can review and submit the
exact version and checksums without hand-editing them.
"""

import argparse
import hashlib
import re
from pathlib import Path


VERSION_RE = re.compile(r"\A\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?\Z")
REPOSITORY_RE = re.compile(r"\A[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")
SHA256_RE = re.compile(r"\A[0-9a-f]{64}\Z")


ASSETS = {
    "linux": "speedtest-linux-x86_64.tar.gz",
    "mac_arm": "speedtest-macos-aarch64.tar.gz",
    "mac_intel": "speedtest-macos-x86_64.tar.gz",
    "windows": "speedtest-windows-x86_64.zip",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--dist", type=Path, default=Path("dist"))
    parser.add_argument(
        "--winget-id", default="CmdrChara.SpeedtestCli", help="WinGet package identifier"
    )
    return parser.parse_args()


def validate_args(args: argparse.Namespace) -> None:
    if not VERSION_RE.fullmatch(args.version):
        raise SystemExit("version must be a semantic release version")
    if not REPOSITORY_RE.fullmatch(args.repository):
        raise SystemExit("repository must be owner/name")
    if not re.fullmatch(r"[A-Za-z0-9.-]+", args.winget_id):
        raise SystemExit("winget package ID contains unsafe characters")


def read_checksums(dist: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for key, asset in ASSETS.items():
        if not (dist / asset).is_file():
            raise SystemExit(f"release asset not found: {dist / asset}")
        checksum_path = dist / f"{asset}.sha256"
        try:
            fields = checksum_path.read_text(encoding="utf-8").split()
        except OSError as error:
            raise SystemExit(f"failed to read {checksum_path}: {error}") from error
        if len(fields) != 2 or fields[1] != asset or not SHA256_RE.fullmatch(fields[0]):
            raise SystemExit(f"invalid checksum record in {checksum_path}")
        digest = hashlib.sha256((dist / asset).read_bytes()).hexdigest()
        if digest != fields[0]:
            raise SystemExit(f"checksum does not match {dist / asset}")
        checksums[key] = fields[0]
    return checksums


def write(path: Path, content: str) -> None:
    path.write_text(content.rstrip() + "\n", encoding="utf-8")


def homebrew_formula(version: str, repository: str, checksums: dict[str, str]) -> str:
    base = f"https://github.com/{repository}/releases/download/v{version}"
    return f'''class SpeedtestCli < Formula
  desc "Terminal network quality analyzer"
  homepage "https://github.com/{repository}"
  version "{version}"
  # The project uses a custom Source Available License 1.0; review LICENSE.
  license "LicenseRef-Source-Available-1.0"

  if OS.mac? && Hardware::CPU.arm?
    url "{base}/{ASSETS["mac_arm"]}"
    sha256 "{checksums["mac_arm"]}"
  elsif OS.mac?
    url "{base}/{ASSETS["mac_intel"]}"
    sha256 "{checksums["mac_intel"]}"
  elsif OS.linux? && Hardware::CPU.intel?
    url "{base}/{ASSETS["linux"]}"
    sha256 "{checksums["linux"]}"
  else
    odie "This release provides macOS arm64/x86_64 and Linux x86_64 archives"
  end

  def install
    bin.install Dir["*/speedtest"].first
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/speedtest --version")
  end
end
'''


def winget_manifests(version: str, repository: str, package_id: str, windows_sha: str) -> dict[str, str]:
    url = (
        f"https://github.com/{repository}/releases/download/v{version}/"
        f"{ASSETS['windows']}"
    )
    common = f"PackageIdentifier: {package_id}\nPackageVersion: {version}\n"
    return {
        "speedtest-winget-version.yaml": common
        + "DefaultLocale: en-US\n"
        + "ManifestType: version\nManifestVersion: 1.9.0\n",
        "speedtest-winget-installer.yaml": common
        + "Installers:\n"
        + "- Architecture: x64\n"
        + "  InstallerType: zip\n"
        + f"  InstallerUrl: {url}\n"
        + f"  InstallerSha256: {windows_sha}\n"
        + "  NestedInstallerType: portable\n"
        + "  NestedInstallerFiles:\n"
        + "  - RelativeFilePath: speedtest-windows-x86_64/speedtest.exe\n"
        + "    PortableCommandAlias: speedtest\n"
        + "ManifestType: installer\nManifestVersion: 1.9.0\n",
        "speedtest-winget-locale.yaml": common
        + "PackageLocale: en-US\n"
        + "Publisher: Cmdr Chara\n"
        + "PublisherUrl: https://github.com/cmdr-chara\n"
        + "PackageName: speedtest-cli\n"
        + "License: Source Available License 1.0\n"
        + f"LicenseUrl: https://github.com/{repository}/blob/v{version}/LICENSE\n"
        + "ShortDescription: Measure throughput, latency, and connection quality\n"
        + "ManifestType: defaultLocale\nManifestVersion: 1.9.0\n",
    }


def main() -> None:
    args = parse_args()
    validate_args(args)
    checksums = read_checksums(args.dist)
    args.dist.mkdir(parents=True, exist_ok=True)

    write(args.dist / "speedtest-homebrew.rb", homebrew_formula(args.version, args.repository, checksums))
    for filename, content in winget_manifests(
        args.version, args.repository, args.winget_id, checksums["windows"]
    ).items():
        write(args.dist / filename, content)

    print("created package-manager metadata")


if __name__ == "__main__":
    main()
