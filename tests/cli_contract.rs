//! Black-box CLI contracts. No test here needs Internet access or changes DNS.
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

use chrono::Utc;
use speedtest_cli::monitor::MonitorRecord;

fn run(arguments: &[&str], input: Option<&str>) -> Output {
    run_with_history(arguments, input, None)
}

fn run_with_history(arguments: &[&str], input: Option<&str>, history: Option<&str>) -> Output {
    let home = tempfile::tempdir().unwrap();
    if let Some(history) = history {
        #[cfg(target_os = "macos")]
        let root = home.path().join("Library/Application Support/speedtest");
        #[cfg(not(target_os = "macos"))]
        let root = home.path().join("speedtest");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("history.jsonl"), history).unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_speedtest"))
        .args(arguments)
        .env("LC_ALL", "C")
        .env("SPEEDTEST_LANGUAGE", "en")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("RUST_BACKTRACE", "0")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .env("LOCALAPPDATA", home.path())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn offline_history_fixture() -> (String, Vec<speedtest_cli::model::TestResult>) {
    let base: speedtest_cli::model::TestResult =
        serde_json::from_str(include_str!("fixtures/result.json")).unwrap();
    let mut results = Vec::new();
    for (minutes, backend, host, download) in [
        (4, "cloudflare", "edge.example", 123.0),
        (3, "cloudflare", "edge.example", 456.0),
        (2, "cloudflare", "other.example", 789.0),
        (1, "lan", "edge.example", 999.0),
    ] {
        let mut result = base.clone();
        result.timestamp = Utc::now() - chrono::Duration::minutes(minutes);
        result.backend = backend.into();
        result.server.host = host.into();
        result.download.mbps = download;
        results.push(result);
    }
    let history = results
        .iter()
        .rev()
        .map(|result| serde_json::to_string(result).unwrap() + "\n")
        .collect::<String>();
    (history, results)
}

#[test]
fn offline_filters_are_consistent_and_table_limits_do_not_trim_json() {
    let (history, _) = offline_history_fixture();
    let filters = [
        "--backend",
        "CLOUDFLARE",
        "--scope",
        "internet",
        "--server",
        "EDGE.EXAMPLE",
    ];
    let mut args = vec!["history", "--json", "--limit", "1"];
    args.extend(filters);
    let result = run_with_history(&args, None, Some(&history));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let selected: Vec<speedtest_cli::model::TestResult> =
        serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].download.mbps, 123.0);
    assert_eq!(selected[1].download.mbps, 456.0);
    assert!(selected[0].timestamp < selected[1].timestamp);

    let mut args = vec!["history", "--limit", "1"];
    args.extend(filters);
    let table = run_with_history(&args, None, Some(&history));
    assert!(table.status.success());
    let table = String::from_utf8(table.stdout).unwrap();
    assert!(table.contains("456.0 M"));
    assert!(!table.contains("123.0 M"));
    for command in ["stats", "insights"] {
        let mut args = vec![command, "--json"];
        args.extend(filters);
        let result = run_with_history(&args, None, Some(&history));
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["runs"], 2);
        if command == "stats" {
            assert_eq!(report["median_download_mbps"], 289.5);
        }
    }
    let unmatched = run_with_history(
        &["history", "--json", "--server", "edge"],
        None,
        Some(&history),
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&unmatched.stdout).unwrap(),
        serde_json::json!([])
    );
}

