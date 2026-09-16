# Transfer performance and deadline contracts

This change improves the existing Cloudflare and LibreSpeed engines without
changing CLI flags, canonical events, units, result schemas, or saved history.
Constructors and cockpit navigation still do not start network requests.

## Bounded server selection

Automatic LibreSpeed selection probes the existing built-in registry concurrently.
All candidates share a five-second selection budget, including connection setup,
response bodies, and the three latency probes per candidate. It chooses the lowest
median among candidates observed to complete within that budget. Equal medians
retain registry order rather than depending on task completion order.

Previously selection could wait on each candidate's 25-second request timeout,
including repeated slow requests, even when healthy candidates were already ready.
At the shared cutoff, outstanding probes are aborted and joined before the chosen
server starts its measurement. Cancelling selection also drops its owned JoinSet.
If no candidate qualifies, the existing unreachable-server error is returned.
A very slow connection can therefore fail automatic selection earlier. An explicit
custom server continues to bypass discovery, and the command's outer timeout is
unchanged. The budget is a cooperative async deadline, not a real-time guarantee
under OS or executor starvation.

## Lower upload overhead

Upload payload chunks now borrow a single immutable 64 KiB static buffer. The old
path allocated and zeroed a fresh 64 KiB Vec for every upload request, including
small adaptive requests. The new payload path avoids those allocations and zero-initialization,
as well as reference-counted payload slices. HTTP request objects and stream wrappers
still have their normal overhead: this is not a claim that an entire request is
allocation-free.

The byte contract is unchanged. The submitted counter advances only when chunks
are polled. The public upload total advances only after the complete successful
response is observed before the cutoff and the entire request body was submitted.
Rejected, incomplete, early-response, and deadline-cancelled uploads are not
reported as successful goodput.

## Accurate phase cutoffs

Both download engines use the same streaming counter, with one pinned deadline
timer per response rather than constructing a timer for every body chunk. The
cutoff is checked before polling and again before accepting the result of a poll.
This matters when a ready chunk takes a long time to poll: a timer race alone does
not exclude work completed after the deadline. Partial download bytes received
before the cutoff remain valid. Earlier body errors still propagate, and empty
completed downloads still fail.

Request headers, upload acknowledgements, and loaded-latency samples use the same
before/after deadline rule. Loaded probes have a three-second deadline each, bounded
by the remaining phase time. A stalled LibreSpeed loaded probe can no longer
occupy the entire remainder of a long phase without permitting another sample.

Live samplers stop at the actual phase deadline instead of waiting for their next
200 ms display tick. The smoothing windows and final result denominators are
unchanged. LibreSpeed endpoint URLs are resolved once per worker or probe batch,
not repeatedly inside transfer loops. Both engines preserve the gaps between idle
latency samples but no longer sleep for 75 ms after the final sample.

## Failure cleanup and CI evidence

A failed or panicking transfer worker triggers an explicit abort-and-join of its
siblings before failure is returned. The original error is preserved. Cancelling
the outer operation retains the existing owned-JoinSet cancellation behavior.

CI artifact names include the matrix runner, preventing the Apple Silicon and
Intel macOS jobs from attempting to create the same artifact. All four platform
lanes and all existing test, smoke, packaging, and lint checks remain enabled.
No dependency or lockfile changes are required.

## Regression checks

The additional Rust tests use in-memory streams, owned futures, and loopback only.
They cover shared upload storage, partial polling and cancellation, download
accounting, stalled and failed bodies, expired work, post-poll deadline overruns,
selection ranking and ties, failed candidates, selection cleanup, a real stalled
HTTP response, expired samplers, and transfer-worker cleanup on errors and panics.
The original upload, CLI, output, serialization, history, and terminal suites
remain in place.

Run the repository's normal verification commands:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --bin speedtest
python .github/scripts/cli_smoke.py
python .github/scripts/cockpit_smoke.py
python .github/scripts/localization_smoke.py
python .github/scripts/test_package_release.py
```

These changes have structural benefits and executable regression coverage, not a
claimed percentage increase in WAN throughput. Loopback results are not evidence
of public endpoint availability or Internet measurement accuracy. Public-network
comparisons and real Windows-console restoration require separate deliberate
validation, as described in [CONTRIBUTING.md](../CONTRIBUTING.md).
