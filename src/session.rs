//! Application-level options and completion policy shared by the CLI and cockpit.
//! Constructing options is offline. Only running an engine sends network traffic.
use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};

use crate::{
    cli::{Cli, InternetBackendArg, OutputFormat},
    engine::{
        cloudflare::CloudflareEngine, internet::InternetEngine, librespeed::LibreSpeedEngine,
        EngineConfig,
    },
    model::TestResult,
    storage,
};

#[derive(Debug, Clone)]
pub struct TestOptions {
    pub backend: InternetBackendArg,
    pub librespeed_server: Option<String>,
    pub duration: u64,
    pub streams: u8,
    pub fps: u16,
    pub timeout: u64,
    pub no_save: bool,
    pub output: Option<PathBuf>,
    pub format: OutputFormat,
}

impl From<&Cli> for TestOptions {
    fn from(cli: &Cli) -> Self {
        Self {
            backend: cli.backend,
            librespeed_server: cli.librespeed_server.clone(),
            duration: cli.duration,
            streams: cli.streams,
            fps: cli.fps,
            timeout: cli.timeout,
            no_save: cli.no_save,
            output: cli.output.clone(),
            format: cli.format,
        }
    }
}

impl TestOptions {
    pub fn engine(&self) -> Result<InternetEngine> {
        let config = EngineConfig {
            streams: usize::from(self.streams),
            phase_duration: Duration::from_secs(self.duration),
        };
        match self.backend {
            InternetBackendArg::Cloudflare => {
                Ok(InternetEngine::Cloudflare(CloudflareEngine::new(config)?))
            }
            InternetBackendArg::Librespeed => Ok(InternetEngine::LibreSpeed(
                LibreSpeedEngine::new(config, self.librespeed_server.as_deref())?,
            )),
        }
    }

    pub const fn backend_label(&self) -> &'static str {
        match self.backend {
            InternetBackendArg::Cloudflare => "Cloudflare",
            InternetBackendArg::Librespeed => "LibreSpeed",
        }
    }

    /// Preserve the CLI's explicit-export-before-history ordering and error contract.
    /// Called exactly once after a completed measurement, never for engine events.
    pub fn finish(&self, result: &TestResult) -> Result<()> {
        if let Some(path) = &self.output {
            match self.format {
                OutputFormat::Json => storage::write_json(path, result)?,
                OutputFormat::Csv => storage::write_csv(path, result)?,
            }
        }
        if !self.no_save {
            storage::persist_default(result).context("failed to persist speed-test history")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn options_preserve_cli_flags_and_exports_even_with_no_save() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("explicit.csv");
        let cli = Cli::parse_from([
            "speedtest",
            "--backend",
            "librespeed",
            "--duration",
            "3",
            "--streams",
            "1",
            "--fps",
            "144",
            "--no-save",
            "--output",
            file.to_str().unwrap(),
            "--format",
            "csv",
        ]);
        let options = TestOptions::from(&cli);
        assert_eq!(options.backend_label(), "LibreSpeed");
        assert_eq!(
            (options.duration, options.streams, options.fps),
            (3, 1, 144)
        );
        let result = serde_json::from_str(include_str!("../tests/fixtures/result.json")).unwrap();
        options.finish(&result).unwrap();
        let csv = std::fs::read_to_string(&file).unwrap();
        assert!(csv.contains("download_mbps"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
