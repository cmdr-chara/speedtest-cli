use std::{
    io::{self, IsTerminal, Write},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{CommandFactory, FromArgMatches};
use speedtest_cli::{
    check,
    cli::{
        CheckArgs, Cli, ColorMode, Command, CompareArgs, DnsArgs, DnsBenchmarkArgs,
        DnsBenchmarkProfileArg, DnsCommand, DnsListArgs, DnsOptimizeArgs, DnsProtocolArg,
        DnsResetArgs, DnsRollbackArgs, DnsSetArgs, DnsShowArgs, DnsTestArgs, DoctorArgs,
        HistoryArgs, InternetBackendArg, LanArgs, LossArgs, ServeArgs, StabilityArgs, StatsArgs,
        VerifyArgs, WifiArgs,
    },
    compare::{self, CompareResult},
    dns::{self, BenchmarkProfile, DnsBenchmarkResult, DnsProviderBenchmark},
    dns_custom,
    doctor::{self, DoctorReport},
    engine::{cloudflare::CloudflareEngine, internet::InternetEngine, EngineConfig, EngineEvent},
    history::{self, HistorySummary},
    i18n, lan, loss,
    model::TestResult,
    output, runtime,
    session::TestOptions,
    stability::{self, StabilityResult},
    storage, tui, verify, wifi,
};
use tokio::sync::mpsc;

// Every line write returns its I/O error instead of panicking on a closed pipe.
macro_rules! println {
    () => { output::line(format_args!(""))? };
    ($($arg:tt)*) => { output::line(format_args!($($arg)*))? };
}

fn tr(key: &str) -> String {
    i18n::text(i18n::cli_language(), key)
}
fn msg(key: &str, values: &[String]) -> String {
    i18n::message(i18n::cli_language(), key, values)
}
fn narr(source: &str) -> String {
    i18n::narrative(i18n::cli_language(), source)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // Clap's own help/error coloring uses the same explicit preference as the UI.
    let arguments: Vec<_> = std::env::args_os().collect();
    let language = i18n::from_arguments(&arguments);
    i18n::initialize_cli(language);
    let mut color = ColorMode::Auto;
    for (index, argument) in arguments.iter().enumerate() {
        let value = argument.to_str().unwrap_or("");
        let choice = value.strip_prefix("--color=").or_else(|| {
            (value == "--color")
                .then(|| arguments.get(index + 1).and_then(|s| s.to_str()))
                .flatten()
        });
        if let Some(choice) = choice {
            color = match choice {
                "never" => ColorMode::Never,
                "always" => ColorMode::Always,
                _ => ColorMode::Auto,
            };
        }
    }
    let clap_color = if color == ColorMode::Always {
        clap::ColorChoice::Always
    } else if !color.allows_tui() {
        clap::ColorChoice::Never
    } else {
        clap::ColorChoice::Auto
    };
    let matches = i18n::command(Cli::command(), language)
        .color(clap_color)
        .get_matches_from(arguments);
    if matches.subcommand().is_some() {
        for name in [
            "run",
            "timeout",
            "backend",
            "librespeed_server",
            "streams",
            "duration",
            "fps",
            "plain",
            "json",
            "output",
            "format",
            "no_save",
        ] {
            if matches.value_source(name) == Some(clap::parser::ValueSource::CommandLine) {
                Cli::command().error(clap::error::ErrorKind::ArgumentConflict,
                    msg("--{0} is a default speed-test option; place subcommand options after the command", &[name.replace('_', "-").to_string()])).exit();
            }
        }
    }
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    if cli.command.is_none()
        && cli.librespeed_server.is_some()
        && !matches!(cli.backend, InternetBackendArg::Librespeed)
    {
        Cli::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "--librespeed-server requires --backend librespeed; no measurement was started",
            )
            .exit();
    }
    let json = cli.json_requested();
    // DNS writes retain their existing transaction/rollback lifecycle. Do not drop
    // a configuration transaction halfway through because a generic select fired.
    let interruptible = matches!(
        cli.command,
        None | Some(Command::Stability(_))
            | Some(Command::Loss(_))
            | Some(Command::Verify(_))
            | Some(Command::Lan(_))
            | Some(Command::Serve(_))
    );
    let result = if interruptible {
        runtime::interruptible(dispatch(cli)).await
    } else {
        dispatch(cli).await
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let code = runtime::exit_code(&error);
            if code != 0 && code != 3 {
                if json {
                    let error =
                        serde_json::json!({"error":{"code":code,"message":format!("{error:#}")}});
                    let _ = output::diagnostic(format_args!("{error}"));
                } else {
                    let _ = output::diagnostic(format_args!("speedtest: {error:#}"));
                }
            }
            std::process::ExitCode::from(code)
        }
    }
}

