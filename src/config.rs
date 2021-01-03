use chrono::Utc;
use clap::{crate_version, App, Arg};
use regex::Regex;
use std::fs;
use yaml_rust::{Yaml, YamlLoader};

static DEFAULT_LOCK_FILE: &str = "/tmp/backapp.lock";
static DEFAULT_CONFIG_FILE: &str = "/home/heiko/backups.yml";
static DEFAULT_LOGDIR: &str = "/tmp";
static DEFAULT_THREADPOOLSIZE: usize = 4;
static DEFAULT_RETRYINTERVAL: u64 = 3600;
static DEFAULT_RETRYCOUNT: u32 = 23;

#[derive(Clone, Debug)]
pub struct Config {
    pub lock_file: String,
    pub config_file: String,
    pub log_dir: String,
    pub threadpool_size: usize,
    pub retry_interval_sec: u64,
    pub retry_count: u32,
    pub doc: Yaml,
}

impl Config {
    pub fn new() -> Config {
        let matches = App::new("Backupmatic")
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
                Arg::with_name("logdir")
                    .short("l")
                    .long("logdir")
                    .value_name("DIRECTORY")
                    .required(false)
                    .help("define the directory to store log files")
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
        let cfg_file = String::from(
            matches
                .value_of("configfile")
                .unwrap_or(DEFAULT_CONFIG_FILE)
                .to_string(),
        );
        let docs = Config::get_config(&cfg_file).expect("unable to read backup config file");
        match matches.occurrences_of("v") {
            0 => println!("No verbose info"),
            1 => println!("Some verbose info"),
            2 => println!("Tons of verbose info"),
            3 | _ => println!("Don't be crazy"),
        }
        Config {
            lock_file: String::from(DEFAULT_LOCK_FILE.to_string()),
            config_file: cfg_file,
            log_dir: String::from(
                matches
                    .value_of("logdir")
                    .unwrap_or(DEFAULT_LOGDIR)
                    .to_string(),
            ),
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

    pub fn filenamify(input: &str) -> String {
        let re = Regex::new("[!<>:\'\"/\\|?*+]").expect("Problem in RegEx.");
        return re.replace_all(input, "_").to_string();
    }

    pub fn generate_logfilename(log_dir: &str, buptype: &str, src: &str, dest: &str) -> String {
        let logstring: &str = &format!(
            "{}-{}---{}_{}.log",
            buptype,
            src,
            dest,
            Utc::now().format("%Y%m%d%H%M")
        );
        return String::from(&format!("{}/{}", log_dir, &Config::filenamify(logstring)));
    }

    pub fn get_config(filename: &str) -> Result<Vec<Yaml>, &str> {
        let s = fs::read_to_string(filename).expect("Cannot read backup configuration file.");
        // TODO: use yaml_validator and define schema to check against <07-12-20, Heiko Riemer> //
        let docs = YamlLoader::load_from_str(&s).expect("Cannot load deserialize yaml content.");
        Ok(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_multiple_instances() {}
}
