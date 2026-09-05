//! Black-box CLI contracts. No test here needs Internet access or changes DNS.
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn run(arguments: &[&str], input: Option<&str>) -> Output {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_speedtest"))
        .args(arguments)
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
    assert!(text.contains("Examples:"));
    assert!(text.contains("--run"));
    assert!(text.contains("Mbps"));
    assert!(!text.contains('\x1b'));
    assert!(result.stderr.is_empty());
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
