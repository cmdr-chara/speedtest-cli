# Network cockpit

## Scope and design

The default interactive entry point is now an offline home dashboard. The existing
measurement engine, result schema, CSV/JSON formats, storage layout, native command
implementations, and immediate plain/JSON command paths are unchanged. `--run`
explicitly opts into the original live-speedometer workflow. Redirected/dumb/no-color
terminals still use the existing automatic plain path.

The default visual system inherits the terminal's foreground, background, and ANSI
palette, with cyan focus, green success, yellow warnings, and red errors. Terminal
color depth is not treated as evidence of a dark or light theme. Graphite, Light,
and Monochrome are explicit session preferences, not terminal-profile changes. A persistent section rail and predictable breadcrumbs
anchor each screen; the dashboard has one prominent start action rather than a grid
of identical menu boxes. Status words, selection markers, units, and better/worse
labels carry meaning without depending on color. Settings include reduced motion.
Fixed palettes select truecolor/indexed rendering from advertised capabilities and
fall back to native colors on basic terminals. Monochrome retains markers, bold
labels, and inverse selection without emitting specific foreground/background colors.
Comfortable layout uses multi-cell digits when complete values fit; Compact and
small areas use exact ordinary values without rounding away data. The workspace
is centered and bounded to 120 columns by 38 rows, keeping table values near their
headers and limiting the gauge size. Body font size remains the terminal's setting.
Action highlights are label-sized; descriptions and blank rows are not selected.

## Ownership

| Surface | Responsibility |
| --- | --- |
| `src/bin/speedtest.rs` | Preserve command dispatch and terminal policy; choose menu versus immediate run |
| `src/session.rs` | Share CLI-derived test options, existing engine construction, and export-before-history completion policy |
| `src/tui/cockpit/state.rs` | Pure navigation stack, selections, modal decisions, settings, task/result states, effects |
| `src/tui/cockpit/mod.rs` | Own the terminal, asynchronous work, events, physics and rendering schedules |
| `src/tui/cockpit/view.rs`, `theme.rs` | Compose widgets, accessible states, responsive geometry, semantic colors |
| `src/tui/cockpit/services.rs` | Adapt existing local history/analysis and read-only CLI reports |
| `src/tui/speedometer/` | Existing gauge and physics; shell-supplied palette and optional large readout |
| `src/i18n/` | Embedded catalogs, locale selection, presentation-only formatting and safe built-in narrative translation |
| `src/tui/numerals.rs` | Three-/five-row digit rendering with complete-value width validation and a caller fallback |

There is no new network measurement implementation. UI configuration maps into
`EngineConfig` through `TestOptions`; live data arrives through `EngineEvent` and
final results use the existing `TestResult`. The canonical result returned by the
engine, not a potentially duplicated progress event, triggers completion once.
History summaries call `history::summarize`; comparison calls `compare::compare`.

## Lifecycle and navigation

The root Home page is never popped. Opening a child pushes a page; Back restores the
parent's selection and scroll. Switching siblings replaces the section branch
rather than filling history with tab changes. Starting a test opens Live; success
replaces it with Results, failure with Retry. Starting another test from Results
returns to a fresh configuration branch instead of growing an unbounded stack.

Pure effects distinguish navigation from starting work. Constructors, render code,
help, settings, and section navigation perform no network I/O. Startup and reload
read local history off the UI thread. A network tool has its own explicit Start
screen; simply opening DNS Tools or Diagnostics does not launch a process.

One owned future holds the current test or diagnostic. Dropping it cancels the
existing engine's owned workers or the diagnostic child (`kill_on_drop`). A new
event receiver is created for each test, so old events cannot complete a new run.
The engine remains authoritative for timeouts, throughput, latency and protocol
handling; the existing overall deadline additionally bounds each started operation.
Menu dwell time is never part of the measurement deadline.

Local history reads and result writes use `spawn_blocking`, not the terminal loop.
The save future is not dropped on keyboard cancellation; the result is retained if
writing fails. External SIGINT restores the terminal through the existing runtime
and guard; already-running blocking file work is not aborted halfway through.
Export/persistence keep their existing ordering and failure semantics; no transaction
or concurrency guarantees are added to storage by this UI.

Input polling drains bounded batches every 16 ms. Live physics keep the existing
240 Hz schedule independently of the requested render cap; idle screens redraw only
when dirty. Reduced motion snaps live values instead of interpolating and uses
static activity labels. Resize invalidates rendering but does not reset navigation.
Below 80×24, hidden start/navigation controls are disabled while back, help, quit and
cancellation remain available. Long reports clamp their scroll offsets to the new
wrapped height after resizing.

