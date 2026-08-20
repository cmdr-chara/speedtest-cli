use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Serialize;

use crate::{model::TestResult, stability::StabilityResult};

pub fn data_root() -> Result<PathBuf> {
    data_dir().context("could not determine a platform data directory")
}

pub fn persist_default(result: &TestResult) -> Result<(PathBuf, PathBuf)> {
    let root = data_root()?;
    let results_dir = root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create results directory")?;

    let filename = result.timestamp.format("%Y%m%dT%H%M%SZ.json").to_string();
    let result_path = results_dir.join(filename);
    write_json(&result_path, result)?;

    let history_path = root.join("history.jsonl");
    append_jsonl(&history_path, result)?;

    Ok((result_path, history_path))
}

pub fn persist_stability(result: &StabilityResult) -> Result<(PathBuf, PathBuf)> {
    let root = data_root()?;
    let stability_root = root.join("stability");
    let results_dir = stability_root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create stability results directory")?;

    let filename = result.timestamp.format("%Y%m%dT%H%M%SZ.json").to_string();
    let result_path = results_dir.join(filename);
    write_stability_json(&result_path, result)?;

    let history_path = stability_root.join("history.jsonl");
    append_jsonl_value(&history_path, result)?;
    Ok((result_path, history_path))
}

pub fn load_history() -> Result<Vec<TestResult>> {
    let root = data_root()?;
    load_history_path(&root.join("history.jsonl"))
}

pub fn load_history_since(days: u64) -> Result<Vec<TestResult>> {
    let cutoff = Utc::now() - ChronoDuration::days(days.min(i64::MAX as u64) as i64);
    let mut results: Vec<_> = load_history()?
        .into_iter()
        .filter(|result| result.timestamp >= cutoff)
        .collect();
    results.sort_by_key(|result| result.timestamp);
    Ok(results)
}

pub fn read_result(path: &Path) -> Result<TestResult> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse speed-test result from {}", path.display()))
}

pub fn write_json(path: &Path, result: &TestResult) -> Result<()> {
    write_json_value(path, result)
}

pub fn write_stability_json(path: &Path, result: &StabilityResult) -> Result<()> {
    write_json_value(path, result)
}

pub fn write_csv(path: &Path, result: &TestResult) -> Result<()> {
    ensure_parent(path)?;
    let mut writer = csv::Writer::from_path(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    writer
        .serialize(CsvRecord::from(result))
        .context("failed to serialize CSV result")?;
    writer.flush().context("failed to flush CSV result")?;
    Ok(())
}

fn write_json_value<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_parent(path)?;
    let content = serde_json::to_string_pretty(value).context("failed to serialize JSON output")?;
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
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
    append_jsonl_value(path, result)
}

fn append_jsonl_value<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::to_writer(&mut file, value).context("failed to serialize history record")?;
    file.write_all(b"\n")
        .context("failed to append history record")?;
    Ok(())
}

fn load_history_path(path: &Path) -> Result<Vec<TestResult>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut results = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("failed reading history line {}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let result = serde_json::from_str::<TestResult>(&line)
            .with_context(|| format!("invalid history record on line {}", index + 1))?;
        results.push(result);
    }
    results.sort_by_key(|result| result.timestamp);
    Ok(results)
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
    idle_p95_ms: Option<f64>,
    idle_p99_ms: Option<f64>,
    jitter_p95_ms: Option<f64>,
    jitter_p99_ms: Option<f64>,
    download_loaded_ms: Option<f64>,
    upload_loaded_ms: Option<f64>,
    packet_loss_percent: Option<f64>,
    quality_score: Option<u8>,
    quality_grade: Option<&'static str>,
    quality_confidence: Option<&'static str>,
    bufferbloat_grade: Option<&'static str>,
    download_bufferbloat_ms: Option<f64>,
    upload_bufferbloat_ms: Option<f64>,
    gaming_grade: Option<&'static str>,
    video_calls_grade: Option<&'static str>,
    streaming_grade: Option<&'static str>,
    cloud_gaming_grade: Option<&'static str>,
    primary_diagnosis: Option<&'a str>,
    download_bytes: u64,
    upload_bytes: u64,
}

impl<'a> From<&'a TestResult> for CsvRecord<'a> {
    fn from(result: &'a TestResult) -> Self {
        let analysis = result.analysis.as_ref();
        let latency_analysis = analysis.map(|analysis| &analysis.latency);
        let quality = analysis.map(|analysis| &analysis.quality);

        Self {
            timestamp: result.timestamp.to_rfc3339(),
            backend: &result.backend,
            server_name: &result.server.name,
            server_host: &result.server.host,
            download_mbps: result.download.mbps,
            upload_mbps: result.upload.mbps,
            idle_latency_ms: result.latency.idle_ms,
            jitter_ms: result.latency.jitter_ms,
            idle_p95_ms: latency_analysis.map(|latency| latency.idle.p95_ms),
            idle_p99_ms: latency_analysis.map(|latency| latency.idle.p99_ms),
            jitter_p95_ms: latency_analysis
                .and_then(|latency| latency.jitter.as_ref())
                .map(|jitter| jitter.p95_ms),
            jitter_p99_ms: latency_analysis
                .and_then(|latency| latency.jitter.as_ref())
                .map(|jitter| jitter.p99_ms),
            download_loaded_ms: result.latency.download_loaded_ms,
            upload_loaded_ms: result.latency.upload_loaded_ms,
            packet_loss_percent: result.latency.packet_loss_percent,
            quality_score: quality.map(|quality| quality.score),
            quality_grade: quality.map(|quality| quality.grade.label()),
            quality_confidence: quality.map(|quality| quality.confidence.label()),
            bufferbloat_grade: quality
                .and_then(|quality| quality.bufferbloat.grade)
                .map(|grade| grade.label()),
            download_bufferbloat_ms: quality
                .and_then(|quality| quality.bufferbloat.download_increase_ms),
            upload_bufferbloat_ms: quality
                .and_then(|quality| quality.bufferbloat.upload_increase_ms),
            gaming_grade: quality.map(|quality| quality.workloads.gaming.label()),
            video_calls_grade: quality.map(|quality| quality.workloads.video_calls.label()),
            streaming_grade: quality.map(|quality| quality.workloads.streaming.label()),
            cloud_gaming_grade: quality.map(|quality| quality.workloads.cloud_gaming.label()),
            primary_diagnosis: quality
                .and_then(|quality| quality.findings.first())
                .map(|finding| finding.title.as_str()),
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
            analysis: None,
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
        assert!(content.contains("quality_score"));
        assert!(content.contains("download_mbps"));
        assert!(content.contains("100"));
    }

    #[test]
    fn reads_saved_result() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("result.json");
        write_json(&path, &result()).unwrap();
        let loaded = read_result(&path).unwrap();
        assert_eq!(loaded.download.mbps, 100.0);
    }

    #[test]
    fn loads_jsonl_history_in_timestamp_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut earlier = result();
        earlier.timestamp -= ChronoDuration::minutes(1);
        append_jsonl(&path, &result()).unwrap();
        append_jsonl(&path, &earlier).unwrap();

        let loaded = load_history_path(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].timestamp <= loaded[1].timestamp);
    }
}
