#!/usr/bin/env python3
"""Render the cockpit redesign gallery from actual captured Ratatui buffers.

Run with only --before-dir to prepare baseline screenshots. Add --after-dir and
--motion-dir after capturing the redesigned renderer. Requires Pillow and DejaVu
Sans Mono; rendering is shared with the reviewed motion evidence rasterizer.
"""
import argparse
import importlib.util
from pathlib import Path

RASTER_PATH = Path(__file__).resolve().parent.parent / "motion" / "render_motion.py"
SPEC = importlib.util.spec_from_file_location("motion_gallery_renderer", RASTER_PATH)
RASTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RASTER)

SCENES = (
    ("home", "HOME", 120, 38),
    ("live", "LIVE TEST", 120, 38),
    ("results", "RESULTS", 120, 38),
    ("live", "LIVE TEST", 80, 24),
)


def stills(source_dir, destination_dir, label):
    for screen, title, width, height in SCENES:
        source = source_dir / f"graphite-{screen}-en-{width}x{height}.json"
        suffix = f"-{width}x{height}" if width == 80 else ""
        destination = destination_dir / f"{label}-{screen}{suffix}.png"
        RASTER.raster(
            source,
            f"{label.upper()} | {title} | {width} x {height} | SAME OFFLINE FIXTURE",
            font_size=16,
        ).save(destination)
        print(f"{destination.name}: {destination.stat().st_size:,} bytes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before-dir", required=True, type=Path)
    parser.add_argument("--after-dir", type=Path)
    parser.add_argument("--motion-dir", type=Path)
    parser.add_argument("--out-dir", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stills(args.before_dir, args.out_dir, "before")
    if args.after_dir:
        stills(args.after_dir, args.out_dir, "after")
    if args.motion_dir:
        RASTER.gif(
            args.motion_dir / "live",
            args.out_dir / "live-motion.gif",
            "LIVE COCKPIT | synthetic download, upload, and result reveal",
        )


if __name__ == "__main__":
    main()
