# speedtest-cli

A fast, polished terminal network quality analyzer written in Rust.

It measures throughput and latency, then explains how the connection behaves under load: tail latency, jitter, bufferbloat, workload grades, stability, historical trends, and concrete diagnostic findings. The measurement engine, analysis model, terminal UI, and persistence layer are intentionally separated so each can evolve independently.

## Preview

```text
┌──────────────────────────── SPEEDTEST ────────────────────────────┐
│                     NETWORK ANALYSIS COMPLETE                     │
│                                                                   │
│                         ⣿⣿⣿⣿⣿⣿⣷⣄                        │
│                    ⣠⣿⠟          ⠻⣿⣄                     │
│                  ⣰⣿⠃       ╱       ⠹⣿⡄                   │
│                 ⣿⡏        ╱          ⢹⣿                  │
│                            ●                                      │
│                         842.6 Mbps                                │
│                                                                   │
│   DOWNLOAD  842.6 Mbps             UPLOAD  193.4 Mbps             │
│   PING        8.2 ms               JITTER    1.1 ms               │
│   LOADED ↓   19.4 ms               LOADED ↑ 82.7 ms               │
│                                                                   │
│ QUALITY 99/100 A+  ◆ S-TIER  high confidence                      │
│ Gaming A+  Calls A   Streaming A+  Cloud gaming A                 │
│ tails  idle p95 10.1 ms  p99 11.0 ms  •  jitter p95 2.0 ms       │
│ bufferbloat A  ↓ +11.2 ms  ↑ +6.5 ms                              │
└───────────────────────────────────────────────────────────────────┘
```

## Features

- Async Rust measurement engine with concurrent transfer streams
- Idle latency and jitter
- Loaded latency during download and upload
- Download and upload throughput sampling
- **p95/p99 latency and jitter tails** from retained probe distributions
- **Transparent 0–100 network quality score** with A+–F grade and confidence level
- Rare **◆ S-TIER** distinction for exceptional high-confidence runs
- **Bufferbloat grading** with measured download/upload latency increase
- **Gaming, video-call, streaming, and cloud-gaming grades**
- **Human-readable diagnostic findings and recommendations**
- **Long-running stability mode** with rolling latency trace, tail latency, probe availability, and failure bursts
- **Historical run browser** with quality/S-tier context and terminal sparkline
- **Historical statistics and anomaly detection** against the saved baseline
- Layered animated Ratatui speedometer and live sparkline
- 240 Hz speedometer physics with a configurable 30–240 FPS render cap
- JSON output for scripts
- CSV export including quality/percentile fields
- Automatic per-test JSON files and JSONL history
- Plain terminal mode for CI, logs, and unsupported terminals
- Native release binaries for Windows, Linux, Intel macOS, and Apple Silicon macOS
- Cloudflare-aware request sizing and HTTP 429 backoff

Packet loss is intentionally nullable rather than depending on a deprecated public TURN service. When it is unavailable, real-time workload grades say so instead of pretending the measurement exists.

## Network intelligence

The quality analysis is deliberately **explainable rather than authoritative**. It uses documented-in-code heuristic bands over the measurements collected by this client. The score is not an industry standard and is not presented as one.

A completed result can include:

```text
Quality            88/100 A
Confidence         high
Bufferbloat        C
Gaming             A
Video calls        B
Streaming          A+
Cloud gaming       A
Idle p95/p99       10.1 / 11.0 ms
Jitter p95         2.0 ms
Diagnosis          High upload bufferbloat
Evidence           idle 8.2 ms → loaded upload 82.7 ms (+74.5 ms)
Recommendation     enable SQM/CAKE/FQ-CoDel or shape upstream traffic
```

Confidence is based on measurement coverage. If loaded-latency probes are missing or sparse, the client lowers confidence and leaves the bufferbloat grade unavailable rather than fabricating one.

## Stability mode

`speedtest stability` continuously sends conservative zero-byte HTTP probes instead of repeatedly saturating the link. It is designed to expose latency spikes and short disruptions that a normal short speed test can miss.

