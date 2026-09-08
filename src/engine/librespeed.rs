use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use tokio::{
    sync::mpsc::UnboundedSender,
    task::JoinSet,
    time::{sleep, sleep_until, Instant},
};

use crate::{
    analysis,
    engine::{finish_phase, http, EngineConfig, EngineEvent},
    model::{ServerInfo, TestPhase, TestResult, ThroughputResult},
};

const IDLE_SAMPLES: usize = 20;
const DOWNLOAD_CHUNK_MB: u32 = 32;
const UPLOAD_CHUNK_BYTES: usize = 64 * 1024;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const LOADED_LATENCY_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy)]
pub struct LibreSpeedServer {
    pub name: &'static str,
    pub base: &'static str,
    pub download_path: &'static str,
    pub upload_path: &'static str,
    pub ping_path: &'static str,
}

pub const PUBLIC_SERVERS: &[LibreSpeedServer] = &[
    LibreSpeedServer {
        name: "Nuremberg, Germany (LibreSpeed)",
        base: "https://de4.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Nuremberg, Germany (LibreSpeed 2)",
        base: "https://de3.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Nottingham, UK (LibreSpeed)",
        base: "https://uk1.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Vilnius, Lithuania (LibreSpeed)",
        base: "https://lt1.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Bangalore, India (LibreSpeed)",
        base: "https://in1.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Johannesburg, South Africa (LibreSpeed)",
        base: "https://za1.backend.librespeed.org/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
    LibreSpeedServer {
        name: "Rome, Italy (GARR)",
        base: "https://st-be-rm2.infra.garr.it/",
        download_path: "garbage.php",
        upload_path: "empty.php",
        ping_path: "empty.php",
    },
];

#[derive(Clone)]
pub struct LibreSpeedEngine {
    client: Client,
    config: EngineConfig,
    server: Option<ResolvedServer>,
}

#[derive(Debug, Clone)]
struct ResolvedServer {
    name: String,
    base: Url,
    download_path: String,
    upload_path: String,
    ping_path: String,
}

impl LibreSpeedEngine {
    pub fn new(config: EngineConfig, custom_server: Option<&str>) -> Result<Self> {
        config.validate()?;
        let client = Client::builder()
            .user_agent(concat!("speedtest-cli/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(25))
            .pool_max_idle_per_host(config.streams.saturating_add(4))
            .build()
            .context("failed to build LibreSpeed HTTP client")?;
        let server = custom_server.map(resolve_custom_server).transpose()?;
        Ok(Self {
            client,
            config,
            server,
        })
    }

    pub async fn run(&self, tx: UnboundedSender<EngineEvent>) -> Result<TestResult> {
        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Preparing));
        let server = match &self.server {
            Some(server) => server.clone(),
            None => select_public_server(&self.client).await?,
        };

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Latency));
        let idle_samples = measure_latency(&self.client, &server, IDLE_SAMPLES).await?;
        let latency_preview = analysis::summarize_latency(&idle_samples, &[], &[], None);
        self.emit(
            &tx,
            EngineEvent::IdleLatency {
                ping_ms: latency_preview.idle_ms,
                jitter_ms: latency_preview.jitter_ms,
            },
        );

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Download));
        let (download, download_loaded) = self.measure_download(&server, &tx).await?;
        sleep(Duration::from_millis(400)).await;

        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Upload));
        let (upload, upload_loaded) = self.measure_upload(&server, &tx).await?;

        let latency =
            analysis::summarize_latency(&idle_samples, &download_loaded, &upload_loaded, None);
        let network_analysis = analysis::build_network_analysis(
            &idle_samples,
            &download_loaded,
            &upload_loaded,
            &latency,
            &download,
            &upload,
        );
        let result = TestResult {
            timestamp: Utc::now(),
            backend: "librespeed".to_string(),
            server: ServerInfo {
                host: server.base.as_str().to_string(),
                name: server.name,
            },
            latency,
            download,
            upload,
            analysis: Some(network_analysis),
        };
        self.emit(&tx, EngineEvent::PhaseChanged(TestPhase::Complete));
        self.emit(&tx, EngineEvent::Complete(result.clone()));
        Ok(result)
    }

    async fn measure_download(
        &self,
        server: &ResolvedServer,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Vec<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let deadline = Instant::now() + self.config.phase_duration;
        let mut workers = JoinSet::new();
        for _ in 0..self.config.streams.max(1) {
            workers.spawn(download_worker(
                self.client.clone(),
                server.clone(),
                Arc::clone(&total),
                deadline,
            ));
        }
        let loaded_samples = finish_phase(
            workers,
            self.sample_transfer(TestPhase::Download, Arc::clone(&total), deadline, tx),
            measure_loaded_latency(self.client.clone(), server.clone(), deadline),
        )
        .await?;
        let bytes = total.load(Ordering::Relaxed);
        if bytes == 0 {
            return Err(anyhow!("LibreSpeed server delivered no download data"));
        }
        emit_loaded(&loaded_samples, TestPhase::Download, tx);
        Ok((
            ThroughputResult {
                mbps: mbps(bytes, self.config.phase_duration.as_secs_f64()),
                bytes,
                seconds: self.config.phase_duration.as_secs_f64(),
            },
            loaded_samples,
        ))
    }

    async fn measure_upload(
        &self,
        server: &ResolvedServer,
        tx: &UnboundedSender<EngineEvent>,
    ) -> Result<(ThroughputResult, Vec<f64>)> {
        let total = Arc::new(AtomicU64::new(0));
        let deadline = Instant::now() + self.config.phase_duration;
        let mut workers = JoinSet::new();
        for _ in 0..self.config.streams.max(1) {
            workers.spawn(upload_worker(
                self.client.clone(),
                server.clone(),
                Arc::clone(&total),
                UPLOAD_CHUNK_BYTES,
                deadline,
            ));
        }
        let loaded_samples = finish_phase(
            workers,
            self.sample_transfer(TestPhase::Upload, Arc::clone(&total), deadline, tx),
            measure_loaded_latency(self.client.clone(), server.clone(), deadline),
        )
        .await?;
        let bytes = total.load(Ordering::Relaxed);
        if bytes == 0 {
            return Err(anyhow!("LibreSpeed server accepted no upload data"));
        }
        emit_loaded(&loaded_samples, TestPhase::Upload, tx);
        Ok((
            ThroughputResult {
                mbps: mbps(bytes, self.config.phase_duration.as_secs_f64()),
                bytes,
                seconds: self.config.phase_duration.as_secs_f64(),
            },
            loaded_samples,
        ))
    }

    async fn sample_transfer(
        &self,
        phase: TestPhase,
        total: Arc<AtomicU64>,
        deadline: Instant,
        tx: &UnboundedSender<EngineEvent>,
    ) {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous_bytes = 0_u64;
        let mut previous_at = Instant::now();
        loop {
            interval.tick().await;
            let now = Instant::now();
            let current = total.load(Ordering::Relaxed);
            let delta_seconds = now.duration_since(previous_at).as_secs_f64();
            if delta_seconds > 0.0 {
                self.emit(
                    tx,
                    EngineEvent::ThroughputSample {
                        phase,
                        mbps: mbps(current.saturating_sub(previous_bytes), delta_seconds),
                    },
                );
            }
            previous_bytes = current;
            previous_at = now;
            if now >= deadline {
                break;
            }
        }
    }

    fn emit(&self, tx: &UnboundedSender<EngineEvent>, event: EngineEvent) {
        let _ = tx.send(event);
    }
}

