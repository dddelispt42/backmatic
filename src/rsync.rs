use crate::config::Config;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use regex::Regex;
use std::convert::TryFrom;
use std::process::Command;
use std::{fs, path, thread};
use threadpool::ThreadPool;
use yaml_rust::Yaml;

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["rsync"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            // TODO: mount if "mount" key exist (see borg)
            // TODO: check if user can mount - or skip
            run_rsync_backup(
                &my_cfg,
                my_item["comment"].as_str().unwrap_or(""),
                my_item["src"]
                    .as_str()
                    .expect("unable to find src key in backup config"),
                my_item["dest"]
                    .as_str()
                    .expect("unable to find dest key in backup config"),
                my_item["exclude"].as_vec().unwrap_or(&Vec::new()),
            );
            retain_rsync_backup(
                my_item["dest"]
                    .as_str()
                    .expect("unable to find dest key in backup config"),
                my_item["keep_hourly"].as_i64().unwrap_or(0),
                my_item["keep_daily"].as_i64().unwrap_or(0),
                my_item["keep_weekly"].as_i64().unwrap_or(0),
                my_item["keep_monthly"].as_i64().unwrap_or(0),
                my_item["keep_yearly"].as_i64().unwrap_or(0),
            );
            // TODO: skip in case of Windows and for bad file system
        });
    }
    pool.join();
}

fn run_rsync_backup(cfg: &Config, comment: &str, src: &str, dest: &str, excludelist: &Vec<Yaml>) {
    Command::new("test")
        .arg("-x")
        .arg("/usr/bin/rsync")
        .output()
        .expect("unable to locate rsync command");
    for _ in 1..cfg.retry_count {
        let logfile = Config::generate_logfilename(&cfg.log_dir, "rsync", src, dest);
        println!(
            "Run rsync backup ({}): \"{}\" --> \"{}\"",
            comment, src, dest,
        );
        let mut cmd = Command::new("rsync");
        cmd.arg("-avHAXEh")
            .arg("--stats")
            .arg("--delete")
            .arg("--delete-excluded")
            .arg("--info=BACKUP,COPY,DEL,MOUNT,NAME1,SKIP,STATS3,SYMSAFE")
            .arg(&format!("--log-file={}", &logfile));
        for exclude in excludelist {
            let exstr = exclude
                .as_str()
                .expect("Exclusion string cannot be used to create rsync parameter.");
            cmd.arg(&format!("--exclude={}", exstr));
        }
        let output = cmd
            .arg(src)
            .arg(dest)
            .output()
            .expect("rsync - failed to execute process");
        println!("End rsync backup ({}): {}", comment, output.status);
        if !output.status.success() {
            fs::rename(
                &logfile,
                &format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
            )
            .expect("logfile cannot be renamed");
            thread::sleep(std::time::Duration::from_secs(cfg.retry_interval_sec));
        } else {
            break;
        }
    }
}

fn needs_retention(dest: &str, retention: &str, interval: i64) -> bool {
    let mut is_needed = true;
    let dest = dest.trim_end_matches('/');
    let parent = path::Path::new(dest)
        .parent()
        .expect("cannot get parent of backup path");
    let re = Regex::new(&format!("{}.{}", dest, retention)).expect("invalid regex");
    for entry in parent.read_dir().expect("backup dir parent not existing") {
        if let Ok(entry) = entry {
            if entry.path().is_dir() {
                if let Some(dir) = entry.path().to_str() {
                    if dir.starts_with(&format!("{}.{}", dest, retention)) {
                        let naivetimestamp =
                            NaiveDateTime::parse_from_str(&re.replace(dir, ""), "%Y-%m-%d-%H-%M")
                                .expect("no valid datetime format");
                        let localtimestamp: DateTime<Local> =
                            Local.from_local_datetime(&naivetimestamp).unwrap();
                        let period = Local::now().signed_duration_since(localtimestamp);
                        if period.num_seconds() < interval {
                            is_needed = false
                        }
                    }
                }
            }
        }
    }
    if is_needed {
        println!("Retain {} backup ({})!", retention, dest);
    }
    is_needed
}

