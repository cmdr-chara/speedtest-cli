# Verification evidence and residual dependency risks

Review date: 2026-09-05, Europe/Rome. See [the assessment and research](cli-reliability-review.md) for scope, compatibility changes, and non-dependency limitations. Use the PR's current commit checks for final cross-platform status; evidence from an earlier revision is not interchangeable with a later one.

## Identified implementation evidence

Production-source commit `f997615a712484d48df88d6dac0a7c4280dc6ffa` was produced by [run 33927855644](https://github.com/cmdr-chara/speedtest-cli/actions/runs/33927855644), after verifying the complete implementation patch's SHA-256 and the lockfile's SHA-256. That run's event SHA is the preceding transfer commit; the tested source is the applied patch, then committed as `f997615`. Temporary transfer/toolchain workflows are removed from the final branch tree. They do not become part of normal project operation.

On that source, Linux passed formatting, Clippy with warnings denied, all 69 Rust tests (59 library, 8 executable contracts, 2 serialization), the doctest phase (0 doctests), the Python loopback/PTY smoke suite, two packaging test methods with six cases, and crate packaging with `--no-verify`. Build, lint, and tests separately validated the executable; `--no-verify` packaging is not represented as a second packaged-crate build. The final packaging follow-up explicitly sets POSIX executable mode in tar archives regardless of the build host and is covered by the same packaging test.

Local Linux also passed all 69 Rust tests, Clippy, formatting, packaging, and smoke checks. The isolated local toolchain did not include rustdoc; the remote full test invocation resolved that local evidence gap. No subagent review is claimed.

## Network checks actually performed

The same Linux CI run executed both public backends with `--json --no-save --duration 3 --streams 1 --timeout 35`, with an additional subprocess deadline of 45 seconds. Cloudflare and LibreSpeed each exited 0 with parseable results and empty stderr. LibreSpeed selected the GARR Rome endpoint. Artifacts `implementation-verification/verification/public-*` preserve the commands, outputs, and exit codes. These short tests consumed real network traffic; they establish compatibility from that CI runner, not calibrated capacity, vendor agreement, geographic coverage, or accuracy on the user's connection.

Normal CI uses loopback only: custom LibreSpeed success/stall, LAN server/client/refusal/occupied-port, output file equality, JSON/stdout separation, progress/stderr, threshold exit codes, and Unix SIGINT/PTY restoration. Native Windows console restoration, privileged OS DNS changes, and real native ICMP/locale combinations are not certified by those fixtures.

## Dependency audit: warnings remain visible

[Audit run 33926870157](https://github.com/cmdr-chara/speedtest-cli/actions/runs/33926870157) scanned 213 resolved dependencies against RustSec database commit `5a0ebedfe8bdd2e295b171f4162f8c977bcad9a5` (updated 2026-09-02). It reported zero entries under `vulnerabilities`, but three informational warnings. The final audit workflow checks the committed lockfile, preserves JSON, and surfaces informational warnings in the job summary instead of equating exit 0 with a clean security bill. No advisories are ignored.

Both dependency paths are `speedtest-cli -> ratatui 0.29.0`: `lru 0.12.5` and the build-time procedural macro `paste 1.0.15`.

- [RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html): `lru` panic-safety/use-after-free in `pop`, patched in 0.18.2. The described preconditions include caught unwinding and a key destructor that can panic.
- [RUSTSEC-2026-0002](https://rustsec.org/advisories/RUSTSEC-2026-0002.html): `lru::IterMut` pointer soundness, patched in 0.16.3.
- [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436.html): `paste` is unmaintained; no patched version is listed.

Inspection of the resolved Ratatui source found its sole LRU cache in `src/layout/layout.rs`, keyed by `(Rect, Layout)`, using `get_or_insert`, capacity management, and cloning. The reviewed application and cache call sites do not invoke the two affected operations. That narrows observed reachability; it is not a proof that the dependency is sound. The audit warnings remain unresolved. A tested Ratatui/dependency upgrade is follow-up work rather than an unverified major-version substitution or an advisory suppression in this change.
