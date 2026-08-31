use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use fs2::FileExt;
use serde::Serialize;

use crate::{model::TestResult, stability::StabilityResult};

pub fn data_root() -> Result<PathBuf> {
    data_dir().context("could not determine a platform data directory")
}

pub fn persist_default(result: &TestResult) -> Result<(PathBuf, PathBuf)> {
    let root = data_root()?;
    let results_dir = root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create results directory")?;

    let result_path = write_unique_timestamped_json(&results_dir, &result.timestamp, result)?;

    let history_path = root.join("history.jsonl");
    append_jsonl(&history_path, result)?;

    Ok((result_path, history_path))
}

pub fn persist_stability(result: &StabilityResult) -> Result<(PathBuf, PathBuf)> {
    let root = data_root()?;
    let stability_root = root.join("stability");
    let results_dir = stability_root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create stability results directory")?;

    let result_path = write_unique_timestamped_json(&results_dir, &result.timestamp, result)?;

    let history_path = stability_root.join("history.jsonl");
    append_jsonl_value(&history_path, result)?;
    Ok((result_path, history_path))
}

pub fn load_history() -> Result<Vec<TestResult>> {
    let root = data_root()?;
    load_history_path(&root.join("history.jsonl"))
}

pub fn load_history_since(days: u64) -> Result<Vec<TestResult>> {
    let now = Utc::now();
    let cutoff = now - ChronoDuration::days(days.min(i64::MAX as u64) as i64);
    let mut results: Vec<_> = load_history()?
        .into_iter()
        .filter(|result| {
            result.timestamp >= cutoff && is_plausible_history_timestamp(&result.timestamp, &now)
        })
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

fn write_unique_timestamped_json<T: Serialize>(
    directory: &Path,
    timestamp: &chrono::DateTime<Utc>,
    value: &T,
) -> Result<PathBuf> {
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let mut content =
        serde_json::to_vec_pretty(value).context("failed to serialize JSON output")?;
    content.push(b'\n');
    let stem = timestamp.format("%Y%m%dT%H%M%SZ").to_string();

    for collision in 0_u64.. {
        let filename = if collision == 0 {
            format!("{stem}.json")
        } else {
            format!("{stem}-{collision}.json")
        };
        let path = directory.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&content)
                    .with_context(|| format!("failed to write {}", path.display()))?;
                file.flush()
                    .with_context(|| format!("failed to flush {}", path.display()))?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to create {}", path.display()));
            }
        }
    }

    unreachable!("u64 filename collision counter is exhaustive")
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
    let mut record = serde_json::to_vec(value).context("failed to serialize history record")?;
    record.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    FileExt::lock_exclusive(&file).with_context(|| format!("failed to lock {}", path.display()))?;
    if let Err(error) = file.write_all(&record) {
        let _ = FileExt::unlock(&file);
        return Err(error).context("failed to append history record");
    }
    if let Err(error) = file.flush() {
        let _ = FileExt::unlock(&file);
        return Err(error).context("failed to flush history record");
    }
    FileExt::unlock(&file).with_context(|| format!("failed to unlock {}", path.display()))?;
    Ok(())
}

