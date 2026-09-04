# CLI reliability and automation review

Review date: 2026-09-05 (Europe/Rome). Original candidate: `b88363d0a84dbd0349e930ad085b2ae071aa1240`, default branch `determination`. Work is isolated on `improve/cli-reliability-and-automation`. No releases, deployments, DNS writes, or merges are part of this review.

## Assessment and protected contracts

This is a Rust library and `speedtest` binary, not a single-screen application. `src/cli.rs` owns Clap contracts; `src/bin/speedtest.rs` dispatches and prints; `engine/` implements HTTP backends; `lan.rs` implements a binary TCP protocol; `analysis/` derives local quality heuristics; `tui/` renders events; `storage/` persists canonical results; DNS, doctor, Wi-Fi, loss, history, and comparison are independent modules.

The baseline has useful breadth: idle/loaded latency, quality explanation, historical analysis, Cloudflare/LibreSpeed cross-checking, self-hosted LAN, DNS tooling, ICMP response-loss, and native Wi-Fi inspection. Replacing those systems or adding another cosmetic dashboard would be lower value than fixing their boundaries. Preserve canonical result keys, decimal Mbps/ms units, legacy JSON without analysis, existing command names, and DNS rollback behavior. Avoid changing scores without a calibrated study.

Baseline evidence: original default CI succeeded; the unchanged application was additionally built/linted/tested on Linux, macOS, and Windows in [baseline run 33925080457](https://github.com/cmdr-chara/speedtest-cli/actions/runs/33925080457). Running that baseline binary with redirected output and `--no-save` reproduced an immediate raw-terminal-mode error. Source inspection also identified pre-response upload byte accounting, detached task ownership, unbounded LAN sessions, and option-shaped native ping targets. These findings led to focused reproductions, not a claim of exhaustive audit coverage.

## Competitive research: verified capabilities, not absence claims

Primary documentation was retrieved during this review. Capabilities below are documented, not measured head-to-head. An unmentioned capability is **unknown**, not absent. The offline threshold workflow is useful differentiation for this project; it is not claimed to be unique among every speed-test tool or wrapper.

| Source | Verified capability/pattern | Decision for this CLI |
| --- | --- | --- |
| [Ookla publisher repository](https://packagecloud.io/ookla/speedtest-cli) and [inspected package](https://packagecloud.io/ookla/speedtest-cli/packages/ubuntu/bionic/speedtest_1.2.0.84-1.ea6b6773cf_armhf.deb?distro_version_id=190) | Published manual documents server selection/listing, interface/IP binding, human/CSV/TSV/JSON/JSONL formats, interactive-default progress, and fixed machine-output units. | Automatically detect terminals, make progress explicit, and keep machine units stable. Do not copy Ookla's bytes/second convention into the existing Mbps schema. |
| [LibreSpeed CLI README](https://github.com/librespeed/speedtest-cli) | Custom server lists, PHP/Go backends, ping/jitter/transfers, JSON/CSV, and optional telemetry. | Retain self-hosted endpoint support; validate URLs and test protocol behavior against local servers. Do not add telemetry or a certificate-verification bypass. |
| [sivel/speedtest-cli README](https://github.com/sivel/speedtest-cli) | Simple/JSON/CSV output, source/timeout/server controls, directional tests; explicitly discusses differences in latency methodology. | Document HTTP latency versus ICMP and avoid marketing cross-tool agreement as accuracy proof. |
| [ESnet iperf3 documentation](https://software.es.net/iperf/invoking.html) | Explicit server/client roles, JSON/JSON streaming, flush controls, and receive/send timeout controls. The rendered manual warns it may lag the installed version. | Bound connection lifetimes and protocol I/O; keep readiness truthful and automation outcomes explicit. |
| [GitHub CLI environment manual](https://cli.github.com/manual/gh_help_environment) | NO_COLOR/CLICOLOR controls, forced terminal behavior, disabled prompts, and accessible/textual interaction choices. | Honor color preferences, provide a non-animated path, and refuse implicit DNS prompts without a terminal. |
| [curl manual](https://curl.se/docs/manpage.html) | Separate connection/whole-operation timeouts, progress/error controls, and explicit HTTP failure semantics. | Use a whole-measurement deadline as well as request timeouts, and reserve stdout for results. |

The Ookla package was downloaded in CI, SHA-256 checked against publisher metadata (`f00c46b4945e3e1fea08e4858db78c9d8f3e35a8c9daa4c033df7eb03931294d`), and only its manual extracted. No proprietary binary was installed or executed. Evidence is version-specific: manual 1.2.0.84, not an unsupported assertion that this is the newest release. The primary marketing page was inaccessible in the review environment. No competitor performance, privacy, or feature-absence claim is inferred from that access failure.

Additional implementation references: [NO_COLOR convention](https://no-color.org/), [Tokio process cancellation](https://docs.rs/tokio/latest/tokio/process/struct.Command.html), [Cloudflare's engine](https://github.com/cloudflare/speedtest), and [RustSec release history](https://github.com/rustsec/rustsec/releases).

## Prioritized execution plan and completion evidence

Magnitude describes scope; priority describes execution order, not line count.

| Priority / magnitude | Evidence-backed problem | Completed slice and verification |
| --- | --- | --- |
| P1 / medium | Default redirected invocation enters raw TUI; print macros can panic on broken pipes; errors lack scripting status contracts. | Terminal detection; color/progress policy; fallible writes; structured runtime stderr; explicit usage/runtime/threshold/timeout/cancel statuses. Rust black-box tests plus real loopback CLI/PTY checks. |
| P1 / major measurement semantics | Cloudflare incremented shared upload totals as request bodies were pulled, before successful responses; cancellation could leave detached engine/probe tasks. | Per-request successful-response accounting, adaptive payloads, owned phase futures/JoinSets, bounded control responses and overall deadline. Local peers consume then reject, delay acknowledgements, and fail mid-phase. Canonical field names remain stable; upload semantics intentionally change. |
| P1 / medium | LAN listener exposed all interfaces by default, spawned unbounded sessions, accepted false echoes, and trusted upload byte acknowledgements. | Loopback default with explicit network bind, 64 live-connection ceiling, header/idle/session deadlines, exact acknowledgement matching, truthful readiness. Protocol tests and real server/client round trip. |
| P2 / medium | Arbitrary native ping targets could become program options; custom URL credentials/query values could enter serialized history. | Validate targets before process creation, cancellable bounded ping child, reject secret-bearing/non-HTTP URLs, refuse measurement redirects. Unit/CLI tests prove rejected values do not become network requests or echoed secrets. |
| P2 / medium | Saved metrics need shell-friendly, explicit pass/fail assessment rather than re-running transfers or scraping tables. | Offline `check`: file/stdin, thresholds, freshness, missing-value failure, bounded JSON input, machine report and exit 3. Boundary, invalid-input, and subprocess tests. |
| P2 / minor delivery | No committed dependency resolution; CLI/package behavior not covered by original CI. | Commit lockfile, use `--locked`, add cross-platform CLI/loopback/package tests and scheduled RustSec audit with retained artifacts. Packaging input validation prevents traversal before output mutation. |

## Interpretation and compatibility

Upload throughput is now conservative application goodput: count a complete request only after a successful response is fully received before the phase deadline. This excludes refused and deadline-cancelled payloads, but can undercount a large final in-flight upload or slow connection. Adaptive small starting requests reduce that boundary effect. Neither successful HTTP responses nor a LAN peer's acknowledgement authenticate an honest server or measure TCP wire bytes. LAN upload elapsed time includes acknowledgement drain. Do not compare old and new upload numbers as a pure network trend without noting this methodology change.

Plain output is selected for redirected stdin/stdout, TERM=dumb, NO_COLOR, CLICOLOR=0, `--color never`, or `--progress never`. The TUI remains available in capable interactive terminals and completed results remain in scrollback. Explicit `--color always` overrides color environment preferences but does not force raw mode on pipes or dumb terminals. `--progress always` adds plain phase lines on stderr for the default speed test; JSON stdout is not contaminated. Runtime JSON errors go to stderr; Clap usage errors remain human-readable.

`serve` now binds 127.0.0.1 by default. Cross-machine callers must explicitly bind a trusted LAN address. Normal test options placed before subcommands are rejected instead of silently ignored; put subcommand-specific flags after the command. Unsupported URL credentials/query/fragment/redirect behavior is deliberately rejected. The native loss target accepts IP addresses and ASCII/punycode hostnames, not arbitrary command-like strings or scoped IPv6 interface expressions.

## Coverage limits and remaining risks

The focused slices do not certify the repository as secure or prove calibrated WAN accuracy. No OS DNS configuration was modified. Existing DNS transaction/rollback and synchronous doctor/Wi-Fi/native administration calls still need representative platform, privilege, locale, and timeout testing. The Windows DNS override inspected already escapes single quotes; shell injection there was not claimed as a confirmed issue. Native ICMP parsing remains locale/tool dependent and now fails instead of inventing a loss result for unsupported output.

The LAN protocol remains unauthenticated and unencrypted; limits reduce resource exposure but are not Internet-facing service hardening. Only use it on a trusted network with firewall controls. HTTP custom endpoints remain explicitly permitted for self-hosting; TLS is verified for HTTPS, and redirects are not followed. URLs may still contain user-chosen sensitive path segments; do not put secrets in them. Plain output filters terminal/bidi control characters, not all possible misleading text.

Storage remains a follow-up: JSONL writers lack cross-process coordination, automatic per-result filenames use second precision, and general history/compare reads are not uniformly bounded or transactional. Prefer `--no-save` and unique explicit result files for concurrent automation until that subsystem is addressed. The new offline `check` reader is bounded to 4 MiB. Filesystem permissions and historical data retention are not comprehensively redesigned here.

Automated success on loopback validates lifecycle, accounting formulas, error paths, and script contracts, not Internet server policies. Public endpoint checks must be recorded separately with their actual outcome. PTY behavior is exercised on Unix; Windows native console restoration requires additional direct evidence. No claim is made of screen-reader certification, exhaustive security audit, or equivalent scores across vendors.