```bash
speedtest stability
speedtest stability --duration 5m
speedtest stability --duration 5m --interval 750ms
speedtest stability --plain
speedtest stability --json
speedtest stability --output stability.json
```

The default run lasts one minute with one probe per second. Supported durations range from 10 seconds to 24 hours, and probe intervals range from 500 ms to 10 seconds.

Example plain result:

```text
Network Stability
  Duration:       300s
  Probe interval: 1000 ms
  Probes:         300 successful / 0 failed
  Availability:   100.00% (HTTP probe availability, not packet loss)
  Failure bursts: 0
  Median:         8.3 ms
  p95 / p99:      10.8 / 14.2 ms
  Max:            18.1 ms
  Jitter p95:     2.2 ms
  Stability:      99/100 A+
  Tier:           ◆ S-TIER
```

**Probe availability is not packet loss.** A failed HTTP probe may result from endpoint throttling, a transient route/server issue, or the local network. Consecutive failures are grouped into failure bursts so one disruption is not presented as several unrelated incidents.

## History and stats

Completed normal speed tests are already saved locally. v0.3 makes that history queryable without external scripts.

```bash
speedtest history
speedtest history --days 7
speedtest history --days 30 --limit 50
speedtest history --json

speedtest stats
speedtest stats --days 90
speedtest stats --json
```

`history` displays recent runs newest-first, including download/upload, idle latency, quality grade, and S-tier status when available.

`stats` calculates:

- median and best download/upload
- median and p95 idle latency
- median quality score
- S-tier run count
- an overall download trend
- a Unicode throughput history sparkline
- latest-run anomaly detection against the previous saved baseline

Anomaly detection requires at least five prior runs before evaluating the latest result. It currently flags material download/upload regressions, significantly elevated idle latency, and large quality-score drops.

## Installation

### Prebuilt binaries

GitHub Releases are the recommended installation path; Rust is not required.

#### Windows x86_64

```powershell
Invoke-WebRequest -Uri "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-windows-x86_64.zip" -OutFile speedtest.zip
Expand-Archive .\speedtest.zip -DestinationPath . -Force
.\speedtest-windows-x86_64\speedtest.exe
```

Move `speedtest.exe` into a directory on your `PATH` if you want to run `speedtest` from anywhere.

#### Linux x86_64

The Linux release uses musl for broad distribution compatibility.

```bash
curl -L "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-linux-x86_64.tar.gz" -o speedtest.tar.gz
tar -xzf speedtest.tar.gz
sudo install -m 0755 speedtest-linux-x86_64/speedtest /usr/local/bin/speedtest
speedtest
```

#### macOS

The command below automatically selects Apple Silicon or Intel:

```bash
case "$(uname -m)" in
  arm64) ASSET="aarch64" ;;
  x86_64) ASSET="x86_64" ;;
  *) echo "Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

curl -L "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-macos-${ASSET}.tar.gz" -o speedtest.tar.gz
tar -xzf speedtest.tar.gz
sudo install -m 0755 "speedtest-macos-${ASSET}/speedtest" /usr/local/bin/speedtest
speedtest
```

Each release also includes a `.sha256` file for every archive.

### Install from source

With Rust installed:

```bash
cargo install --git https://github.com/cmdr-chara/speedtest-cli --branch determination
speedtest
```

## Usage

```bash
# Normal speed / network-quality analysis
speedtest
speedtest --fps 144
speedtest --plain
speedtest --json
speedtest --streams 4 --duration 10
speedtest --output result.json
speedtest --output result.csv --format csv
speedtest --no-save

# Stability
speedtest stability --duration 5m --interval 1s

# Historical intelligence
speedtest history --days 30
speedtest stats --days 30
```

For development from a checkout, replace `speedtest` with `cargo run --release --`.

### Normal-test options

```text
--streams <N>       Concurrent transfer streams (default: 2)
--duration <SEC>    Seconds for each throughput phase (default: 8)
--fps <N>           Interactive render cap, 30–240 FPS (default: 240)
--plain             Disable the interactive TUI
--json              Print the canonical result as JSON
--output <PATH>     Also write the result to this path
--format <FORMAT>   Output file format: json or csv (default: json)
--no-save           Disable automatic result/history persistence
```

