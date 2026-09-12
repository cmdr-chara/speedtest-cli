# Speedtest agent instructions

## Contracts

- Keep measurement, analysis, persistence, and terminal rendering separate. Shared completion behavior belongs in `session`.
- Opening the cockpit, changing sections, or reading history must remain offline. Network work starts only from an explicit test or diagnostic action.
- Preserve command names, flags, exit codes, units, and canonical JSON/CSV/history schemas. Translate human presentation, not machine identifiers or saved data.
- Cancellation must stop owned workers and restore terminal state. Propagate output, storage, and native-command failures rather than reporting partial work as success.
- Preserve file locking and atomic history/export writes; unsupported locking is an error, not permission to write without coordination.
- Inherit the terminal's palette by default. Never change the user's terminal font, theme, or OS configuration to improve the UI.

## Task-specific guidance

Use [CONTRIBUTING.md](CONTRIBUTING.md) for validation and measurement contracts, [docs/network-cockpit.md](docs/network-cockpit.md) for cockpit work, and [docs/localization.md](docs/localization.md) for catalog changes. Use the committed Rust toolchain and lockfile.

## Verification and completion

For Rust behavior changes, use `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo test --locked --all-features`. After building, select the CLI, cockpit, localization, or packaging Python checks documented in CONTRIBUTING for the affected surface.

Use loopback fixtures and temporary history directories for automated tests. Public WAN probes and privileged DNS changes are separate deliberate operations, not automatic validation. Unix PTY results do not establish Windows-console behavior.

Finish with the changed contracts checked, relevant documentation synchronized, and actual platform/network gaps reported. Do not equate fixture throughput with WAN accuracy or a dependency audit with a security certification.
