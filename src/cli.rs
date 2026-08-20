use std::{net::IpAddr, path::PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Csv,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "speedtest")]
#[command(version, about = "A fast, polished terminal network quality analyzer")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Number of concurrent transfer streams.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub streams: u8,

    /// Duration of each throughput phase in seconds.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(3..=30))]
    pub duration: u64,

    /// Maximum interactive TUI render rate. The speedometer physics always run at 240 Hz.
    #[arg(long, default_value_t = 240, value_parser = clap::value_parser!(u16).range(30..=240))]
    pub fps: u16,

    /// Disable the interactive terminal UI.
    #[arg(long)]
    pub plain: bool,

    /// Print the canonical result as JSON.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,

    /// Also write the completed result to this path.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Format used by --output.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,

    /// Do not persist automatic history/results.
    #[arg(long)]
    pub no_save: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Continuously probe latency to expose spikes and short disruptions.
    Stability(StabilityArgs),
    /// Show recent saved speed-test runs.
    History(HistoryArgs),
    /// Summarize saved runs and flag unusual recent performance.
    Stats(StatsArgs),
    /// Inspect, benchmark, optimize, and configure DNS resolvers.
    Dns(DnsArgs),
    /// Compare two saved results, or the two most recent history entries.
    Compare(CompareArgs),
    /// Diagnose routing, DNS, IP connectivity, HTTPS, and optionally throughput.
    Doctor(DoctorArgs),
}

#[derive(Debug, Clone, Args)]
pub struct StabilityArgs {
    /// Total probe duration. Supports s, m, or h suffixes (for example 90s or 5m).
    #[arg(long, default_value = "1m", value_parser = parse_stability_duration)]
    pub duration: u64,

    /// Time between probes. Supports ms or s suffixes (for example 750ms or 1s).
    #[arg(long = "interval", default_value = "1s", value_parser = parse_probe_interval)]
    pub interval_ms: u64,

    /// Maximum TUI render rate.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u16).range(10..=240))]
    pub fps: u16,

    /// Disable the stability TUI.
    #[arg(long)]
    pub plain: bool,

    /// Print the completed stability result as JSON.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,

    /// Write the completed stability result to this JSON file.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Do not persist the stability run.
    #[arg(long)]
    pub no_save: bool,
}

#[derive(Debug, Clone, Args)]
pub struct HistoryArgs {
    /// Only include results from the last N days.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3650))]
    pub days: u64,

    /// Maximum number of runs displayed in the table.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=200))]
    pub limit: u16,

    /// Print matching history as a JSON array.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct StatsArgs {
    /// Analyze results from the last N days.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3650))]
    pub days: u64,

    /// Print the history summary as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsArgs {
    #[command(subcommand)]
    pub command: DnsCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DnsCommand {
    /// List built-in resolver profiles and protocol capabilities.
    List(DnsListArgs),
    /// Show DNS configuration for the active or selected interface.
    Show(DnsShowArgs),
    /// Test the current resolver or one or more explicit DNS server IPs.
    Test(DnsTestArgs),
    /// Race DNS providers and rank latency, tail latency, reliability, and stability.
    #[command(alias = "speedtest")]
    Benchmark(DnsBenchmarkArgs),
    /// Configure a specific built-in resolver profile.
    Set(DnsSetArgs),
    /// Benchmark a resolver league and configure the best eligible provider.
    Optimize(DnsOptimizeArgs),
    /// Return the selected interface to automatic/DHCP-managed DNS.
    Reset(DnsResetArgs),
    /// Restore the exact DNS snapshot saved before the most recent change when possible.
    Rollback(DnsRollbackArgs),
}

#[derive(Debug, Clone, Args)]
pub struct DnsListArgs {
    /// Print provider metadata as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsShowArgs {
    /// Interface alias, network service, or device. Defaults to the active route.
    #[arg(long)]
    pub interface: Option<String>,

    /// Print configuration as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsTestArgs {
    /// DNS server IP to test. May be supplied multiple times; defaults to current system DNS.
    #[arg(long = "resolver", value_name = "IP")]
    pub resolvers: Vec<IpAddr>,

