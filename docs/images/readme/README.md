# README screenshot fixtures

These screenshots are deterministic captures of the current `speedtest-cli`
Ratatui interface. They use the same render path as the interactive application,
but the values are illustrative fixtures: no public speed test, DNS query, system
configuration change, or personal history was used to create them.

The gallery is captured at 120 × 38 cells with the explicitly selected Graphite
palette and English interface. The app still inherits the terminal palette by
default. Home, Live, Results, and History come from
`capture_readability_frames_when_explicitly_requested`; Compare comes from
`capture_review_frames_when_explicitly_requested`.

Regenerate the source buffers from the repository root:

```bash
READABILITY_SNAPSHOT_DIR=/tmp/speedtest-readme-frames \
COCKPIT_SNAPSHOT_DIR=/tmp/speedtest-readme-review \
COCKPIT_MOTION_SNAPSHOT_DIR=/tmp/speedtest-readme-motion \
cargo test --locked --lib capture_ -- --nocapture
```

The JSON buffers record every cell's symbol, foreground, background, and style.
The committed PNG files are raster previews of those buffers. Regeneration is
opt-in so ordinary test runs do not modify documentation assets.

Render the gallery with Pillow 12.3.0 and DejaVu Sans Mono installed:

```bash
python docs/images/readme/render_gallery.py \
  --readability-dir /tmp/speedtest-readme-frames \
  --review-dir /tmp/speedtest-readme-review \
  --motion-dir /tmp/speedtest-readme-motion
```

`live-demo.gif` uses the actual live renderer's five-second sequence at 50 ms
intervals, with one-second Home holds before and after. Synthetic engine events
drive download, upload, and completion; no intermediate UI states are invented.
`cockpit-tour.gif` is an 8.05-second slideshow of actual Home, History, Compare,
and Results buffers, with two-second holds and a final Home frame. Compare uses
the pinned-baseline review fixture. Both loops end on their starting frame and
use shared 256-color palettes; identical adjacent frames may be coalesced.

Current assets were captured from [`e1e175e`](https://github.com/cmdr-chara/speedtest-cli/commit/e1e175e24a47f447779f8ea39c545ebcea9aa64a),
including the requested removal of the top-right measuring badge.
The [capture workflow](../../../.github/workflows/cockpit-visuals.yml) reproduces
the README and design galleries without changing the historical before images.

| File | Screen |
| --- | --- |
| `home.png` | Offline dashboard and latest saved result |
| `live.png` | Download phase and live measurement readings |
| `results.png` | Completed result, quality summary, and findings |
| `history.png` | Saved-result table and selected-result preview |
| `compare.png` | Pinned before/after comparison |
| `live-demo.gif` | Home → live measurement → Home walkthrough |
| `cockpit-tour.gif` | Home → History → Compare → Results → Home walkthrough |
