#!/usr/bin/env python3
"""Render README screenshots and walkthroughs from actual Ratatui captures."""
import argparse
import importlib.util
import json
import tempfile
from pathlib import Path

SOURCE = Path(__file__).resolve().parent.parent / "motion" / "render_motion.py"
SPEC = importlib.util.spec_from_file_location("readme_raster", SOURCE)
RASTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RASTER)


def sequence(frames, destination, title):
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        for index, (source, duration) in enumerate(frames):
            data = json.loads(source.read_text())
            data["frame_ms"] = duration
            (root / f"{index:04}.json").write_text(json.dumps(data))
        RASTER.gif(root, destination, title)


def render(readability, review, motion, destination):
    destination.mkdir(parents=True, exist_ok=True)
    sources = {
        name: readability / f"graphite-{name}-en-120x38.json"
        for name in ("home", "live", "results", "history")
    }
    sources["compare"] = review / "compare-pinned-120x38.json"
    for name, source in sources.items():
        RASTER.raster(source, f"{name.upper()} | actual cockpit renderer", 16).save(destination / f"{name}.png")
    home = motion / "home" / "059.json"
    live = [(home, 1000)]
    live.extend((source, 50) for source in sorted((motion / "live").glob("*.json")))
    live.append((home, 1000))
    sequence(live, destination / "live-demo.gif", "LIVE DEMO | home, download, upload, results, home")
    tour = [(sources[name], 2000) for name in ("home", "history", "compare", "results")]
    tour.append((sources["home"], 50))
    sequence(tour, destination / "cockpit-tour.gif", "COCKPIT TOUR | home, history, comparison, results")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--readability-dir", type=Path, required=True)
    parser.add_argument("--review-dir", type=Path, required=True)
    parser.add_argument("--motion-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    render(args.readability_dir, args.review_dir, args.motion_dir, args.out_dir)


if __name__ == "__main__":
    main()
