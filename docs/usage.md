# User guide

[← Back to the project overview](../README.md)

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
- Guided connection diagnosis with workload-specific recommendations
- Repeated JSONL monitoring with bounded intervals and explicit failure records
- History, trends, sparklines, and anomaly detection
- DNS inspection, health testing, benchmarking, configuration, rollback, and optimization
- 20 built-in DNS resolver profiles across multiple providers
- DNS-over-UDP and real DNS-over-HTTPS benchmarking
- DNS-over-TLS and DNS-over-QUIC benchmarking where providers advertise support
- Cloudflare and LibreSpeed Internet backends
- Backend cross-checking with `speedtest verify`
- Explicit IPv4/IPv6 selection and backend A/B comparison
- Offline LibreSpeed server catalog with optional latency probes, pinning, and exclusions
- Source-address binding for multi-WAN and VPN hosts
- TCP/TLS decomposition plus HTTP/2 and HTTP/3 capability probes
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
explicitly start a network operation. Idle status badges are intentionally omitted;
the measurement profile explains that no background network probes run.

```bash
speedtest                 # Open the dashboard; no automatic network traffic
speedtest --run           # Bypass the menu; start the existing live speedometer
speedtest --plain         # Run immediately, with noninteractive text output
speedtest --json          # Run immediately, with the existing canonical JSON output
speedtest --jsonl         # Stream versioned phase/progress/result events as JSONL
```

The main flow is **Home → Run Speed Test → configuration → Start test → live gauge →
results**. Review duration, concurrent streams, backend, render rate, timeout, and
history preference before starting. Existing command-line options seed those values;
for example, `speedtest --backend librespeed --duration 5 --no-save` opens the cockpit
with that profile. `--run` retains the immediate-test workflow with the same options.

Home also provides **History**, **Statistics**, **DNS Tools**, **Diagnostics**, and
**Settings**. The recent result opens with `v`. History shows the last 30 days with
keyboard selection; `Enter` opens a saved result without saving it again. `c` compares
the selected run with its immediately older run. Pin any selected run as the **before**
baseline with `b`, select another run, and press `c` to compare it as **after**; press
`b` on the pinned run to clear it. Reloading history preserves the selected run and
pinned baseline even when a new run arrives. Statistics and comparison reuse the CLI's existing
analysis, including explicit better/worse labels rather than color alone.

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select a row; scroll on report/result screens |
| `Enter` | Open, start, or edit the selected item |
| `Tab` / `Shift+Tab`, `←` / `→` | Switch sibling sections |
| `Esc` / `Backspace` | Back to the previous screen, preserving its selection |
| `Space` on Home / Results | Start a test with the current session settings |
| `+` / `-`, `Space` in configuration/settings | Change the selected value |
| `PgUp` / `PgDn` | Page through history or scroll long reports |
| `Home` / `End` | Jump to the newest/oldest history run |
| `b` / `c` | Pin/unpin a history baseline / compare the selected run |
| `r` | Reload history or retry/start the current tool or failed test |
| `?` | Open/close the keyboard guide |
| `z` | Open the scrollable text-size guide, including Arch/Omarchy guidance |
| `q` | Quit; ask before cancelling active work |
| `Ctrl+C` | Cancel and exit (130); keyboard cancellation waits for an ongoing save |

During a test or diagnostic, section navigation pauses but help, resize, and cancellation
remain responsive. `Esc` opens a confirmation with **Continue** selected by default;
choose **Cancel** or press `y` to stop. Incomplete measurements are never saved. A
completed result remains visible if export/history fails, with a **SAVE FAILED** notice.

Settings apply to **this session only**; they do not change a configuration file or the
CLI defaults for future launches. They include a reduced-motion option that removes
needle interpolation and animated activity markers, freezes the dial highlight,
and immediately settles page, chart, selection, and results reveals. The segmented
scale and measured peak marker remain visible. The balanced timing preset uses
8-second phases, 2 streams, 60 FPS, and a 120-second deadline. CLI `--timeout` starts
when an operation starts, not while browsing the menu.