async fn dispatch(mut cli: Cli) -> Result<()> {
    let can_interact = io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && cli.color.allows_tui()
        && !matches!(cli.progress, speedtest_cli::cli::ProgressMode::Never);
    if !can_interact && !cli.json {
        cli.plain = true;
    }
    match cli.command.clone() {
        Some(Command::Check(args)) => run_check(args),
        Some(Command::Stability(mut args)) => {
            if !can_interact && !args.json {
                args.plain = true;
            }
            run_stability(args).await
        }
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
        None if can_interact && !cli.plain && !cli.json && !cli.run => {
            tui::run_cockpit(TestOptions::from(&cli)).await
        }
        None => run_speedtest(cli).await,
    }
}

async fn run_speedtest(cli: Cli) -> Result<()> {
    let options = TestOptions::from(&cli);
    let engine = options.engine()?;

    let result = if cli.plain || cli.json {
        run_non_interactive(
            &engine,
            Duration::from_secs(cli.timeout),
            cli.progress.enabled_for(cli.json),
        )
        .await?
    } else {
        run_interactive(engine, cli.fps, Duration::from_secs(cli.timeout)).await?
    };

    options.finish(&result)?;
    if cli.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_result(&result)?;
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

    if let Some(path) = &args.output {
        storage::write_stability_json(path, &result)?;
    }
    if !args.no_save {
        storage::persist_stability(&result).context("failed to persist stability history")?;
    }
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_stability(&result)?;
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
        println!(
            "{}",
            msg(
                "No saved speed-test results in the last {0} days.",
                &[format!("{}", args.days)]
            )
        );
        return Ok(());
    }

    let summary = history::summarize(&results, args.days).expect("non-empty history has summary");
    println!(
        "{}",
        msg(
            "SPEEDTEST HISTORY · {0} DAYS · {1} RUNS",
            &[format!("{}", args.days), format!("{}", results.len())]
        )
    );
    println!();
    println!(
        "{}",
        tr("  DATE / UTC         BACKEND       DOWNLOAD     UPLOAD       PING      QUALITY")
    );
    println!(
        "  ─────────────────  ────────────  ───────────  ───────────  ────────  ─────────────"
    );
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
            "{}",
            msg(
                "  {0}  {1}  {2} M  {3} M  {4}ms  {5}",
                &[
                    format!("{:<17}", result.timestamp.format("%Y-%m-%d %H:%M")),
                    format!("{:<12}", truncate(&result.backend, 12)),
                    format!("{:>8.1}", result.download.mbps),
                    format!("{:>8.1}", result.upload.mbps),
                    format!("{:>6.1}", result.latency.idle_ms),
                    quality.to_string()
                ]
            )
        );
    }
    println!();
    println!(
        "{}",
        msg(
            "  Download trend  {0}",
            std::slice::from_ref(&summary.download_sparkline)
        )
    );
    println!(
        "{}",
        msg(
            "  Trend           {0}",
            &[tr(summary.trend.label()).to_string()]
        )
    );
    Ok(())
}

