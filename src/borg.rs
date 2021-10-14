use crate::config::{BackupConfig, Config};
use crate::mount::Mounter;
use std::ffi::OsStr;
use std::process::Command;
use std::{thread, time};
use threadpool::ThreadPool;

static BUPTYPE: &str = "borg";
static BUPCMD: &str = "/usr/bin/borg";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["borg"].as_vec().unwrap_or(&Vec::new()) {
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
                            match run_borg_backup(&bupcfg) {
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
                            match prune_borg_backup(&bupcfg) {
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
        });
    }
    pool.join();
}

fn is_repo_existing(dest: &str, pw: &Option<String>) -> bool {
    let mut cmd = Command::new(BUPCMD);
    if let Some(pw) = &pw {
        cmd.env("BORG_PASSPHRASE", OsStr::new(&pw));
    }
    cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", OsStr::new("yes"));
    cmd.arg("list").arg(&dest);
    log::debug!("Check if repo exist: Command={:?}", cmd);
    let output = cmd.output().expect("cannot check if file exists");
    output.status.success()
}

fn init_repo(logfile: &str, dest: &str, pw: &Option<String>) -> bool {
    let mut cmd = Command::new(BUPCMD);
    cmd.arg("init");
    cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", OsStr::new("yes"));
    if let Some(pw) = &pw {
        cmd.env("BORG_PASSPHRASE", OsStr::new(&pw));
        cmd.arg("-e").arg("repokey");
    } else {
        cmd.arg("-e").arg("none");
    }
    cmd.arg(dest);
    log::info!("{} repo not existing - calling: {:?}", BUPTYPE, cmd);
    let output = cmd.output().expect("borg - failed to init repo");
    Config::log_output(&logfile, &output);
    output.status.success()
}

fn run_borg_backup(bup: &BackupConfig) -> Result<(), ()> {
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
        cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", OsStr::new("yes"));
        if let Some(pw) = &bup.password {
            cmd.env("BORG_PASSPHRASE", OsStr::new(&pw));
        }
        cmd.arg("create").arg("--exclude-caches");
        for exclude in &bup.exclude {
            cmd.arg(&format!("--exclude={}", exclude));
        }
        cmd.arg(&format!(
            "{}::{} {}",
            dest,
            BackupConfig::filenamify(&bup.comment),
            r"{user}@{hostname}_{now:%Y-%m-%d_%H-%M}"
        ));
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

fn prune_borg_backup(bup: &BackupConfig) -> Result<(), ()> {
    for dest in &bup.dest {
        log::info!("Prune {} backup: \"{}\"", BUPTYPE, dest);
        let mut cmd = Command::new(BUPCMD);
        cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", OsStr::new("yes"));
        if let Some(pw) = &bup.password {
            cmd.env("BORG_PASSPHRASE", OsStr::new(&pw));
        }
        cmd.arg("prune")
            .arg("-s")
            .arg("--list")
            .arg("-H")
            .arg(bup.keep_hourly.to_string())
            .arg("-d")
            .arg(bup.keep_daily.to_string())
            .arg("-w")
            .arg(bup.keep_weekly.to_string())
            .arg("-m")
            .arg(bup.keep_monthly.to_string())
            .arg("-y")
            .arg(bup.keep_yearly.to_string())
            .arg(dest);
        log::debug!("{} backup - pruning starting: Command={:?}", BUPTYPE, cmd);
        let output = cmd.output().expect("borg - failed to execute process");
        Config::log_output(&bup.logfile, &output);
        if !output.status.success() {
            log::warn!("End borg pruning: {}", output.status);
            return Err(());
        }
        log::info!("End borg pruning: {}", output.status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_borg_config() {
        assert!(false)
    }
}
