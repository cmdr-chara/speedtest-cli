//! Black-box CLI contracts. No test here needs Internet access or changes DNS.
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

use chrono::Utc;
use speedtest_cli::monitor::MonitorRecord;

fn run(arguments: &[&str], input: Option<&str>) -> Output {
    let home = tempfile::tempdir().unwrap();
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
