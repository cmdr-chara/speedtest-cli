# speedtest-cli

A fast, polished terminal network quality analyzer written in Rust.

`speedtest-cli` measures throughput and latency, then explains how the connection behaves under load. It includes network-quality scoring, bufferbloat analysis, stability monitoring, historical intelligence, DNS diagnostics/configuration, multiple Internet backends, real ICMP loss testing, Wi-Fi inspection, and a self-hosted LAN mode.

## Highlights

- Download/upload throughput with concurrent streams
- Idle and loaded latency, jitter, p95/p99 tails
- Explainable 0–100 quality score with A+–F grades
- Rare `◆ S-TIER` distinction for exceptional high-confidence runs
- Bufferbloat and workload analysis for gaming, calls, streaming, and cloud gaming
- Long-running stability mode
- History, trends, sparklines, and anomaly detection
- DNS inspection, health testing, benchmarking, configuration, rollback, and optimization
- 20 built-in DNS resolver profiles across multiple providers
- DNS-over-UDP and real DNS-over-HTTPS benchmarking
- Cloudflare and LibreSpeed Internet backends
- Backend cross-checking with `speedtest verify`
- Real ICMP echo response-loss measurement with `speedtest loss`
- Native Wi-Fi diagnostics on Windows, macOS, and Linux
- Built-in self-hosted LAN speed-test server/client
- JSON output, CSV export, per-run JSON, and JSONL history
- Native release binaries for Windows, Linux, Intel macOS, and Apple Silicon macOS

## v0.5 Network Lab

### Adaptive Cloudflare backend

The Cloudflare backend no longer depends on one fixed request such as:

```text
https://speed.cloudflare.com/__down?bytes=250000000
```

Large requests to the public endpoint can be rejected depending on endpoint policy, edge behavior, network, or request size. v0.5 uses a time-based adaptive download strategy instead: it starts with a moderate payload, scales up when responses complete quickly, and automatically downshifts when Cloudflare returns size/rejection statuses such as HTTP 403, 413, or 400. HTTP 429 still uses bounded `Retry-After`/backoff handling.

The objective is to measure sustained throughput without making a successful test depend on one 250 MB response.

### Multiple Internet backends

Cloudflare remains the default:

```bash
speedtest
speedtest --backend cloudflare
```

LibreSpeed is also available:

```bash
speedtest --backend librespeed
```

A compatible custom LibreSpeed installation can be selected with:

```bash
speedtest --backend librespeed --librespeed-server https://speed.example.com
```

The custom URL is treated as the LibreSpeed base URL and standard `garbage.php` / `empty.php` endpoints are assumed. It must use HTTP or HTTPS, include a host, and must not contain credentials, a query, or a fragment. Nested base paths are supported.

### Backend verification

Use both Internet engines to check whether a result is strongly backend/path dependent:

```bash
speedtest verify
speedtest verify --duration 8 --streams 2
speedtest verify --json
```

`verify` compares Cloudflare and LibreSpeed results rather than assuming a single public endpoint is ground truth.

### Real ICMP response loss

```bash
speedtest loss
speedtest loss --target 1.1.1.1 --count 50
speedtest loss --json
```

This is a real ICMP echo response-loss measurement. It is deliberately separate from HTTP probe availability: HTTP failures, DNS failures, and endpoint throttling are **not** labeled packet loss.

ICMP still has an important limitation: some hosts, routers, and firewalls block or deprioritize echo traffic, so ICMP loss can look worse than application traffic.

### Wi-Fi diagnostics

```bash
speedtest wifi
speedtest wifi --json
speedtest wifi --interface Wi-Fi
```

Depending on the OS and driver/tooling, the report can include:

- active interface
- SSID
- signal strength / estimated dBm
- band
- channel
- PHY/link rate
- radio metadata

PHY/link rate is not presented as Internet throughput.

### Self-hosted LAN mode

Run a server on another machine in the LAN:

```bash
speedtest serve
```

Default bind address:

```text
0.0.0.0:9876
```

Then from another machine:

```bash
speedtest lan 192.168.1.50:9876
speedtest lan 192.168.1.50:9876 --duration 10 --streams 4
speedtest lan 192.168.1.50:9876 --json
speedtest lan 192.168.1.50:9876 --output lan-result.csv --format csv
speedtest lan 192.168.1.50:9876 --no-save
```