The default cockpit includes a scanning brand entrance, short page and dialog light
sweeps, a moving selection underline, and staggered result highlights. Live tests
use a segmented dial, a short animated cursor, a peak marker, phase indicators,
and a phase-labeled recent-sample trace when there is enough vertical space. The
primary number is the exact latest sample to one decimal; only dial geometry is
smoothed. Missing samples stay unavailable, and the previous phase is shown
separately. The trace is hidden during completion so upload samples cannot appear
beneath the final download reading. Statistics traces the selected saved series
without changing its scale or sample count. Exact values remain readable during
every transition, and navigation stays immediate. Idle pages stop animating after
their entrance. Use **Reduced motion** or a lower `--fps` cap for a quieter display.
See the [before/after and animated gallery](images/design/README.md).

Home shows the current backend, phase duration, streams, save/export status, and
data-use warning beside the quick-start action. Enter continues to open
configuration without network traffic. On Results, Space uses the **current
session settings** shown above the measurements; it does not reconstruct settings
from a historical result. Enter opens configuration. Repeated tests replace the
run page and preserve its Back destination. Busy states, dialogs, key repeats,
search entry, and terminals below 80×24 cannot trigger quick start.

DNS and diagnostic tools have a separate **Ready to start** screen. They run the
existing read-only commands and show their reports in scrollable panels, with bounded
output, timeout, cancellation, and retry. Available tools include DNS configuration
inspection/catalog/testing/UDP, DoH, DoT, and DoQ benchmarks, Network Doctor, guided
diagnosis, Wi-Fi, ICMP loss, stability monitoring, and backend verification. They do **not** change DNS settings;
configuration/rollback and all specialized options remain available through the
unchanged CLI subcommands. Stability in the menu runs for 60 seconds without saving.

The cockpit supports **80×24** and larger terminals. Below that size, navigation is
preserved behind a resize notice and hidden controls cannot start a test. The default
**Terminal (adaptive)** palette inherits the terminal's foreground, background, and
ANSI colors on Linux, macOS, and Windows, rather than forcing a dark canvas. Settings
also offers Graphite, Light, and Monochrome palettes. Fixed palettes use truecolor or
256 colors when advertised and safely fall back to terminal-native colors otherwise.
No terminal profile or palette is modified, and no mouse is required. For screen readers, no-color environments, pipes, or non-animated
reports, use `--plain`; automatic terminal detection is unchanged.

**Comfortable** layout uses wide five-row metric digits on spacious screens, three-row
digits on medium screens, and ordinary values where space is limited; **Compact** uses ordinary text. Enlarged digits also show an exact ordinary value with its unit
underneath. The dashboard uses the terminal width with a small edge margin. Metric
groups stay together, and short summaries keep their controls nearby. History uses
only the rows it needs, while longer histories and reports keep the full scroll
viewport. The live dial stays beside its readings as the terminal grows.
Highlights cover the selected action label,
not its description or an entire empty row. To enlarge all body text, use your
terminal's Zoom In or font-size setting: the application reflows after resizing
but never changes your font or window size. Appearance settings are session-only.

`--output` and `--format json|csv` keep their existing export behavior. In a menu
session, the explicit output path is reused for each completed test (overwriting
that file); history remains controlled independently by `--no-save` or the session
setting. Use distinct paths between launches to keep separate exports. Browsing an
old result never exports or persists it again.

See [cockpit architecture and verification](network-cockpit.md) for the state
machine, service boundaries, and test commands.

## Interface language and text size

The interface supports **English, Italian, Spanish, French, German, Portuguese,
Simplified Chinese, and Japanese**. Select a language in **Settings → Language**
without restarting or making a network request, or use the global option:

```bash
speedtest --language it
speedtest --language ja --help
speedtest dns benchmark --language de --help
SPEEDTEST_LANGUAGE=fr speedtest history
speedtest --language en --json --no-save > result.json
```