fn resolve_custom_server(base: &str) -> Result<ResolvedServer> {
    let base = http::base_url(base)?;
    Ok(ResolvedServer {
        name: "Custom LibreSpeed server".to_string(),
        base,
        download_path: "garbage.php".to_string(),
        upload_path: "empty.php".to_string(),
        ping_path: "empty.php".to_string(),
    })
}

fn resolve_builtin(server: LibreSpeedServer) -> Result<ResolvedServer> {
    Ok(ResolvedServer {
        name: server.name.to_string(),
        base: Url::parse(server.base).context("invalid built-in LibreSpeed URL")?,
        download_path: server.download_path.to_string(),
        upload_path: server.upload_path.to_string(),
        ping_path: server.ping_path.to_string(),
    })
}

async fn select_public_server(client: &Client) -> Result<ResolvedServer> {
    let mut workers = JoinSet::new();
    for server in PUBLIC_SERVERS.iter().copied() {
        let client = client.clone();
        workers.spawn(async move {
            let resolved = resolve_builtin(server)?;
            let samples = measure_latency(&client, &resolved, 3).await?;
            let median = analysis::distribution(&samples)
                .map(|stats| stats.median_ms)
                .unwrap_or(f64::INFINITY);
            Result::<_, anyhow::Error>::Ok((resolved, median))
        });
    }
    let mut candidates = Vec::new();
    while let Some(result) = workers.join_next().await {
        if let Ok(Ok(candidate)) = result {
            candidates.push(candidate);
        }
    }
    candidates
        .into_iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(server, _)| server)
        .ok_or_else(|| anyhow!("no built-in LibreSpeed server was reachable"))
}

