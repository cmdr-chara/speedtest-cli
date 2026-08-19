use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use speedtest_cli::{
    cli::{Cli, OutputFormat},
    engine::{cloudflare::CloudflareEngine, EngineConfig, EngineEvent},
    model::TestResult,
    storage, tui,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
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

fn format_delta(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("+{value:.1} ms"))
}