Supported codes: `en`, `it`, `es`, `fr`, `de`, `pt`, `zh-CN`, `ja`, and `auto`.
Precedence: explicit `--language`, then `SPEEDTEST_LANGUAGE`, then the first nonempty
`LC_ALL`, `LC_MESSAGES`, or `LANG`. Unsupported system locales fall back to English;
unsupported explicit codes are usage errors. Regional tags such as `it_IT.UTF-8`
and `pt-BR` resolve to their base language. Traditional Chinese locales are not
mislabeled as Simplified Chinese. Windows users can select a language explicitly or
use `SPEEDTEST_LANGUAGE`; a Windows display-language API is not queried.

Navigation, settings, help, built-in human summaries, findings, and the immediate
speedometer/stability views are localized. **Commands, flags, shortcuts, provider
names, units, canonical JSON (including machine error records), CSV columns and
stored results do not change language.** Native OS/provider error details and
unknown/custom saved prose are preserved verbatim; generated parser annotations
and some low-level error details remain English. Diagnostic reports retain the
language used when started; rerun a report after changing the interface language.
All translations are embedded, require no downloads, and have key/placeholder
parity tests. Community linguistic review is welcome.

For larger ordinary text, press **`z`** for guidance. The application cannot portably
change a terminal's font size: larger panels do not enlarge each character.
On **Omarchy's default Alacritty**, edit the existing `[font]` section in
`~/.config/alacritty/alacritty.toml`, for example `size = 14.0`. Preserve the other
settings and imports; do not add a duplicate `[font]` section. The cockpit reflows
when the resulting terminal grid changes; keep at least **80 columns × 24 rows**.
No terminal or desktop configuration is modified by Speedtest. Details and primary
terminal documentation are in [localization and sizing](localization.md).

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

#### Server discovery and path control

List the built-in LibreSpeed catalog without contacting the network:

```bash
speedtest servers
speedtest servers --json
```

Add `--probe` when you want a bounded latency probe for every catalog entry:

```bash
speedtest servers --probe
speedtest servers --probe --family ipv4 --json
```

The catalog assigns stable IDs for the current built-in registry. Pin a run to one
server or keep specific servers out of automatic selection:

```bash
speedtest --backend librespeed --server-id 7 --no-save
speedtest --backend librespeed --exclude-server-id 1 --exclude-server-id 2
```

`--server-id` and `--exclude-server-id` apply to the built-in LibreSpeed registry;
they cannot be combined with `--librespeed-server`. The default remains latency-based
automatic selection. The catalog command is offline unless `--probe` is supplied.

For machines with multiple WAN links, VPN routes, or policy-based routing, bind the
HTTP measurement sockets to a local source address:

```bash
speedtest --source-ip 192.0.2.10 --family ipv4 --no-save
```

The source address must match `--family` when a family is forced. This is an address
binding control, not an interface-name resolver; use the address assigned to the route
you want to test.

### Backend verification

Use both Internet engines to check whether a result is strongly backend/path dependent:

```bash
speedtest verify
speedtest verify --duration 8 --streams 2
speedtest verify --json
speedtest verify --family ipv4
speedtest verify --compare-families --json
```

`verify` compares Cloudflare and LibreSpeed results rather than assuming a single public endpoint is ground truth.
Use `--family ipv4` or `--family ipv6` to constrain both backend runs. `--compare-families`
runs an additional IPv4/IPv6 A/B pass and records unavailable families explicitly; it can
take substantially longer than a normal verification.

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

Without `--bind`, the server listens only on `127.0.0.1:9876`. The LAN protocol is unauthenticated and unencrypted: do not expose it to the Internet. Use firewall rules on shared networks. The server bounds live connections and retained connection tasks, reaps completed work before admitting more, and expires idle or overlong sessions; those limits are not authentication.

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

Benchmark encrypted stream transports when the provider registry advertises them:

```bash
speedtest dns benchmark --protocol dot
speedtest dns benchmark --protocol doq
speedtest dns benchmark --protocol dot --family ipv6 --json
```

