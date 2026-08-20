use std::{
    io::{self, Write},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use speedtest_cli::{
    cli::{
        Cli, Command, CompareArgs, DnsArgs, DnsBenchmarkArgs, DnsBenchmarkProfileArg, DnsCommand,
        DnsListArgs, DnsOptimizeArgs, DnsProtocolArg, DnsResetArgs, DnsRollbackArgs, DnsSetArgs,
        DnsShowArgs, DnsTestArgs, DoctorArgs, HistoryArgs, InternetBackendArg, LanArgs, LossArgs,
        OutputFormat, ServeArgs, StabilityArgs, StatsArgs, VerifyArgs, WifiArgs,
    },
    compare::{self, CompareResult},
    dns::{self, BenchmarkProfile, DnsBenchmarkResult, DnsProviderBenchmark},
    dns_custom,
    doctor::{self, DoctorReport},
    engine::{
        cloudflare::CloudflareEngine, internet::InternetEngine, librespeed::LibreSpeedEngine,
        EngineConfig, EngineEvent,
    },
    history::{self, HistorySummary},
    lan, loss,
    model::TestResult,
    stability::{self, StabilityResult},
    storage, tui, verify, wifi,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.clone() {
        Some(Command::Stability(args)) => run_stability(args).await,
        Some(Command::History(args)) => run_history(args),
        Some(Command::Stats(args)) => run_stats(args),
        Some(Command::Dns(args)) => run_dns(args).await,
        Some(Command::Compare(args)) => run_compare(args),
        Some(Command::Doctor(args)) => run_doctor(args).await,
        Some(Command::Loss(args)) => run_loss(args).await,
        Some(Command::Wifi(args)) => run_wifi(args),
        Some(Command::Verify(args)) => run_verify(args).await,
        Some(Command::Serve(args)) => run_serve(args).await,
        Some(Command::Lan(args)) => run_lan(args).await,
        None => run_speedtest(cli).await,
    }
}

async fn run_speedtest(cli: Cli) -> Result<()> {
    let config = EngineConfig {
        streams: usize::from(cli.streams),
        phase_duration: Duration::from_secs(cli.duration),
    };
    let engine = match cli.backend {
        InternetBackendArg::Cloudflare => {
            InternetEngine::Cloudflare(CloudflareEngine::new(config)?)
        }
        InternetBackendArg::Librespeed => InternetEngine::LibreSpeed(LibreSpeedEngine::new(
            config,
            cli.librespeed_server.as_deref(),
        )?),
    };

    let result = if cli.plain || cli.json {
        run_non_interactive(&engine).await?
    } else {
        run_interactive(engine, cli.fps).await?
    };

    if cli.json {
        println!("{}", result.pretty_json()?);
    } else if cli.plain {
        print_result(&result);
    }

    if let Some(path) = &cli.output {
        match cli.format {
            OutputFormat::Json => storage::write_json(path, &result)?,
            OutputFormat::Csv => storage::write_csv(path, &result)?,
        }
    }
    if !cli.no_save {
        storage::persist_default(&result).context("failed to persist speed-test history")?;
    }
    Ok(())
}

async fn run_stability(args: StabilityArgs) -> Result<()> {
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
        storage::persist_stability(&result).context("failed to persist stability history")?;
    }
    Ok(())
}

fn run_history(args: HistoryArgs) -> Result<()> {
    let results = storage::load_history_since(args.days)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if results.is_empty() {
        println!("No saved speed-test results in the last {} days.", args.days);
        return Ok(());
    }

    let summary = history::summarize(&results, args.days).expect("non-empty history has summary");
    println!("SPEEDTEST HISTORY · {} DAYS · {} RUNS", args.days, results.len());
    println!();
    println!("  DATE / UTC         BACKEND       DOWNLOAD     UPLOAD       PING      QUALITY");
    println!("  ─────────────────  ────────────  ───────────  ───────────  ────────  ─────────────");
    for result in results.iter().rev().take(usize::from(args.limit)) {
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
            "  {:<17}  {:<12}  {:>8.1} M  {:>8.1} M  {:>6.1}ms  {}",
            result.timestamp.format("%Y-%m-%d %H:%M"),
            truncate(&result.backend, 12),
            result.download.mbps,
            result.upload.mbps,
            result.latency.idle_ms,
            quality
        );
    }
    println!();
    println!("  Download trend  {}", summary.download_sparkline);
    println!("  Trend           {}", summary.trend.label());
    Ok(())
}

