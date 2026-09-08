use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use tempfile::NamedTempFile;

use crate::{model::TestResult, stability::StabilityResult};

pub fn data_root() -> Result<PathBuf> {
    data_dir().context("could not determine a platform data directory")
}

pub fn persist_default(result: &TestResult) -> Result<(PathBuf, PathBuf)> {
    persist_at(&data_root()?, result.timestamp, result)
}

pub fn persist_stability(result: &StabilityResult) -> Result<(PathBuf, PathBuf)> {
    persist_at(&data_root()?.join("stability"), result.timestamp, result)
}

fn persist_at<T: Serialize>(
    root: &Path,
    timestamp: DateTime<Utc>,
    result: &T,
) -> Result<(PathBuf, PathBuf)> {
    let results_dir = root.join("results");
    fs::create_dir_all(&results_dir).context("failed to create results directory")?;
    let stamp = timestamp.format("%Y%m%dT%H%M%S%.9fZ");
    let mut pending = prepared_file(&results_dir, &json_bytes(result)?)?;
    let mut sequence = 0_u64;
    let result_path = loop {
        let filename = if sequence == 0 {
            format!("{stamp}.json")
        } else {
            format!("{stamp}-{sequence}.json")
        };
        let path = results_dir.join(filename);
        match pending.persist_noclobber(&path) {
            Ok(_) => break path,
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                pending = error.file;
                sequence = sequence
                    .checked_add(1)
                    .context("too many result filename collisions")?;
            }
            Err(error) => return Err(error.error).context("failed to publish saved result"),
        }
    };
    let history_path = root.join("history.jsonl");
    append_jsonl_value(&history_path, result)?;
    Ok((result_path, history_path))
}

pub fn load_history() -> Result<Vec<TestResult>> {
    let root = data_root()?;
    load_history_path(&root.join("history.jsonl"))
}

pub fn load_history_since(days: u64) -> Result<Vec<TestResult>> {
    load_history_path_since(&data_root()?.join("history.jsonl"), history_cutoff(days))
}

fn history_cutoff(days: u64) -> DateTime<Utc> {
    i64::try_from(days)
        .ok()
        .and_then(ChronoDuration::try_days)
        .and_then(|duration| Utc::now().checked_sub_signed(duration))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

pub fn read_result(path: &Path) -> Result<TestResult> {
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    crate::check::read_result(file)
        .with_context(|| format!("failed to parse speed-test result from {}", path.display()))
}

pub fn write_json(path: &Path, result: &TestResult) -> Result<()> {
    write_json_value(path, result)
}

pub fn write_stability_json(path: &Path, result: &StabilityResult) -> Result<()> {
    write_json_value(path, result)
}

pub fn write_csv(path: &Path, result: &TestResult) -> Result<()> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .serialize(CsvRecord::from(result))
        .context("failed to serialize CSV result")?;
    let content = writer.into_inner().context("failed to finish CSV result")?;
    atomic_write(path, &content)
}

fn write_json_value<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write(path, &json_bytes(value)?)
}

fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut content =
        serde_json::to_vec_pretty(value).context("failed to serialize JSON output")?;
    content.push(b'\n');
    Ok(content)
}

