# speedtest-cli

A fast, polished terminal network quality analyzer written in Rust.

It measures throughput and latency, then explains how the connection behaves under load: tail latency, jitter, bufferbloat, workload grades, and concrete diagnostic findings. The measurement engine, analysis model, terminal UI, and persistence layer are intentionally separated so each can evolve independently.

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
│ QUALITY 88/100 A  high confidence                                 │
│ Gaming A   Calls B   Streaming A+   Cloud gaming A                │
│ tails  idle p95 10.1 ms  p99 11.0 ms  •  jitter p95 2.0 ms       │
│ bufferbloat C  ↓ +11.2 ms  ↑ +74.5 ms                             │
│ WARNING High upload bufferbloat — enable SQM/CAKE/FQ-CoDel        │
└───────────────────────────────────────────────────────────────────┘
```

## Features

- Async Rust measurement engine with concurrent transfer streams
- Idle latency and jitter
- Loaded latency during download and upload
- Download and upload throughput sampling
- **p95/p99 latency and jitter tails** from retained probe distributions
- **Transparent 0–100 network quality score** with A+–F grade and confidence level
- **Bufferbloat grading** with measured download/upload latency increase
- **Gaming, video-call, streaming, and cloud-gaming grades**
- **Human-readable diagnostic findings and recommendations**
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

The v0.2 analysis is deliberately **explainable rather than authoritative**. It uses documented-in-code heuristic bands over the measurements collected by this client. The score is not an industry standard and is not presented as one.

A completed result contains:

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
speedtest
speedtest --fps 144
speedtest --plain
speedtest --json
speedtest --streams 4 --duration 10
speedtest --output result.json
speedtest --output result.csv --format csv
speedtest --no-save
```

For development from a checkout, replace `speedtest` with `cargo run --release --`.

### CLI options

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

The existing summary fields remain stable and v0.2 adds an optional `analysis` object, so older saved results can still be deserialized.

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
└── results/
    ├── 20260819T170000Z.json
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
├── model/
│   └── mod.rs
├── storage/
│   └── mod.rs
├── tui/
│   ├── app.rs
│   ├── mod.rs
│   ├── speedometer.rs
│   ├── speedometer/
│   │   └── gauge.rs
│   └── view.rs
├── lib.rs
└── main.rs
```

The UI never measures the network directly. It consumes `EngineEvent`s and renders them. The analysis layer consumes completed measurements and sample distributions. The storage layer only consumes `TestResult`, which keeps scripting and future alternate frontends straightforward.

## Releases

The release workflow reads the package version from `Cargo.toml`. When a commit reaches `determination` and `v<version>` has not been released yet, GitHub Actions builds all supported targets, generates SHA-256 checksums, creates the version tag, and publishes the assets to a GitHub Release. Pull requests that touch release-sensitive files build the same packages without publishing them.

## Accuracy notes

This is an independent CLI, not an official Cloudflare client. Network measurements vary with routing, congestion, Wi-Fi conditions, endpoint behavior, protocol overhead, and test methodology. Percentiles become more informative with more samples; v0.2 uses 24 idle probes and retains loaded-latency samples throughout both transfer phases. Quality and workload grades are local heuristics built from those measurements, not standardized certifications.

## Roadmap

- Historical `speedtest history` and `speedtest stats` commands
- Baseline and anomaly detection across saved results
- Long-running stability tests with spike/loss distributions
- Adaptive stream count and payload sizing
- Pluggable measurement backends / server discovery
- Ethernet/Wi-Fi, VPN, and IPv4/IPv6 comparison workflows
- Better packet-loss implementation without deprecated infrastructure
- Configurable but restrained themes
- Homebrew and WinGet packages

## License

MIT
