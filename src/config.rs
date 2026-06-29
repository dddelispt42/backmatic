use chrono::Local;
use clap::Parser;
use log::LevelFilter;
use regex::Regex;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Output};
use yaml_rust::{Yaml, YamlLoader};

static DEFAULT_LOCK_FILE: &str = "/tmp/backmatic.lock";
// TODO: change to XDG config dir
static DEFAULT_CONFIG_FILE: &str = "/home/heiko/.config/backmatic.yml";
static DEFAULT_LOGDIR: &str = "/tmp";
static DEFAULT_THREADPOOLSIZE: usize = 4;
static DEFAULT_RETRYINTERVAL: u64 = 3600;
static DEFAULT_RETRYCOUNT: u32 = 23;
static DEFAULT_CONTINUOUS: u64 = 0;

/// Automate rsnc/borg/restic backups centrally
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// define the backups to be executed
    #[arg(short, long, default_value_t = DEFAULT_CONFIG_FILE.to_string())]
    configfile: String,
    /// define the number of parallel threads
    #[arg(short, long, default_value_t = DEFAULT_THREADPOOLSIZE)]
    threads: usize,
    /// define the time between retries
    #[arg(short='i', long, default_value_t = DEFAULT_RETRYINTERVAL)]
    retryinterval: u64,
    /// define the number of retry attempts
    #[arg(short, long, default_value_t = DEFAULT_RETRYCOUNT)]
    retries: u32,
    /// run endlessly until aborted - define the hourse between runs
    #[arg(short = 'C', long, default_value_t = DEFAULT_CONTINUOUS)]
    continuous: u64,
    /// Turn verbose information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub lock_file: String,
    pub threadpool_size: usize,
    pub retry_interval_sec: u64,
    pub retry_count: u32,
    pub doc: Yaml,
    pub cycle_time: u64,
}

#[derive(Clone, Debug)]
pub struct DestMountConfig {
    pub mountpoint: Option<String>,
    pub uuid: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BackupConfig {
    pub buptype: String,
    pub comment: String,
    pub logfile: String,
    pub src: Vec<String>,
    pub dest: Vec<String>,
    pub destmount: Option<DestMountConfig>,
    pub host: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub exclude: Vec<String>,
    pub keep_hourly: i64,
    pub keep_daily: i64,
    pub keep_weekly: i64,
    pub keep_monthly: i64,
    pub keep_yearly: i64,
}

impl Config {
    pub fn new() -> Config {
        let cli = Args::parse();
        let docs = Config::get_config(&cli.configfile).expect("unable to read backup config file");
        match cli.verbose {
            0 => Config::configure_logging(LevelFilter::Warn),
            1 => Config::configure_logging(LevelFilter::Info),
            _ => Config::configure_logging(LevelFilter::Debug),
        }
        Config {
            lock_file: DEFAULT_LOCK_FILE.to_string(),
            threadpool_size: cli.threads,
            retry_interval_sec: cli.retryinterval,
            retry_count: cli.retries,
            doc: docs[0].clone(),
            cycle_time: cli.continuous,
        }
    }

