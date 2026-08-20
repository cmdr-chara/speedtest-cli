use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::{sleep, sleep_until, timeout, Instant},
};

use crate::{
    analysis,
    model::{ServerInfo, TestResult, ThroughputResult},
};

const MAGIC: &[u8; 4] = b"NSL1";
const OP_PING: u8 = 0;
const OP_DOWNLOAD: u8 = 1;
const OP_UPLOAD: u8 = 2;
const IO_CHUNK: usize = 1024 * 1024;
const PING_TIMEOUT: Duration = Duration::from_secs(2);
const LOADED_PING_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_SAMPLES: usize = 20;

#[derive(Debug, Clone, Copy)]
pub struct LanConfig {
    pub streams: usize,
    pub phase_duration: Duration,
}

pub async fn serve(bind: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind LAN speed-test server to {bind}"))?;
    loop {
        let (stream, _) = listener.accept().await.context("LAN server accept failed")?;
        tokio::spawn(async move {
            let _ = handle_connection(stream).await;
        });
    }
}

pub async fn run(server: SocketAddr, config: LanConfig) -> Result<TestResult> {
    let mut idle_samples = Vec::with_capacity(IDLE_SAMPLES);
    for _ in 0..IDLE_SAMPLES {
        idle_samples.push(ping_once(server).await?);
        sleep(Duration::from_millis(50)).await;
    }

    let (download, download_loaded) = measure_download(server, config).await?;
    sleep(Duration::from_millis(250)).await;
    let (upload, upload_loaded) = measure_upload(server, config).await?;

    let latency = analysis::summarize_latency(
        &idle_samples,
        &download_loaded,
        &upload_loaded,
        None,
    );
    let network_analysis = analysis::build_network_analysis(
        &idle_samples,
        &download_loaded,
        &upload_loaded,
        &latency,
        &download,
        &upload,
    );

    Ok(TestResult {
        timestamp: Utc::now(),
        backend: "lan".to_string(),
        server: ServerInfo {
            host: server.to_string(),
            name: "Self-hosted LAN endpoint".to_string(),
        },
        latency,
        download,
        upload,
        analysis: Some(network_analysis),
    })
}

async fn handle_connection(mut stream: TcpStream) -> Result<()> {
    stream.set_nodelay(true).context("failed to enable TCP_NODELAY")?;
    let mut header = [0_u8; 5];
    stream
        .read_exact(&mut header)
        .await
        .context("LAN client closed before protocol header")?;
    if &header[..4] != MAGIC {
        return Err(anyhow!("invalid LAN speed-test protocol magic"));
    }

    match header[4] {
        OP_PING => {
            stream.write_all(MAGIC).await?;
            stream.write_u8(OP_PING).await?;
            stream.flush().await?;
        }
        OP_DOWNLOAD => {
            let chunk = vec![0x5a_u8; IO_CHUNK];
            loop {
                if stream.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        }
        OP_UPLOAD => {
            let mut total = 0_u64;
            let mut buffer = vec![0_u8; IO_CHUNK];
            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(count) => total = total.saturating_add(count as u64),
                    Err(_) => break,
                }
            }
            let _ = stream.write_u64(total).await;
            let _ = stream.flush().await;
        }
        opcode => return Err(anyhow!("unknown LAN speed-test opcode {opcode}")),
    }
    Ok(())
}

async fn ping_once(server: SocketAddr) -> Result<f64> {
    let started = Instant::now();
    let mut stream = timeout(PING_TIMEOUT, TcpStream::connect(server))
        .await
        .context("LAN ping connection timed out")??;
    stream.set_nodelay(true)?;
    timeout(PING_TIMEOUT, async {
        stream.write_all(MAGIC).await?;
        stream.write_u8(OP_PING).await?;
        stream.flush().await?;
        let mut response = [0_u8; 5];
        stream.read_exact(&mut response).await?;
        Result::<(), std::io::Error>::Ok(())
    })
    .await
    .context("LAN echo timed out")??;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

async fn measure_download(
    server: SocketAddr,
    config: LanConfig,
) -> Result<(ThroughputResult, Vec<f64>)> {
    let total = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + config.phase_duration;
    let mut workers = JoinSet::new();

    for _ in 0..config.streams.max(1) {
        let total = Arc::clone(&total);
        workers.spawn(download_worker(server, total, deadline));
    }
    let loaded = tokio::spawn(measure_loaded_latency(server, deadline));

    while let Some(result) = workers.join_next().await {
        result.context("LAN download worker panicked")??;
    }
    let loaded_samples = loaded.await.context("LAN loaded-latency task panicked")?;
    let bytes = total.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(anyhow!("LAN server delivered no download data"));
    }

    Ok((
        ThroughputResult {
            mbps: mbps(bytes, config.phase_duration.as_secs_f64()),
            bytes,
            seconds: config.phase_duration.as_secs_f64(),
        },
        loaded_samples,
    ))
}