DoT validates the TLS certificate and sends length-prefixed DNS messages over TCP.
DoQ uses a QUIC bidirectional stream with the `doq` ALPN and validates the DNS response
length and contents. These probes are opt-in network operations; a resolver that does not
advertise the selected transport is omitted rather than treated as a failed UDP resolver.

UDP, DoH, and explicit-resolver probes share the same response validation. A success
requires a complete answer to the requested A/IN question, with matching transaction
ID and name and a usable address either directly or through a valid CNAME chain.
Truncated, malformed, unrelated, and negative answers do not count as successful
resolution. DoH response bodies are bounded to the DNS message-size limit. This
validates probe responses; it does not add DNSSEC verification or UDP-to-TCP fallback.

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

### Guided connection diagnosis

Use `diagnose` when you want an ordered assessment rather than a single raw measurement:

```bash
speedtest diagnose
speedtest diagnose --profile calls
speedtest diagnose --profile gaming --family ipv4 --no-stability --json
speedtest diagnose --backend librespeed --librespeed-server https://speed.example.test
```

The command combines the read-only Doctor checks with an optional stability run, a normal
throughput result, VPN/interface hints, path-MTU probing, DNS/TCP/HTTPS timing, and explicit
HTTP/2/HTTP/3 probes. `--no-stability` and `--no-speedtest` are useful for a narrower pass.
It never changes DNS configuration. Throughput uses the normal history/export policy, while
the aggregate diagnosis is printed or serialized as its own versioned report.

### Repeated monitoring

`monitor` runs the normal Internet engine at a controlled cadence and writes one versioned
JSON object per attempt:

```bash
speedtest monitor --count 4 --interval 15m --json --output monitor.jsonl --no-save
speedtest monitor --backend librespeed --family ipv6 --continue-on-error
```

Without `--count`, monitoring continues until Ctrl+C. A failed attempt is recorded with
`ok: false` and an `error` field; `--continue-on-error` keeps the schedule running but the
command still exits nonzero after a bounded run if any attempt failed. The monitor file is
append-only and uses the same locking guarantees as history.

Review the saved stream later without starting a probe:

```bash
speedtest monitor --report
speedtest monitor --report --input monitor.jsonl --json
```

The report has its own versioned JSON schema and summarizes attempts, success rate,
failure streaks, recent failure messages, and distributions for successful download,
upload, idle latency, jitter, and quality scores. A missing monitor file is an empty
report; malformed or oversized records fail with a line-specific error. `--report` is
offline and cannot be combined with measurement scheduling flags such as `--count` or
`--output`.

### Live JSONL measurement events

The one-shot measurement can stream progress without contaminating stdout with human
text:

```bash
speedtest --backend librespeed --librespeed-server https://speed.example.com --jsonl --no-save
```

Each line has `schema_version: 1` and a `type` such as `phase`, `idle_latency`,
`throughput_sample`, `loaded_latency`, or `result`. The final `result` event contains
the same canonical `TestResult` object emitted by `--json`. Progress and errors remain
separate from the event stream, making the mode suitable for dashboards and CI
consumers. `--jsonl` is mutually exclusive with `--json` and `--plain`.

### Path and transport interpretation

The guided report's VPN hint is interface-name evidence, not a claim that traffic is or is
not encrypted. MTU probing uses platform `ping` flags and can be unavailable when ICMP is
blocked or the native utility is missing. HTTP/2 and HTTP/3 are tested against a fixed
Cloudflare endpoint with certificate verification enabled; failure can reflect endpoint,
firewall, proxy, or QUIC policy rather than a general connection outage.

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
speedtest history --scope internet --backend cloudflare
speedtest history --server speed.cloudflare.com --output history.jsonl --format jsonl
speedtest history --scope lan --output lan-history.csv --format csv

speedtest stats
speedtest stats --days 90
speedtest stats --json
speedtest stats --backend librespeed --server example.net