fn run_stats(args: StatsArgs) -> Result<()> {
    let results = storage::load_history_since(args.days)?;
    let Some(summary) = history::summarize(&results, args.days) else {
        if args.json {
            println!("null");
        } else {
            println!("No saved speed-test results in the last {} days.", args.days);
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

async fn run_dns(args: DnsArgs) -> Result<()> {
    match args.command {
        DnsCommand::List(args) => run_dns_list(args),
        DnsCommand::Show(args) => run_dns_show(args),
        DnsCommand::Test(args) => run_dns_test(args).await,
        DnsCommand::Benchmark(args) => run_dns_benchmark(args).await,
        DnsCommand::Set(args) => run_dns_set(args).await,
        DnsCommand::Optimize(args) => run_dns_optimize(args).await,
        DnsCommand::Reset(args) => run_dns_reset(args).await,
        DnsCommand::Rollback(args) => run_dns_rollback(args).await,
    }
}

fn run_dns_list(args: DnsListArgs) -> Result<()> {
    if args.json {
        let values = dns::PROVIDERS
            .iter()
            .map(|provider| {
                serde_json::json!({
                    "id": provider.id,
                    "provider": provider.provider,
                    "profile": provider.profile,
                    "category": provider.category,
                    "privacy_oriented": provider.privacy_oriented,
                    "ipv4": provider.ipv4,
                    "ipv6": provider.ipv6,
                    "doh": provider.doh,
                    "dot": provider.dot,
                    "doq": provider.doq,
                    "dnssec": provider.dnssec,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
        return Ok(());
    }

    println!("DNS PROVIDERS · {} PROFILES", dns::PROVIDERS.len());
    println!();
    println!("  ID                       TYPE         DNS53  DoH  DoT  DoQ  DNSSEC");
    println!("  ───────────────────────  ───────────  ─────  ───  ───  ───  ──────");
    for provider in dns::PROVIDERS {
        println!(
            "  {:<23}  {:<11}    {}    {}    {}    {}      {}",
            provider.id,
            provider.category.label(),
            yes(!provider.ipv4.is_empty()),
            yes(provider.doh.is_some()),
            yes(provider.dot.is_some()),
            yes(provider.doq.is_some()),
            yes(provider.dnssec)
        );
    }
    Ok(())
}

fn run_dns_show(args: DnsShowArgs) -> Result<()> {
    let state = dns::system::inspect(args.interface.as_deref())?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&state)?);
        return Ok(());
    }
    println!("DNS CONFIGURATION");
    println!();
    println!("  Interface:      {}", state.interface);
    if let Some(device) = &state.device {
        println!("  Device/profile: {device}");
    }
    println!("  Backend:        {}", state.backend);
    println!("  Source:         {}", state.mode.label());
    println!("  DNS servers:    {}", format_servers(&state.servers));
    if let Some(gateway) = state.gateway {
        println!("  Gateway:        {gateway}");
    }
    println!("  IPv6 default:   {}", yes(state.ipv6_default_route));
    println!("  Writable:       {}", yes(state.can_configure()));
    Ok(())
}

async fn run_dns_test(args: DnsTestArgs) -> Result<()> {
    let result = if args.resolvers.is_empty() {
        dns::test_current(usize::from(args.queries)).await?
    } else {
        dns_custom::test_servers(args.resolvers, usize::from(args.queries)).await?
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_dns_test(&result);
    }
    Ok(())
}

async fn run_dns_benchmark(args: DnsBenchmarkArgs) -> Result<()> {
    let profile = dns_profile(args.profile);
    let result = match args.protocol {
        DnsProtocolArg::Udp => dns::benchmark(profile, usize::from(args.queries)).await?,
        DnsProtocolArg::Doh => dns::doh::benchmark(profile, usize::from(args.queries)).await?,
    };
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_dns_benchmark(&result);
    }
    Ok(())
}

async fn run_dns_set(args: DnsSetArgs) -> Result<()> {
    let provider = dns::provider(&args.provider).ok_or_else(|| {
        anyhow!(
            "unknown DNS provider profile `{}`; run `speedtest dns list` to see IDs",
            args.provider
        )
    })?;
    let state = dns::system::inspect(args.interface.as_deref())?;
    if !state.can_configure() && !args.dry_run {
        bail!(
            "{} can inspect DNS but cannot safely persist DNS changes on this interface",
            state.backend
        );
    }
    let servers = provider.addresses(state.ipv6_default_route);
    if servers.is_empty() {
        bail!("{} does not expose DNS53 addresses", provider.display_name());
    }
    let preflight = dns_custom::test_servers(servers.clone(), 6).await?;
    if preflight.success_rate_percent < 80.0 {
        bail!(
            "refusing to configure {} because preflight DNS success was only {:.0}%",
            provider.display_name(),
            preflight.success_rate_percent
        );
    }

    print_dns_change(&state, &provider.display_name(), &servers);
    if args.dry_run {
        println!("DRY RUN · no DNS settings were changed.");
        return Ok(());
    }
    if !args.yes && !confirm("Apply this DNS configuration?")? {
        println!("No changes made.");
        return Ok(());
    }
    apply_dns_change(&state, &servers, &provider.display_name()).await
}

async fn run_dns_optimize(args: DnsOptimizeArgs) -> Result<()> {
    let benchmark = dns::benchmark(dns_profile(args.profile), usize::from(args.queries)).await?;
    print_dns_benchmark(&benchmark);
    let winner = benchmark
        .winner()
        .ok_or_else(|| anyhow!("no DNS resolver completed enough queries to select a winner"))?;
    if winner.is_current {
        println!();
        println!("◆ Your current DNS already won this resolver league. No change recommended.");
        return Ok(());
    }
    let provider = dns::provider(&winner.provider_id)
        .ok_or_else(|| anyhow!("benchmark winner is not a configurable built-in provider"))?;
    let state = dns::system::inspect(args.interface.as_deref())?;
    if !state.can_configure() && !args.dry_run {
        bail!(
            "{} can inspect DNS but cannot safely persist DNS changes on this interface",
            state.backend
        );
    }
    let servers = provider.addresses(state.ipv6_default_route);
    println!();
    println!("DNS OPTIMIZER RECOMMENDATION");
    println!("  Winner:         {}", provider.display_name());
    if let Some(latency) = &winner.latency {
        println!("  Median / p95:   {:.1} / {:.1} ms", latency.median_ms, latency.p95_ms);
    }
    print_dns_change(&state, &provider.display_name(), &servers);
    if args.dry_run {
        println!("DRY RUN · benchmark completed; no DNS settings were changed.");
        return Ok(());
    }
    if !args.yes && !confirm("Apply the recommended DNS configuration?")? {
        println!("No changes made.");
        return Ok(());
    }
    apply_dns_change(&state, &servers, &provider.display_name()).await
}

async fn run_dns_reset(args: DnsResetArgs) -> Result<()> {
    let state = dns::system::inspect(args.interface.as_deref())?;
    println!("DNS RESET");
    println!("  Interface: {}", state.interface);
    println!("  Current:   {}", format_servers(&state.servers));
    println!("  Target:    automatic / DHCP-managed DNS");
    if args.dry_run {
        println!("DRY RUN · no DNS settings were changed.");
        return Ok(());
    }
    if !state.can_configure() {
        bail!("{} does not support safe automatic DNS reset", state.backend);
    }
    if !args.yes && !confirm("Reset this interface to automatic DNS?")? {
        println!("No changes made.");
        return Ok(());
    }

    let backup_path = dns::save_backup(&state)?;
    dns::system::reset(&state)?;
    dns::system::flush_cache();
    if let Err(error) = dns::verify_system_resolution().await {
        let rollback = dns::system::restore(&state);
        dns::system::flush_cache();
        if let Err(rollback_error) = rollback {
            bail!(
                "automatic DNS verification failed ({error:#}) and rollback also failed ({rollback_error:#}); snapshot: {}",
                backup_path.display()
            );
        }
        bail!("automatic DNS verification failed ({error:#}); previous DNS was restored");
    }
    println!("✓ DNS returned to automatic configuration and passed post-change resolution.");
    Ok(())
}

async fn run_dns_rollback(args: DnsRollbackArgs) -> Result<()> {
    let backup = dns::load_backup()?;
    println!("DNS ROLLBACK");
    println!("  Snapshot:   {}", backup.timestamp.to_rfc3339());
    println!("  Interface:  {}", backup.state.interface);
    println!("  Mode:       {}", backup.state.mode.label());
    println!("  Servers:    {}", format_servers(&backup.state.servers));
    if args.dry_run {
        println!("DRY RUN · rollback snapshot was not applied.");
        return Ok(());
    }
    if !args.yes && !confirm("Restore this DNS snapshot?")? {
        println!("No changes made.");
        return Ok(());
    }
    dns::system::restore(&backup.state)?;
    dns::system::flush_cache();
    dns::verify_system_resolution()
        .await
        .context("rollback was applied but system DNS verification failed")?;
    println!("✓ Previous DNS snapshot restored and verified.");
    Ok(())
}

async fn apply_dns_change(
    state: &dns::system::DnsSystemState,
    servers: &[std::net::IpAddr],
    label: &str,
) -> Result<()> {
    let backup_path = dns::save_backup(state)?;
    dns::system::apply_servers(state, servers)?;
    dns::system::flush_cache();
    if let Err(error) = dns::verify_system_resolution().await {
        let rollback = dns::system::restore(state);
        dns::system::flush_cache();
        match rollback {
            Ok(()) => bail!(
                "post-change DNS verification failed ({error:#}); previous configuration was automatically restored"
            ),
            Err(rollback_error) => bail!(
                "post-change DNS verification failed ({error:#}) and rollback failed ({rollback_error:#}); recovery snapshot: {}",
                backup_path.display()
            ),
        }
    }
    println!("✓ {label} configured successfully.");
    println!("✓ Resolver cache flushed and system DNS verification passed.");
    println!("  Rollback: speedtest dns rollback");
    Ok(())
}

fn run_compare(args: CompareArgs) -> Result<()> {
    let (before, after) = match (&args.before, &args.after) {
        (Some(before), Some(after)) => (storage::read_result(before)?, storage::read_result(after)?),
        (None, None) => {
            let history = storage::load_history()?;
            if history.len() < 2 {
                bail!("compare requires two saved results or explicit BEFORE and AFTER JSON files");
            }
            (
                history[history.len() - 2].clone(),
                history[history.len() - 1].clone(),
            )
        }
        _ => bail!("supply both BEFORE and AFTER JSON files, or omit both"),
    };
    let comparison = compare::compare(&before, &after);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&comparison)?);
    } else {
        print_comparison(&comparison);
    }
    Ok(())
}