async fn download_worker(
    server: SocketAddr,
    total: Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut stream = TcpStream::connect(server)
        .await
        .with_context(|| format!("failed to connect to LAN server {server}"))?;
    stream.set_nodelay(true)?;
    stream.write_all(MAGIC).await?;
    stream.write_u8(OP_DOWNLOAD).await?;
    stream.flush().await?;
    let mut buffer = vec![0_u8; 64 * 1024];

    while Instant::now() < deadline {
        let read = tokio::select! {
            value = stream.read(&mut buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        if read == 0 {
            break;
        }
        total.fetch_add(read as u64, Ordering::Relaxed);
    }
    Ok(())
}

async fn measure_upload(
    server: SocketAddr,
    config: LanConfig,
) -> Result<(ThroughputResult, Vec<f64>)> {
    let total = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + config.phase_duration;
    let mut workers = JoinSet::new();

    for _ in 0..config.streams.max(1) {
        let total = Arc::clone(&total);
        workers.spawn(upload_worker(server, total, deadline));
    }
    let loaded = tokio::spawn(measure_loaded_latency(server, deadline));

    while let Some(result) = workers.join_next().await {
        result.context("LAN upload worker panicked")??;
    }
    let loaded_samples = loaded.await.context("LAN loaded-latency task panicked")?;
    let bytes = total.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(anyhow!("LAN server accepted no upload data"));
    }

    Ok((
        ThroughputResult {
            mbps: mbps(bytes, config.phase_duration.as_secs_f64()),
            bytes,
            seconds: config.phase_duration.as_secs_f64(),
        },
        loaded_samples,
    ))
}

async fn upload_worker(
    server: SocketAddr,
    total: Arc<AtomicU64>,
    deadline: Instant,
) -> Result<()> {
    let mut stream = TcpStream::connect(server)
        .await
        .with_context(|| format!("failed to connect to LAN server {server}"))?;
    stream.set_nodelay(true)?;
    stream.write_all(MAGIC).await?;
    stream.write_u8(OP_UPLOAD).await?;
    stream.flush().await?;
    let buffer = vec![0xa5_u8; IO_CHUNK];

    while Instant::now() < deadline {
        let write = tokio::select! {
            value = stream.write(&buffer) => value?,
            _ = sleep_until(deadline) => break,
        };
        if write == 0 {
            break;
        }
    }

    stream.shutdown().await?;
    let acknowledged = timeout(Duration::from_secs(3), stream.read_u64())
        .await
        .context("LAN upload acknowledgement timed out")??;
    total.fetch_add(acknowledged, Ordering::Relaxed);
    Ok(())
}

async fn measure_loaded_latency(server: SocketAddr, deadline: Instant) -> Vec<f64> {
    let mut samples = Vec::new();
    while Instant::now() < deadline {
        if let Ok(ms) = ping_once(server).await {
            samples.push(ms);
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        sleep(LOADED_PING_INTERVAL.min(deadline.saturating_duration_since(now))).await;
    }
    samples
}

fn mbps(bytes: u64, seconds: f64) -> f64 {
    if seconds <= f64::EPSILON {
        return 0.0;
    }
    bytes as f64 * 8.0 / seconds / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_throughput_conversion_is_decimal_mbps() {
        assert!((mbps(125_000_000, 1.0) - 1000.0).abs() < 0.001);
    }
}
