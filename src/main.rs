use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use speedtest_cli::{
    cli::{Cli, Command, HistoryArgs, OutputFormat, StabilityArgs, StatsArgs},
    engine::{cloudflare::CloudflareEngine, EngineConfig, EngineEvent},
    history::{self, HistorySummary},
    model::TestResult,
    stability::{self, StabilityResult},
    storage, tui,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.clone() {
        Some(Command::Stability(args)) => run_stability_command(args).await,
        Some(Command::History(args)) => run_history_command(args),
        Some(Command::Stats(args)) => run_stats_command(args),
        None => run_speedtest(cli).await,
    }
}

async fn run_speedtest(cli: Cli) -> Result<()> {
    let config = EngineConfig {
        streams: cli.streams as usize,
        phase_duration: Duration::from_secs(cli.duration),
    };
    let engine = CloudflareEngine::new(config)?;

    let result = if cli.plain || cli.json {
        run_non_interactive(&engine).await?
    } else {
        run_interactive(engine, cli.fps).await?
    };

    if cli.json {
        println!("{}", result.pretty_json()?);
    } else if cli.plain {
        print_plain(&result);
    }

    if let Some(path) = &cli.output {
        match cli.format {
            OutputFormat::Json => storage::write_json(path, &result)?,
            OutputFormat::Csv => storage::write_csv(path, &result)?,
        }
    }

    if !cli.no_save {
        let _ =
            storage::persist_default(&result).context("failed to persist speed-test history")?;
    }

    Ok(())
}

async fn run_stability_command(args: StabilityArgs) -> Result<()> {
    let duration = Duration::from_secs(args.duration);
    let interval = Duration::from_millis(args.interval_ms);
    let result = if args.plain || args.json {
        stability::run(duration, interval, None).await?
    } else {
        run_interactive_stability(duration, interval, args.fps).await?
    };

    if args.json {
        println!("{}", result.pretty_json()?);
    } else if args.plain {
        print_stability(&result);
    }

    if let Some(path) = &args.output {
        storage::write_stability_json(path, &result)?;
    }
    if !args.no_save {
        let _ =
            storage::persist_stability(&result).context("failed to persist stability history")?;
    }
    Ok(())
}

fn run_history_command(args: HistoryArgs) -> Result<()> {
    let results = storage::load_history_since(args.days)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    if results.is_empty() {
        println!(
            "No saved speed-test results in the last {} days.",
            args.days
        );
        return Ok(());
    }

    let summary = history::summarize(&results, args.days).expect("non-empty history has summary");
    let limit = usize::from(args.limit);
    println!(
        "SPEEDTEST HISTORY · {} DAYS · {} RUNS",
        args.days,
        results.len()
    );
    println!();
    println!("  DATE / UTC         DOWNLOAD     UPLOAD       PING      QUALITY");
    println!("  ─────────────────  ───────────  ───────────  ────────  ─────────────");
    for result in results.iter().rev().take(limit) {
        let quality = result.analysis.as_ref().map_or_else(
            || "—".to_string(),
            |analysis| {
                let quality = &analysis.quality;
                let tier = quality
                    .tier_label()
                    .map_or(String::new(), |tier| format!(" ◆ {tier}"));
                format!("{}/100 {}{tier}", quality.score, quality.grade.label())
            },
        );
        println!(
            "  {:<17}  {:>8.1} M  {:>8.1} M  {:>6.1}ms  {}",
            result.timestamp.format("%Y-%m-%d %H:%M"),
            result.download.mbps,
            result.upload.mbps,
            result.latency.idle_ms,
            quality
        );
    }
    println!();
    println!("  Download trend  {}", summary.download_sparkline);
    println!("  Trend           {}", summary.trend.label());
    if results.len() > limit {
        println!("  Showing latest {} of {} runs.", args.limit, results.len());
    }
    Ok(())
}

fn run_stats_command(args: StatsArgs) -> Result<()> {
    let results = storage::load_history_since(args.days)?;
    let Some(summary) = history::summarize(&results, args.days) else {
        if args.json {
            println!("null");
        } else {
            println!(
                "No saved speed-test results in the last {} days.",
                args.days
            );
        }
        return Ok(());
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_stats(&summary);
    }
    Ok(())
}

async fn run_non_interactive(engine: &CloudflareEngine) -> Result<TestResult> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let engine = engine.clone();
    let handle = tokio::spawn(async move { engine.run(tx).await });

    while let Some(event) = rx.recv().await {
        match event {
            EngineEvent::Complete(result) => {
                handle.await.context("measurement task panicked")??;
                return Ok(result);
            }
            EngineEvent::Error(error) => anyhow::bail!(error),
            _ => {}
        }
    }

    handle.await.context("measurement task panicked")?
}

async fn run_interactive(engine: CloudflareEngine, fps: u16) -> Result<TestResult> {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        if let Err(error) = engine.run(tx.clone()).await {
            let _ = tx.send(EngineEvent::Error(format!("{error:#}")));
            return Err(error);
        }
        Ok(())
    });

    match tui::run(rx, fps).await {
        Ok(result) => {
            handle.await.context("measurement task panicked")??;
            Ok(result)
        }
        Err(error) => {
            handle.abort();
            Err(error)
        }
    }
}

