# Localization and readable terminals

## User contract

Use `speedtest --language it`, or select **Settings → Language**. The global flag
also works after a subcommand. The selected CLI locale is fixed at startup; the
cockpit can change its locale immediately without changing the environment,
measurement profile, network activity, or canonical results. Appearance and
language choices in Settings are session-only.

| Code | Language |
| --- | --- |
| `en` | English |
| `it` | Italiano |
| `es` | Español |
| `fr` | Français |
| `de` | Deutsch |
| `pt` | Português |
| `zh-CN` | 简体中文 (Simplified Chinese) |
| `ja` | 日本語 |

`auto` resolves an explicit `SPEEDTEST_LANGUAGE` override first, then POSIX locale
variables in `LC_ALL`, `LC_MESSAGES`, `LANG` order, ignoring empty variables. `C` and
`POSIX` mean English. An unknown highest-priority system locale falls back to
English rather than consulting lower-priority variables. Traditional Chinese
(`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant`) is not claimed as a supported translation.
On Windows, use the flag or environment override when no POSIX locale is set.

## Translation boundaries

Embedded catalogs cover owned interface copy: navigation, state messages,
configuration, help, CLI descriptions, human reports, built-in quality findings,
and immediate speedometer/stability labels. They are not an online translation
service. No locale package, font, or network download is required to choose a
language. A terminal with missing CJK glyphs still needs an appropriate installed
font or fallback font; the application cannot supply that automatically.

Command names, flags, key bindings, enum values, units, timestamps, result schema,
JSON records (including structured errors), CSV column names, stored results and
exit codes remain invariant. Decimal measurement formatting is retained. Generic
Clap annotations such as `default`/`possible values`, parser-generated errors, and
raw low-level/OS/provider error chains are not rewritten. Unknown historical or
custom finding text is displayed unchanged. This prevents corrupting paths,
identifiers, third-party messages or numeric evidence while translating UI text.

The cockpit runs the existing read-only diagnostic CLI commands with an explicit
locale. Their captured reports are snapshots; rerun after switching language to
regenerate a report. Native platform tools may return text in their own locale.
Privileged DNS workflows keep their existing confirmation and rollback logic;
`y`/`N` confirmations are not changed into locale-specific commands.

The catalogs have mechanical completeness, placeholder, control-character and
layout checks. They have not been independently reviewed by native speakers of
every supported language; terminology improvements should preserve semantics and
be accompanied by updated render/contract tests.

## Text size on Arch / Omarchy

There are two different controls:

- **Comfortable/Compact** changes spacing and whether metrics use five-row,
  three-row or ordinary digits. It does not change the physical size of a cell.
- **Terminal font size/zoom** enlarges every character, including explanatory
  paragraphs, navigation and help. This is a terminal preference, not an app flag.

Press `z` anywhere in the cockpit for the scrollable sizing guide. Omarchy's manual
identifies Alacritty as its default and also supports Ghostty, Foot and Kitty.
With Alacritty, find the existing `[font]` section in
`~/.config/alacritty/alacritty.toml` and adjust its `size`, for example:

```toml
[font]
size = 14.0
```

This is an example of the existing section, **not a block to append blindly**.
Keep imports, font family and other settings intact; duplicate TOML sections are
invalid. Use your terminal's own Zoom In action for temporary changes. Ghostty
uses `font-size = 14` in its configuration; macOS Terminal and Windows Terminal
provide profile font settings. No shell command or application code here edits a
terminal, font, monitor, desktop, locale or user configuration.

After enlarging text, preserve at least 80 columns by 24 rows. The cockpit reflows
on resize, keeps the active tab and selection visible, and disables hidden Start
controls below its minimum size. More window area does not itself make text larger.

Primary documentation checked 2026-09-05:

- [Omarchy: Terminal](https://learn.omacom.io/2/the-omarchy-manual/106/terminal)
- [Omarchy: Fonts](https://learn.omacom.io/2/the-omarchy-manual/94/fonts)
- [Alacritty configuration: font table](https://alacritty.org/config-alacritty.html)
- [Ghostty configuration reference: font-size](https://ghostty.org/docs/config/reference)

## Maintainer checks

The English source phrase, normalized by trimming and ASCII case, is the lookup
key. Whitespace and uppercase heading treatment are handled at presentation time.
Each locale must have exactly the same keys and numbered placeholders as English.
Substitutions happen once: braces in an inserted filename are data, not another
format string. Display columns use terminal cell width, including CJK characters.

Dynamic analysis text uses a closed set of validated built-in templates in
`src/i18n/narrative.rs`; score computation and canonical serialization are untouched.
Do not extend this into unconstrained string replacement over saved/user data.

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

The PTY scripts run on Unix and explicitly skip on Windows. Linux/macOS/Windows
all run Rust rendering/navigation and executable contracts in CI. No WAN speed
test, privileged network write, or physical Windows-console validation is implied
by a passing unit test. Existing storage collision/concurrent-writer limitations
and dependency advisories remain outside this presentation-focused change.
