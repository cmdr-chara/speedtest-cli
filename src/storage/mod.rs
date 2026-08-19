use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::model::TestResult;

pub fn persist_default(result: &TestResult) -> Result<(PathBuf, PathBuf)> {
    let root = data_dir().context("could not determine a platform data directory")?;
    let results_dir = root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create results directory")?;

    let filename = result.timestamp.format("%Y%m%dT%H%M%SZ.json").to_string();
    let result_path = results_dir.join(filename);
    write_json(&result_path, result)?;

    let history_path = root.join("history.jsonl");
    append_jsonl(&history_path, result)?;

    Ok((result_path, history_path))
}

pub fn write_json(path: &Path, result: &TestResult) -> Result<()> {
    ensure_parent(path)?;
    let content = result.pretty_json()?;
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn write_csv(path: &Path, result: &TestResult) -> Result<()> {
    ensure_parent(path)?;
    let mut writer = csv::Writer::from_path(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    writer.serialize(CsvRecord::from(result)).context("failed to serialize CSV result")?;
    writer.flush().context("failed to flush CSV result")?;
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).context("failed to create output directory")?;
        }
    }
    Ok(())
}

fn append_jsonl(path: &Path, result: &TestResult) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::to_writer(&mut file, result).context("failed to serialize history record")?;
    file.write_all(b"\n").context("failed to append history record")?;
    Ok(())
}

#[derive(Serialize)]
struct CsvRecord<'a> {
    timestamp: String,
    backend: &'a str,
    server_name: &'a str,
    server_host: &'a str,
    download_mbps: f64,
    upload_mbps: f64,
    idle_latency_ms: f64,
    jitter_ms: f64,
    download_loaded_ms: Option<f64>,
    upload_loaded_ms: Option<f64>,
    packet_loss_percent: Option<f64>,
    download_bytes: u64,
    upload_bytes: u64,
}

impl<'a> From<&'a TestResult> for CsvRecord<'a> {
    fn from(result: &'a TestResult) -> Self {
        Self {
            timestamp: result.timestamp.to_rfc3339(),
            backend: &result.backend,
            server_name: &result.server.name,
            server_host: &result.server.host,
            download_mbps: result.download.mbps,
            upload_mbps: result.upload.mbps,
            idle_latency_ms: result.latency.idle_ms,
            jitter_ms: result.latency.jitter_ms,
            download_loaded_ms: result.latency.download_loaded_ms,
            upload_loaded_ms: result.latency.upload_loaded_ms,
            packet_loss_percent: result.latency.packet_loss_percent,
            download_bytes: result.download.bytes,
            upload_bytes: result.upload.bytes,
        }
    }
}

fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("speedtest"));
    }

    #[cfg(target_os = "macos")]
    {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join("Library/Application Support/speedtest"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_DATA_HOME") {
            return Some(PathBuf::from(path).join("speedtest"));
        }
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join(".local/share/speedtest"));
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use crate::model::{LatencyResult, ServerInfo, ThroughputResult};

    use super::*;

    fn result() -> TestResult {
        TestResult {
            timestamp: Utc::now(),
            backend: "test".into(),
            server: ServerInfo {
                host: "example.test".into(),
                name: "Example".into(),
            },
            latency: LatencyResult {
                idle_ms: 10.0,
                jitter_ms: 1.0,
                download_loaded_ms: Some(20.0),
                upload_loaded_ms: Some(22.0),
                packet_loss_percent: None,
            },
            download: ThroughputResult {
                mbps: 100.0,
                bytes: 1_000,
                seconds: 1.0,
            },
            upload: ThroughputResult {
                mbps: 50.0,
                bytes: 500,
                seconds: 1.0,
            },
        }
    }

    #[test]
    fn writes_json_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        write_json(&path, &result()).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("\"download\""));
    }

    #[test]
    fn writes_csv_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.csv");
        write_csv(&path, &result()).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("download_mbps"));
        assert!(content.contains("100"));
    }
}
