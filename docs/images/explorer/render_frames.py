#!/usr/bin/env python3
"""Rasterize exact Ratatui JSON cell buffers; all values are synthetic fixtures.

Usage: python render_frames.py --label BEFORE --out-dir images frame.json [...]
No screenshot content is fabricated: glyphs/styles come from the captured cells.
Reset colors use an explicit neutral dark terminal palette for reproducibility.
"""
import argparse
import json
import re
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ANSI = [
    "#171b22", "#e27878", "#9cd9a2", "#e7ce8a", "#89b4e5", "#c7a0dc", "#8ad8df", "#d7dde5",
    "#788593", "#ff9b9b", "#b6ecc0", "#f4df9e", "#abd0fb", "#dec0ef", "#b4f0f5", "#f4f6f9",
]
NAMES = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "Gray", "DarkGray", "LightRed", "LightGreen", "LightYellow", "LightBlue", "LightMagenta", "LightCyan", "White"]
DEFAULT_BG, DEFAULT_FG = "#101820", "#e8eef4"
FONT_DIR = Path("/usr/share/fonts/truetype/dejavu")


def color(value, default):
    if value == "Reset":
        return default
    if value in NAMES:
        return ANSI[NAMES.index(value)]
    rgb = re.fullmatch(r"Rgb\((\d+),\s*(\d+),\s*(\d+)\)", value)
    if rgb:
        return tuple(map(int, rgb.groups()))
    indexed = re.fullmatch(r"Indexed\((\d+)\)", value)
    if indexed:
        number = int(indexed[1])
        if number < 16:
            return ANSI[number]
        if number < 232:
            value = number - 16
            levels = [0, 95, 135, 175, 215, 255]
            return levels[value // 36], levels[(value // 6) % 6], levels[value % 6]
        return (8 + 10 * (number - 232),) * 3
    raise ValueError(f"Unsupported color {value!r}")


def block(draw, symbol, x, y, width, height, foreground, background):
    """Block symbols occupy cell edges, independent of font glyph metrics."""
    code = ord(symbol) if len(symbol) == 1 else 0
    rect = lambda x0, y0, x1, y1: draw.rectangle((x + x0, y + y0, x + x1 - 1, y + y1 - 1), fill=foreground)
    if 0x2800 <= code <= 0x28FF:
        # Ratatui charts encode two columns by four rows of dots per cell.
        # Render these directly: not every installed monospace has Braille glyphs.
        bits = code - 0x2800
        positions = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)]
        radius = max(1, min(width / 7, height / 14))
        for bit, (column, row) in enumerate(positions):
            if bits & (1 << bit):
                cx = x + width * (column + 0.5) / 2
                cy = y + height * (row + 0.5) / 4
                draw.ellipse((cx - radius, cy - radius, cx + radius, cy + radius), fill=foreground)
    elif 0x2581 <= code <= 0x2588:
        filled = round(height * (code - 0x2580) / 8)
        rect(0, height - filled, width, height)
    elif symbol == "▀":
        rect(0, 0, width, height // 2)
    elif symbol == "▔":
        rect(0, 0, width, max(1, round(height / 8)))
    elif 0x2589 <= code <= 0x258F:
        rect(0, 0, round(width * (0x2590 - code) / 8), height)
    elif symbol == "▐":
        rect(width // 2, 0, width, height)
    elif symbol == "▕":
        rect(width - max(1, round(width / 8)), 0, width, height)
    elif symbol in "░▒▓":
        density = {"░": 1, "▒": 2, "▓": 3}[symbol]
        for dy in range(height):
            for dx in range(width):
                if (dx + dy * 2) % 4 < density:
                    draw.point((x + dx, y + dy), fill=foreground)
    elif 0x2596 <= code <= 0x259F:
        quadrants = {
            "▖": (2,), "▗": (3,), "▘": (0,), "▙": (0, 2, 3), "▚": (0, 3),
            "▛": (0, 1, 2), "▜": (0, 1, 3), "▝": (1,), "▞": (1, 2), "▟": (1, 2, 3),
        }
        for q in quadrants[symbol]:
            x0, x1 = (0, width // 2) if q % 2 == 0 else (width // 2, width)
            y0, y1 = (0, height // 2) if q < 2 else (height // 2, height)
            rect(x0, y0, x1, y1)
    else:
        return False
    return True


def render(source, destination, label, font_size=18):
    frame = json.loads(source.read_text())
    columns, rows = frame["width"], frame["height"]
    if len(frame["cells"]) != columns * rows:
        raise ValueError(f"Invalid dimensions in {source}")
    regular = ImageFont.truetype(str(FONT_DIR / "DejaVuSansMono.ttf"), font_size)
    bold = ImageFont.truetype(str(FONT_DIR / "DejaVuSansMono-Bold.ttf"), font_size)
    small = ImageFont.truetype(str(FONT_DIR / "DejaVuSansMono.ttf"), 13)
    width = round(regular.getlength("M"))
    height = font_size + 6
    margin, header, footer = 24, 72, 42
    image = Image.new("RGB", (columns * width + margin * 2, rows * height + header + footer), DEFAULT_BG)
    draw = ImageDraw.Draw(image)
    draw.text((margin, 15), f"{label.upper()}  |  {source.stem.replace('-', ' ').upper()}", font=bold, fill=DEFAULT_FG)
    draw.text((margin, 44), "Actual application render / synthetic fixture data / no WAN measurement", font=small, fill="#9fafbe")
    # Paint all backgrounds first; terminal wide glyphs can span adjacent cells.
    for index, cell in enumerate(frame["cells"]):
        x, y = margin + index % columns * width, header + index // columns * height
        fg, bg = color(cell["fg"], DEFAULT_FG), color(cell["bg"], DEFAULT_BG)
        if cell.get("reversed"):
            fg, bg = bg, fg
        draw.rectangle((x, y, x + width - 1, y + height - 1), fill=bg)
    for index, cell in enumerate(frame["cells"]):
        symbol = cell["symbol"]
        if not symbol or symbol == " ":
            continue
        x, y = margin + index % columns * width, header + index // columns * height
        fg, bg = color(cell["fg"], DEFAULT_FG), color(cell["bg"], DEFAULT_BG)
        if cell.get("reversed"):
            fg, bg = bg, fg
        if not block(draw, symbol, x, y, width, height, fg, bg):
            draw.text((x, y + 1), symbol, font=bold if cell.get("bold") else regular, fill=fg)
    draw.text((margin, header + rows * height + 13), f"{columns} columns x {rows} rows | exact Ratatui cells | DejaVu Sans Mono", font=small, fill="#9fafbe")
    destination.parent.mkdir(parents=True, exist_ok=True)
    image.save(destination)
    print(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frames", nargs="+", type=Path)
    parser.add_argument("--label", default="Fixture")
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument("--font-size", type=int, default=18)
    args = parser.parse_args()
    for frame in args.frames:
        destination = (args.out_dir / frame.with_suffix(".png").name) if args.out_dir else frame.with_suffix(".png")
        render(frame, destination, args.label, args.font_size)


if __name__ == "__main__":
    main()