fn date_to_file_suffix() -> String {
    let now = Local::now();
    format!(
        "{:02}-{:02}-{:02}-{:02}-{:02}",
        now.year(),
        now.month(),
        now.day(),
        now.hour(),
        now.minute(),
    )
    .to_string()
}

fn retain_for_period(dest: &str, retention: &str, interval: i64, count: i64) {
    if count < 1 {
        println!("No {} backup to be kept!", retention);
        return;
    }
    if needs_retention(&dest, retention, interval) {
        let mut cmd = Command::new("cp");
        cmd.arg("-al").arg(dest).arg(&format!(
            "{}.{}{}",
            dest.trim_end_matches('/'),
            retention,
            date_to_file_suffix()
        ));
        let output = cmd.output().expect("cp - failed to execute process");
        println!(
            "Retained {} backup ({}): {}",
            retention, dest, output.status
        );
    }
    prune_for_period(dest, retention, count);
}

fn prune_for_period(dest: &str, retention: &str, count: i64) {
    let mut bup_list: Vec<String> = vec![];
    let dest = dest.trim_end_matches('/');
    let parent = path::Path::new(dest)
        .parent()
        .expect("cannot get parent of backup path");
    let mut paths: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    paths.sort_by_key(|dir| dir.path());
    for entry in paths {
        if entry.metadata().expect("not a valid directory").is_dir() {
            if let Some(dirname) = entry.path().to_str() {
                if dirname.starts_with(&format!("{}.{}", dest, retention)) {
                    bup_list.push(String::from(dirname));
                }
            }
        }
    }
    let size = usize::try_from(count).expect("number of backups out of usize range");
    if bup_list.len() > size {
        bup_list.truncate(bup_list.len() - size);
        for bupdir in bup_list {
            let mut cmd = Command::new("rm");
            cmd.arg("-rf").arg(&bupdir);
            let output = cmd.output().expect("rm - failed to execute process");
            println!(
                "Deleted {} backup ({}): {}",
                retention, bupdir, output.status
            );
        }
    }
}

fn retain_rsync_backup(
    dest: &str,
    hourly: i64,
    daily: i64,
    weekly: i64,
    monthly: i64,
    yearly: i64,
) {
    // ^(((?P<user>[a-zA-Z][a-zA-Z_-]*)@)?(?P<host>[a-zA-Z][.a-zA-Z0-9_-]*):)?(?P<path>/|(/[\w-]+)+$)$
    let re = regex::Regex::new(
        // r"^(((?P<user>[a-zA-Z][a-zA-Z_-]*)@)?(?P<host>[a-zA-Z][.a-zA-Z0-9_-]*):)?(?P<path>/|(/[\w-]+)+$)$",
        r"^(([a-zA-Z][a-zA-Z_-]*)@)?([a-zA-Z][.a-zA-Z0-9_-]*):",
    );
    if re.expect("could not get capture groups").is_match(dest) {
        println!(
            "Skpping rsync backup retention for remote destination path! ({})",
            dest
        );
        return;
    }
    retain_for_period(dest, "hourly", 3600, hourly);
    retain_for_period(dest, "daily", 3600 * 24, daily);
    retain_for_period(dest, "weekly", 3600 * 24 * 7, weekly);
    retain_for_period(dest, "monthly", 3600 * 24 * 30, monthly);
    retain_for_period(dest, "yearly", 3600 * 24 * 364, yearly);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_rsync_config() {
        assert!(false)
    }
    #[test]
    fn test_read_invalid_rsync_config() {
        assert!(false)
    }
    #[test]
    fn test_read_incomplete_rsync_config() {
        assert!(false)
    }
}
