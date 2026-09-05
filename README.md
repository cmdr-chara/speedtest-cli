# speedtest-cli

A fast, polished terminal network quality analyzer written in Rust.

`speedtest-cli` measures throughput and latency, then explains how the connection behaves under load. It includes network-quality scoring, bufferbloat analysis, stability monitoring, historical intelligence, DNS diagnostics/configuration, multiple Internet backends, real ICMP loss testing, Wi-Fi inspection, and a self-hosted LAN mode.

## Highlights

- Full-screen keyboard-driven network cockpit with an offline home dashboard
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
- Script-safe terminal detection, explicit progress/color policy, and stable exit statuses
- Offline threshold/freshness checks for saved JSON results
- JSON output, CSV export, per-run JSON, and JSONL history
- Native release binaries for Windows, Linux, Intel macOS, and Apple Silicon macOS

## Network cockpit

In an interactive terminal, `speedtest` opens a full-screen home dashboard rather than
starting a measurement immediately. Opening the menu reads **local history only**:
no connectivity check, DNS query, server discovery, or throughput test runs until you
explicitly start a network operation. The connection status says **NETWORK NOT PROBED**
instead of guessing whether you are online.

```bash
speedtest                 # Open the dashboard; no automatic network traffic
speedtest --run           # Bypass the menu; start the existing live speedometer
speedtest --plain         # Run immediately, with noninteractive text output
speedtest --json          # Run immediately, with the existing canonical JSON output
```

The main flow is **Home → Run Speed Test → configuration → Start test → live gauge →
results**. Review duration, concurrent streams, backend, render rate, timeout, and
history preference before starting. Existing command-line options seed those values;
for example, `speedtest --backend librespeed --duration 5 --no-save` opens the cockpit
with that profile. `--run` retains the immediate-test workflow with the same options.

Home also provides **History**, **Statistics**, **DNS Tools**, **Diagnostics**, and
**Settings**. The recent result opens with `v`. History shows the last 30 days with
keyboard selection; `Enter` opens a saved result without saving it again, and `c`
compares the two newest saved runs. Statistics and comparison reuse the CLI's existing
analysis, including explicit better/worse labels rather than color alone.

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select a row; scroll on report/result screens |
| `Enter` | Open, start, or edit the selected item |
| `Tab` / `Shift+Tab`, `←` / `→` | Switch sibling sections |
| `Esc` / `Backspace` | Back to the previous screen, preserving its selection |
| `+` / `-`, `Space` | Change the selected configuration/setting value |
| `PgUp` / `PgDn` | Scroll long reports |
| `r` | Reload history or retry/start the current tool or failed test |
| `?` | Open/close the keyboard guide |
| `q` | Quit; ask before cancelling active work |
| `Ctrl+C` | Cancel and exit (130); keyboard cancellation waits for an ongoing save |

During a test or diagnostic, section navigation pauses but help, resize, and cancellation
remain responsive. `Esc` opens a confirmation with **Continue** selected by default;
choose **Cancel** or press `y` to stop. Incomplete measurements are never saved. A
completed result remains visible if export/history fails, with a **SAVE FAILED** notice.

Settings apply to **this session only**; they do not change a configuration file or the
CLI defaults for future launches. They include a reduced-motion option that removes
needle interpolation and animated activity markers. The balanced timing preset uses
8-second phases, 2 streams, 60 FPS, and a 120-second deadline. CLI `--timeout` starts
when an operation starts, not while browsing the menu.

DNS and diagnostic tools have a separate **Ready to start** screen. They run the
existing read-only commands and show their reports in scrollable panels, with bounded
output, timeout, cancellation, and retry. Available tools include DNS configuration
inspection/catalog/testing/UDP and DoH benchmarks, Network Doctor, Wi-Fi, ICMP loss,
stability monitoring, and backend verification. They do **not** change DNS settings;
configuration/rollback and all specialized options remain available through the
unchanged CLI subcommands. Stability in the menu runs for 60 seconds without saving.