This gives you a local throughput/latency baseline. If LAN performance is poor, the problem is likely local before the ISP/WAN path is even involved.

## DNS suite

Inspect the current resolver configuration:

```bash
speedtest dns show
speedtest dns list
```

Test the active resolver or explicit DNS server IPs:

```bash
speedtest dns test
speedtest dns test --resolver 1.1.1.1 --resolver 8.8.8.8
```

Benchmark comparable resolver leagues over classic UDP/53:

```bash
speedtest dns benchmark
speedtest dns benchmark --profile privacy
speedtest dns benchmark --profile security
speedtest dns benchmark --profile adblock
speedtest dns benchmark --profile family
```

Benchmark providers using real DNS-over-HTTPS wire-format requests:

```bash
speedtest dns benchmark --protocol doh
speedtest dns benchmark --profile privacy --protocol doh
```

DoH benchmarking performs connection warm-up separately so the measured query distribution is not simply the first TCP/TLS handshake time.

Configure a known resolver profile:

```bash
speedtest dns set cloudflare --dry-run
speedtest dns set cloudflare
speedtest dns set quad9
```

Automatically benchmark a league and select the best eligible resolver:

```bash
speedtest dns optimize --dry-run
speedtest dns optimize
speedtest dns optimize --profile privacy
speedtest dns optimize --profile security
```

Resolver winners and DNS changes require at least an 80% query-success rate. `set` and `optimize` also preflight the selected addresses immediately before any configuration change.

Recovery:

```bash
speedtest dns rollback
speedtest dns reset
```

DNS writes snapshot the existing configuration before applying changes, verify resolution afterward, and attempt automatic rollback if post-change validation fails. Changes are limited to the active network interface so verification follows the configuration being changed. On Linux, persistent automatic configuration currently requires NetworkManager; unmanaged resolver setups remain read-only.

## Network Doctor and comparison

```bash
speedtest doctor
speedtest doctor --full
speedtest doctor --json
```

The lightweight doctor checks route/interface state, gateway latency where available, IPv4/IPv6 reachability, DNS health, HTTPS, and Wi-Fi context where available. `--full` also runs throughput/bufferbloat analysis.

Compare the two latest saved runs:

```bash
speedtest compare
```

Or compare explicit canonical JSON results:

```bash
speedtest compare before.json after.json
speedtest compare before.json after.json --json
```

## Stability

`speedtest stability` continuously sends conservative HTTP probes instead of repeatedly saturating the connection:

```bash
speedtest stability
speedtest stability --duration 5m
speedtest stability --duration 5m --interval 750ms
speedtest stability --plain
speedtest stability --json
```

**HTTP probe availability is not packet loss.** A failed stability probe can be caused by endpoint throttling, route/server behavior, or the local connection. Use `speedtest loss` when you specifically want ICMP echo response-loss measurement.
Probes skipped because an earlier request overran its schedule are included in the availability denominator and reported separately.

## History and statistics

```bash
speedtest history
speedtest history --days 30 --limit 50
speedtest history --json

speedtest stats
speedtest stats --days 90
speedtest stats --json
```

History/statistics include median/best throughput, latency statistics, quality context, S-tier counts, trend detection, a Unicode throughput sparkline, and latest-run anomaly detection against prior saved results. Statistics use Internet runs when any exist, otherwise LAN runs; implicit comparisons always pair runs from the same scope.

## Installation

### Prebuilt binaries

GitHub Releases are the recommended installation method; Rust is not required.

#### Windows x86_64

```powershell
Invoke-WebRequest -Uri "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-windows-x86_64.zip" -OutFile speedtest.zip
Expand-Archive .\speedtest.zip -DestinationPath . -Force
.\speedtest-windows-x86_64\speedtest.exe
```

#### Linux x86_64

```bash
curl -L "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-linux-x86_64.tar.gz" -o speedtest.tar.gz
tar -xzf speedtest.tar.gz
sudo install -m 0755 speedtest-linux-x86_64/speedtest /usr/local/bin/speedtest
speedtest
```

The Linux release uses musl for broad distribution compatibility.