    /// Number of DNS queries in the test.
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u16).range(3..=100))]
    pub queries: u16,

    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DnsBenchmarkProfileArg {
    Fastest,
    Privacy,
    Security,
    Adblock,
    Family,
    All,
}

#[derive(Debug, Clone, Args)]
pub struct DnsBenchmarkArgs {
    /// Resolver league to benchmark.
    #[arg(long, value_enum, default_value_t = DnsBenchmarkProfileArg::Fastest)]
    pub profile: DnsBenchmarkProfileArg,

    /// Queries sent to each resolver profile.
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u16).range(3..=100))]
    pub queries: u16,

    /// Print the full benchmark as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsSetArgs {
    /// Built-in resolver profile ID, for example cloudflare, quad9, adguard, or controld-ads.
    pub provider: String,

    /// Interface alias, network service, or device. Defaults to the active route.
    #[arg(long)]
    pub interface: Option<String>,

    /// Apply without the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,

    /// Show the proposed change without modifying DNS.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsOptimizeArgs {
    /// Resolver league to optimize for.
    #[arg(long, value_enum, default_value_t = DnsBenchmarkProfileArg::Fastest)]
    pub profile: DnsBenchmarkProfileArg,

    /// Queries sent to each resolver profile before selecting a winner.
    #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u16).range(3..=100))]
    pub queries: u16,

    /// Interface alias, network service, or device. Defaults to the active route.
    #[arg(long)]
    pub interface: Option<String>,

    /// Apply without the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,

    /// Benchmark and recommend only; do not modify DNS.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsResetArgs {
    /// Interface alias, network service, or device. Defaults to the active route.
    #[arg(long)]
    pub interface: Option<String>,

    /// Reset without the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,

    /// Show what would be reset without modifying DNS.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DnsRollbackArgs {
    /// Roll back without the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,

    /// Show the saved snapshot without modifying DNS.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CompareArgs {
    /// Baseline JSON result. If omitted with AFTER, the two newest saved runs are compared.
    pub before: Option<PathBuf>,

    /// New JSON result. Supply together with BEFORE.
    pub after: Option<PathBuf>,

    /// Print the comparison as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {
    /// Also run the full throughput/bufferbloat test after lightweight diagnostics.
    #[arg(long)]
    pub full: bool,

    /// Interface alias, network service, or device. Defaults to the active route.
    #[arg(long)]
    pub interface: Option<String>,

    /// Print the complete diagnostic report as JSON.
    #[arg(long)]
    pub json: bool,
}

fn parse_stability_duration(value: &str) -> Result<u64, String> {
    let seconds = parse_time(value, 1_000)? / 1_000;
    if !(10..=86_400).contains(&seconds) {
        return Err("stability duration must be between 10s and 24h".to_string());
    }
    Ok(seconds)
}

fn parse_probe_interval(value: &str) -> Result<u64, String> {
    let milliseconds = parse_time(value, 1)?;
    if !(500..=10_000).contains(&milliseconds) {
        return Err("probe interval must be between 500ms and 10s".to_string());
    }
    Ok(milliseconds)
}

fn parse_time(value: &str, bare_multiplier: u64) -> Result<u64, String> {
    let value = value.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1_u64)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1_000)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60_000)
    } else if let Some(number) = value.strip_suffix('h') {
        (number, 3_600_000)
    } else {
        (value.as_str(), bare_multiplier)
    };

    number
        .trim()
        .parse::<u64>()
        .map(|number| number.saturating_mul(multiplier))
        .map_err(|_| format!("invalid duration: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_stability_duration() {
        assert_eq!(parse_stability_duration("90s").unwrap(), 90);
        assert_eq!(parse_stability_duration("5m").unwrap(), 300);
    }

    #[test]
    fn parses_probe_interval() {
        assert_eq!(parse_probe_interval("750ms").unwrap(), 750);
        assert_eq!(parse_probe_interval("2s").unwrap(), 2_000);
    }
}
