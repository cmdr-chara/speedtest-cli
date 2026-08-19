# speedtest-cli

A fast, polished terminal speed test written in Rust.

The project separates the measurement engine, canonical result model, terminal UI, and persistence layer so each can evolve independently. The initial backend measures against Cloudflare's public speed-test endpoints.

## Preview

```text
┌────────────────────────── SPEEDTEST ──────────────────────────┐
│                                                               │
│                          842.6 Mbps                            │
│                                                               │
│                    ╭────────────────╮                         │
│                ╭───╯                ╰───╮                     │
│              ╭─╯           ╱            ╰─╮                   │
│             │             ╱                │                  │
│             │            ●                 │                  │
│              ╰─╮                        ╭─╯                   │
│                ╰───╮                ╭───╯                     │
│                    ╰────────────────╯                         │
│                                                               │
│   DOWNLOAD  842.6 Mbps             UPLOAD  293.4 Mbps         │
│   PING        8.2 ms               JITTER    1.1 ms           │
│   LOADED ↓   19.4 ms               LOADED ↑ 17.8 ms           │
│                                                               │
│   ▁▂▄▆▇██████▇████████▇▆▆▇████                            │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

## Features

- Async Rust measurement engine with concurrent transfer streams
- Idle latency and jitter
- Loaded latency during download and upload
- Download and upload throughput sampling
- Animated Ratatui speedometer and live sparkline
- JSON output for scripts
- Automatic per-test JSON files and JSONL history
- Plain terminal mode for CI, logs, and unsupported terminals
- Graceful separation between engine, UI, model, and storage

Packet loss is intentionally nullable in v0.1 rather than depending on a deprecated public TURN service.

## Usage

```bash
cargo run --release
cargo run --release -- --plain
cargo run --release -- --json
cargo run --release -- --streams 6 --duration 10
cargo run --release -- --output result.json
cargo run --release -- --output result.csv --format csv
cargo run --release -- --no-save
```

### CLI options

```text
--streams <N>       Concurrent transfer streams (default: 4)
--duration <SEC>    Seconds for each throughput phase (default: 8)
--plain             Disable the interactive TUI
--json              Print the canonical result as JSON
--output <PATH>     Also write the result to this path
--format <FORMAT>    Output file format: json or csv (default: json)
--no-save           Disable automatic result/history persistence
```

## Result model

```json
{
  "timestamp": "2026-08-19T17:00:00Z",
  "backend": "cloudflare",
  "server": {
    "host": "speed.cloudflare.com",
    "name": "Cloudflare Edge"
  },
  "latency": {
    "idle_ms": 8.2,
    "jitter_ms": 1.1,
    "download_loaded_ms": 19.4,
    "upload_loaded_ms": 17.8,
    "packet_loss_percent": null
  },
  "download": {
    "mbps": 842.6,
    "bytes": 842600000,
    "seconds": 8.0
  },
  "upload": {
    "mbps": 293.4,
    "bytes": 293400000,
    "seconds": 8.0
  }
}
```

## Data storage

Completed tests are saved to the platform data directory unless `--no-save` is used.

```text
speedtest/
├── history.jsonl
└── results/
    ├── 20260819T170000Z.json
    └── ...
```

## Architecture

```text
src/
├── cli.rs
├── engine/
│   ├── mod.rs
│   └── cloudflare.rs
├── model/
│   └── mod.rs
├── storage/
│   └── mod.rs
├── tui/
│   ├── mod.rs
│   └── speedometer.rs
├── lib.rs
└── main.rs
```

The UI never measures the network directly. It consumes `EngineEvent`s and renders them. The storage layer only consumes `TestResult`, which keeps scripting and future alternate frontends straightforward.

## Accuracy notes

This is an independent CLI, not an official Cloudflare client. Network speed measurements vary with routing, congestion, Wi-Fi conditions, endpoint behavior, protocol overhead, and test methodology. The engine uses warm-up traffic, multiple streams, monotonic timing, and time-window sampling to reduce obvious measurement artifacts, but broader validation against controlled links is still required before treating v0.1 as a reference benchmark.

## Roadmap

- Adaptive stream count and payload sizing
- Pluggable measurement backends / server discovery
- Historical `speedtest history` and `speedtest stats` commands
- Better packet-loss implementation without deprecated infrastructure
- Configurable but restrained themes
- Additional terminal compatibility fallbacks
- Cross-platform release binaries

## License

MIT
