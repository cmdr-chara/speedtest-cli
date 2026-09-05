# Contributing

## Local checks

Use current stable Rust and Python 3.10 or newer. Cargo.lock is committed for repeatable dependency resolution; dependency changes must include an intentional lockfile update.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --bin speedtest
python .github/scripts/cli_smoke.py
python .github/scripts/test_package_release.py
```

The Rust suites cover units, serialization compatibility, subprocess arguments, HTTP/LAN protocol failures, and black-box command contracts. The Python CLI suite uses only loopback servers and temporary data directories. It verifies human/JSON output, explicit stderr progress, exports, no-save, thresholds, timeout, refusal, LAN throughput, readiness, and (on Unix) SIGINT and PTY restoration. Its transcript is written to `verification/cli-smoke.txt`. Windows runs the network/output cases; Unix PTY assertions are deliberately not claimed for Windows.

Packaging tests exercise ZIP/tar contents, executable permissions, checksums, and path validation in temporary directories. They do not publish or execute release artifacts. CI runs the checks on Linux, Windows, and macOS and retains verification artifacts.

## Public network checks

These consume bandwidth and contact third parties. They are **not** part of deterministic CI. Run only deliberately, and identify the host/network in any report:

```bash
cargo run --locked --release -- --plain --no-save --duration 3 --streams 1 --timeout 35
cargo run --locked --release -- --backend librespeed --json --no-save --duration 3 --streams 1 --timeout 35
```

A local fixture proves protocol and lifecycle behavior, not public endpoint availability or WAN accuracy. A CI runner's result is not the maintainer's Internet speed. Do not silently retry failed public measurements until they pass.

## Measurement and scripting contracts

Keep measurement independent from rendering and persistence. Engines emit canonical events and return `TestResult`, rather than writing terminal output. Use owned futures/JoinSets so cancellation cannot detach network workers. Upload metrics must not count rejected requests or merely queued application bytes. Document methodological changes rather than claiming cross-tool equivalence.

New commands must handle redirected input/output, preserve stdout for results, and define exit status. Add a regression test for the failure being fixed. Use `output::line`/`diagnostic` rather than infallible printing at the CLI boundary. Do not turn terminal control characters from external data into escape sequences.

DNS configuration writes require separate transaction/rollback verification on representative machines; do not run them on CI hosts. Native Windows/macOS/Linux administration tools and localized output require platform-specific evidence.

## Dependency review

The separate scheduled dependency workflow uses pinned `cargo-audit` against RustSec. Review the advisory details and dependency path before remediation; never add an ignore simply to make CI green. Current advisory data requires network access. A clean audit does not certify the program's security.
