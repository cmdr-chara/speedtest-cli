# Contributing

## Local checks

Run the same checks used by CI before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

For a manual smoke test of the network engine without the TUI:

```bash
cargo run --release -- --plain --no-save
```

Keep measurement logic independent from rendering and persistence. New measurement backends should emit the canonical engine events and return `TestResult` rather than writing directly to the terminal or filesystem.