async fn run_doctor(args: DoctorArgs) -> Result<()> {
    let mut report = doctor::run(args.interface.as_deref()).await?;
    if args.full {
        let engine = InternetEngine::Cloudflare(CloudflareEngine::new(EngineConfig {
            streams: 2,
            phase_duration: Duration::from_secs(8),
        })?);
        let result = run_non_interactive(&engine)
            .await
            .context("full doctor speed test failed")?;
        report.attach_speedtest(result);
    }
    if args.json {
        println!("{}", report.pretty_json()?);
    } else {
        print_doctor(&report);
    }
    Ok(())
}

async fn run_loss(args: LossArgs) -> Result<()> {
    let result = loss::measure(&args.target, args.count).await?;
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_packet_loss(&result);
    }
    Ok(())
}

fn run_wifi(args: WifiArgs) -> Result<()> {
    let result = wifi::inspect(args.interface.as_deref())?;
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_wifi(&result);
    }
    Ok(())
}

async fn run_verify(args: VerifyArgs) -> Result<()> {
    let report = verify::run(
        EngineConfig {
            streams: usize::from(args.streams),
            phase_duration: Duration::from_secs(args.duration),
        },
        args.librespeed_server.as_deref(),
    )
    .await?;
    if args.json {
        println!("{}", report.pretty_json()?);
    } else {
        print_verify(&report);
    }
    Ok(())
}

