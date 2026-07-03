use clap::Parser;
use std::path::PathBuf;

use crate::config::defaults;

/// Automate rsync/borg/restic backups centrally
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    /// Path to the YAML backup configuration
    #[arg(short, long, env = "BACKMATIC_CONFIG")]
    pub configfile: Option<PathBuf>,

    /// Number of parallel worker threads (default: half of CPU cores)
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// Seconds between retry attempts
    #[arg(short = 'i', long, default_value_t = defaults::DEFAULT_RETRY_INTERVAL_SEC)]
    pub retryinterval: u64,

    /// Maximum retry attempts per job
    #[arg(short, long, default_value_t = defaults::DEFAULT_RETRY_COUNT)]
    pub retries: u32,

    /// Run continuously; value is hours between scheduled cycle starts (0 = run once)
    #[arg(short = 'C', long, default_value_t = defaults::DEFAULT_CONTINUOUS_HOURS)]
    pub continuous: u64,

    /// Print planned actions without executing backups
    #[arg(long)]
    pub dry_run: bool,

    /// Increase log verbosity (-v info, -vv debug)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}