Ratatui 0.29's `unstable-rendered-line-info` feature provides the exact wrapped-line
count for that clamp. This is an explicit feature opt-in on the existing version,
not a dependency upgrade. Recheck `Paragraph::line_count` behavior when upgrading
Ratatui; a regression test covers scroll position after resize.

## Diagnostics boundary

The CLI already owns human report formatting and several synchronous,
platform-specific native tool integrations. Rather than duplicate those implementations
or let a native call block the terminal thread, the cockpit runs a **fixed allowlist
of existing read-only subcommands** of `current_exe()` through `tokio::process::Command`.
There is no shell interpolation. Stdin is closed; stdout/stderr are captured
concurrently, limited to 256 KiB each, sanitized, and rendered in a scrollable panel.
Forced plain/no-color flags prevent nested TUIs. Reports remain the same reports as
the corresponding CLI commands; errors include stderr and allow retry.

The parent diagnostic command is killed on cancellation, timeout, or output overflow.
Native helper descendants retain the lifecycle of their existing CLI implementation;
this does not introduce a cross-platform process-tree supervisor. Native Wi-Fi,
route and DNS inspection still depend on platform tools, permissions and localization.
These are read-only paths: DNS set/reset/rollback/optimize and server exposure remain
explicit CLI operations outside the menu.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --bin speedtest
python .github/scripts/cli_smoke.py
python .github/scripts/cockpit_smoke.py
python .github/scripts/localization_smoke.py
python .github/scripts/test_package_release.py
```

Rust tests cover offline construction and effects, navigation stack limits,
selection restoration, Enter/repeat/release handling, help/confirmation defaults,
stale completion, cancellation, retry, settings limits, save failure, all screen
states, terminal sizes down to 1×1, history table visibility, and wrapped scrolling.
Existing canonical JSON/CLI contracts remain in the suite; session policy tests
cover explicit CSV export even with automatic history disabled.

The Unix PTY suite launches the real executable, checks 80×24 output and alternate
screen/termios restoration, counts local fixture requests to verify no hidden test
starts, runs a real loopback measurement, reads its JSON export, navigates saved
history/statistics/comparison, and tests retry/cancellation. A local rejecting HTTPS
proxy verifies that cancelling the actual Stability child stops its probe traffic.
`--plain` on a real terminal and `--run` cancellation remain covered. Windows runs
Rust render/navigation and existing executable contract tests, but the Unix PTY
script explicitly skips; manual real-console evidence is still required there.

These tests do not calibrate WAN throughput or contact public speed-test providers.
No public network test, privileged DNS write, release publication, or deployment is
required to verify this presentation-layer change. Existing storage collision and
concurrent-writer limitations remain unchanged.

### Readability regression coverage

Appearance tests cover native/reset colors on every screen, monochrome rendering,
fixed-palette text contrast, label-sized home focus, responsive workspace bounds,
large/compact metrics, aligned history columns, settings navigation/scrolling, and
long save errors. Appearance changes never construct an engine or alter test/export
options. The real-PTY suite additionally switches palettes/layouts and resizes a
maximized terminal back to 80×24.

The PTY harness launches its child in a fresh session. Without isolation, a shell
with a controlling terminal makes Crossterm query that terminal through `/dev/tty`
instead of the fixture: an ioctl resize of the fixture then leaves the observed
screen blank. This was reproduced with both the original cockpit and the revised
binary, and resolved by changing only the harness session ownership.

### Localization and sizing verification

Idle header badges have been removed; running tasks still show a status. The `z`
text-size guide is available even below the minimum size and scrolls independently
from report pages. Opening a confirmation resets modal scrolling. Language and
Text size help stay reachable at 80×24 through a stateful settings table. Explanatory
translated paragraphs wrap, the selected tab stays visible, and CJK text is measured
in terminal cells rather than UTF-8 bytes or codepoint counts.

Locale state belongs to the cockpit, not to the stored result or engine. The CLI
fixes its locale at startup and passes it to read-only diagnostic subprocesses.
Narrative localization recognizes a closed set of built-in English templates without
modifying numeric/direction evidence. It never recomputes scores or translates
unknown user/OS content. The canonical model and serialization are unchanged.

Tests cover eight-language key and placeholder parity; CLI grammar/metadata;
locale precedence; byte-identical JSON, errors and exit codes across languages;
all cockpit screens at 80×24 and larger; CJK menu widths; modal scrolling; and the
results summary/details composition with five-row digits. The localized Unix PTY suite opens
all eight languages, changes language live, resizes, runs the existing DNS catalog
child, exercises the immediate gauge in Italian, and finishes a loopback test with
an unchanged JSON export. Screenshots generated from Ratatui buffers are synthetic
render fixtures, not observations of WAN speed or physical Windows consoles.

See [localization and sizing](localization.md) for scope, contributor checks,
terminal documentation and the distinction between metric glyphs and font zoom.