speedtest insights
speedtest insights --days 90 --scope internet
speedtest insights --backend cloudflare --json
speedtest insights --scope lan --server 192.168.1.50:9876 --json
```

All three commands support `--scope all|internet|lan`, `--backend BACKEND`, and
`--server HOST`. Backends and hostnames match case-insensitively. Complete HTTP(S)
URLs normalize scheme/host but retain case-sensitive paths and query strings;
server display names and partial host names do not match. Filters combine. Use the
host value from a saved JSON result when an endpoint includes a scheme or port.
The existing `stats --scope all` policy prefers Internet results when both Internet
and LAN data match; choose `--scope lan` to summarize LAN measurements explicitly.

`history --output PATH --format json|csv|jsonl` atomically replaces an export with
**all matching runs in chronological order**. `--limit` affects only the terminal
table; it never truncates JSON output or file exports. JSON is a canonical result
array, JSONL contains one canonical result per line, and CSV retains the existing
single-result column names and units. Empty selections export `[]`, an empty JSONL
file, or a CSV header. Export failures return an error without claiming success.

History/statistics include median/best throughput, latency statistics, quality context, S-tier counts, trend detection, a Unicode throughput sparkline, and latest-run anomaly detection against prior saved results.
Anomaly detection requires at least five earlier runs on the same backend/server path;
it does not flag a provider or server switch as a connection regression.

`insights` is a deeper, offline report for saved history. It keeps each backend/server
path in its own population, reports interpolated p10/median/p95/max distributions for
download, upload, idle latency, jitter, loaded latency, and quality, and gives a
direction-aware trend when at least six samples are available. `--scope internet` and
`--scope lan` prevent unlike measurements from being compared; `--backend` is
case-insensitive. The time-of-day comparison is intentionally conservative: each window
needs at least three usable download samples spread across two distinct UTC dates.
Missing analysis or partial loaded-latency samples remain unavailable rather than being
treated as zero. JSON output has `schema_version: 1` and is intended for dashboards and
automation; it never starts network activity or changes saved data.

The cockpit comparison includes download, upload, latency, jitter, quality score,
and bufferbloat, with before/after values, changes, and explicit better/worse labels.
Each side identifies its timestamp and backend. Missing metrics remain unavailable;
they are never shown as zero. Comparison and baseline selection are offline and
session-only.

### Cockpit history explorer

In **History**, press `/` to search backend, server host/name, or date. While editing,
characters such as `q`, `b`, and `c` are query text; Enter applies and Esc cancels.
Press `f` to cycle All / Internet / LAN, `s` to change sort order, `p` to isolate the
selected backend/server path, and `x` to reset the explorer. The toolbar shows active
filters and matching counts. These controls never rewrite or delete saved history.

Enter, baseline pinning, and comparison always act on the visible selected record.
Without a pinned baseline, comparison uses older chronological evidence even when
the table is sorted by speed or latency. A pinned baseline remains a snapshot when
filters change. Empty matches have a reset hint and cannot open or compare a run.

In **Statistics**, `m` cycles download, upload, idle latency, jitter, loaded latency,
and quality; `f` changes scope and `p` changes path. Values retain their units and
indicate whether higher or lower is better. Missing optional measurements remain
unavailable. The chart describes saved test samples, not continuous monitoring.

## Prometheus textfile export

```bash
speedtest metrics                                      # Latest saved run
speedtest metrics --backend cloudflare --max-age 3600
speedtest metrics result.json                          # One canonical JSON file
speedtest --json --no-save | speedtest metrics -        # Explicit test, then conversion
speedtest metrics --max-age 3600 --output speedtest.prom
```

This command is an offline converter for the
[Prometheus text exposition format](https://prometheus.io/docs/instrumenting/exposition_formats/).
It does not start an HTTP server, schedule measurements, or contact a speed-test provider.
Point a separately configured node-exporter textfile collector at the output directory,
or read stdout in your existing automation.

On Unix, new metrics files use `0666` restricted by your process umask: the usual
`022` creates `0644`, while `027` creates `0640` and `077` creates `0600`.
Replacing a file preserves its existing permissions. Choose a directory and file
permissions readable by the collector's account; a restrictive umask remains
restrictive. JSON, CSV, and history exports retain their private creation defaults.

With no input file, the latest matching saved run is selected. `--backend`, `--scope`,
and `--server` apply only to saved-history selection; they cannot be combined with an
explicit file or stdin. `--max-age SECONDS` rejects stale and future results before
writing. Missing history, malformed input, and output errors fail explicitly.
An unsuccessful conversion leaves an existing regular output file intact.

Metric names use the `speedtest_` prefix. Throughput is exported in bits per second,
latency in seconds, and transferred data in bytes. The result timestamp is a Unix-time
gauge so consumers can detect stale evidence. Metrics describe one test, so byte
readings are gauges rather than lifetime counters. Backend and server labels are
escaped. Optional loaded-latency, loss, and quality readings are omitted when absent;
unavailable measurements are never converted to zero. Textfile output has no
per-sample Prometheus timestamps.

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

Release jobs also generate a Homebrew formula and WinGet manifests from the published archive
checksums. These are reviewable submission artifacts, not a claim that an external package
repository has already accepted or published them.

### Install from source

```bash
cargo install --locked --git https://github.com/cmdr-chara/speedtest-cli --branch main --force
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
--family <FAMILY>            any, ipv4, or ipv6
--librespeed-server <URL>    custom LibreSpeed base URL
--server-id <ID>             pin a built-in LibreSpeed server
--exclude-server-id <ID>     exclude a built-in server; may be repeated
--source-ip <IP>             bind active Internet sockets to a local address
--streams <N>                concurrent transfer streams (default: 2)
--duration <SEC>             seconds for each throughput phase (default: 8)
--fps <N>                    interactive render cap, 30–240 FPS
--plain                      disable interactive TUI
--json                       print canonical JSON
--jsonl                      stream versioned live JSON events
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