The cockpit supports **80×24** and larger terminals. Below that size, navigation is
preserved behind a resize notice and hidden controls cannot start a test. It uses
true color when advertised, otherwise 256-color or basic ANSI colors, and never
requires a mouse. For screen readers, no-color environments, pipes, or non-animated
reports, use `--plain`; automatic terminal detection is unchanged.

`--output` and `--format json|csv` keep their existing export behavior. In a menu
session, the explicit output path is reused for each completed test (overwriting
that file); history remains controlled independently by `--no-save` or the session
setting. Use distinct paths between launches to keep separate exports. Browsing an
old result never exports or persists it again.

See [cockpit architecture and verification](docs/network-cockpit.md) for the state
machine, service boundaries, and test commands.

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

The custom URL is treated as the LibreSpeed base URL and standard `garbage.php` / `empty.php` endpoints are assumed.

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

Run a server on another machine in the LAN, binding its trusted LAN address explicitly:

```bash
speedtest serve --bind 192.168.1.50:9876
```

Without `--bind`, the server listens only on `127.0.0.1:9876`. The LAN protocol is unauthenticated and unencrypted: do not expose it to the Internet. Use firewall rules on shared networks. The server bounds live connections and expires idle or overlong sessions; those limits are not authentication.

Then from another machine:

```bash
speedtest lan 192.168.1.50:9876
speedtest lan 192.168.1.50:9876 --duration 10 --streams 4
speedtest lan 192.168.1.50:9876 --json
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

Recovery:

```bash
speedtest dns rollback
speedtest dns reset
```

DNS writes snapshot the existing configuration before applying changes, verify resolution afterward, and attempt automatic rollback if post-change validation fails. On Linux, persistent automatic configuration currently requires NetworkManager; unmanaged resolver setups remain read-only.

## Network Doctor and comparison

```bash
speedtest doctor
speedtest doctor --full
speedtest doctor --json
```

The lightweight doctor checks route/interface state, gateway latency where available, IPv4/IPv6 reachability, DNS health, HTTPS, and platform network context. `--full` also runs throughput/bufferbloat analysis.

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

## History and statistics

```bash
speedtest history
speedtest history --days 30 --limit 50
speedtest history --json

speedtest stats
speedtest stats --days 90
speedtest stats --json
```

History/statistics include median/best throughput, latency statistics, quality context, S-tier counts, trend detection, a Unicode throughput sparkline, and latest-run anomaly detection against prior saved results.

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
cargo install --locked --git https://github.com/cmdr-chara/speedtest-cli --branch determination --force
speedtest --version
```

## Normal speed-test usage

```bash
speedtest
speedtest --run
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
--run                        bypass the menu and start immediately
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
--timeout <SEC>              overall Internet measurement deadline (default: 120)
--color <POLICY>             auto, always, or never
--progress <POLICY>          auto, always, or never; phase lines use stderr
```

The speedometer physics run independently of the render cap, so lowering `--fps` reduces terminal work without changing network measurements.

## Terminal and automation behavior

Normal tests automatically use plain output when stdin or stdout is redirected. `TERM=dumb`, nonempty `NO_COLOR`, `CLICOLOR=0`, `--color never`, and `--progress never` also select the non-animated interface. `--color always` overrides color environment preferences but never forces raw mode on a pipe or a dumb terminal. The `--run` speedometer keeps its completed result in terminal scrollback. The cockpit keeps results in its Results screen and, when enabled, local history.

Results go to stdout. Default-test phase progress goes to stderr: `auto` shows it only on a terminal and keeps JSON mode quiet; `always` explicitly enables it; `never` suppresses it. Plain reports use text labels rather than relying on color. JSON field names and units do not vary with terminal preferences.

```bash
speedtest --json --no-save --timeout 45 > result.json
speedtest --plain --progress always --no-save > result.txt 2> progress.log
speedtest --color never
```