async fn run_serve(args: ServeArgs) -> Result<()> {
    println!("LAN SPEEDTEST SERVER");
    println!("  Listening: {}", args.bind);
    println!("  Client:    speedtest lan <this-host>:{}", args.bind.port());
    println!("  Stop with Ctrl+C.");
    lan::serve(args.bind).await
}

async fn run_lan(args: LanArgs) -> Result<()> {
    let result = lan::run(
        args.server,
        lan::LanConfig {
            streams: usize::from(args.streams),
            phase_duration: Duration::from_secs(args.duration),
        },
    )
    .await?;
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_result(&result);
    }
    Ok(())
}

async fn run_non_interactive(engine: &InternetEngine) -> Result<TestResult> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let engine = engine.clone();
    let handle = tokio::spawn(async move { engine.run(tx).await });
    while let Some(event) = rx.recv().await {
        match event {
            EngineEvent::Complete(result) => {
                handle.await.context("measurement task panicked")??;
                return Ok(result);
            }
            EngineEvent::Error(error) => bail!(error),
            _ => {}
        }
    }
    handle.await.context("measurement task panicked")?
}

async fn run_interactive(engine: InternetEngine, fps: u16) -> Result<TestResult> {
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

fn print_result(result: &TestResult) {
    println!("Speedtest");
    println!("  Backend:       {}", result.backend);
    println!("  Server:        {} ({})", result.server.name, result.server.host);
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
        if let Some(grade) = quality.bufferbloat.grade {
            println!(
                "  Bufferbloat:   {} (down {} / up {})",
                grade.label(),
                format_delta(quality.bufferbloat.download_increase_ms),
                format_delta(quality.bufferbloat.upload_increase_ms)
            );
        }
        if let Some(finding) = quality.findings.first() {
            println!("  Diagnosis:     {}: {}", finding.severity.label(), finding.title);
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
        println!("  p95 / p99:      {:.1} / {:.1} ms", latency.p95_ms, latency.p99_ms);
        println!("  Max:            {:.1} ms", latency.max_ms);
    }
    println!("  Stability:      {}/100 {}", result.score, result.grade.label());
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
    println!("  S-tier runs:       {} / {}", summary.s_tier_runs, summary.runs);
    println!("  Trend:             {}", summary.trend.label());
    println!("  Download history:  {}", summary.download_sparkline);
    if !summary.anomalies.is_empty() {
        println!();
        println!("  ANOMALIES");
        for anomaly in &summary.anomalies {
            println!("  {}  {}", anomaly.severity.label(), anomaly.message);
        }
    }
}

fn print_dns_test(result: &DnsProviderBenchmark) {
    println!("DNS HEALTH / SPEED TEST");
    println!();
    println!("  Resolver:       {} {}", result.provider_name, result.profile_name);
    println!("  Servers:        {}", format_servers(&result.servers));
    println!(
        "  Queries:        {} / {} successful ({:.0}%)",
        result.successes, result.queries, result.success_rate_percent
    );
    if let Some(latency) = &result.latency {
        println!("  Median:         {:.1} ms", latency.median_ms);
        println!("  p95 / p99:      {:.1} / {:.1} ms", latency.p95_ms, latency.p99_ms);
    }
    println!("  DNS score:      {}/100 {}", result.score, result.grade.label());
    if let Some(tier) = result.tier_label() {
        println!("  Tier:           ◆ {tier}");
    }
}

fn print_dns_benchmark(result: &DnsBenchmarkResult) {
    println!("DNS BENCHMARK · {}", result.profile.to_ascii_uppercase());
    println!();
    println!("  #  RESOLVER                         MEDIAN     P95   SUCCESS   SCORE");
    println!("  ─  ───────────────────────────────  ───────  ───────  ───────  ─────────────");
    for (index, entry) in result.entries.iter().enumerate() {
        let median = entry
            .latency
            .as_ref()
            .map_or_else(|| "—".to_string(), |latency| format!("{:.1}ms", latency.median_ms));
        let p95 = entry
            .latency
            .as_ref()
            .map_or_else(|| "—".to_string(), |latency| format!("{:.1}ms", latency.p95_ms));
        let name = if entry.profile_name.eq_ignore_ascii_case("standard") {
            entry.provider_name.clone()
        } else {
            format!("{} {}", entry.provider_name, entry.profile_name)
        };
        let tier = entry
            .tier_label()
            .map_or(String::new(), |tier| format!(" ◆{tier}"));
        println!(
            "  {:>2} {:<31} {:>7}  {:>7}  {:>6.0}%  {:>3}/100 {}{}",
            index + 1,
            truncate(&name, 31),
            median,
            p95,
            entry.success_rate_percent,
            entry.score,
            entry.grade.label(),
            tier
        );
    }
    if let Some(winner) = result.winner() {
        println!();
        println!("  ◆ WINNER: {} · {}/100 {}", winner.provider_name, winner.score, winner.grade.label());
    }
}

fn print_dns_change(
    state: &dns::system::DnsSystemState,
    label: &str,
    servers: &[std::net::IpAddr],
) {
    println!("DNS CONFIGURATION CHANGE");
    println!();
    println!("  Interface:      {}", state.interface);
    println!("  Current:        {}", format_servers(&state.servers));
    println!("  New provider:   {label}");
    println!("  New servers:    {}", format_servers(servers));
}

fn print_comparison(result: &CompareResult) {
    println!("NETWORK COMPARISON");
    println!();
    println!("                    BEFORE         AFTER        CHANGE");
    print_metric("Download", &result.download_mbps, "Mbps");
    print_metric("Upload", &result.upload_mbps, "Mbps");
    print_metric("Ping", &result.ping_ms, "ms");
    print_metric("Jitter", &result.jitter_ms, "ms");
    if let (Some(before), Some(after), Some(change)) = (
        result.quality_score.before,
        result.quality_score.after,
        result.quality_score.absolute_change,
    ) {
        println!("  {:<16} {:>10.0}/100 {:>10.0}/100 {change:+.0} pts", "Quality", before, after);
    }
    println!();
    println!("  VERDICT    {}", result.verdict.to_ascii_uppercase());
    println!("  HIGHLIGHT  {}", result.highlight);
}

fn print_metric(label: &str, metric: &compare::MetricDelta, unit: &str) {
    println!(
        "  {:<16} {:>10.1} {:<4} {:>10.1} {:<4} {:+.1}%",
        label,
        metric.before,
        unit,
        metric.after,
        unit,
        metric.percent_change.unwrap_or_default()
    );
}

fn print_doctor(report: &DoctorReport) {
    println!("NETWORK DOCTOR");
    if let Some(interface) = &report.interface {
        println!("  Interface: {interface}");
    }
    println!();
    for check in &report.checks {
        println!("  {} {:<20} {}", check.status.symbol(), check.name, check.detail);
    }
    if let Some(speedtest) = &report.speedtest {
        println!();
        println!("  FULL TEST");
        println!("  Download  {:.1} Mbps", speedtest.download.mbps);
        println!("  Upload    {:.1} Mbps", speedtest.upload.mbps);
        println!("  Ping      {:.1} ms", speedtest.latency.idle_ms);
        if let Some(analysis) = &speedtest.analysis {
            println!(
                "  Quality   {}/100 {}",
                analysis.quality.score,
                analysis.quality.grade.label()
            );
        }
    }
    println!();
    println!("  DIAGNOSIS");
    println!("  {}", report.diagnosis);
    if let Some(recommendation) = &report.recommendation {
        println!("  Recommendation: {recommendation}");
    }
}

fn print_packet_loss(result: &loss::PacketLossResult) {
    println!("ICMP PACKET LOSS");
    println!();
    println!("  Target:          {}", result.target);
    println!("  Sent/received:   {} / {}", result.packets_sent, result.packets_received);
    println!("  Lost:            {} ({:.2}%)", result.packets_lost, result.loss_percent);
    if let Some(rtt) = &result.rtt {
        println!("  RTT median:      {:.1} ms", rtt.median_ms);
        println!("  RTT p95 / p99:   {:.1} / {:.1} ms", rtt.p95_ms, rtt.p99_ms);
        println!("  RTT max:         {:.1} ms", rtt.max_ms);
    }
    println!();
    println!("  Note: {}", result.caveat);
}

fn print_wifi(result: &wifi::WifiSnapshot) {
    println!("WI-FI DIAGNOSTICS");
    println!();
    if !result.available {
        println!("  No active Wi-Fi link detected.");
        println!("  {}", result.detail);
        return;
    }
    println!("  Interface:       {}", result.interface.as_deref().unwrap_or("unknown"));
    println!("  SSID:            {}", result.ssid.as_deref().unwrap_or("unknown"));
    if let Some(dbm) = result.signal_dbm {
        println!("  Signal:          {:.0} dBm", dbm);
    }
    if let Some(percent) = result.signal_percent {
        println!("  Signal quality:  {:.0}%", percent);
    }
    if let Some(band) = &result.band {
        println!("  Band:            {band}");
    }
    if let Some(channel) = result.channel {
        println!("  Channel:         {channel}");
    }
    if let Some(rate) = result.link_mbps {
        println!("  Link rate:       {:.0} Mbps", rate);
    }
    if let Some(radio) = &result.radio {
        println!("  Radio:           {radio}");
    }
    println!("  Detail:          {}", result.detail);
}

fn print_verify(report: &verify::VerifyReport) {
    println!("MULTI-BACKEND VERIFICATION");
    println!();
    println!("                    CLOUDFLARE     LIBRESPEED");
    println!(
        "  Download        {:>9.1} M     {:>9.1} M",
        report.cloudflare.download.mbps, report.librespeed.download.mbps
    );
    println!(
        "  Upload          {:>9.1} M     {:>9.1} M",
        report.cloudflare.upload.mbps, report.librespeed.upload.mbps
    );
    println!(
        "  Ping            {:>9.1} ms    {:>9.1} ms",
        report.cloudflare.latency.idle_ms, report.librespeed.latency.idle_ms
    );
    if let Some(loss) = &report.icmp_loss {
        println!("  ICMP loss       {:>9.2}%     independent reference", loss.loss_percent);
    }
    println!();
    println!("  Agreement:      {}", if report.consistent { "CONSISTENT" } else { "DIVERGENT" });
    println!("  {}", report.verdict);
    println!("  Highlight:      {}", report.comparison.highlight);
}

fn dns_profile(profile: DnsBenchmarkProfileArg) -> BenchmarkProfile {
    match profile {
        DnsBenchmarkProfileArg::Fastest => BenchmarkProfile::Fastest,
        DnsBenchmarkProfileArg::Privacy => BenchmarkProfile::Privacy,
        DnsBenchmarkProfileArg::Security => BenchmarkProfile::Security,
        DnsBenchmarkProfileArg::Adblock => BenchmarkProfile::Adblock,
        DnsBenchmarkProfileArg::Family => BenchmarkProfile::Family,
        DnsBenchmarkProfileArg::All => BenchmarkProfile::All,
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush().context("failed to flush confirmation prompt")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read confirmation")?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn yes(value: bool) -> &'static str {
    if value { "✓" } else { "—" }
}

fn format_servers(servers: &[std::net::IpAddr]) -> String {
    if servers.is_empty() {
        "none detected".to_string()
    } else {
        servers.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    }
}

fn format_delta(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("+{value:.1} ms"))
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_string()
    } else {
        value.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
    }
}