fn prepared_file(directory: &Path, content: &[u8]) -> Result<NamedTempFile> {
    let mut pending = tempfile::Builder::new()
        .prefix(".speedtest-")
        .tempfile_in(directory)
        .context("failed to create temporary output")?;
    pending
        .write_all(content)
        .context("failed to write temporary output")?;
    pending
        .as_file()
        .sync_all()
        .context("failed to sync output")?;
    Ok(pending)
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    // Follow output symlinks as ordinary file writes do, including a link to a
    // target that has not been created yet. Never replace a device or named pipe.
    let destination = output_destination(path)?;
    let metadata = match fs::metadata(&destination) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("failed to inspect output destination"),
    };
    if metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.is_file())
    {
        return fs::write(&destination, content)
            .with_context(|| format!("failed to write {}", path.display()));
    }
    if metadata
        .as_ref()
        .is_some_and(|metadata| metadata.permissions().readonly())
    {
        bail!("output destination is read-only: {}", path.display());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let pending = prepared_file(parent, content)?;
    if let Some(metadata) = metadata {
        pending
            .as_file()
            .set_permissions(metadata.permissions())
            .context("failed to preserve output permissions")?;
    }
    pending
        .persist(&destination)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn output_destination(path: &Path) -> Result<PathBuf> {
    let mut destination = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target =
                    fs::read_link(&destination).context("failed to resolve output symlink")?;
                destination = if target.is_absolute() {
                    target
                } else {
                    destination.parent().unwrap_or(Path::new(".")).join(target)
                };
            }
            Ok(_) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(destination),
            Err(error) => return Err(error).context("failed to resolve output destination"),
        }
    }
    bail!("too many symbolic links in output destination")
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).context("failed to create output directory")?;
        }
    }
    Ok(())
}

fn append_jsonl_value<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    // Serialize before touching history: serialization failures must leave it intact.
    let mut content = serde_json::to_vec(value).context("failed to serialize history record")?;
    content.push(b'\n');
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    // The file handle owns the lock, including on error. Readers take a shared lock.
    file.lock().context("failed to lock history for writing")?;
    let original_len = file.metadata().context("failed to inspect history")?.len();
    if let Err(error) = file.write_all(&content) {
        file.set_len(original_len)
            .context("failed to undo incomplete history append")?;
        return Err(error).context("failed to append history record");
    }
    file.sync_data().context("failed to sync history record")?;
    Ok(())
}

fn load_history_path(path: &Path) -> Result<Vec<TestResult>> {
    load_history_path_since(path, DateTime::<Utc>::MIN_UTC)
}