`--timeout` bounds the default Internet measurement, including server selection. Ctrl+C cancels default, stability, verify, LAN, server, and loss operations; owned network work is dropped and terminal state is restored. Configuration-changing DNS operations retain their existing rollback lifecycle rather than being interrupted halfway through a write. DNS confirmation without a terminal fails with guidance to use `--dry-run` or an explicit `--yes`.

| Exit | Meaning |
| --- | --- |
| 0 | Success; also a consumer deliberately closing stdout early |
| 1 | Runtime, input-file, network, or persistence failure |
| 2 | Invalid command/arguments (Clap usage error) |
| 3 | A valid offline threshold check failed |
| 124 | Overall Internet measurement deadline exceeded |
| 130 | Handled cancellation |

In JSON mode, runtime failures emit `{"error":{"code":1,"message":"..."}}` on **stderr**, without a success result on stdout. Usage errors remain human-readable on stderr. In immediate CLI runs, explicit file exports and automatic persistence must succeed before a completed default/stability result is printed. The menu instead retains the completed result on screen and labels a save/export failure explicitly. A broken pipe is handled without a panic/backtrace; it does not roll back already completed persistence.

Color/progress flags are global. Measurement flags are command-specific: use `speedtest verify --duration 5`, not `speedtest --duration 5 verify`. Options that would otherwise be silently ignored before a subcommand are rejected. `--format` requires `--output`, and `--librespeed-server` requires `--backend librespeed`; these mistakes fail before any measurement starts.

## Offline checks for scripts

Evaluate a saved canonical result without contacting the network or changing history:

```bash
speedtest --json --no-save > result.json
speedtest check result.json --min-download 100 --min-upload 20 --max-latency 30
speedtest check result.json --max-jitter 5 --max-loaded-latency 80 --max-age 300 --json
cat result.json | speedtest check - --min-download 100 --json
```

At least one threshold is required. Throughput thresholds use decimal **Mbps**, latency/jitter use **ms**, and `--max-age` uses **seconds**. Equality passes. `--max-loaded-latency` checks both download and upload loaded latency; a missing value fails, rather than becoming zero. Freshness rejects future timestamps. Input is limited to one JSON document of at most 4 MiB. Nonfinite or negative thresholds are rejected.

The versioned check report contains `schema_version`, `passed`, `result_timestamp`, and per-metric `checks` with `actual`, `limit`, `operator`, `unit`, and `passed`. A missing measurement has `actual: null`. Failed thresholds return **3**; malformed input returns **1**. These are user-selected acceptance criteria, not a certified diagnosis or proof that a saved file is authentic.

For concurrent automation, use `--no-save` and distinct explicit output files: the existing shared JSONL history does not yet coordinate concurrent writers.

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

**Upload accounting:** only complete requests acknowledged by a successful HTTP response count toward Cloudflare/LibreSpeed upload goodput. Rejected, buffered-only, and deadline-cancelled requests do not count. Adaptive payloads start small, but a final in-flight request may still be excluded, so this is a conservative application-level measurement—not TCP wire throughput. LAN upload timing includes acknowledgement drain and validates the returned byte count. Old upload results may not be directly comparable after this correction.

Custom LibreSpeed URLs must use HTTP(S) without credentials, query strings, or fragments. Measurement redirects are not followed; supply the final endpoint URL. HTTPS certificate verification remains enabled. Do not place secrets in URL paths: server metadata is part of the result.

Standalone `speedtest loss` does not silently inject ICMP loss into an unrelated saved throughput run. That separation keeps protocol semantics explicit.

## Data storage

Completed tests are stored in the platform data directory unless `--no-save` is used.

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
│   ├── cockpit/            # reducer, runtime, views, theme, read-only adapters
│   └── speedometer/        # shared live gauge and physics
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

See [the assessment, competitive research, prioritized plan, and remaining risks](docs/cli-reliability-review.md) and [contributor verification instructions](CONTRIBUTING.md).

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