async fn run_interactive_stability(
    duration: Duration,
    interval: Duration,
    fps: u16,
) -> Result<StabilityResult> {
    let (tx, rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move { stability::run(duration, interval, Some(tx)).await });

    match tui::run_stability(rx, duration, fps).await {
        Ok(result) => {
            handle.await.context("stability task panicked")??;
            Ok(result)
        }
        Err(error) => {
            handle.abort();
            Err(error)
        }
    }
}

fn print_plain(result: &TestResult) {
    println!("Speedtest");
    println!(
        "  Server:        {} ({})",
        result.server.name, result.server.host
    );
    println!("  Download:      {:.1} Mbps", result.download.mbps);
    println!("  Upload:        {:.1} Mbps", result.upload.mbps);
    println!("  Ping:          {:.1} ms", result.latency.idle_ms);
    println!("  Jitter:        {:.1} ms", result.latency.jitter_ms);
    println!(
        "  Loaded down:   {}",
        result
            .latency
            .download_loaded_ms
            .map_or_else(|| "n/a".to_string(), |value| format!("{value:.1} ms"))
    );
    println!(
        "  Loaded up:     {}",
        result
            .latency
            .upload_loaded_ms
            .map_or_else(|| "n/a".to_string(), |value| format!("{value:.1} ms"))
    );

    if let Some(analysis) = &result.analysis {
        println!(
            "  Idle p95/p99:  {:.1} / {:.1} ms",
            analysis.latency.idle.p95_ms, analysis.latency.idle.p99_ms
        );
        if let Some(jitter) = &analysis.latency.jitter {
            println!("  Jitter p95:    {:.1} ms", jitter.p95_ms);
        }

        let quality = &analysis.quality;
        println!(
            "  Quality:       {}/100 {} ({} confidence)",
            quality.score,
            quality.grade.label(),
            quality.confidence.label()
        );
        if let Some(tier) = quality.tier_label() {
            println!("  Tier:          ◆ {tier}");
        }
        println!(
            "  Workloads:     gaming {}  calls {}  streaming {}  cloud gaming {}",
            quality.workloads.gaming.label(),
            quality.workloads.video_calls.label(),
            quality.workloads.streaming.label(),
            quality.workloads.cloud_gaming.label()
        );
        if let Some(grade) = quality.bufferbloat.grade {
            println!(
                "  Bufferbloat:   {} (down {} / up {})",
                grade.label(),
                format_delta(quality.bufferbloat.download_increase_ms),
                format_delta(quality.bufferbloat.upload_increase_ms)
            );
        } else {
            println!("  Bufferbloat:   n/a (insufficient loaded-latency data)");
        }

        if let Some(finding) = quality.findings.first() {
            println!(
                "  Diagnosis:     {}: {}",
                finding.severity.label(),
                finding.title
            );
            println!("                 {}", finding.evidence);
            if let Some(recommendation) = &finding.recommendation {
                println!("  Try:           {recommendation}");
            }
        }
    }
}

fn print_stability(result: &StabilityResult) {
    println!("Network Stability");
    println!("  Duration:       {}s", result.duration_seconds);
    println!("  Probe interval: {} ms", result.interval_ms);
    println!(
        "  Probes:         {} successful / {} failed",
        result.successful_probes, result.failed_probes
    );
    println!(
        "  Availability:   {:.2}% (HTTP probe availability, not packet loss)",
        result.probe_availability_percent
    );
    println!("  Failure bursts: {}", result.failure_bursts);
    if let Some(latency) = &result.latency {
        println!("  Median:         {:.1} ms", latency.median_ms);
        println!(
            "  p95 / p99:      {:.1} / {:.1} ms",
            latency.p95_ms, latency.p99_ms
        );
        println!("  Max:            {:.1} ms", latency.max_ms);
    }
    if let Some(jitter) = &result.jitter {
        println!("  Jitter p95:     {:.1} ms", jitter.p95_ms);
    }
    println!(
        "  Stability:      {}/100 {}",
        result.score,
        result.grade.label()
    );
    if let Some(tier) = result.tier_label() {
        println!("  Tier:           ◆ {tier}");
    }
}

fn print_stats(summary: &HistorySummary) {
    println!("NETWORK STATS · {} DAYS", summary.period_days);
    println!();
    println!("  Runs:              {}", summary.runs);
    println!(
        "  Download:          {:.1} Mbps median · {:.1} Mbps best",
        summary.median_download_mbps, summary.best_download_mbps
    );
    println!(
        "  Upload:            {:.1} Mbps median · {:.1} Mbps best",
        summary.median_upload_mbps, summary.best_upload_mbps
    );
    println!(
        "  Ping:              {:.1} ms median · {:.1} ms p95",
        summary.median_ping_ms, summary.p95_ping_ms
    );
    if let Some(score) = summary.median_quality_score {
        println!("  Quality median:    {score:.0}/100");
    }
    println!(
        "  S-tier runs:       {} / {}",
        summary.s_tier_runs, summary.runs
    );
    println!("  Trend:             {}", summary.trend.label());
    println!("  Download history:  {}", summary.download_sparkline);

    if summary.anomalies.is_empty() {
        println!();
        println!("  No major latest-run anomaly detected against the saved baseline.");
    } else {
        println!();
        println!("  ANOMALIES");
        for anomaly in &summary.anomalies {
            println!("  {}  {}", anomaly.severity.label(), anomaly.message);
        }
    }
}

fn format_delta(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("+{value:.1} ms"))
}
