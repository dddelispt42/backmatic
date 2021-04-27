use chrono::Local;
use clap::{crate_version, App, Arg};
use regex::Regex;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::{Command, Output};
use yaml_rust::{Yaml, YamlLoader};

static DEFAULT_LOCK_FILE: &str = "/tmp/backmatic.lock";
static DEFAULT_CONFIG_FILE: &str = "/home/heiko/backups.yml";
static DEFAULT_LOGDIR: &str = "/tmp";
static DEFAULT_THREADPOOLSIZE: usize = 4;
static DEFAULT_RETRYINTERVAL: u64 = 3600;
static DEFAULT_RETRYCOUNT: u32 = 23;

#[derive(Clone, Debug)]
pub struct Config {
    pub lock_file: String,
    pub config_file: String,
    pub threadpool_size: usize,
    pub retry_interval_sec: u64,
    pub retry_count: u32,
    pub doc: Yaml,
}

#[derive(Clone, Debug)]
pub struct BackupConfig {
    pub buptype: String,
    pub comment: String,
    pub logdir: String,
    pub logfile: String,
    pub src: Vec<String>,
    pub dest: Vec<String>,
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
        let matches = App::new("Backmatic")
            .version(crate_version!())
            .author("Heiko Riemer <heiko@eheiko.net>")
            .about("Automate rsync/borg/restic backups centrally")
            .arg(
                Arg::with_name("configfile")
                    .short("c")
                    .long("configfile")
                    .value_name("FILE")
                    .required(false)
                    .help("define the backups to be executed")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("threads")
                    .short("t")
                    .long("threads")
                    .help("defin the number of parallel threads")
                    .value_name("NUMBER")
                    .required(false)
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("retryinterval")
                    .short("i")
                    .long("retryinterval")
                    .help("defines the time between retries")
                    .value_name("NUMBER")
                    .required(false)
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("retries")
                    .short("r")
                    .long("retries")
                    .help("defines the number of retry attempts")
                    .value_name("NUMBER")
                    .required(false)
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("v")
                    .short("v")
                    .multiple(true)
                    .help("Sets the level of verbosity"),
            )
            .get_matches();
        let cfg_file = matches
            .value_of("configfile")
            .unwrap_or(DEFAULT_CONFIG_FILE)
            .to_string();
        let docs = Config::get_config(&cfg_file).expect("unable to read backup config file");
        match matches.occurrences_of("v") {
            0 => println!("No verbose info"),
            1 => println!("Some verbose info"),
            2 => println!("Tons of verbose info"),
            _ => println!("Don't be crazy"),
        }
        Config {
            lock_file: DEFAULT_LOCK_FILE.to_string(),
            config_file: cfg_file,
            threadpool_size: matches
                .value_of("threads")
                .unwrap_or(&DEFAULT_THREADPOOLSIZE.to_string())
                .parse::<usize>()
                .unwrap_or(DEFAULT_THREADPOOLSIZE),
            retry_interval_sec: matches
                .value_of("threads")
                .unwrap_or(&DEFAULT_RETRYINTERVAL.to_string())
                .parse::<u64>()
                .unwrap_or(DEFAULT_RETRYINTERVAL),
            retry_count: matches
                .value_of("retries")
                .unwrap_or(&DEFAULT_RETRYCOUNT.to_string())
                .parse::<u32>()
                .unwrap_or(DEFAULT_RETRYCOUNT),
            doc: docs[0].clone(),
        }
    }

    fn get_config(filename: &str) -> Result<Vec<Yaml>, &str> {
        let s = fs::read_to_string(filename).expect("Cannot read backup configuration file.");
        // TODO: use yaml_validator and define schema to check against <07-12-20, Heiko Riemer> //
        let docs = YamlLoader::load_from_str(&s).expect("Cannot load deserialize yaml content.");
        Ok(docs)
    }

    pub fn command_existing(cmd: &str) -> bool {
        match Command::new("test").arg("-x").arg(cmd).status() {
            Ok(status) => status.success(),
            Err(err) => {
                println!("{} not executable! - {}", cmd, err);
                false
            }
        }
    }

    pub fn log_output(logfile: &str, output: &Output) {
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
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
                &logfile,
                &format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
            )
            .expect("logfile cannot be renamed");
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
        BackupConfig {
            buptype: buptype.to_string(),
            comment: comment.to_string(),
            src: BackupConfig::yaml2string_list(&cfg["src"]),
            dest: BackupConfig::yaml2string_list(&cfg["dest"]),
            logdir: logdir.to_string(),
            logfile: BackupConfig::generate_logfilename(&logdir, buptype, comment),
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
                    retval.push(
                        value
                            .as_str()
                            .expect("not a string value in yaml file")
                            .to_string(),
                    );
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

    fn filenamify(input: &str) -> String {
        let re = Regex::new("[!<> :\'\"/\\|?*+]").expect("Problem in RegEx.");
        return re.replace_all(input, "_").to_string();
    }

    fn generate_logfilename(log_dir: &str, buptype: &str, comment: &str) -> String {
        let logstring: &str = &format!(
            "backup-{}_{}_{}.log",
            buptype,
            BackupConfig::filenamify(&comment),
            Local::now().format("%Y%m%d%H%M")
        );
        return String::from(&format!("{}/{}", log_dir, logstring));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_multiple_instances() {}
}
