#!/usr/bin/env python3
"""Rasterize actual Ratatui capture cells into a before/after and motion gallery.

Requires Pillow and DejaVu Sans Mono, like the explorer screenshot renderer.
No interpolation, generated graphics, or invented UI: every animation frame comes
from capture_motion_frames_when_explicitly_requested at 50 ms intervals.
"""
import argparse
import importlib.util
import json
import math
from pathlib import Path

from PIL import Image, ImageColor, ImageDraw, ImageFont

EXPLORER = Path(__file__).resolve().parent.parent / "explorer" / "render_frames.py"
SPEC = importlib.util.spec_from_file_location("explorer_render", EXPLORER)
CELLS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CELLS)


def rgb(value):
    return ImageColor.getrgb(value) if isinstance(value, str) else value


def raster(source, title, font_size=14):
    frame = json.loads(source.read_text())
    columns, rows = frame["width"], frame["height"]
    if len(frame["cells"]) != columns * rows:
        raise ValueError(f"Invalid dimensions in {source}")
    fonts = {}
    for bold in (False, True):
        for italic in (False, True):
            suffix = "-BoldOblique" if bold and italic else "-Bold" if bold else "-Oblique" if italic else ""
            fonts[bold, italic] = ImageFont.truetype(
                str(CELLS.FONT_DIR / f"DejaVuSansMono{suffix}.ttf"), font_size
            )
    label_font = ImageFont.truetype(str(CELLS.FONT_DIR / "DejaVuSansMono.ttf"), 12)
    width = math.ceil(fonts[False, False].getlength("M"))
    height = font_size + 6
    margin, header, footer = 24, 66, 38
    image = Image.new("RGB", (columns * width + margin * 2, rows * height + header + footer), CELLS.DEFAULT_BG)
    draw = ImageDraw.Draw(image)
    draw.text((margin, 13), title, font=fonts[True, False], fill=CELLS.DEFAULT_FG)
    draw.text((margin, 39), "Actual Ratatui cells / synthetic fixtures / no WAN measurement", font=label_font, fill="#9fafbe")
    styles = []
    for cell in frame["cells"]:
        foreground = rgb(CELLS.color(cell["fg"], CELLS.DEFAULT_FG))
        background = rgb(CELLS.color(cell["bg"], CELLS.DEFAULT_BG))
        if cell.get("reversed"):
            foreground, background = background, foreground
        if cell.get("dim"):
            foreground = tuple(round((a + b) / 2) for a, b in zip(foreground, background))
        styles.append((foreground, background))
    # Separate passes preserve glyphs that occupy more than one terminal cell.
    for index, (_, background) in enumerate(styles):
        x, y = margin + index % columns * width, header + index // columns * height
        draw.rectangle((x, y, x + width - 1, y + height - 1), fill=background)
    for index, cell in enumerate(frame["cells"]):
        symbol = cell["symbol"]
        if not symbol or symbol == " ":
            continue
        x, y = margin + index % columns * width, header + index // columns * height
        foreground, background = styles[index]
        if not CELLS.block(draw, symbol, x, y, width, height, foreground, background):
            draw.text((x, y + 1), symbol, font=fonts[bool(cell.get("bold")), bool(cell.get("italic"))], fill=foreground)
        if cell.get("underlined"):
            draw.line((x, y + height - 2, x + width - 1, y + height - 2), fill=foreground)
    draw.text(
        (margin, header + rows * height + 11),
        f"{columns} x {rows} cells | Graphite palette | DejaVu Sans Mono | offline render fixture",
        font=label_font,
        fill="#9fafbe",
    )
    return image


def gif(sequence, destination, title):
    sources = sorted(sequence.glob("*.json"))
    if not sources:
        raise ValueError(f"No captured frames in {sequence}")
    frames = [raster(source, title) for source in sources]
    durations = [json.loads(source.read_text())["frame_ms"] for source in sources]
    # A shared palette avoids per-frame palette flicker. Only GIF is quantized;
    # the still PNGs retain all captured RGB colors.
    sample_indices = list(range(0, len(frames), max(1, len(frames) // 12)))
    swatches = Image.new("RGB", (256 * len(sample_indices), 256))
    for column, index in enumerate(sample_indices):
        swatches.paste(frames[index].resize((256, 256), Image.Resampling.NEAREST), (column * 256, 0))
    palette = swatches.quantize(colors=256)
    frames = [frame.quantize(palette=palette, dither=Image.Dither.NONE) for frame in frames]
    frames[0].save(destination, save_all=True, append_images=frames[1:], duration=durations, loop=0, disposal=1, optimize=True)
    print(f"{destination.name}: {len(sources)} captured frames, {sum(durations) / 1000:g}s, {destination.stat().st_size:,} bytes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames-root", required=True, type=Path)
    parser.add_argument("--before-dir", required=True, type=Path)
    parser.add_argument("--out-dir", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    for name in ("live", "statistics"):
        for label, source in (
            ("before", args.before_dir / f"graphite-{name}-en-120x38.json"),
            ("after", args.frames_root / "stills" / f"after-{name}.json"),
        ):
            raster(source, f"{label.upper()} | {name.upper()} | identical fixture values", 18).save(args.out_dir / f"{label}-{name}.png")
    for name, title in (
        ("home", "HOME | arrival sweep and focus movement"),
        ("live", "LIVE TEST | synthetic download, upload, and results reveal"),
        ("statistics", "STATISTICS | chart trace, metric change, and help reveal"),
    ):
        gif(args.frames_root / name, args.out_dir / f"{name}-motion.gif", title)
    raster(args.frames_root / "live" / "032.json", "AFTER | live sample trace | synthetic engine events", 18).save(args.out_dir / "after-live-samples.png")


if __name__ == "__main__":
    main()
