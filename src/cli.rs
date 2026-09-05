use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Csv,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum InternetBackendArg {
    Cloudflare,
    Librespeed,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "speedtest")]
#[command(version, about = "Measure network throughput, latency, and quality")]
#[command(
    after_help = "Examples:\n  speedtest                         # open the network cockpit\n  speedtest --run                   # start immediately\n  speedtest --plain --no-save\n  speedtest --json > result.json\n  speedtest check result.json --min-download 100 --max-latency 30\n  speedtest lan 192.168.1.50:9876 --json\n\nThroughput uses decimal Mbps; latency uses milliseconds.\nRun `speedtest <COMMAND> --help` for command-specific options."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Interface language: auto, en, it, es, fr, de, pt, zh-CN, ja. Machine data stays unchanged.
    #[arg(long, global = true, value_name = "CODE", default_value = "auto", value_parser = crate::i18n::Language::parse)]
    pub language: crate::i18n::Language,

    /// Color policy. Never also selects the accessible, non-animated interface.
    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    /// Phase progress on stderr. Auto enables it only on a terminal.
    #[arg(long, global = true, value_enum, default_value_t = ProgressMode::Auto)]
    pub progress: ProgressMode,

    /// Overall Internet measurement deadline in seconds, including server selection.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=600))]
    pub timeout: u64,

    /// Internet measurement backend.
    #[arg(long, value_enum, default_value_t = InternetBackendArg::Cloudflare)]
    pub backend: InternetBackendArg,

    /// Custom LibreSpeed base URL; requires --backend librespeed. Standard PHP paths are assumed.
    #[arg(long, value_name = "URL")]
    pub librespeed_server: Option<String>,

    /// Number of concurrent transfer streams.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub streams: u8,

    /// Duration of each throughput phase in seconds.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(3..=30))]
    pub duration: u64,

    /// Maximum interactive TUI render rate. The speedometer physics always run at 240 Hz.
    #[arg(long, default_value_t = 240, value_parser = clap::value_parser!(u16).range(30..=240))]
    pub fps: u16,

    /// Start a speed test immediately, bypassing the home menu.
    #[arg(long)]
    pub run: bool,

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
    #[arg(long, value_enum, default_value_t = OutputFormat::Json, requires = "output")]
    pub format: OutputFormat,

    /// Do not persist automatic history/results.
    #[arg(long)]
    pub no_save: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Check a saved JSON result against explicit thresholds, without network traffic.
    Check(CheckArgs),
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
    /// Diagnose routing, DNS, IP connectivity, HTTPS, Wi-Fi, and optionally throughput.
    Doctor(DoctorArgs),
    /// Measure real ICMP echo response loss and RTT distribution.
    Loss(LossArgs),
    /// Inspect the active Wi-Fi link using native platform tooling.
    Wifi(WifiArgs),
    /// Cross-check Cloudflare and LibreSpeed measurements.
    Verify(VerifyArgs),
    /// Run the built-in self-hosted LAN speed-test server.
    Serve(ServeArgs),
    /// Measure a self-hosted LAN endpoint.
    Lan(LanArgs),
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DnsProtocolArg {
    Udp,
    Doh,
}

#[derive(Debug, Clone, Args)]
pub struct DnsBenchmarkArgs {
    /// Resolver league to benchmark.
    #[arg(long, value_enum, default_value_t = DnsBenchmarkProfileArg::Fastest)]
    pub profile: DnsBenchmarkProfileArg,

    /// DNS transport to benchmark.
    #[arg(long, value_enum, default_value_t = DnsProtocolArg::Udp)]
    pub protocol: DnsProtocolArg,

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
    #[arg(requires = "after")]
    pub before: Option<PathBuf>,

    /// New JSON result. Supply together with BEFORE.
    #[arg(requires = "before")]
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

#[derive(Debug, Clone, Args)]
pub struct LossArgs {
    /// ICMP echo target. Defaults to Cloudflare's public resolver address.
    #[arg(long, default_value = "1.1.1.1", value_parser = crate::loss::validate_target)]
    pub target: String,

