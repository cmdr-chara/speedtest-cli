//! Adapters only: analysis and persistence stay in their existing domain modules.
use std::{process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use tokio::{io::AsyncReadExt, process::Command};

use crate::{compare, history, model::TestResult, output, runtime, session::TestOptions, storage};

pub(super) const HISTORY_DAYS: u64 = 30;
const REPORT_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub(super) struct Archive {
    pub results: Vec<TestResult>,
    pub summary: Option<history::HistorySummary>,
    pub comparison: Option<compare::CompareResult>,
}

impl Archive {
    pub fn load() -> Result<Self> {
        Ok(Self::from_results(storage::load_history_since(
            HISTORY_DAYS,
        )?))
    }

    pub fn from_results(results: Vec<TestResult>) -> Self {
        let summary = history::summarize(&results, HISTORY_DAYS);
        let comparison = results
            .len()
            .checked_sub(2)
            .map(|index| compare::compare(&results[index], &results[index + 1]));
        Self {
            results,
            summary,
            comparison,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tool {
    DnsShow,
    DnsList,
    DnsTest,
    DnsUdp,
    DnsDoh,
    Doctor,
    Wifi,
    Loss,
    Stability,
    Verify,
}

impl Tool {
    pub const DNS: &'static [Self] = &[
        Self::DnsShow,
        Self::DnsList,
        Self::DnsTest,
        Self::DnsUdp,
        Self::DnsDoh,
    ];
    pub const DIAGNOSTICS: &'static [Self] = &[
        Self::Doctor,
        Self::Wifi,
        Self::Loss,
        Self::Stability,
        Self::Verify,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::DnsShow => "Current DNS configuration",
            Self::DnsList => "Resolver catalog",
            Self::DnsTest => "Test active resolver",
            Self::DnsUdp => "Benchmark DNS / UDP",
            Self::DnsDoh => "Benchmark DNS / HTTPS",
            Self::Doctor => "Network Doctor",
            Self::Wifi => "Wi-Fi inspection",
            Self::Loss => "ICMP response loss",
            Self::Stability => "Stability monitor",
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
            Self::Doctor => "Check routing, gateway, DNS, IPv4/IPv6 and HTTPS. Does not saturate the connection.",
            Self::Wifi => "Read native Wi-Fi link details. Availability depends on your OS, permissions and driver.",
            Self::Loss => "Send 20 ICMP echoes to 1.1.1.1. Echo loss is not proof of application packet loss.",
            Self::Stability => "Probe HTTP latency for 60 seconds. No saturation test; HTTP availability is not packet loss. Results are not saved.",
            Self::Verify => "Run both Internet backends with 5-second phases and 2 streams. May consume substantial data.",
        }
    }

    pub const fn network(self) -> bool {
        !matches!(self, Self::DnsShow | Self::DnsList | Self::Wifi)
    }

    pub fn arguments(self, options: &TestOptions) -> Vec<String> {
        let args: &[&str] = match self {
            Self::DnsShow => &["dns", "show"],
            Self::DnsList => &["dns", "list"],
            Self::DnsTest => &["dns", "test", "--queries", "12"],
            Self::DnsUdp => &["dns", "benchmark", "--protocol", "udp", "--queries", "12"],
            Self::DnsDoh => &["dns", "benchmark", "--protocol", "doh", "--queries", "12"],
            Self::Doctor => &["doctor"],
            Self::Wifi => &["wifi"],
            Self::Loss => &["loss", "--count", "20", "--target", "1.1.1.1"],
            Self::Stability => &["stability", "--duration", "60s", "--plain", "--no-save"],
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