async fn measure_latency(
    client: &Client,
    server: &ResolvedServer,
    count: usize,
) -> Result<Vec<f64>> {
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let url = server.base.join(&server.ping_path)?;
        let started = Instant::now();
        let response = client
            .get(url)
            .query(&[
                ("cors", "true"),
                ("r", &format!("{index}-{}", Utc::now().timestamp_micros())),
            ])
            .header("cache-control", "no-store")
            .send()
            .await
            .context("LibreSpeed latency request failed")?
            .error_for_status()
            .context("LibreSpeed latency endpoint returned an error")?;
        http::drain(response, 64 * 1024).await?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        sleep(Duration::from_millis(75)).await;
    }
    Ok(samples)
}

async fn download_worker(
    client: Client,
    server: ResolvedServer,
    total: Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut chunk_mb = DOWNLOAD_CHUNK_MB;
    while Instant::now() < deadline {
        let url = server.base.join(&server.download_path)?;
        let request = client
            .get(url)
            .query(&[
                ("ckSize", chunk_mb.to_string()),
                ("cors", "true".to_string()),
                ("r", Utc::now().timestamp_micros().to_string()),
            ])
            .header("cache-control", "no-store")
            .send();
        let response = tokio::select! {
            value = request => value.context("LibreSpeed download request failed")?,
            _ = sleep_until(deadline) => break,
        };
        if matches!(
            response.status(),
            StatusCode::PAYLOAD_TOO_LARGE | StatusCode::FORBIDDEN
        ) && chunk_mb > 4
        {
            chunk_mb = (chunk_mb / 2).max(4);
            continue;
        }
        let response =
            http::success(response).context("LibreSpeed download endpoint returned an error")?;
        let mut body = response.bytes_stream();
        let mut request_bytes = 0u64;
        loop {
            let next = tokio::select! {
                value = body.next() => value,
                _ = sleep_until(deadline) => None,
            };
            match next {
                Some(Ok(chunk)) => {
                    request_bytes += chunk.len() as u64;
                    total.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }
                Some(Err(error)) => return Err(error).context("LibreSpeed download stream failed"),
                None => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        anyhow::ensure!(
            request_bytes > 0 || Instant::now() >= deadline,
            "LibreSpeed returned an empty download body"
        );
    }
    Ok(())
}

async fn upload_worker(
    client: Client,
    server: ResolvedServer,
    total: Arc<AtomicU64>,
    initial_payload: usize,
    deadline: Instant,
) -> Result<()> {
    let mut payload_len = initial_payload;
    while Instant::now() < deadline {
        let submitted = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let url = server.base.join(&server.upload_path)?;
        let response = tokio::select! {
            biased;
            _ = sleep_until(deadline) => break,
            value = client
                .post(url)
                .query(&[("cors", "true"), ("r", &Utc::now().timestamp_micros().to_string())])
                .header("content-type", "application/octet-stream")
                .header(reqwest::header::CONTENT_LENGTH, payload_len)
                .body(http::upload_body(Arc::clone(&submitted), payload_len))
                .send() => value,
        };
        if Instant::now() >= deadline {
            break;
        }
        let response = response.context("LibreSpeed upload request failed")?;
        if response.status() == StatusCode::PAYLOAD_TOO_LARGE && payload_len > 16 * 1024 {
            payload_len = (payload_len / 2).max(16 * 1024);
            continue;
        }
        let response =
            http::success(response).context("LibreSpeed upload endpoint returned an error")?;
        if !http::complete_upload(
            http::drain(response, 1024 * 1024),
            &submitted,
            payload_len,
            &total,
            deadline,
        )
        .await?
        {
            break;
        }
        if started.elapsed() < Duration::from_millis(250) {
            payload_len = (payload_len * 2).min(8 * 1024 * 1024);
        } else if started.elapsed() > Duration::from_secs(1) {
            payload_len = (payload_len / 2).max(16 * 1024);
        }
    }
    Ok(())
}

async fn measure_loaded_latency(
    client: Client,
    server: ResolvedServer,
    deadline: Instant,
) -> Vec<f64> {
    let mut samples = Vec::new();
    let mut index = 0_u64;
    while Instant::now() < deadline {
        let url = match server.base.join(&server.ping_path) {
            Ok(url) => url,
            Err(_) => break,
        };
        let started = Instant::now();
        let response = tokio::select! {
            value = client
                .get(url)
                .query(&[("cors", "true"), ("r", &format!("load-{index}-{}", Utc::now().timestamp_micros()))])
                .header("cache-control", "no-store")
                .send() => value.ok(),
            _ = sleep_until(deadline) => None,
        };
        if let Some(response) = response {
            if matches!(
                tokio::time::timeout_at(deadline, http::drain(response, 64 * 1024)).await,
                Ok(Ok(()))
            ) {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        index += 1;
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        sleep(LOADED_LATENCY_INTERVAL.min(deadline.saturating_duration_since(now))).await;
    }
    samples
}

fn mbps(bytes: u64, seconds: f64) -> f64 {
    if seconds <= f64::EPSILON {
        return 0.0;
    }
    bytes as f64 * 8.0 / seconds / 1_000_000.0
}

fn emit_loaded(samples: &[f64], phase: TestPhase, tx: &UnboundedSender<EngineEvent>) {
    if let Some(ms) = analysis::distribution(samples).map(|stats| stats.median_ms) {
        let _ = tx.send(EngineEvent::LoadedLatency { phase, ms });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_server_registry_has_global_coverage() {
        assert!(PUBLIC_SERVERS.len() >= 6);
        assert!(PUBLIC_SERVERS
            .iter()
            .any(|server| server.base.contains("librespeed.org")));
    }

    #[test]
    fn resolves_custom_server_with_standard_paths() {
        let server = resolve_custom_server("https://speed.example.test/backend").unwrap();
        assert_eq!(server.base.as_str(), "https://speed.example.test/backend/");
        assert_eq!(server.download_path, "garbage.php");
    }

    #[tokio::test]
    async fn completed_phases_emit_their_loaded_latency_before_returning() {
        let (url, peer) = crate::engine::test_support::measurement_peer().await;
        let engine = LibreSpeedEngine {
            client: Client::builder().no_proxy().build().unwrap(),
            config: EngineConfig {
                streams: 1,
                phase_duration: Duration::from_millis(150),
            },
            server: None,
        };
        let server = resolve_custom_server(&url).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for phase in [TestPhase::Download, TestPhase::Upload] {
            let (result, samples) = match phase {
                TestPhase::Download => engine.measure_download(&server, &tx).await.unwrap(),
                TestPhase::Upload => engine.measure_upload(&server, &tx).await.unwrap(),
                _ => unreachable!(),
            };
            assert!(result.bytes > 0);
            let expected = analysis::distribution(&samples).unwrap().median_ms;
            let mut loaded_events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                if let EngineEvent::LoadedLatency { phase, ms } = event {
                    loaded_events.push((phase, ms));
                }
            }
            assert_eq!(loaded_events, vec![(phase, expected)]);
        }
        peer.abort();
        let _ = peer.await;
    }

    #[test]
    fn failed_loaded_probes_do_not_invent_a_latency_event() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        emit_loaded(&[], TestPhase::Download, &tx);
        assert!(rx.try_recv().is_err());
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use crate::engine::test_support::upload_peer;

    #[tokio::test]
    async fn rejected_uploads_never_count_as_goodput() {
        let (url, peer) = upload_peer(vec![(413, Duration::ZERO), (500, Duration::ZERO)]).await;
        let total = Arc::new(AtomicU64::new(0));
        let result = upload_worker(
            Client::builder().no_proxy().build().unwrap(),
            resolve_custom_server(&url).unwrap(),
            Arc::clone(&total),
            64 * 1024,
            Instant::now() + Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(total.load(Ordering::Relaxed), 0);
        assert_eq!(peer.await.unwrap(), vec![64 * 1024, 32 * 1024]);
    }

    #[tokio::test]
    async fn deadline_cancelled_upload_is_not_counted() {
        let (url, peer) = upload_peer(vec![(200, Duration::from_millis(250))]).await;
        let total = Arc::new(AtomicU64::new(0));
        upload_worker(
            Client::builder().no_proxy().build().unwrap(),
            resolve_custom_server(&url).unwrap(),
            Arc::clone(&total),
            64 * 1024,
            Instant::now() + Duration::from_millis(100),
        )
        .await
        .unwrap();
        assert_eq!(total.load(Ordering::Relaxed), 0);
        peer.abort();
        let _ = peer.await;
    }

    #[tokio::test]
    async fn only_successful_complete_requests_commit_bytes() {
        let (url, peer) = upload_peer(vec![(200, Duration::ZERO), (500, Duration::ZERO)]).await;
        let total = Arc::new(AtomicU64::new(0));
        assert!(upload_worker(
            Client::builder().no_proxy().build().unwrap(),
            resolve_custom_server(&url).unwrap(),
            Arc::clone(&total),
            64 * 1024,
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .is_err());
        assert_eq!(total.load(Ordering::Relaxed), 64 * 1024);
        peer.await.unwrap();
    }
}