    fn configure_logging(level: LevelFilter) {
        let config = simplelog::ConfigBuilder::new()
            .set_time_offset_to_local()
            .expect("no locale found")
            .set_thread_mode(simplelog::ThreadLogMode::Both)
            .set_time_format_custom(time::macros::format_description!(
                "[year]:[month]:[day] [hour]:[minute]:[second]"
            ))
            .build();
        simplelog::TermLogger::init(
            level,
            config,
            // simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )
        .expect("cannot create a logger");
        log::debug!("Initialized logger!");
    }

    fn get_config(filename: &str) -> Result<Vec<Yaml>, &str> {
        let s = fs::read_to_string(filename).expect("Cannot read backup configuration file.");
        // TODO: use yaml_validator and define schema to check against <07-12-20, Heiko Riemer> //
        // TODO sanitize all inputs from the yaml files
        let docs = YamlLoader::load_from_str(&s).expect("Cannot load deserialize yaml content.");
        Ok(docs)
    }

    pub fn command_existing(cmd: &str) -> bool {
        match Command::new("test").arg("-x").arg(cmd).status() {
            Ok(status) => status.success(),
            Err(err) => {
                log::error!("{} not executable! - {}", cmd, err);
                false
            }
        }
    }

    pub fn log_output(logfile: &str, output: &Output) {
        {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(logfile)
                .expect("logfile cannot be created");
            file.write_all(&output.stdout)
                .expect("logfile - stdout cannot be written");
            file.write_all(&output.stderr)
                .expect("logfile - stderr cannot be written");
        }
        if !output.status.success() {
            std::fs::rename(
                logfile,
                format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
            )
            .expect("logfile cannot be renamed");
        }
    }
}

impl DestMountConfig {
    pub fn new(cfg: &Yaml) -> DestMountConfig {
        DestMountConfig {
            mountpoint: cfg["mountpoint"].as_str().map(|value| value.to_string()),
            uuid: cfg["uuid"].as_str().map(|value| value.to_string()),
            password: cfg["password"].as_str().map(|value| value.to_string()),
        }
    }
}

impl BackupConfig {
    pub fn new(cfg: &Yaml, buptype: &str) -> BackupConfig {
        let logdir: &str = match cfg["logdir"].as_str() {
            None => DEFAULT_LOGDIR,
            Some(value) => value,
        };
        let comment: &str = cfg["comment"].as_str().unwrap_or("");
        let mut destmount: Option<DestMountConfig> = None;
        if !cfg["destmount"].is_badvalue() && !cfg["destmount"].is_null() {
            destmount = Some(DestMountConfig::new(&cfg["destmount"]));
        }
        BackupConfig {
            buptype: buptype.to_string(),
            comment: comment.to_string(),
            src: BackupConfig::yaml2string_list(&cfg["src"]),
            dest: BackupConfig::yaml2string_list(&cfg["dest"]),
            destmount,
            logfile: BackupConfig::generate_logfilename(logdir, buptype, comment),
            host: cfg["host"].as_str().map(|value| value.to_string()),
            user: cfg["user"].as_str().map(|value| value.to_string()),
            password: cfg["password"].as_str().map(|value| value.to_string()),
            exclude: BackupConfig::yaml2string_list(&cfg["exclude"]),
            keep_hourly: cfg["keep_hourly"].as_i64().unwrap_or(0),
            keep_daily: cfg["keep_daily"].as_i64().unwrap_or(0),
            keep_weekly: cfg["keep_weekly"].as_i64().unwrap_or(0),
            keep_monthly: cfg["keep_monthly"].as_i64().unwrap_or(0),
            keep_yearly: cfg["keep_yearly"].as_i64().unwrap_or(0),
        }
    }
    fn yaml2string_list(yaml: &Yaml) -> Vec<String> {
        let mut retval: Vec<String> = Vec::new();
        if !yaml.is_null() && !yaml.is_badvalue() {
            if yaml.is_array() {
                for value in yaml.as_vec().unwrap_or(&Vec::new()) {
                    retval.append(&mut BackupConfig::yaml2string_list(value));
                }
            } else {
                retval.push(
                    yaml.as_str()
                        .expect("not a string value in yaml file")
                        .to_string(),
                );
            }
        }
        retval
    }

    pub fn filenamify(input: &str) -> String {
        let re = Regex::new("[!<> :\'\"/\\|?*+]").expect("Problem in RegEx.");
        re.replace_all(input, "_").to_string()
    }

    fn generate_logfilename(log_dir: &str, buptype: &str, comment: &str) -> String {
        let logstring: &str = &format!(
            "backup-{}_{}_{}.log",
            buptype,
            BackupConfig::filenamify(comment),
            Local::now().format("%Y%m%d%H%M")
        );
        String::from(&format!("{}/{}", log_dir, logstring))
    }
}

#[cfg(test)]
mod tests {

    #[test]
    #[should_panic]
    fn test_multiple_instances() {}
}