fn load_history_path_since(path: &Path, cutoff: DateTime<Utc>) -> Result<Vec<TestResult>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to open {}", path.display()))
        }
    };
    file.lock_shared()
        .context("failed to lock history for reading")?;
    let mut reader = BufReader::new(file);
    let mut results = Vec::new();
    let mut line = Vec::new();
    let mut index = 0_u64;
    loop {
        line.clear();
        index += 1;
        let length = reader
            .by_ref()
            .take(crate::check::MAX_RESULT_BYTES + 1)
            .read_until(b'\n', &mut line)
            .with_context(|| format!("failed reading history line {index}"))?;
        if length == 0 {
            break;
        }
        if length as u64 > crate::check::MAX_RESULT_BYTES {
            bail!("history record on line {index} exceeds the 4 MiB input limit");
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let result = serde_json::from_slice::<TestResult>(&line)
            .with_context(|| format!("invalid history record on line {index}"))?;
        if result.timestamp >= cutoff {
            results.push(result);
        }
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
        append_jsonl_value(&path, &result()).unwrap();
        append_jsonl_value(&path, &earlier).unwrap();

        let loaded = load_history_path(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].timestamp <= loaded[1].timestamp);
    }

    #[test]
    fn repeated_timestamps_preserve_every_saved_result() {
        let dir = tempdir().unwrap();
        let mut saved = result();
        let (first, history) = persist_at(dir.path(), saved.timestamp, &saved).unwrap();
        saved.download.mbps = 250.0;
        let (second, _) = persist_at(dir.path(), saved.timestamp, &saved).unwrap();
        assert_ne!(first, second);
        assert_eq!(read_result(&first).unwrap().download.mbps, 100.0);
        assert_eq!(read_result(&second).unwrap().download.mbps, 250.0);
        assert_eq!(load_history_path(&history).unwrap().len(), 2);
        assert_eq!(fs::read_dir(dir.path().join("results")).unwrap().count(), 2);
    }

    #[test]
    fn concurrent_saves_keep_complete_unique_results_and_history_records() {
        let dir = tempdir().unwrap();
        let saved = result();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let dir = dir.path();
                let barrier = &barrier;
                let mut saved = saved.clone();
                saved.server.name = "large field ".repeat(1024);
                scope.spawn(move || {
                    barrier.wait();
                    for index in 0..8 {
                        saved.download.bytes = worker * 8 + index;
                        persist_at(dir, saved.timestamp, &saved).unwrap();
                    }
                });
            }
        });
        let mut records = load_history_path(&dir.path().join("history.jsonl")).unwrap();
        records.sort_by_key(|result| result.download.bytes);
        assert_eq!(records.len(), 64);
        assert_eq!(
            records
                .iter()
                .map(|result| result.download.bytes)
                .collect::<Vec<_>>(),
            (0..64).collect::<Vec<_>>()
        );
        let files: Vec<_> = fs::read_dir(dir.path().join("results")).unwrap().collect();
        assert_eq!(files.len(), 64);
        for file in files {
            read_result(&file.unwrap().path()).unwrap();
        }
    }

    #[test]
    fn serialization_failure_leaves_previous_export_and_history_intact() {
        struct FailingRecord;
        impl Serialize for FailingRecord {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::{Error, SerializeSeq};
                let mut sequence = serializer.serialize_seq(Some(2))?;
                sequence.serialize_element("partial")?;
                Err(S::Error::custom("intentional serialization failure"))
            }
        }
        let dir = tempdir().unwrap();
        let export = dir.path().join("result.json");
        let history = dir.path().join("history.jsonl");
        write_json(&export, &result()).unwrap();
        append_jsonl_value(&history, &result()).unwrap();
        let original_export = fs::read(&export).unwrap();
        let original_history = fs::read(&history).unwrap();
        assert!(write_json_value(&export, &FailingRecord).is_err());
        assert!(append_jsonl_value(&history, &FailingRecord).is_err());
        assert_eq!(fs::read(&export).unwrap(), original_export);
        assert_eq!(fs::read(&history).unwrap(), original_history);
    }

    #[test]
    fn failed_atomic_replacement_cleans_up_and_keeps_destination() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), "existing content").unwrap();
        assert!(write_json(&destination, &result()).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("keep")).unwrap(),
            "existing content"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn history_ranges_are_bounded_without_losing_timestamp_order() {
        assert_eq!(history_cutoff(u64::MAX), DateTime::<Utc>::MIN_UTC);
        assert_eq!(history_cutoff(i64::MAX as u64), DateTime::<Utc>::MIN_UTC);
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let recent = result();
        let mut old = recent.clone();
        old.timestamp -= ChronoDuration::days(40);
        append_jsonl_value(&path, &recent).unwrap();
        append_jsonl_value(&path, &old).unwrap();
        let filtered = load_history_path_since(&path, history_cutoff(30)).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].timestamp, recent.timestamp);
        assert_eq!(
            load_history_path_since(&path, history_cutoff(u64::MAX))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn oversized_and_corrupt_history_records_fail_with_line_context() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append_jsonl_value(&path, &result()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{broken\n").unwrap();
        assert!(load_history_path(&path)
            .unwrap_err()
            .to_string()
            .contains("line 2"));
        fs::write(
            &path,
            vec![b'x'; crate::check::MAX_RESULT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(load_history_path(&path)
            .unwrap_err()
            .to_string()
            .contains("4 MiB"));
        assert!(read_result(&path)
            .unwrap_err()
            .chain()
            .any(|error| error.to_string().contains("4 MiB")));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_export_preserves_symlinks_permissions_and_device_destinations() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.json");
        let link = dir.path().join("link.json");
        symlink("target.json", &link).unwrap();
        write_json(&link, &result()).unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(read_result(&target).unwrap().download.mbps, 100.0);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        write_json(&link, &result()).unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        write_json(Path::new("/dev/null"), &result()).unwrap();
        assert!(!fs::metadata("/dev/null").unwrap().is_file());
        fs::remove_file(&link).unwrap();
        symlink("link.json", &link).unwrap();
        assert!(write_json(&link, &result()).is_err());
    }
}
