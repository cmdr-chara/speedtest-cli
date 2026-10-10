//! Adapters only: analysis and persistence stay in their existing domain modules.
use std::{process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use tokio::{io::AsyncReadExt, process::Command};

use crate::{
    compare, history, insights::InsightsReport, model::TestResult, output, runtime,
    session::TestOptions, storage,
};

pub(super) const HISTORY_DAYS: u64 = 30;
const REPORT_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub(super) struct Archive {
    pub results: Vec<TestResult>,
    pub summary: Option<history::HistorySummary>,
    pub comparison: Option<ComparedRuns>,
    pub insights: InsightsReport,
}

/// A comparison keeps the source evidence alongside the existing domain analysis.
/// These session snapshots remain valid if local history changes or cannot reload.
#[derive(Debug, Clone)]
pub(super) struct ComparedRuns {
    pub before: TestResult,
    pub after: TestResult,
    pub metrics: compare::CompareResult,
}

impl ComparedRuns {
    pub fn new(before: &TestResult, after: &TestResult) -> Self {
        Self {
            before: before.clone(),
            after: after.clone(),
            metrics: compare::compare(before, after),
        }
    }
}

/// Timestamp collisions are valid. Match the complete canonical record, and keep
/// the occurrence when an archive contains identical duplicate records.
#[derive(Debug, Clone)]
pub(super) struct HistoryAnchor {
    timestamp: chrono::DateTime<chrono::Utc>,
    record: String,
    occurrence: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopeFilter {
    #[default]
    All,
    Internet,
    Lan,
}

impl ScopeFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Internet,
            Self::Internet => Self::Lan,
            Self::Lan => Self::All,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All runs",
            Self::Internet => "Internet",
            Self::Lan => "LAN",
        }
    }
    fn matches(self, result: &TestResult) -> bool {
        match self {
            Self::All => true,
            Self::Internet => !history::is_lan(result),
            Self::Lan => history::is_lan(result),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistorySort {
    #[default]
    Newest,
    Oldest,
    Metric(crate::insights::HistoryMetric),
}

impl HistorySort {
    pub fn next(self) -> Self {
        use crate::insights::HistoryMetric;
        match self {
            Self::Newest => Self::Oldest,
            Self::Oldest => Self::Metric(HistoryMetric::Download),
            Self::Metric(HistoryMetric::Quality) => Self::Newest,
            Self::Metric(metric) => Self::Metric(metric.next()),
        }
    }
    pub fn label(self) -> &'static str {
        use crate::insights::HistoryMetric;
        match self {
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
            Self::Metric(HistoryMetric::Download) => "Fastest download",
            Self::Metric(HistoryMetric::Upload) => "Fastest upload",
            Self::Metric(HistoryMetric::IdleLatency) => "Lowest latency",
            Self::Metric(HistoryMetric::Jitter) => "Lowest jitter",
            Self::Metric(HistoryMetric::LoadedLatency) => "Lowest loaded latency",
            Self::Metric(HistoryMetric::Quality) => "Highest quality",
        }
    }
}

/// A projection only: raw results and their duplicate-aware anchors never move.
#[derive(Debug, Default)]
pub(super) struct HistoryView {
    pub query: String,
    pub scope: ScopeFilter,
    pub sort: HistorySort,
    pub path: Option<(String, String)>,
}

impl HistoryView {
    pub fn indices(&self, archive: &Archive) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        let mut indices: Vec<_> = (0..archive.results.len())
            .filter(|index| {
                let result = archive.newest(*index).expect("archive index");
                self.scope.matches(result)
                    && self.path.as_ref().is_none_or(|(backend, host)| {
                        result.backend.eq_ignore_ascii_case(backend)
                            && history::server_identity(&result.server.host)
                                == history::server_identity(host)
                    })
                    && (query.is_empty()
                        || format!(
                            "{} {} {} {}",
                            result.backend,
                            result.server.host,
                            result.server.name,
                            result.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                        )
                        .to_lowercase()
                        .contains(&query))
            })
            .collect();
        indices.sort_by(|left, right| {
            let a = archive.newest(*left).expect("archive index");
            let b = archive.newest(*right).expect("archive index");
            let order = match self.sort {
                HistorySort::Newest => b.timestamp.cmp(&a.timestamp),
                HistorySort::Oldest => a.timestamp.cmp(&b.timestamp),
                HistorySort::Metric(metric) => match (metric.value(a), metric.value(b)) {
                    (Some(a), Some(b)) if metric.higher_is_better() => b.total_cmp(&a),
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
            };
            order.then_with(|| left.cmp(right))
        });
        indices
    }
}

impl HistoryAnchor {
    fn matches(&self, result: &TestResult) -> bool {
        result.timestamp == self.timestamp
            && serde_json::to_string(result).is_ok_and(|record| record == self.record)
    }
}

pub(super) fn same_result(left: &TestResult, right: &TestResult) -> bool {
    left.timestamp == right.timestamp
        && matches!(
            (serde_json::to_string(left), serde_json::to_string(right)),
            (Ok(left), Ok(right)) if left == right
        )
}

impl Archive {
    pub fn load() -> Result<Self> {
        Ok(Self::from_results(storage::load_history_since(
            HISTORY_DAYS,
        )?))
    }

    pub fn from_results(results: Vec<TestResult>) -> Self {
        let summary = history::summarize(&results, HISTORY_DAYS);
        let comparison = history::latest_comparable_pair(&results)
            .map(|(before, after)| ComparedRuns::new(&before, &after));
        let insights = crate::insights::analyze(&results, HISTORY_DAYS);
        Self {
            results,
            summary,
            comparison,
            insights,
        }
    }

    pub fn newest(&self, index: usize) -> Option<&TestResult> {
        self.results
            .len()
            .checked_sub(index.checked_add(1)?)
            .and_then(|index| self.results.get(index))
    }

    pub fn anchor(&self, selected: usize) -> Option<HistoryAnchor> {
        let result = self.newest(selected)?;
        let mut anchor = HistoryAnchor {
            timestamp: result.timestamp,
            record: serde_json::to_string(result).ok()?,
            occurrence: 0,
        };
        anchor.occurrence = self
            .results
            .iter()
            .rev()
            .take(selected)
            .filter(|result| anchor.matches(result))
            .count();
        Some(anchor)
    }

    pub fn position(&self, anchor: &HistoryAnchor) -> Option<usize> {
        self.results
            .iter()
            .rev()
            .enumerate()
            .filter(|(_, result)| anchor.matches(result))
            .nth(anchor.occurrence)
            .map(|(index, _)| index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    DnsShow,
    DnsList,
    DnsTest,
    DnsUdp,
    DnsDoh,
    DnsDot,
    DnsDoq,
    Doctor,
    Wifi,
    Loss,
    Stability,
    MonitorReport,
    Verify,
}

impl Tool {
    pub const DNS: &'static [Self] = &[
        Self::DnsShow,
        Self::DnsList,
        Self::DnsTest,
        Self::DnsUdp,
        Self::DnsDoh,
        Self::DnsDot,
        Self::DnsDoq,
    ];
    pub const DIAGNOSTICS: &'static [Self] = &[
        Self::Doctor,
        Self::Wifi,
        Self::Loss,
        Self::Stability,
        Self::MonitorReport,
        Self::Verify,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::DnsShow => "Current DNS configuration",
            Self::DnsList => "Resolver catalog",
            Self::DnsTest => "Test active resolver",
            Self::DnsUdp => "Benchmark DNS / UDP",
            Self::DnsDoh => "Benchmark DNS / HTTPS",
            Self::DnsDot => "Benchmark DNS / TLS",
            Self::DnsDoq => "Benchmark DNS / QUIC",
            Self::Doctor => "Network Doctor",
            Self::Wifi => "Wi-Fi inspection",
            Self::Loss => "ICMP response loss",
            Self::Stability => "Stability monitor",
            Self::MonitorReport => "Monitor report",
            Self::Verify => "Cross-backend verification",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::DnsShow => "Read the active interface and resolver configuration. No changes are applied.",
            Self::DnsList => "Explore the built-in resolver profiles and supported protocols. Entirely offline.",
            Self::DnsTest => "Send 12 DNS queries through the current resolver. This does not change DNS settings.",
            Self::DnsUdp => "Compare the fastest resolver league with 12 UDP queries per provider. Read-only.",
            Self::DnsDoh => "Compare the fastest resolver league using real DNS-over-HTTPS queries. Read-only.",
            Self::DnsDot => "Compare the fastest resolver league using real DNS-over-TLS queries. Read-only.",
            Self::DnsDoq => "Compare the fastest resolver league using real DNS-over-QUIC queries. Read-only.",
            Self::Doctor => "Check routing, gateway, DNS, IPv4/IPv6 and HTTPS. Does not saturate the connection.",
            Self::Wifi => "Read native Wi-Fi link details. Availability depends on your OS, permissions and driver.",
            Self::Loss => "Send 20 ICMP echoes to 1.1.1.1. Echo loss is not proof of application packet loss.",
            Self::Stability => "Probe HTTP latency for 60 seconds. No saturation test; HTTP availability is not packet loss. Results are not saved.",
            Self::MonitorReport => "Summarize saved monitoring attempts and failures offline. No network activity.",
            Self::Verify => "Run both Internet backends with 5-second phases and 2 streams. May consume substantial data.",
        }
    }

    pub const fn network(self) -> bool {
        !matches!(
            self,
            Self::DnsShow | Self::DnsList | Self::Wifi | Self::MonitorReport
        )
    }

    pub fn arguments(self, options: &TestOptions) -> Vec<String> {
        let args: &[&str] = match self {
            Self::DnsShow => &["dns", "show"],
            Self::DnsList => &["dns", "list"],
            Self::DnsTest => &["dns", "test", "--queries", "12"],
            Self::DnsUdp => &["dns", "benchmark", "--protocol", "udp", "--queries", "12"],
            Self::DnsDoh => &["dns", "benchmark", "--protocol", "doh", "--queries", "12"],
            Self::DnsDot => &["dns", "benchmark", "--protocol", "dot", "--queries", "12"],
            Self::DnsDoq => &["dns", "benchmark", "--protocol", "doq", "--queries", "12"],
            Self::Doctor => &["doctor"],
            Self::Wifi => &["wifi"],
            Self::Loss => &["loss", "--count", "20", "--target", "1.1.1.1"],
            Self::Stability => &["stability", "--duration", "60s", "--plain", "--no-save"],
            Self::MonitorReport => &["monitor", "--report", "--json"],
            Self::Verify => &["verify", "--duration", "5", "--streams", "2"],
        };
        let mut arguments: Vec<String> = ["--color", "never", "--progress", "never"]
            .iter()
            .chain(args)
            .map(|s| (*s).to_owned())
            .collect();
        if self == Self::Verify {
            if let Some(server) = &options.librespeed_server {
                arguments.extend(["--librespeed-server".to_owned(), server.clone()]);
            }
        }
        arguments
    }
}