#[test]
fn history_exports_all_selected_runs_in_canonical_formats() {
    let (history, results) = offline_history_fixture();
    let directory = tempfile::tempdir().unwrap();
    for format in ["json", "jsonl", "csv"] {
        let path = directory.path().join(format!("history.{format}"));
        let result = run_with_history(
            &[
                "history",
                "--json",
                "--limit",
                "1",
                "--backend",
                "cloudflare",
                "--server",
                "edge.example",
                "--output",
                path.to_str().unwrap(),
                "--format",
                format,
            ],
            None,
            Some(&history),
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Vec<serde_json::Value>>(&result.stdout)
                .unwrap()
                .len(),
            2
        );
        let content = std::fs::read_to_string(path).unwrap();
        match format {
            "json" => assert_eq!(
                serde_json::from_str::<serde_json::Value>(&content).unwrap(),
                serde_json::to_value(&results[..2]).unwrap()
            ),
            "jsonl" => {
                let records: Vec<serde_json::Value> = content
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                assert_eq!(
                    records,
                    serde_json::to_value(&results[..2])
                        .unwrap()
                        .as_array()
                        .unwrap()
                        .clone()
                );
                assert!(content.ends_with('\n'));
            }
            "csv" => {
                let mut reader = csv::Reader::from_reader(content.as_bytes());
                let canonical = directory.path().join("canonical.csv");
                speedtest_cli::storage::write_csv(&canonical, &results[0]).unwrap();
                let mut canonical_reader = csv::Reader::from_path(canonical).unwrap();
                assert_eq!(
                    reader.headers().unwrap(),
                    canonical_reader.headers().unwrap()
                );
                let records = reader.records().collect::<Result<Vec<_>, _>>().unwrap();
                assert_eq!(records.len(), 2);
                assert_eq!(&records[0][4], "123.0");
                assert_eq!(&records[1][4], "456.0");
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(
        run(&["history", "--format", "csv"], None).status.code(),
        Some(2)
    );
}

#[test]
fn export_errors_do_not_emit_success_or_replace_existing_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("export.json");
    std::fs::write(&path, "previous export").unwrap();
    let corrupt = run_with_history(
        &["history", "--json", "--output", path.to_str().unwrap()],
        None,
        Some("invalid JSON\n"),
    );
    assert_eq!(corrupt.status.code(), Some(1));
    assert!(corrupt.stdout.is_empty());
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("line 1"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
    let unwritable = run(
        &[
            "history",
            "--json",
            "--output",
            directory.path().to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(unwritable.status.code(), Some(1));
    assert!(unwritable.stdout.is_empty());
}

#[test]
fn metrics_reads_stdin_files_or_latest_selected_history_offline() {
    let (history, results) = offline_history_fixture();
    let selected = run_with_history(
        &[
            "metrics",
            "--backend",
            "cloudflare",
            "--server",
            "edge.example",
        ],
        None,
        Some(&history),
    );
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert!(selected.stderr.is_empty());
    let expected = speedtest_cli::metrics::render(&results[1]).unwrap();
    assert_eq!(String::from_utf8(selected.stdout).unwrap(), expected);
    let input = serde_json::to_string(&results[1]).unwrap();
    let stdin = run(&["metrics", "-"], Some(&input));
    assert!(
        stdin.status.success(),
        "{}",
        String::from_utf8_lossy(&stdin.stderr)
    );
    assert_eq!(String::from_utf8(stdin.stdout).unwrap(), expected);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("result.json");
    let destination = directory.path().join("speedtest.prom");
    std::fs::write(&source, input).unwrap();
    let exported = run(
        &[
            "metrics",
            source.to_str().unwrap(),
            "--output",
            destination.to_str().unwrap(),
        ],
        None,
    );
    assert!(exported.status.success());
    assert!(exported.stdout.is_empty());
    assert_eq!(std::fs::read_to_string(destination).unwrap(), expected);
    let empty = run(&["metrics"], None);
    assert_eq!(empty.status.code(), Some(1));
    assert!(empty.stdout.is_empty());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no saved result matches"));
}

#[cfg(unix)]
#[test]
fn metrics_textfiles_respect_umask_and_preserve_existing_modes() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("result.json");
    std::fs::write(&source, include_str!("fixtures/result.json")).unwrap();
    // Set umask only in the child shell, never in this parallel test process.
    let run_with_umask = |mask: &str, arguments: &[&str]| {
        let output = Command::new("sh")
            .args([
                "-c",
                "umask \"$1\" && shift && exec \"$@\"",
                "speedtest-permissions",
                mask,
            ])
            .arg(env!("CARGO_BIN_EXE_speedtest"))
            .args(arguments)
            .env("HOME", directory.path())
            .env("XDG_DATA_HOME", directory.path())
            .env("SPEEDTEST_LANGUAGE", "en")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

    for (mask, expected) in [("022", 0o644), ("027", 0o640), ("077", 0o600)] {
        let destination = directory.path().join(format!("metrics-{mask}.prom"));
        let output = run_with_umask(
            mask,
            &[
                "metrics",
                source.to_str().unwrap(),
                "--output",
                destination.to_str().unwrap(),
            ],
        );
        assert!(output.stdout.is_empty());
        assert_eq!(mode(&destination), expected);
        assert!(std::fs::read_to_string(destination)
            .unwrap()
            .contains("speedtest_download_bits_per_second"));
    }

    for (existing, mask) in [(0o640, "077"), (0o600, "022")] {
        let destination = directory.path().join("existing.prom");
        std::fs::write(&destination, "previous metrics\n").unwrap();
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(existing)).unwrap();
        run_with_umask(
            mask,
            &[
                "metrics",
                source.to_str().unwrap(),
                "--output",
                destination.to_str().unwrap(),
            ],
        );
        assert_eq!(mode(&destination), existing);
    }

    for format in ["json", "csv", "jsonl"] {
        let destination = directory.path().join(format!("history.{format}"));
        run_with_umask(
            "022",
            &[
                "history",
                "--json",
                "--output",
                destination.to_str().unwrap(),
                "--format",
                format,
            ],
        );
        assert_eq!(mode(&destination), 0o600);
    }
}

#[test]
fn metrics_fail_closed_on_invalid_input_stale_data_or_conflicting_filters() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("speedtest.prom");
    std::fs::write(&destination, "previous metrics\n").unwrap();
    for arguments in [
        vec![
            "metrics",
            "-",
            "--max-age",
            "1",
            "--output",
            destination.to_str().unwrap(),
        ],
        vec![
            "metrics",
            "missing-result.json",
            "--output",
            destination.to_str().unwrap(),
        ],
    ] {
        let input = (arguments[1] == "-").then_some(include_str!("fixtures/result.json"));
        let result = run(&arguments, input);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "previous metrics\n"
        );
    }
    let invalid = run(&["metrics", "-"], Some("{}{}"));
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    for flag in ["--backend", "--server", "--scope"] {
        let conflict = run(&["metrics", "result.json", flag, "all"], None);
        assert_eq!(conflict.status.code(), Some(2));
        assert!(conflict.stdout.is_empty());
    }
    assert_eq!(
        run(&["history", "--server", " "], None).status.code(),
        Some(2)
    );
}

#[test]
fn help_is_discoverable_uncolored_and_documents_units() {
    let result = run(&["--help"], None);
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("check"));
    assert!(text.contains("insights"));
    assert!(text.contains("Examples:"));
    assert!(text.contains("--run"));
    assert!(text.contains("Mbps"));
    assert!(!text.contains('\x1b'));
    assert!(result.stderr.is_empty());
}

#[test]
fn health_commands_expose_family_backend_and_monitor_controls() {
    for (command, expected) in [
        ("diagnose", vec!["--backend", "--family", "--no-stability"]),
        ("monitor", vec!["--backend", "--family", "--interval"]),
        ("verify", vec!["--family", "--compare-families"]),
        ("dns benchmark", vec!["--protocol", "dot", "doq"]),
    ] {
        let arguments = command
            .split_whitespace()
            .chain(["--help"])
            .collect::<Vec<_>>();
        let result = run(&arguments, None);
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let help = String::from_utf8(result.stdout).unwrap();
        for item in expected {
            assert!(help.contains(item), "{command} missing {item}: {help}");
        }
    }
}

#[test]
fn server_catalog_is_offline_and_versioned() {
    let result = run(&["servers", "--json"], None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["backend"], "librespeed");
    assert_eq!(report["probed"], false);
    assert_eq!(report["servers"].as_array().unwrap().len(), 7);
    assert_eq!(report["servers"][0]["id"], 1);
}

#[test]
fn new_automation_and_server_controls_are_discoverable() {
    let help = run(&["--help"], None);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for flag in [
        "--jsonl",
        "--server-id",
        "--exclude-server-id",
        "--source-ip",
    ] {
        assert!(help.contains(flag), "missing {flag}: {help}");
    }
    let server_help = run(&["servers", "--help"], None);
    assert!(server_help.status.success());
    assert!(String::from_utf8(server_help.stdout)
        .unwrap()
        .contains("--probe"));
}

#[test]
fn server_controls_fail_closed_before_network_work() {
    let backend_mismatch = run(
        &[
            "--backend",
            "cloudflare",
            "--server-id",
            "1",
            "--plain",
            "--no-save",
        ],
        None,
    );
    assert_eq!(backend_mismatch.status.code(), Some(2));
    assert!(backend_mismatch.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&backend_mismatch.stderr).contains("require --backend librespeed")
    );

    let unknown = run(
        &[
            "--backend",
            "librespeed",
            "--server-id",
            "999",
            "--plain",
            "--no-save",
        ],
        None,
    );
    assert_eq!(unknown.status.code(), Some(1));
    assert!(unknown.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown LibreSpeed server ID"));

    let excluded = run(
        &[
            "--backend",
            "librespeed",
            "--exclude-server-id",
            "1",
            "--exclude-server-id",
            "2",
            "--exclude-server-id",
            "3",
            "--exclude-server-id",
            "4",
            "--exclude-server-id",
            "5",
            "--exclude-server-id",
            "6",
            "--exclude-server-id",
            "7",
            "--plain",
            "--no-save",
        ],
        None,
    );
    assert_eq!(excluded.status.code(), Some(1));
    assert!(excluded.stdout.is_empty());
    assert!(String::from_utf8_lossy(&excluded.stderr)
        .contains("all built-in LibreSpeed servers were excluded"));
}

#[test]
fn monitor_rejects_a_custom_server_without_the_librespeed_backend() {
    let result = run(
        &[
            "monitor",
            "--librespeed-server",
            "http://127.0.0.1:1",
            "--count",
            "1",
            "--no-save",
        ],
        None,
    );
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
}

#[test]
fn insights_is_offline_and_returns_a_versioned_empty_report() {
    let result = run(&["insights", "--json"], None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["period_days"], 30);
    assert_eq!(report["runs"], 0);
    assert_eq!(report["groups"], serde_json::json!([]));
}

#[test]
fn insights_json_is_language_invariant() {
    let mut baseline = None;
    for language in speedtest_cli::i18n::Language::ALL {
        let result = run(&["insights", "--json", "--language", language.code()], None);
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        if let Some(expected) = &baseline {
            assert_eq!(&result.stdout, expected);
        } else {
            baseline = Some(result.stdout);
        }
    }
}

#[test]
fn monitor_report_is_offline_versioned_and_accepts_custom_jsonl() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("monitor.jsonl");
    let result: speedtest_cli::model::TestResult =
        serde_json::from_str(include_str!("fixtures/result.json")).unwrap();
    let first = MonitorRecord::success(1, Utc::now(), Utc::now(), result);
    let second = MonitorRecord::failure(2, Utc::now(), Utc::now(), "fixture timeout");
    std::fs::write(
        &path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        ),
    )
    .unwrap();

    let result = run(
        &[
            "monitor",
            "--report",
            "--input",
            path.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["attempts"], 2);
    assert_eq!(report["successes"], 1);
    assert_eq!(report["failures"], 1);
    assert_eq!(report["recent_failures"][0], "fixture timeout");
}

