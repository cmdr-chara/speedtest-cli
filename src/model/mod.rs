use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPhase {
    Preparing,
    Latency,
    Download,
    Upload,
    Complete,
}

impl TestPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "PREPARING",
            Self::Latency => "LATENCY",
            Self::Download => "DOWNLOAD",
            Self::Upload => "UPLOAD",
            Self::Complete => "COMPLETE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub host: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    pub idle_ms: f64,
    pub jitter_ms: f64,
    pub download_loaded_ms: Option<f64>,
    pub upload_loaded_ms: Option<f64>,
    pub packet_loss_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputResult {
    pub mbps: f64,
    pub bytes: u64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub timestamp: DateTime<Utc>,
    pub backend: String,
    pub server: ServerInfo,
    pub latency: LatencyResult,
    pub download: ThroughputResult,
    pub upload: ThroughputResult,
}

impl TestResult {
    pub fn pretty_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}