/// Execute the existing read-only command implementation rather than copying its
/// platform-specific logic or formatting. No shell, inherited stdin, or TUI recursion.
/// The child is owned by this future and killed on cancellation or output overflow.
/// Native helper descendants retain the lifecycle of the existing CLI commands.
pub(super) async fn run_tool(
    tool: Tool,
    options: TestOptions,
    language: crate::i18n::Language,
) -> Result<String> {
    let executable = std::env::current_exe().context("cannot locate speedtest executable")?;
    let mut child = Command::new(executable)
        .args(tool.arguments(&options))
        .args(["--language", language.code()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not start diagnostic command")?;
    let stdout = child
        .stdout
        .take()
        .context("diagnostic stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("diagnostic stderr unavailable")?;
    let collect = async {
        tokio::try_join!(child.wait(), read_report(stdout), read_report(stderr))
            .context("failed to read diagnostic report")
    };
    let (status, stdout, stderr) =
        runtime::deadline(Duration::from_secs(options.timeout), collect).await?;
    if !status.success() {
        let message = if stderr.trim().is_empty() {
            "Diagnostic ended without a report."
        } else {
            stderr.trim()
        };
        bail!(
            "{} failed (exit {}): {}",
            tool.title(),
            status.code().unwrap_or(1),
            message
        );
    }
    Ok(if stderr.trim().is_empty() {
        stdout
    } else {
        format!("{stdout}\nNOTICES\n{stderr}")
    })
}

async fn read_report(reader: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(REPORT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > REPORT_LIMIT {
        return Err(std::io::Error::other("diagnostic report exceeds 256 KiB"));
    }
    Ok(output::safe_text(&String::from_utf8_lossy(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    #[test]
    fn diagnostic_routes_are_read_only_and_never_reenter_the_menu() {
        let options = TestOptions::from(&Cli::parse_from(["speedtest"]));
        for tool in Tool::DNS.iter().chain(Tool::DIAGNOSTICS) {
            let args = tool.arguments(&options);
            assert_eq!(&args[..4], ["--color", "never", "--progress", "never"]);
            assert!(!args.iter().any(|a| [
                "set", "reset", "rollback", "optimize", "serve", "--yes"
            ]
            .contains(&a.as_str())));
            assert!(args.len() >= 5);
        }
        assert!(!Tool::DnsList.network());
        assert!(!Tool::MonitorReport.network());
        assert!(Tool::DnsDot
            .arguments(&options)
            .windows(2)
            .any(|window| window.iter().map(String::as_str).eq(["--protocol", "dot"])));
        assert!(Tool::DnsDoq
            .arguments(&options)
            .windows(2)
            .any(|window| window.iter().map(String::as_str).eq(["--protocol", "doq"])));
        assert!(Tool::Doctor.network());
    }

    #[tokio::test]
    async fn bounds_reports_and_filters_terminal_controls() {
        assert_eq!(
            read_report("hello\x1b\u{202e}".as_bytes()).await.unwrap(),
            "hello"
        );
        assert!(read_report(vec![b'x'; REPORT_LIMIT + 1].as_slice())
            .await
            .is_err());
    }
}
