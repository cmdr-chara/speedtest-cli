use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Csv,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "speedtest")]
#[command(version, about = "A fast, polished terminal speed test")]
pub struct Cli {
    /// Number of concurrent transfer streams.
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub streams: u8,

    /// Duration of each throughput phase in seconds.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(3..=30))]
    pub duration: u64,

    /// Disable the interactive terminal UI.
    #[arg(long)]
    pub plain: bool,

    /// Print the canonical result as JSON.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,

    /// Also write the completed result to this path.
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Format used by --output.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,

    /// Do not persist automatic history/results.
    #[arg(long)]
    pub no_save: bool,
}
