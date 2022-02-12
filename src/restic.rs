use crate::config::{BackupConfig, Config};
use crate::mount::Mounter;
use std::ffi::OsStr;
use std::process::Command;
use std::{thread, time};
use threadpool::ThreadPool;

static BUPTYPE: &str = "restic";
static BUPCMD: &str = "/usr/bin/restic";

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
                        for _ in 1..my_cfg.retry_count {
                            match run_restic_backup(&bupcfg) {
                                Ok(_) => {}
                                Err(_) => {
                                    log::warn!(
                                        "{} backup ({}) failed - retrying!",
                                        BUPTYPE,
                                        bupcfg.comment
                                    );
                                    thread::sleep(time::Duration::from_secs(
                                        my_cfg.retry_interval_sec,
                                    ));
                                    continue;
                                }
                            }
                            match prune_restic_backup(&bupcfg) {
                                Ok(_) => break,
                                Err(_) => {
                                    log::warn!(
                                        "{} backup ({}) pruning failed - retrying!",
                                        BUPTYPE,
                                        bupcfg.comment
                                    );
                                    thread::sleep(time::Duration::from_secs(
                                        my_cfg.retry_interval_sec,
                                    ));
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
            } else {
                log::error!("{} not installed on machine!", BUPCMD);
            }
        })
    }
    pool.join();
}

fn is_repo_existing(dest: &str, pw: &Option<String>) -> bool {
    let mut cmd = Command::new(BUPCMD);
    if let Some(pw) = &pw {
        cmd.env("RESTIC_PASSWORD", OsStr::new(&pw));
    }
    cmd.arg("-r").arg(&dest).arg("snapshots");
    log::debug!("Check if repo exist: Command={:?}", cmd);
    let output = cmd.output().expect("cannot check if file exists");
    output.status.success()
}

fn init_repo(logfile: &str, dest: &str, pw: &Option<String>) -> bool {
    let mut cmd = Command::new(BUPCMD);
    cmd.arg("init").arg("--repo");
    if let Some(pw) = &pw {
        cmd.env("RESTIC_PASSWORD", OsStr::new(&pw));
    }
    cmd.arg(dest);
    log::info!("{} repo not existing - calling: {:?}", BUPTYPE, cmd);
    let output = cmd.output().expect("borg - failed to init repo");
    Config::log_output(&logfile, &output);
    output.status.success()
}

fn run_restic_backup(bup: &BackupConfig) -> Result<(), ()> {
    for dest in &bup.dest {
        log::info!(
            "Start {} ({}): \"{:?}\" --> \"{:?}\"",
            bup.buptype,
            bup.comment,
            bup.src,
            dest,
        );
        if !is_repo_existing(&dest, &bup.password) && !init_repo(&bup.logfile, &dest, &bup.password)
        {
            log::error!("{} repo {} not initialized!", BUPTYPE, dest);
            return Err(());
        }
        let mut cmd = Command::new(BUPCMD);
        if let Some(pw) = &bup.password {
            cmd.env("RESTIC_PASSWORD", OsStr::new(&pw));
        }
        cmd.arg("-r").arg(dest).arg("backup");
        for exclude in &bup.exclude {
            cmd.arg(&format!("--exclude={}", exclude));
        }
        for src in &bup.src {
            cmd.arg(src);
        }
        log::debug!("{} backup starting: Command={:?}", BUPTYPE, cmd);
        let output = cmd.output().expect("borg - failed to execute process");
        Config::log_output(&bup.logfile, &output);
        if !output.status.success() {
            log::warn!("End borg backup ({}): {}", bup.comment, output.status);
            return Err(());
        }
        log::info!("End borg backup ({}): {}", bup.comment, output.status);
    }
    Ok(())
}

fn prune_restic_backup(bup: &BackupConfig) -> Result<(), ()> {
    for dest in &bup.dest {
        log::info!("Prune {} backup: \"{}\"", BUPTYPE, dest);
        let mut cmd = Command::new(BUPCMD);
        if let Some(pw) = &bup.password {
            cmd.env("RESTIC_PASSWORD", OsStr::new(&pw));
        }
        cmd.arg("-r")
            .arg(dest)
            .arg("forget")
            .arg("--keep-hourly")
            .arg(bup.keep_hourly.to_string())
            .arg("--keep-daily")
            .arg(bup.keep_daily.to_string())
            .arg("--keep-weekly")
            .arg(bup.keep_weekly.to_string())
            .arg("--keep-monthly")
            .arg(bup.keep_monthly.to_string())
            .arg("--keep-yearly")
            .arg(bup.keep_yearly.to_string());
        log::debug!("{} backup - pruning starting: Command={:?}", BUPTYPE, cmd);
        let output = cmd.output().expect("restic - failed to execute process");
        Config::log_output(&bup.logfile, &output);
        if !output.status.success() {
            log::warn!("End {} pruning: {}", BUPTYPE, output.status);
            return Err(());
        }
        log::info!("End {} pruning: {}", BUPTYPE, output.status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_restic_config() {
        assert!(false)
    }
}