fn load_history_path(path: &Path) -> Result<Vec<TestResult>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    FileExt::lock_shared(&file)
        .with_context(|| format!("failed to lock {} for reading", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut results = Vec::new();
    let now = Utc::now();
    for (index, line) in (&mut reader).split(b'\n').enumerate() {
        let line = line.with_context(|| format!("failed reading history line {}", index + 1))?;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<TestResult>(&line) {
            Ok(result) => {
                if !is_plausible_history_timestamp(&result.timestamp, &now) {
                    eprintln!(
                        "warning: retaining history record on line {} in {} even though timestamp {} is implausibly far in the future; time-window views will ignore it",
                        index + 1,
                        path.display(),
                        result.timestamp.to_rfc3339()
                    );
                }
                results.push(result);
            }
            Err(error) => eprintln!(
                "warning: skipping invalid history record on line {} in {}: {error}",
                index + 1,
                path.display()
            ),
        }
    }
    FileExt::unlock(reader.get_ref())
        .with_context(|| format!("failed to unlock {} after reading", path.display()))?;
    results.sort_by_key(|result| result.timestamp);
    Ok(results)
}

fn is_plausible_history_timestamp(
    timestamp: &chrono::DateTime<Utc>,
    now: &chrono::DateTime<Utc>,
) -> bool {
    *timestamp <= *now + ChronoDuration::minutes(5)
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
        return absolute_env_path(env::var_os("LOCALAPPDATA")).map(|path| path.join("speedtest"));
    }

    #[cfg(target_os = "macos")]
    {
        return absolute_env_path(env::var_os("HOME"))
            .map(|path| path.join("Library/Application Support/speedtest"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return unix_data_dir(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"));
    }

    #[allow(unreachable_code)]
    None
}

fn absolute_env_path(value: Option<OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unix_data_dir(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    absolute_env_path(xdg_data_home)
        .map(|path| path.join("speedtest"))
        .or_else(|| absolute_env_path(home).map(|path| path.join(".local/share/speedtest")))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{mpsc, Arc, Barrier},
        thread,
        time::Duration,
    };

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

    #[test]
    fn concurrent_history_appends_preserve_every_record() {
        const WRITERS: usize = 64;

        let dir = tempdir().unwrap();
        let path = Arc::new(dir.path().join("history.jsonl"));
        let barrier = Arc::new(Barrier::new(WRITERS));
        let handles: Vec<_> = (0..WRITERS)
            .map(|index| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let mut record = result();
                    record.backend = format!("writer-{index}");
                    barrier.wait();
                    append_jsonl(&path, &record).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let loaded = load_history_path(&path).unwrap();
        assert_eq!(loaded.len(), WRITERS);
        let backends: HashSet<_> = loaded
            .iter()
            .map(|record| record.backend.as_str())
            .collect();
        assert_eq!(backends.len(), WRITERS);
    }

    #[test]
    fn history_reader_waits_for_an_in_progress_locked_record() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let serialized = serde_json::to_vec(&result()).unwrap();
        let split = serialized.len() / 2;
        let writer_path = path.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(writer_path)
                .unwrap();
            FileExt::lock_exclusive(&file).unwrap();
            file.write_all(&serialized[..split]).unwrap();
            file.flush().unwrap();
            started_tx.send(()).unwrap();
            thread::sleep(Duration::from_millis(25));
            file.write_all(&serialized[split..]).unwrap();
            file.write_all(b"\n").unwrap();
            file.flush().unwrap();
            FileExt::unlock(&file).unwrap();
        });

        started_rx.recv().unwrap();
        let loaded = load_history_path(&path).unwrap();
        writer.join().unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn identical_timestamps_create_distinct_result_files() {
        let dir = tempdir().unwrap();
        let first = result();
        let mut second = first.clone();
        second.download.mbps = 200.0;

        let first_path =
            write_unique_timestamped_json(dir.path(), &first.timestamp, &first).unwrap();
        let second_path =
            write_unique_timestamped_json(dir.path(), &second.timestamp, &second).unwrap();

        assert_ne!(first_path, second_path);
        assert_eq!(read_result(&first_path).unwrap().download.mbps, 100.0);
        assert_eq!(read_result(&second_path).unwrap().download.mbps, 200.0);
    }

    #[test]
    fn corrupt_history_line_does_not_hide_valid_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let first = result();
        let mut second = result();
        second.backend = "second".to_string();
        let content = format!(
            "{}\nnot valid json\n{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        fs::write(&path, content).unwrap();

        let loaded = load_history_path(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|record| record.backend == "test"));
        assert!(loaded.iter().any(|record| record.backend == "second"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn invalid_xdg_data_home_falls_back_only_to_absolute_home() {
        let fallback = unix_data_dir(
            Some(OsString::from("relative/xdg")),
            Some(OsString::from("/home/example")),
        );
        assert_eq!(
            fallback,
            Some(PathBuf::from("/home/example/.local/share/speedtest"))
        );
        assert_eq!(
            unix_data_dir(Some(OsString::new()), Some(OsString::from("relative-home"))),
            None
        );
    }

    #[test]
    fn implausibly_future_history_timestamp_is_retained_in_base_history() {
        let now = Utc::now();
        assert!(is_plausible_history_timestamp(
            &(now + ChronoDuration::minutes(5)),
            &now
        ));
        assert!(!is_plausible_history_timestamp(
            &(now + ChronoDuration::minutes(6)),
            &now
        ));

        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let mut future = result();
        future.timestamp = now + ChronoDuration::days(365);
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&future).unwrap()),
        )
        .unwrap();

        let loaded = load_history_path(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].timestamp, future.timestamp);
    }
}