Linux route/DNS diagnostics require `ip`; real ICMP loss measurement requires
`ping` (normally `iputils-ping`), and Wi-Fi inspection requires `iw`.
Persistent DNS changes additionally require a NetworkManager-managed connection
and `nmcli`. Commands that do not need these helpers continue to work without
them and report a clear unavailable status where appropriate.

#### macOS

```bash
case "$(uname -m)" in
  arm64) ASSET="aarch64" ;;
  x86_64) ASSET="x86_64" ;;
  *) echo "Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

curl -L "https://github.com/cmdr-chara/speedtest-cli/releases/latest/download/speedtest-macos-${ASSET}.tar.gz" -o speedtest.tar.gz
tar -xzf speedtest.tar.gz
sudo install -m 0755 "speedtest-macos-${ASSET}/speedtest" /usr/local/bin/speedtest
```

Each packaged release includes SHA-256 checksum files.

### Install from source

```bash
cargo install --locked --git https://github.com/cmdr-chara/speedtest-cli --branch main --force
speedtest --version
```

## Normal speed-test usage

```bash
speedtest
speedtest --backend cloudflare
speedtest --backend librespeed
speedtest --fps 144
speedtest --plain
speedtest --json
speedtest --streams 4 --duration 10
speedtest --output result.json
speedtest --output result.csv --format csv
speedtest --no-save
```

Main options:

```text
--backend <BACKEND>          cloudflare or librespeed
--librespeed-server <URL>    custom LibreSpeed base URL
--streams <N>                concurrent transfer streams (default: 2)
--duration <SEC>             seconds for each throughput phase (default: 8)
--fps <N>                    interactive render cap, 30–240 FPS
--plain                      disable interactive TUI
--json                       print canonical JSON
--output <PATH>              also write completed result
--format <FORMAT>            json or csv
--no-save                    disable automatic history/result persistence
```

The speedometer physics run independently of the render cap, so lowering `--fps` reduces terminal work without changing network measurements.

## Result semantics

Normal Internet and LAN tests use the existing canonical result structure:

```json
{
  "timestamp": "2026-08-20T14:00:00Z",
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
  }
}
```

Standalone `speedtest loss` does not silently inject ICMP loss into an unrelated saved throughput run. That separation keeps protocol semantics explicit.

## Data storage

Completed Internet and LAN tests are stored in the platform data directory unless `--no-save` is used. Concurrent runs use locked history appends and collision-safe per-run filenames.

```text
speedtest/
├── history.jsonl
├── results/
├── dns/
│   └── last-backup.json
└── stability/
    ├── history.jsonl
    └── results/
```

## Architecture

```text
src/
├── analysis/
├── bin/
│   └── speedtest.rs
├── dns/
│   ├── doh.rs
│   ├── mod.rs
│   └── system.rs
├── engine/
│   ├── cloudflare_adaptive.rs
│   ├── internet.rs
│   ├── librespeed.rs
│   └── mod.rs
├── compare.rs
├── doctor.rs
├── history.rs
├── lan.rs
├── loss.rs
├── model/
├── stability.rs
├── storage/
├── tui/
├── verify.rs
├── wifi.rs
└── lib.rs
```

Measurement, analysis, UI, persistence, DNS, local-network diagnostics, and backend implementations are kept separate so they can evolve independently.

## Accuracy notes

This is an independent CLI, not an official Cloudflare or LibreSpeed client. Results vary with routing, congestion, Wi-Fi conditions, endpoint behavior, protocol overhead, server capacity, and test methodology.

The quality score, workload grades, stability grades, history trend, anomaly flags, and DNS scores are local heuristics rather than standardized certifications.

A public speed-test server is part of the path being measured. Use `speedtest verify` when you need cross-backend evidence and `speedtest lan` when you need to isolate the local network from the WAN.

ICMP echo loss measures ICMP echo response behavior; it does not prove every transport/application experiences identical loss.

## Roadmap

- deeper IPv4 vs IPv6 A/B workflow
- VPN on/off comparison workflow
- MTU and fragmentation diagnostics
- TCP/TLS handshake decomposition
- HTTP/2 vs HTTP/3 / QUIC diagnostics
- DoT and DoQ active benchmarks
- additional Internet measurement backends and server discovery
- configurable but restrained TUI themes
- Homebrew and WinGet packages

## License

MIT
