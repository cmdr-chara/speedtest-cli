use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    engine::{cloudflare::CloudflareEngine, librespeed::LibreSpeedEngine, EngineEvent},
    model::TestResult,
};

#[derive(Clone)]
pub enum InternetEngine {
    Cloudflare(CloudflareEngine),
    LibreSpeed(LibreSpeedEngine),
}

impl InternetEngine {
    pub async fn run(&self, tx: UnboundedSender<EngineEvent>) -> Result<TestResult> {
        match self {
            Self::Cloudflare(engine) => engine.run(tx).await,
            Self::LibreSpeed(engine) => engine.run(tx).await,
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Cloudflare(_) => "cloudflare",
            Self::LibreSpeed(_) => "librespeed",
        }
    }
}
