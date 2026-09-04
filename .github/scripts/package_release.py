#!/usr/bin/env python3

import argparse
import hashlib
import re
import shutil
import tarfile
import zipfile
from pathlib import Path


def safe_name(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", value):
        raise argparse.ArgumentTypeError("expected a plain filename, not a path")
    return value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Package a speedtest release binary")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--artifact", required=True, type=safe_name)
    parser.add_argument("--archive", required=True, type=safe_name)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if not args.archive.endswith((".zip", ".tar.gz")):
        raise SystemExit("archive must end in .zip or .tar.gz")
    root = Path.cwd()
    binary = root / args.binary
    if not binary.is_file():
        raise SystemExit(f"release binary not found: {binary}")

    dist = root / "dist"
    package_dir = dist / args.artifact
    if package_dir.exists():
        shutil.rmtree(package_dir)
    package_dir.mkdir(parents=True, exist_ok=True)

    binary_name = "speedtest.exe" if binary.suffix.lower() == ".exe" else "speedtest"
    packaged_binary = package_dir / binary_name
    shutil.copy2(binary, packaged_binary)
    if binary_name == "speedtest":
        packaged_binary.chmod(0o755)

    for filename in ("README.md", "LICENSE"):
        source = root / filename
        if source.is_file():
            shutil.copy2(source, package_dir / filename)

    archive = dist / args.archive
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as handle:
            for path in sorted(package_dir.rglob("*")):
                if path.is_file():
                    handle.write(path, path.relative_to(dist))
    elif archive.name.endswith(".tar.gz"):
        with tarfile.open(archive, "w:gz") as handle:
            handle.add(package_dir, arcname=args.artifact)
    else:
        raise SystemExit(f"unsupported archive format: {archive.name}")

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_name(f"{archive.name}.sha256")
    checksum.write_text(f"{digest}  {archive.name}\n", encoding="utf-8")

    print(f"created {archive.relative_to(root)}")
    print(f"created {checksum.relative_to(root)}")


if __name__ == "__main__":
    main()