The speedometer spring simulation always advances at 240 Hz. `--fps` only caps terminal redraws, so lowering it reduces terminal/CPU work without changing network measurements or animation physics. Redraws are suppressed once the gauge is settled and no UI data has changed.

The Cloudflare backend deliberately defaults to two streams and long transfer bodies to avoid turning a short speed test into a burst of many HTTP requests. If Cloudflare's public endpoint returns HTTP 429, the client respects numeric `Retry-After` values when available and otherwise uses bounded exponential backoff. Persistent throttling can still happen on the public service; in that case, wait before retrying or use `--streams 1`.

## Result model

The existing summary fields remain stable and newer releases add an optional `analysis` object, so older saved results can still be deserialized.

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
    "upload_loaded_ms": 82.7,
    "packet_loss_percent": null
  },
  "download": {
    "mbps": 842.6,
    "bytes": 842600000,
    "seconds": 8.0
  },
  "upload": {
    "mbps": 193.4,
    "bytes": 193400000,
    "seconds": 8.0
  },
  "analysis": {
    "latency": {
      "idle": {
        "samples": 24,
        "min_ms": 7.9,
        "median_ms": 8.2,
        "p95_ms": 10.1,
        "p99_ms": 11.0,
        "max_ms": 11.2
      }
    },
    "quality": {
      "score": 88,
      "grade": "a",
      "confidence": "high",
      "bufferbloat": {
        "download_increase_ms": 11.2,
        "upload_increase_ms": 74.5,
        "worst_increase_ms": 74.5,
        "grade": "d"
      }
    }
  }
}
```

## Data storage

Completed tests are saved to the platform data directory unless `--no-save` is used.

```text
speedtest/
├── history.jsonl
├── results/
│   ├── 20260819T170000Z.json
│   └── ...
└── stability/
    ├── history.jsonl
    └── results/
        ├── 20260820T105000Z.json
        └── ...
```

## Architecture

```text
src/
├── analysis/
│   └── mod.rs
├── cli.rs
├── engine/
│   ├── mod.rs
│   └── cloudflare.rs
├── history.rs
├── model/
│   └── mod.rs
├── stability.rs
├── storage/
│   └── mod.rs
├── tui/
│   ├── app.rs
│   ├── mod.rs
│   ├── speedometer.rs
│   ├── speedometer/
│   │   └── gauge.rs
│   ├── stability.rs
│   └── view.rs
├── lib.rs
└── main.rs
```

The normal TUI never measures the network directly; it consumes `EngineEvent`s. Stability follows the same event-driven approach with `StabilityEvent`s. History analysis consumes persisted canonical results. This keeps measurement, rendering, analysis, and storage independently testable.

## Releases

The release workflow reads the package version from `Cargo.toml`. When a commit reaches `determination` and `v<version>` has not been released yet, GitHub Actions builds all supported targets, generates SHA-256 checksums, creates the version tag, and publishes the assets to a GitHub Release. Pull requests that touch release-sensitive files build the same packages without publishing them.

## Accuracy notes

This is an independent CLI, not an official Cloudflare client. Network measurements vary with routing, congestion, Wi-Fi conditions, endpoint behavior, protocol overhead, and test methodology. Percentiles become more informative with more samples. Quality, workload, stability, trend, and anomaly grades are local heuristics built from the measurements collected by this client, not standardized certifications.

Stability mode deliberately avoids calling failed HTTP probes packet loss. A proper packet-loss implementation remains a separate future measurement problem.

## Roadmap

- Adaptive stream count and payload sizing
- Pluggable measurement backends / server discovery
- Ethernet/Wi-Fi, VPN, and IPv4/IPv6 comparison workflows
- Dedicated packet-loss implementation without deprecated infrastructure
- Local/self-hosted LAN measurement mode
- Configurable but restrained themes
- Homebrew and WinGet packages

## License

MIT
