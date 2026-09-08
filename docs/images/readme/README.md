# README screenshot fixtures

These screenshots are deterministic captures of the current `speedtest-cli`
Ratatui interface. They use the same render path as the interactive application,
but the values are illustrative fixtures: no public speed test, DNS query, system
configuration change, or personal history was used to create them.

The gallery is captured at 120 × 38 cells with the terminal-adaptive palette
and English interface. Home, Live, Results, and History come from
`capture_readability_frames_when_explicitly_requested`; Compare comes from
`capture_review_frames_when_explicitly_requested`.

Regenerate the source buffers from the repository root:

```bash
READABILITY_SNAPSHOT_DIR=/tmp/speedtest-readme-frames \
COCKPIT_SNAPSHOT_DIR=/tmp/speedtest-readme-frames \
cargo test --locked --lib capture_ -- --nocapture
```

The JSON buffers record every cell's symbol, foreground, background, and style.
The committed PNG files are raster previews of those buffers. Regeneration is
opt-in so ordinary test runs do not modify documentation assets.

`live-demo.gif` and `cockpit-tour.gif` animate those same fixtures with a
deterministic HyperFrames/GSAP timeline. The 14-second silent master passed the
HyperFrames browser audit for runtime errors, layout, motion, and contrast. It
was split into a 6-second live-measurement loop and an 8-second cockpit-tour
loop, encoded at 12 frames per second with 128-color palettes. Each GIF's final
frame is identical to its first frame for a clean infinite loop.

| File | Screen |
| --- | --- |
| `home.png` | Offline dashboard and latest saved result |
| `live.png` | Download phase and live measurement readings |
| `results.png` | Completed result, quality summary, and findings |
| `history.png` | Saved-result table and selected-result preview |
| `compare.png` | Pinned before/after comparison |
| `live-demo.gif` | Home → live measurement → Home walkthrough |
| `cockpit-tour.gif` | Home → History → Compare → Results → Home walkthrough |