    /// Number of ICMP echo requests.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(3..=200))]
    pub count: u16,

    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct WifiArgs {
    /// Wireless interface/device to inspect. Defaults to platform auto-detection.
    #[arg(long)]
    pub interface: Option<String>,

    /// Print the result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct VerifyArgs {
    /// Seconds for each download/upload phase on each backend.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(3..=15))]
    pub duration: u64,

    /// Concurrent streams per backend.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub streams: u8,

    /// Optional custom LibreSpeed base URL.
    #[arg(long, value_name = "URL")]
    pub librespeed_server: Option<String>,

    /// Print the report as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    /// Address for the self-hosted LAN endpoint.
    #[arg(long, default_value = "127.0.0.1:9876")]
    pub bind: SocketAddr,
}

#[derive(Debug, Clone, Args)]
pub struct LanArgs {
    /// Self-hosted endpoint, for example 192.168.1.50:9876.
    pub server: SocketAddr,

    /// Seconds for each LAN throughput phase.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(2..=30))]
    pub duration: u64,

    /// Concurrent LAN transfer streams.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub streams: u8,

    /// Print the canonical result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn allows_tui(self) -> bool {
        if self == Self::Never || std::env::var("TERM").is_ok_and(|value| value == "dumb") {
            return false;
        }
        if self == Self::Always {
            return true;
        }
        !std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
            && !std::env::var("CLICOLOR").is_ok_and(|value| value == "0")
    }
}

impl ProgressMode {
    pub fn enabled_for(self, json: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => !json && std::io::stderr().is_terminal(),
        }
    }
}

impl Cli {
    pub fn json_requested(&self) -> bool {
        match &self.command {
            None => self.json,
            Some(Command::Check(a)) => a.json,
            Some(Command::Stability(a)) => a.json,
            Some(Command::History(a)) => a.json,
            Some(Command::Stats(a)) => a.json,
            Some(Command::Compare(a)) => a.json,
            Some(Command::Doctor(a)) => a.json,
            Some(Command::Loss(a)) => a.json,
            Some(Command::Wifi(a)) => a.json,
            Some(Command::Verify(a)) => a.json,
            Some(Command::Lan(a)) => a.json,
            Some(Command::Serve(_)) => false,
            Some(Command::Dns(a)) => match &a.command {
                DnsCommand::List(a) => a.json,
                DnsCommand::Show(a) => a.json,
                DnsCommand::Test(a) => a.json,
                DnsCommand::Benchmark(a) => a.json,
                _ => false,
            },
        }
    }
}

#[derive(Debug, Clone, Args)]
#[command(group(clap::ArgGroup::new("thresholds").required(true).multiple(true)
    .args(["min_download", "min_upload", "max_latency", "max_jitter", "max_loaded_latency", "max_age"])))]
pub struct CheckArgs {
    /// Canonical JSON file, or - to read one result from stdin (maximum 4 MiB).
    pub result: String,
    /// Minimum download throughput in decimal Mbps.
    #[arg(long, value_parser = parse_threshold)]
    pub min_download: Option<f64>,
    /// Minimum upload throughput in decimal Mbps.
    #[arg(long, value_parser = parse_threshold)]
    pub min_upload: Option<f64>,
    /// Maximum idle HTTP latency in ms.
    #[arg(long, value_parser = parse_threshold)]
    pub max_latency: Option<f64>,
    /// Maximum idle jitter in ms.
    #[arg(long, value_parser = parse_threshold)]
    pub max_jitter: Option<f64>,
    /// Maximum latency in BOTH loaded phases; missing samples fail the check.
    #[arg(long, value_parser = parse_threshold)]
    pub max_loaded_latency: Option<f64>,
    /// Maximum result age in seconds; future timestamps fail the check.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_age: Option<u64>,
    /// Emit a versioned JSON report. Exit 0 passes, 3 fails, 1 means invalid input.
    #[arg(long)]
    pub json: bool,
}

fn parse_threshold(value: &str) -> Result<f64, String> {
    let number: f64 = value
        .parse()
        .map_err(|_| "expected a finite, non-negative number")?;
    if !number.is_finite() || number < 0.0 {
        return Err("expected a finite, non-negative number".to_string());
    }
    Ok(number)
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