`--timeout` bounds the default Internet measurement, including server selection. Ctrl+C cancels default, stability, verify, diagnose, monitor, LAN, server, and loss operations; owned network work is dropped and terminal state is restored. Configuration-changing DNS operations retain their existing rollback lifecycle rather than being interrupted halfway through a write. DNS confirmation without a terminal fails with guidance to use `--dry-run` or an explicit `--yes`.

| Exit | Meaning |
| --- | --- |
| 0 | Success; also a consumer deliberately closing stdout early |
| 1 | Runtime, input-file, network, or persistence failure |
| 2 | Invalid command/arguments (Clap usage error) |
| 3 | A valid offline threshold check failed |
| 124 | Overall Internet measurement deadline exceeded |
| 130 | Handled cancellation |

In JSON mode, runtime failures emit `{"error":{"code":1,"message":"..."}}` on **stderr**, without a success result on stdout. Usage errors remain human-readable on stderr. In immediate CLI runs, explicit file exports and automatic persistence must succeed before a completed default/stability result is printed. The menu instead retains the completed result on screen and labels a save/export failure explicitly. A broken pipe is handled without a panic/backtrace; it does not roll back already completed persistence.

Color/progress flags are global. Measurement flags are command-specific: use `speedtest verify --duration 5`, `speedtest diagnose --family ipv4`, or `speedtest monitor --interval 15m`, not `speedtest --duration 5 verify`. Options that would otherwise be silently ignored before a subcommand are rejected. `--format` requires `--output`, and `--librespeed-server` requires `--backend librespeed`; these mistakes fail before any measurement starts.

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

Concurrent runs coordinate automatic history writes with file locks and retain
separate per-run files even when timestamps match. Use distinct explicit `--output`
paths when each job needs its own export: a shared export path still means the last
completed replacement wins.

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

**Upload accounting:** only complete requests acknowledged by a fully received successful HTTP response before the phase deadline count toward Cloudflare/LibreSpeed upload goodput. The cutoff is checked again after response processing, so a delayed task cannot count late bytes. Rejected, buffered-only, and deadline-cancelled requests do not count. Adaptive payloads start small, but a final in-flight request may still be excluded, so this is a conservative application-level measurement—not TCP wire throughput. LAN upload timing includes acknowledgement drain and validates the returned byte count. Old upload results may not be directly comparable after this correction.