#[test]
fn monitor_report_rejects_malformed_records_without_success_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.jsonl");
    std::fs::write(&path, "not json\n").unwrap();
    let result = run(
        &[
            "monitor",
            "--report",
            "--input",
            path.to_str().unwrap(),
            "--json",
        ],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid monitor record"));
}

#[test]
fn check_pass_and_failure_are_json_with_distinct_exit_codes() {
    for (minimum, code, passed) in [("100", 0, true), ("101", 3, false)] {
        let result = run(
            &["check", "-", "--min-download", minimum, "--json"],
            Some(include_str!("fixtures/result.json")),
        );
        assert_eq!(
            result.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["passed"], passed);
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn invalid_json_produces_no_success_record_and_a_structured_error() {
    let result = run(
        &["check", "-", "--max-latency", "20", "--json"],
        Some("not JSON"),
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["error"]["code"], 1);
}

#[test]
fn invalid_thresholds_and_incomplete_comparisons_are_usage_errors() {
    for arguments in [
        vec!["check", "-"],
        vec!["check", "-", "--min-download", "NaN"],
        vec!["check", "-", "--max-latency", "inf"],
        vec!["compare", "before.json"],
        vec!["loss", "--target=-f"],
        vec!["--format", "csv"],
        vec!["--librespeed-server", "http://localhost:1"],
    ] {
        let result = run(&arguments, None);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn global_color_and_progress_work_on_subcommands() {
    for arguments in [
        vec!["--color", "never", "history", "--json"],
        vec!["history", "--color", "never", "--json"],
        vec!["--progress", "never", "dns", "list", "--json"],
    ] {
        let result = run(&arguments, None);
        assert!(
            result.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let _: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn root_flags_are_not_silently_ignored_by_a_subcommand() {
    let result = run(&["--duration", "3", "history"], None);
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn secret_bearing_urls_are_rejected_without_echoing_secrets() {
    let result = run(
        &[
            "--backend",
            "librespeed",
            "--librespeed-server",
            "https://user:sentinel-secret@localhost/",
            "--json",
            "--no-save",
        ],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("sentinel-secret"));
    let _: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
}

#[test]
fn closed_stdout_is_a_quiet_success_not_a_panic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_speedtest"))
        .args(["dns", "list", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The pipe has no reader before output is produced.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn menu_shortcut_does_not_override_machine_output_or_accept_ignored_command_flags() {
    let result = run(&["--run", "history"], None);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    for mode in ["--json", "--plain"] {
        let result = run(
            &[
                "--run",
                mode,
                "--backend",
                "librespeed",
                "--librespeed-server",
                "file:///invalid",
                "--no-save",
            ],
            None,
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(!result.stderr.contains(&0x1b));
        if mode == "--json" {
            let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
            assert_eq!(error["error"]["code"], 1);
        }
    }
}

#[test]
fn eight_language_help_and_human_reports_keep_cli_names_unchanged() {
    use speedtest_cli::i18n::{message, Language};
    for language in Language::ALL {
        let help = run(&["--language", language.code(), "--help"], None);
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("--language"));
        assert!(help.contains("--json"));
        assert!(help.contains("--run"));
        let report = run(&["dns", "list", "--language", language.code()], None);
        assert!(report.status.success());
        assert!(report.stderr.is_empty());
        let report = String::from_utf8(report.stdout).unwrap();
        assert!(report.contains("cloudflare"));
        assert!(
            report.contains(&message(
                language,
                "DNS PROVIDERS · {0} PROFILES",
                &["20".into()]
            )),
            "{}: {report}",
            language.code()
        );
    }
}

#[test]
fn languages_never_change_machine_records_or_exit_codes() {
    let mut baseline = None;
    for language in speedtest_cli::i18n::Language::ALL {
        let output = run(
            &[
                "check",
                "-",
                "--min-download",
                "101",
                "--json",
                "--language",
                language.code(),
            ],
            Some(include_str!("fixtures/result.json")),
        );
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stderr.is_empty());
        if let Some(expected) = &baseline {
            assert_eq!(&output.stdout, expected);
        } else {
            baseline = Some(output.stdout);
        }
        let invalid = run(&["--language", "not-a-language", "--help"], None);
        assert_eq!(invalid.status.code(), Some(2));
        assert!(invalid.stdout.is_empty());
    }
}

#[test]
fn localized_commands_preserve_structured_runtime_errors() {
    let mut baseline = None;
    for language in speedtest_cli::i18n::Language::ALL {
        let result = run(
            &[
                "check",
                "-",
                "--min-download",
                "0",
                "--json",
                "--language",
                language.code(),
            ],
            Some("not valid JSON"),
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        if let Some(expected) = &baseline {
            assert_eq!(&result.stderr, expected);
        } else {
            baseline = Some(result.stderr);
        }
    }
}