fn run_stats(args: StatsArgs) -> Result<()> {
    let results = storage::load_history_since(args.days)?;
    let Some(summary) = history::summarize(&results, args.days) else {
        if args.json {
            println!("null");
        } else {
            println!(
                "{}",
                msg(
                    "No saved speed-test results in the last {0} days.",
                    &[format!("{}", args.days)]
                )
            );
        }
        return Ok(());
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_stats(&summary)?;
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

    println!(
        "{}",
        msg(
            "DNS PROVIDERS · {0} PROFILES",
            &[format!("{}", dns::PROVIDERS.len())]
        )
    );
    println!();
    println!(
        "{}",
        tr("  ID                       TYPE         DNS53  DoH  DoT  DoQ  DNSSEC")
    );
    println!("  ───────────────────────  ───────────  ─────  ───  ───  ───  ──────");
    for provider in dns::PROVIDERS {
        println!(
            "  {:<23}  {:<11}    {}    {}    {}    {}      {}",
            provider.id,
            tr(provider.category.label()),
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
    println!("{}", tr("DNS CONFIGURATION"));
    println!();
    println!(
        "{}",
        msg(
            "  Interface:      {0}",
            std::slice::from_ref(&state.interface)
        )
    );
    if let Some(device) = &state.device {
        println!(
            "{}",
            msg("  Device/profile: {0}", std::slice::from_ref(device))
        );
    }
    println!(
        "{}",
        msg(
            "  Backend:        {0}",
            std::slice::from_ref(&state.backend)
        )
    );
    println!(
        "{}",
        msg(
            "  Source:         {0}",
            &[tr(state.mode.label()).to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  DNS servers:    {0}",
            &[format_servers(&state.servers).to_string()]
        )
    );
    if let Some(gateway) = state.gateway {
        println!(
            "{}",
            msg("  Gateway:        {0}", &[format!("{}", gateway)])
        );
    }
    println!(
        "{}",
        msg(
            "  IPv6 default:   {0}",
            &[yes(state.ipv6_default_route).to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Writable:       {0}",
            &[yes(state.can_configure()).to_string()]
        )
    );
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
        print_dns_test(&result)?;
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
        print_dns_benchmark(&result)?;
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
        bail!(
            "{} does not expose DNS53 addresses",
            provider.display_name()
        );
    }
    let preflight = dns_custom::test_servers(servers.clone(), 6).await?;
    if preflight.success_rate_percent < 80.0 {
        bail!(
            "refusing to configure {} because preflight DNS success was only {:.0}%",
            provider.display_name(),
            preflight.success_rate_percent
        );
    }

    print_dns_change(&state, &provider.display_name(), &servers)?;
    if args.dry_run {
        println!("{}", tr("DRY RUN · no DNS settings were changed."));
        return Ok(());
    }
    if !args.yes && !confirm("Apply this DNS configuration?")? {
        println!("{}", tr("No changes made."));
        return Ok(());
    }
    apply_dns_change(&state, &servers, &provider.display_name()).await
}

async fn run_dns_optimize(args: DnsOptimizeArgs) -> Result<()> {
    let benchmark = dns::benchmark(dns_profile(args.profile), usize::from(args.queries)).await?;
    print_dns_benchmark(&benchmark)?;
    let winner = benchmark
        .winner()
        .ok_or_else(|| anyhow!("no DNS resolver completed enough queries to select a winner"))?;
    if winner.is_current {
        println!();
        println!(
            "{}",
            tr("◆ Your current DNS already won this resolver league. No change recommended.")
        );
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
    println!("{}", tr("DNS OPTIMIZER RECOMMENDATION"));
    println!(
        "{}",
        msg(
            "  Winner:         {0}",
            &[provider.display_name().to_string()]
        )
    );
    if let Some(latency) = &winner.latency {
        println!(
            "{}",
            msg(
                "  Median / p95:   {0} / {1} ms",
                &[
                    format!("{:.1}", latency.median_ms),
                    format!("{:.1}", latency.p95_ms)
                ]
            )
        );
    }
    print_dns_change(&state, &provider.display_name(), &servers)?;
    if args.dry_run {
        println!(
            "{}",
            tr("DRY RUN · benchmark completed; no DNS settings were changed.")
        );
        return Ok(());
    }
    if !args.yes && !confirm("Apply the recommended DNS configuration?")? {
        println!("{}", tr("No changes made."));
        return Ok(());
    }
    apply_dns_change(&state, &servers, &provider.display_name()).await
}

async fn run_dns_reset(args: DnsResetArgs) -> Result<()> {
    let state = dns::system::inspect(args.interface.as_deref())?;
    println!("{}", tr("DNS RESET"));
    println!(
        "{}",
        msg("  Interface: {0}", std::slice::from_ref(&state.interface))
    );
    println!(
        "{}",
        msg(
            "  Current:   {0}",
            &[format_servers(&state.servers).to_string()]
        )
    );
    println!("{}", tr("  Target:    automatic / DHCP-managed DNS"));
    if args.dry_run {
        println!("{}", tr("DRY RUN · no DNS settings were changed."));
        return Ok(());
    }
    if !state.can_configure() {
        bail!(
            "{} does not support safe automatic DNS reset",
            state.backend
        );
    }
    if !args.yes && !confirm("Reset this interface to automatic DNS?")? {
        println!("{}", tr("No changes made."));
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
    println!(
        "{}",
        tr("✓ DNS returned to automatic configuration and passed post-change resolution.")
    );
    Ok(())
}

async fn run_dns_rollback(args: DnsRollbackArgs) -> Result<()> {
    let backup = dns::load_backup()?;
    println!("{}", tr("DNS ROLLBACK"));
    println!(
        "{}",
        msg(
            "  Snapshot:   {0}",
            &[backup.timestamp.to_rfc3339().to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Interface:  {0}",
            std::slice::from_ref(&backup.state.interface)
        )
    );
    println!(
        "{}",
        msg(
            "  Mode:       {0}",
            &[tr(backup.state.mode.label()).to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Servers:    {0}",
            &[format_servers(&backup.state.servers).to_string()]
        )
    );
    if args.dry_run {
        println!("{}", tr("DRY RUN · rollback snapshot was not applied."));
        return Ok(());
    }
    if !args.yes && !confirm("Restore this DNS snapshot?")? {
        println!("{}", tr("No changes made."));
        return Ok(());
    }
    dns::system::restore(&backup.state)?;
    dns::system::flush_cache();
    dns::verify_system_resolution()
        .await
        .context("rollback was applied but system DNS verification failed")?;
    println!("{}", tr("✓ Previous DNS snapshot restored and verified."));
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
    println!(
        "{}",
        msg("✓ {0} configured successfully.", &[label.to_string()])
    );
    println!(
        "{}",
        tr("✓ Resolver cache flushed and system DNS verification passed.")
    );
    println!("{}", tr("  Rollback: speedtest dns rollback"));
    Ok(())
}

fn run_compare(args: CompareArgs) -> Result<()> {
    let (before, after) = match (&args.before, &args.after) {
        (Some(before), Some(after)) => {
            (storage::read_result(before)?, storage::read_result(after)?)
        }
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
        print_comparison(&comparison)?;
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
        let result = run_non_interactive(&engine, Duration::from_secs(120), false)
            .await
            .context("full doctor speed test failed")?;
        report.attach_speedtest(result);
    }
    if args.json {
        println!("{}", report.pretty_json()?);
    } else {
        print_doctor(&report)?;
    }
    Ok(())
}

async fn run_loss(args: LossArgs) -> Result<()> {
    let result = loss::measure(&args.target, args.count).await?;
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_packet_loss(&result)?;
    }
    Ok(())
}

fn run_wifi(args: WifiArgs) -> Result<()> {
    let result = wifi::inspect(args.interface.as_deref())?;
    if args.json {
        println!("{}", result.pretty_json()?);
    } else {
        print_wifi(&result)?;
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
        print_verify(&report)?;
    }
    Ok(())
}

async fn run_serve(args: ServeArgs) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .context("failed to bind LAN server")?;
    let bound = listener.local_addr()?;
    println!("{}", tr("LAN SPEEDTEST SERVER"));
    println!("{}", msg("  Listening: {0}", &[format!("{}", bound)]));
    println!(
        "{}",
        msg(
            "  Client:    speedtest lan <this-host>:{0}",
            &[format!("{}", bound.port())]
        )
    );
    println!("{}", tr("  Stop with Ctrl+C."));
    if !bound.ip().is_loopback() {
        output::diagnostic(format_args!("Warning: unauthenticated LAN service; use only on a trusted network with firewall rules."))?;
    }
    io::stdout().flush()?;
    lan::serve_listener(listener).await
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
        print_result(&result)?;
    }
    Ok(())
}

async fn run_non_interactive(
    engine: &InternetEngine,
    limit: Duration,
    progress: bool,
) -> Result<TestResult> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let measurement = runtime::deadline(limit, engine.run(tx));
    tokio::pin!(measurement);
    loop {
        tokio::select! {
            result = &mut measurement => return result,
            Some(event) = rx.recv() => {
                if progress {
                    if let EngineEvent::PhaseChanged(phase) = event {
                        output::diagnostic(format_args!("speedtest: {}", tr(&format!("{phase:?}"))))?;
                    }
                }
            }
        }
    }
}

async fn run_interactive(engine: InternetEngine, fps: u16, limit: Duration) -> Result<TestResult> {
    let (tx, rx) = mpsc::unbounded_channel();
    // Both futures are owned here. Cancellation drops the measurement, not a
    // detached JoinHandle; JoinSets inside the engine abort their workers.
    let measurement = runtime::deadline(limit, engine.run(tx));
    tokio::try_join!(measurement, tui::run(rx, fps)).map(|(_, result)| result)
}

async fn run_interactive_stability(
    duration: Duration,
    interval: Duration,
    fps: u16,
) -> Result<StabilityResult> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::try_join!(
        stability::run(duration, interval, Some(tx)),
        tui::run_stability(rx, duration, fps)
    )
    .map(|(_, result)| result)
}

fn run_check(args: CheckArgs) -> Result<()> {
    let result = if args.result == "-" {
        check::read_result(io::stdin().lock())?
    } else {
        let file = std::fs::File::open(&args.result).context("failed to open result file")?;
        check::read_result(file)?
    };
    let limits = check::Thresholds {
        min_download: args.min_download,
        min_upload: args.min_upload,
        max_latency: args.max_latency,
        max_jitter: args.max_jitter,
        max_loaded_latency: args.max_loaded_latency,
        max_age: args.max_age,
    };
    let report = check::evaluate(&result, &limits, chrono::Utc::now())?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{}",
            msg(
                "THRESHOLD CHECK: {0}",
                &[(if report.passed { "PASS" } else { "FAIL" }).to_string()]
            )
        );
        for check in &report.checks {
            let actual = check
                .actual
                .map_or_else(|| "unavailable".to_string(), |n| format!("{n:.2}"));
            println!(
                "  {:4} {:<25} {} {} {} {:.2} {}",
                if check.passed { "PASS" } else { "FAIL" },
                check.metric,
                actual,
                check.unit,
                check.operator,
                check.limit,
                check.unit
            );
        }
    }
    if !report.passed {
        return Err(runtime::Outcome::ThresholdFailed.into());
    }
    Ok(())
}

fn print_result(result: &TestResult) -> Result<()> {
    println!("{}", tr("Speedtest"));
    println!(
        "{}",
        msg(
            "  Backend:       {0}",
            std::slice::from_ref(&result.backend)
        )
    );
    println!(
        "{}",
        msg(
            "  Server:        {0} ({1})",
            &[
                result.server.name.to_string(),
                result.server.host.to_string()
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Download:      {0} Mbps",
            &[format!("{:.1}", result.download.mbps)]
        )
    );
    println!(
        "{}",
        msg(
            "  Upload:        {0} Mbps",
            &[format!("{:.1}", result.upload.mbps)]
        )
    );
    println!(
        "{}",
        msg(
            "  Ping:          {0} ms",
            &[format!("{:.1}", result.latency.idle_ms)]
        )
    );
    println!(
        "{}",
        msg(
            "  Jitter:        {0} ms",
            &[format!("{:.1}", result.latency.jitter_ms)]
        )
    );
    println!(
        "{}",
        msg(
            "  Loaded down:   {0}",
            &[result
                .latency
                .download_loaded_ms
                .map_or_else(|| "n/a".to_string(), |value| format!("{value:.1} ms"))
                .to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Loaded up:     {0}",
            &[result
                .latency
                .upload_loaded_ms
                .map_or_else(|| "n/a".to_string(), |value| format!("{value:.1} ms"))
                .to_string()]
        )
    );
    if let Some(analysis) = &result.analysis {
        println!(
            "{}",
            msg(
                "  Idle p95/p99:  {0} / {1} ms",
                &[
                    format!("{:.1}", analysis.latency.idle.p95_ms),
                    format!("{:.1}", analysis.latency.idle.p99_ms)
                ]
            )
        );
        let quality = &analysis.quality;
        println!(
            "{}",
            msg(
                "  Quality:       {0}/100 {1} ({2} confidence)",
                &[
                    format!("{}", quality.score),
                    quality.grade.label().to_string(),
                    tr(quality.confidence.label()).to_string()
                ]
            )
        );
        if let Some(tier) = quality.tier_label() {
            println!("{}", msg("  Tier:          ◆ {0}", &[tier.to_string()]));
        }
        if let Some(grade) = quality.bufferbloat.grade {
            println!(
                "{}",
                msg(
                    "  Bufferbloat:   {0} (down {1} / up {2})",
                    &[
                        grade.label().to_string(),
                        format_delta(quality.bufferbloat.download_increase_ms).to_string(),
                        format_delta(quality.bufferbloat.upload_increase_ms).to_string()
                    ]
                )
            );
        }
        if let Some(finding) = quality.findings.first() {
            println!(
                "{}",
                msg(
                    "  Diagnosis:     {0}: {1}",
                    &[
                        tr(finding.severity.label()).to_string(),
                        narr(&finding.title).to_string()
                    ]
                )
            );
            println!("                 {}", narr(&finding.evidence));
            if let Some(recommendation) = &finding.recommendation {
                println!(
                    "{}",
                    msg("  Try:           {0}", &[narr(recommendation).to_string()])
                );
            }
        }
    }
    Ok(())
}

fn print_stability(result: &StabilityResult) -> Result<()> {
    println!("{}", tr("Network Stability"));
    println!(
        "{}",
        msg(
            "  Duration:       {0}s",
            &[format!("{}", result.duration_seconds)]
        )
    );
    println!(
        "{}",
        msg(
            "  Probe interval: {0} ms",
            &[format!("{}", result.interval_ms)]
        )
    );
    println!(
        "{}",
        msg(
            "  Probes:         {0} successful / {1} failed",
            &[
                format!("{}", result.successful_probes),
                format!("{}", result.failed_probes)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Availability:   {0}% (HTTP probe availability, not packet loss)",
            &[format!("{:.2}", result.probe_availability_percent)]
        )
    );
    println!(
        "{}",
        msg(
            "  Failure bursts: {0}",
            &[format!("{}", result.failure_bursts)]
        )
    );
    if let Some(latency) = &result.latency {
        println!(
            "{}",
            msg(
                "  Median:         {0} ms",
                &[format!("{:.1}", latency.median_ms)]
            )
        );
        println!(
            "{}",
            msg(
                "  p95 / p99:      {0} / {1} ms",
                &[
                    format!("{:.1}", latency.p95_ms),
                    format!("{:.1}", latency.p99_ms)
                ]
            )
        );
        println!(
            "{}",
            msg(
                "  Max:            {0} ms",
                &[format!("{:.1}", latency.max_ms)]
            )
        );
    }
    println!(
        "{}",
        msg(
            "  Stability:      {0}/100 {1}",
            &[
                format!("{}", result.score),
                result.grade.label().to_string()
            ]
        )
    );
    if let Some(tier) = result.tier_label() {
        println!("{}", msg("  Tier:           ◆ {0}", &[tier.to_string()]));
    }
    Ok(())
}

fn print_stats(summary: &HistorySummary) -> Result<()> {
    println!(
        "{}",
        msg(
            "NETWORK STATS · {0} DAYS",
            &[format!("{}", summary.period_days)]
        )
    );
    println!();
    println!(
        "{}",
        msg("  Runs:              {0}", &[format!("{}", summary.runs)])
    );
    println!(
        "{}",
        msg(
            "  Download:          {0} Mbps median · {1} Mbps best",
            &[
                format!("{:.1}", summary.median_download_mbps),
                format!("{:.1}", summary.best_download_mbps)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Upload:            {0} Mbps median · {1} Mbps best",
            &[
                format!("{:.1}", summary.median_upload_mbps),
                format!("{:.1}", summary.best_upload_mbps)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Ping:              {0} ms median · {1} ms p95",
            &[
                format!("{:.1}", summary.median_ping_ms),
                format!("{:.1}", summary.p95_ping_ms)
            ]
        )
    );
    if let Some(score) = summary.median_quality_score {
        println!(
            "{}",
            msg("  Quality median:    {0}/100", &[format!("{:.0}", score)])
        );
    }
    println!(
        "{}",
        msg(
            "  S-tier runs:       {0} / {1}",
            &[
                format!("{}", summary.s_tier_runs),
                format!("{}", summary.runs)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Trend:             {0}",
            &[tr(summary.trend.label()).to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Download history:  {0}",
            std::slice::from_ref(&summary.download_sparkline)
        )
    );
    if !summary.anomalies.is_empty() {
        println!();
        println!("{}", tr("  ANOMALIES"));
        for anomaly in &summary.anomalies {
            println!(
                "  {}  {}",
                tr(anomaly.severity.label()),
                narr(&anomaly.message)
            );
        }
    }
    Ok(())
}

fn print_dns_test(result: &DnsProviderBenchmark) -> Result<()> {
    println!("{}", tr("DNS HEALTH / SPEED TEST"));
    println!();
    println!(
        "{}",
        msg(
            "  Resolver:       {0} {1}",
            &[
                result.provider_name.to_string(),
                result.profile_name.to_string()
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Servers:        {0}",
            &[format_servers(&result.servers).to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  Queries:        {0} / {1} successful ({2}%)",
            &[
                format!("{}", result.successes),
                format!("{}", result.queries),
                format!("{:.0}", result.success_rate_percent)
            ]
        )
    );
    if let Some(latency) = &result.latency {
        println!(
            "{}",
            msg(
                "  Median:         {0} ms",
                &[format!("{:.1}", latency.median_ms)]
            )
        );
        println!(
            "{}",
            msg(
                "  p95 / p99:      {0} / {1} ms",
                &[
                    format!("{:.1}", latency.p95_ms),
                    format!("{:.1}", latency.p99_ms)
                ]
            )
        );
    }
    println!(
        "{}",
        msg(
            "  DNS score:      {0}/100 {1}",
            &[
                format!("{}", result.score),
                result.grade.label().to_string()
            ]
        )
    );
    if let Some(tier) = result.tier_label() {
        println!("{}", msg("  Tier:           ◆ {0}", &[tier.to_string()]));
    }
    Ok(())
}

fn print_dns_benchmark(result: &DnsBenchmarkResult) -> Result<()> {
    println!(
        "{}",
        msg(
            "DNS BENCHMARK · {0}",
            &[result.profile.to_ascii_uppercase().to_string()]
        )
    );
    println!();
    println!(
        "{}",
        tr("  #  RESOLVER                         MEDIAN     P95   SUCCESS   SCORE")
    );
    println!("  ─  ───────────────────────────────  ───────  ───────  ───────  ─────────────");
    for (index, entry) in result.entries.iter().enumerate() {
        let median = entry.latency.as_ref().map_or_else(
            || "—".to_string(),
            |latency| format!("{:.1}ms", latency.median_ms),
        );
        let p95 = entry.latency.as_ref().map_or_else(
            || "—".to_string(),
            |latency| format!("{:.1}ms", latency.p95_ms),
        );
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
        println!(
            "{}",
            msg(
                "  ◆ WINNER: {0} · {1}/100 {2}",
                &[
                    winner.provider_name.to_string(),
                    format!("{}", winner.score),
                    winner.grade.label().to_string()
                ]
            )
        );
    }
    Ok(())
}

fn print_dns_change(
    state: &dns::system::DnsSystemState,
    label: &str,
    servers: &[std::net::IpAddr],
) -> Result<()> {
    println!("{}", tr("DNS CONFIGURATION CHANGE"));
    println!();
    println!(
        "{}",
        msg(
            "  Interface:      {0}",
            std::slice::from_ref(&state.interface)
        )
    );
    println!(
        "{}",
        msg(
            "  Current:        {0}",
            &[format_servers(&state.servers).to_string()]
        )
    );
    println!("{}", msg("  New provider:   {0}", &[label.to_string()]));
    println!(
        "{}",
        msg(
            "  New servers:    {0}",
            &[format_servers(servers).to_string()]
        )
    );
    Ok(())
}

fn print_comparison(result: &CompareResult) -> Result<()> {
    println!("{}", tr("NETWORK COMPARISON"));
    println!();
    println!(
        "{}",
        tr("                    BEFORE         AFTER        CHANGE")
    );
    print_metric("Download", &result.download_mbps, "Mbps")?;
    print_metric("Upload", &result.upload_mbps, "Mbps")?;
    print_metric("Ping", &result.ping_ms, "ms")?;
    print_metric("Jitter", &result.jitter_ms, "ms")?;
    if let (Some(before), Some(after), Some(change)) = (
        result.quality_score.before,
        result.quality_score.after,
        result.quality_score.absolute_change,
    ) {
        println!(
            "{}",
            msg(
                "  {0} {1}/100 {2}/100 {3} pts",
                &[
                    format!("{:<16}", tr("Quality")),
                    format!("{:>10.0}", before),
                    format!("{:>10.0}", after),
                    format!("{:+.0}", change)
                ]
            )
        );
    }
    println!();
    println!(
        "{}",
        msg(
            "  VERDICT    {0}",
            &[narr(&result.verdict).to_uppercase().to_string()]
        )
    );
    println!(
        "{}",
        msg("  HIGHLIGHT  {0}", &[narr(&result.highlight).to_string()])
    );
    Ok(())
}

fn print_metric(label: &str, metric: &compare::MetricDelta, unit: &str) -> Result<()> {
    println!(
        "  {:<16} {:>10.1} {:<4} {:>10.1} {:<4} {:+.1}%",
        tr(label),
        metric.before,
        unit,
        metric.after,
        unit,
        metric.percent_change.unwrap_or_default()
    );
    Ok(())
}

fn print_doctor(report: &DoctorReport) -> Result<()> {
    println!("{}", tr("NETWORK DOCTOR"));
    if let Some(interface) = &report.interface {
        println!(
            "{}",
            msg("  Interface: {0}", std::slice::from_ref(interface))
        );
    }
    println!();
    for check in &report.checks {
        println!(
            "  {} {:<20} {}",
            check.status.symbol(),
            check.name,
            check.detail
        );
    }
    if let Some(speedtest) = &report.speedtest {
        println!();
        println!("{}", tr("  FULL TEST"));
        println!(
            "{}",
            msg(
                "  Download  {0} Mbps",
                &[format!("{:.1}", speedtest.download.mbps)]
            )
        );
        println!(
            "{}",
            msg(
                "  Upload    {0} Mbps",
                &[format!("{:.1}", speedtest.upload.mbps)]
            )
        );
        println!(
            "{}",
            msg(
                "  Ping      {0} ms",
                &[format!("{:.1}", speedtest.latency.idle_ms)]
            )
        );
        if let Some(analysis) = &speedtest.analysis {
            println!(
                "{}",
                msg(
                    "  Quality   {0}/100 {1}",
                    &[
                        format!("{}", analysis.quality.score),
                        analysis.quality.grade.label().to_string()
                    ]
                )
            );
        }
    }
    println!();
    println!("{}", tr("  DIAGNOSIS"));
    println!("  {}", narr(&report.diagnosis));
    if let Some(recommendation) = &report.recommendation {
        println!(
            "{}",
            msg("  Recommendation: {0}", &[narr(recommendation).to_string()])
        );
    }
    Ok(())
}

fn print_packet_loss(result: &loss::PacketLossResult) -> Result<()> {
    println!("{}", tr("ICMP PACKET LOSS"));
    println!();
    println!(
        "{}",
        msg(
            "  Target:          {0}",
            std::slice::from_ref(&result.target)
        )
    );
    println!(
        "{}",
        msg(
            "  Sent/received:   {0} / {1}",
            &[
                format!("{}", result.packets_sent),
                format!("{}", result.packets_received)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Lost:            {0} ({1}%)",
            &[
                format!("{}", result.packets_lost),
                format!("{:.2}", result.loss_percent)
            ]
        )
    );
    if let Some(rtt) = &result.rtt {
        println!(
            "{}",
            msg(
                "  RTT median:      {0} ms",
                &[format!("{:.1}", rtt.median_ms)]
            )
        );
        println!(
            "{}",
            msg(
                "  RTT p95 / p99:   {0} / {1} ms",
                &[format!("{:.1}", rtt.p95_ms), format!("{:.1}", rtt.p99_ms)]
            )
        );
        println!(
            "{}",
            msg("  RTT max:         {0} ms", &[format!("{:.1}", rtt.max_ms)])
        );
    }
    println!();
    println!(
        "{}",
        msg("  Note: {0}", &[narr(&result.caveat).to_string()])
    );
    Ok(())
}

fn print_wifi(result: &wifi::WifiSnapshot) -> Result<()> {
    println!("{}", tr("WI-FI DIAGNOSTICS"));
    println!();
    if !result.available {
        println!("{}", tr("  No active Wi-Fi link detected."));
        println!("  {}", narr(&result.detail));
        return Ok(());
    }
    println!(
        "{}",
        msg(
            "  Interface:       {0}",
            &[result.interface.as_deref().unwrap_or("unknown").to_string()]
        )
    );
    println!(
        "{}",
        msg(
            "  SSID:            {0}",
            &[result.ssid.as_deref().unwrap_or("unknown").to_string()]
        )
    );
    if let Some(dbm) = result.signal_dbm {
        println!(
            "{}",
            msg("  Signal:          {0} dBm", &[format!("{:.0}", dbm)])
        );
    }
    if let Some(percent) = result.signal_percent {
        println!(
            "{}",
            msg("  Signal quality:  {0}%", &[format!("{:.0}", percent)])
        );
    }
    if let Some(band) = &result.band {
        println!(
            "{}",
            msg("  Band:            {0}", std::slice::from_ref(band))
        );
    }
    if let Some(channel) = result.channel {
        println!(
            "{}",
            msg("  Channel:         {0}", &[format!("{}", channel)])
        );
    }
    if let Some(rate) = result.link_mbps {
        println!(
            "{}",
            msg("  Link rate:       {0} Mbps", &[format!("{:.0}", rate)])
        );
    }
    if let Some(radio) = &result.radio {
        println!(
            "{}",
            msg("  Radio:           {0}", std::slice::from_ref(radio))
        );
    }
    println!(
        "{}",
        msg(
            "  Detail:          {0}",
            &[narr(&result.detail).to_string()]
        )
    );
    Ok(())
}

fn print_verify(report: &verify::VerifyReport) -> Result<()> {
    println!("{}", tr("MULTI-BACKEND VERIFICATION"));
    println!();
    println!("{}", tr("                    CLOUDFLARE     LIBRESPEED"));
    println!(
        "{}",
        msg(
            "  Download        {0} M     {1} M",
            &[
                format!("{:>9.1}", report.cloudflare.download.mbps),
                format!("{:>9.1}", report.librespeed.download.mbps)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Upload          {0} M     {1} M",
            &[
                format!("{:>9.1}", report.cloudflare.upload.mbps),
                format!("{:>9.1}", report.librespeed.upload.mbps)
            ]
        )
    );
    println!(
        "{}",
        msg(
            "  Ping            {0} ms    {1} ms",
            &[
                format!("{:>9.1}", report.cloudflare.latency.idle_ms),
                format!("{:>9.1}", report.librespeed.latency.idle_ms)
            ]
        )
    );
    if let Some(loss) = &report.icmp_loss {
        println!(
            "{}",
            msg(
                "  ICMP loss       {0}%     independent reference",
                &[format!("{:>9.2}", loss.loss_percent)]
            )
        );
    }
    println!();
    println!(
        "{}",
        msg(
            "  Agreement:      {0}",
            &[(if report.consistent {
                "CONSISTENT"
            } else {
                "DIVERGENT"
            })
            .to_string()]
        )
    );
    println!("  {}", narr(&report.verdict));
    println!(
        "{}",
        msg(
            "  Highlight:      {0}",
            &[narr(&report.comparison.highlight).to_string()]
        )
    );
    Ok(())
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
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("DNS changes require an interactive terminal; inspect --dry-run, then use --yes explicitly");
    }
    write!(
        io::stderr().lock(),
        "{} [y/N] ",
        output::safe_text(&tr(prompt))
    )?;
    io::stderr()
        .flush()
        .context("failed to flush confirmation prompt")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read confirmation")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn yes(value: bool) -> &'static str {
    if value {
        "✓"
    } else {
        "—"
    }
}

fn format_servers(servers: &[std::net::IpAddr]) -> String {
    if servers.is_empty() {
        "none detected".to_string()
    } else {
        servers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn format_delta(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("+{value:.1} ms"))
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        value.to_string()
    } else {
        value
            .chars()
            .take(width.saturating_sub(1))
            .collect::<String>()
            + "…"
    }
}