Both Internet backends publish loaded-latency summaries when each throughput phase
finishes, allowing the live interface to show download-loaded latency during upload.

Custom LibreSpeed URLs must use HTTP(S) without credentials, query strings, or fragments. Measurement redirects are not followed; supply the final endpoint URL. HTTPS certificate verification remains enabled. Do not place secrets in URL paths: server metadata is part of the result.

Standalone `speedtest loss` does not silently inject ICMP loss into an unrelated saved throughput run. That separation keeps protocol semantics explicit.

## Data storage

Completed tests are stored in the platform data directory unless `--no-save` is used.

JSON/CSV file exports are serialized before a temporary file is written, synced,
and atomically moved into place. A failed replacement keeps the previous export.
Existing file permissions and output symlinks are preserved; device/pipe destinations
retain ordinary streaming-write behavior. New regular files use private temporary-file
permissions where the platform supports them.

Per-run filenames include subsecond timestamps and collision suffixes, so simultaneous
or repeated saves never overwrite an earlier run. JSONL history readers and writers
coordinate through file locks; each serialized record is appended as one protected
operation. These guarantees require filesystem support for file locking. They do not
make the export, per-run file, and history entry one transaction: a later history
failure can leave an already completed export or per-run file available for recovery.

Saved speed-test JSON and individual history records are limited to 4 MiB when read.
Malformed history remains an explicit error with its line number, rather than being
silently discarded. History ranges are filtered while reading; the saved JSON schema
and the CLI's 1–3650 day range are unchanged.

```text
speedtest/
├── history.jsonl
├── results/
├── dns/
│   └── last-backup.json
├── stability/
│   ├── history.jsonl
│   └── results/
└── monitor/
    └── history.jsonl
```

## Architecture

```text
src/
├── analysis/
├── bin/
│   └── speedtest.rs
├── dns/
│   ├── doh.rs
│   ├── secure.rs
│   ├── mod.rs
│   └── system.rs
├── engine/
│   ├── cloudflare_adaptive.rs
│   ├── internet.rs
│   ├── librespeed.rs
│   └── mod.rs
├── compare.rs
├── diagnose.rs
├── doctor.rs
├── history.rs
├── lan.rs
├── loss.rs
├── model/
├── monitor.rs
├── network.rs
├── stability.rs
├── storage/
├── tui/
│   ├── cockpit/            # reducer, runtime, views, theme, read-only adapters
│   └── speedometer/        # shared live gauge and physics
├── verify.rs
├── transport.rs
├── wifi.rs
└── lib.rs
```

Measurement, analysis, UI, persistence, DNS, local-network diagnostics, and backend implementations are kept separate so they can evolve independently.

## Accuracy notes

This is an independent CLI, not an official Cloudflare or LibreSpeed client. Results vary with routing, congestion, Wi-Fi conditions, endpoint behavior, protocol overhead, server capacity, and test methodology.

The quality score, workload grades, stability grades, history trend, anomaly flags, and DNS scores are local heuristics rather than standardized certifications.

A public speed-test server is part of the path being measured. Use `speedtest verify` when you need cross-backend evidence and `speedtest lan` when you need to isolate the local network from the WAN.

ICMP echo loss measures ICMP echo response behavior; it does not prove every transport/application experiences identical loss.

See [the assessment, competitive research, prioritized plan, and remaining risks](cli-reliability-review.md) and [contributor verification instructions](../CONTRIBUTING.md).

## Roadmap

- VPN on/off comparison workflow (the guided report provides interface hints; controlled on/off automation remains future work)
- additional Internet measurement backends and server discovery
- configurable but restrained TUI themes
- upstream acceptance of the generated Homebrew and WinGet submission metadata

## License

See the [Source Available License 1.0](../LICENSE) and the [plain-English summary](../README.md#license-in-plain-english). The summary is explanatory only; the full license controls.
