use crate::config::{BackupConfig, Config};
use crate::mount::Mounter;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use regex::Regex;
use std::convert::TryFrom;
use std::process::Command;
use std::{fs, path, thread};
use threadpool::ThreadPool;

static BUPTYPE: &str = "rsync";
static BUPCMD: &str = "/usr/bin/rsync";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc[BUPTYPE].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        log::debug!("YAML item: {:?}", my_item);
        let my_cfg = cfg.clone();
        pool.execute(move || {
            let bupcfg = BackupConfig::new(&my_item, BUPTYPE);
            log::debug!("BackupConfig: {:?}", bupcfg);
            if Config::command_existing(BUPCMD) {
                let mut mounter = Mounter::new(&bupcfg.destmount);
                match mounter.mount() {
                    Ok(_) => {
                        run_rsync_backup(&my_cfg, &bupcfg);
                        retain_rsync_backup(&bupcfg);
                    }
                    Err(_) => {}
                }
            } else {
                log::error!("{} not installed on machine!", BUPCMD);
            }
        });
    }
    pool.join();
}

fn run_rsync_backup(cfg: &Config, bup: &BackupConfig) {
    for dest in &bup.dest {
        for _ in 1..cfg.retry_count {
            log::info!(
                "Start {} ({}): \"{:?}\" --> \"{:?}\"",
                bup.buptype,
                bup.comment,
                bup.src,
                dest,
            );
            let mut cmd = Command::new("rsync");
            cmd.arg("-avHAXEh")
                // .arg("--stats")
                .arg("--delete")
                .arg("--delete-excluded")
                // .arg("--info=BACKUP,COPY,DEL,MOUNT,NAME1,SKIP,STATS3,SYMSAFE")
                .arg(&format!("--log-file={}", &bup.logfile));
            for exclude in &bup.exclude {
                cmd.arg(&format!("--exclude={}", exclude));
            }
            for src in &bup.src {
                cmd.arg(src);
            }
            cmd.arg(dest);
            log::debug!("{} backup starting: Command={:?}", BUPTYPE, cmd);
            let output = cmd.output().expect("rsync - failed to execute process");
            Config::log_output(&bup.logfile, &output);
            if !output.status.success()
                && (output.status.code() != Some(23) || output.status.code() != Some(24))
            {
                log::warn!("End rsync backup ({}): {}", bup.comment, output.status);
                thread::sleep(std::time::Duration::from_secs(cfg.retry_interval_sec));
            } else {
                log::info!("End rsync backup ({}): {}", bup.comment, output.status);
                break;
            }
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
    for entry in parent
        .read_dir()
        .expect("backup dir parent not existing")
        .flatten()
    {
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
    if is_needed {
        log::info!("Retain {} backup ({})!", retention, dest);
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
}

fn retain_for_period(dest: &str, retention: &str, interval: i64, count: i64) {
    if count < 1 {
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
        log::debug!(
            "{} backup retention starting ({}): Command={:?}",
            BUPTYPE,
            dest,
            cmd
        );
        let output = cmd.output().expect("cp - failed to execute process");
        log::info!(
            "Retained {} backup ({}): {}",
            retention,
            dest,
            output.status
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
            log::debug!(
                "{} backup pruning starting ({}): Command={:?}",
                BUPTYPE,
                dest,
                cmd
            );
            let output = cmd.output().expect("rm - failed to execute process");
            log::info!(
                "Deleted {} backup ({}): {}",
                retention,
                bupdir,
                output.status
            );
        }
    }
}

fn retain_rsync_backup(bup: &BackupConfig) {
    for dest in &bup.dest {
        let re = regex::Regex::new(r"^(([a-zA-Z][a-zA-Z_-]*)@)?([a-zA-Z][.a-zA-Z0-9_-]*):");
        if re.expect("could not get compile regex").is_match(&dest) {
            log::info!(
                "Rsync: Skipping backup retention for remote destination path! ({})",
                dest
            );
            return;
        }
        retain_for_period(&dest, "hourly", 3600, bup.keep_hourly);
        retain_for_period(&dest, "daily", 3600 * 24, bup.keep_daily);
        retain_for_period(&dest, "weekly", 3600 * 24 * 7, bup.keep_weekly);
        retain_for_period(&dest, "monthly", 3600 * 24 * 30, bup.keep_monthly);
        retain_for_period(&dest, "yearly", 3600 * 24 * 364, bup.keep_yearly);
    }
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
