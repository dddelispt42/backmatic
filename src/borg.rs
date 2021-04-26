use crate::config::Config;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::process::{Command, Output};
use std::{env, thread, time};
use threadpool::ThreadPool;
use yaml_rust::Yaml;

const BORG: &str = "/usr/bin/borg";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["borg"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            // TODO sanitize all inputs from the yaml files
            // TODO: mount if "mount" key exist and user has permissions (see borg)
            // let mounter = Mounter::new(uuid, None);
            let src = my_item["src"]
                .as_str()
                .expect("unable to find src key in backup config");
            let dest = my_item["dest"]
                .as_str()
                .expect("unable to find dest key in backup config");
            if Config::command_existing(BORG) {
                for _ in 1..my_cfg.retry_count {
                    let logfile = Config::generate_logfilename(&my_cfg.log_dir, "borg", src, dest);
                    match run_borg_backup(
                        &logfile,
                        my_item["comment"]
                            .as_str()
                            .expect("unable to find comment key in backup config"),
                        src,
                        dest,
                        my_item["password"].as_str(),
                        my_item["exclude"].as_vec().unwrap_or(&Vec::new()),
                    ) {
                        Ok(_) => {}
                        Err(_) => {
                            thread::sleep(time::Duration::from_secs(my_cfg.retry_interval_sec));
                            continue;
                        }
                    }
                    match prune_borg_backup(
                        &logfile,
                        dest,
                        my_item["password"].as_str(),
                        my_item["keep_hourly"].as_i64().unwrap_or(0),
                        my_item["keep_daily"].as_i64().unwrap_or(0),
                        my_item["keep_weekly"].as_i64().unwrap_or(0),
                        my_item["keep_monthly"].as_i64().unwrap_or(0),
                        my_item["keep_yearly"].as_i64().unwrap_or(0),
                    ) {
                        Ok(_) => break,
                        Err(_) => {
                            thread::sleep(time::Duration::from_secs(my_cfg.retry_interval_sec));
                        }
                    }
                }
            }
        });
    }
    pool.join();
}

fn log_output(logfile: &str, output: &Output) {
    // TODO: direct all cmd output into log file
    io::stdout().write_all(&output.stdout).unwrap();
    io::stderr().write_all(&output.stderr).unwrap();

    if !output.status.success() {
        std::fs::rename(
            &logfile,
            &format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
        )
        .expect("logfile cannot be renamed");
    }
}

fn init_repo(logfile: &str, dest: &str) -> bool {
    let mut cmd = Command::new(BORG);
    cmd.arg("init").arg("-e").arg("none").arg(dest); // TODO: define encryption
    println!("Repo not existing - calling: {:?}", cmd);
    let output = cmd.output().expect("borg - failed to init repo");
    log_output(&logfile, &output);
    return output.status.success();
}

fn run_borg_backup(
    logfile: &str,
    comment: &str,
    src: &str,
    dest: &str,
    pw: Option<&str>,
    excludelist: &Vec<Yaml>,
) -> Result<(), ()> {
    println!(
        "Run borg backup ({}): \"{}\" --> \"{}\"",
        comment, src, dest
    );
    match pw {
        Some(pw) => env::set_var("BORG_PASSPHRASE", OsStr::new(pw)),
        None => println!("no borg password given"),
    }
    if !std::path::Path::new(dest).is_dir() {
        if !init_repo(&logfile, &dest) {
            return Err(());
        }
    }
    let mut cmd = Command::new(BORG);
    cmd.arg("create").arg("--exclude-caches");
    for exclude in excludelist {
        let exstr = exclude
            .as_str()
            .expect("Exclusion string cannot be used to create rsync parameter.");
        cmd.arg(&format!("--exclude={}", exstr));
    }
    cmd.arg(&format!(
        "{}::{} {}",
        dest, comment, r"{user}@{hostname}:{now:%Y-%m-%d %H:%M}"
    ));
    cmd.arg(src);
    println!("Cmd borg backup: {:?}", cmd);
    let output = cmd.output().expect("borg - failed to execute process");
    log_output(&logfile, &output);
    println!("End borg backup ({}): {}", comment, output.status);
    if !output.status.success() {
        return Err(());
    }
    Ok(())
}

fn prune_borg_backup(
    logfile: &str,
    dest: &str,
    pw: Option<&str>,
    hourly: i64,
    daily: i64,
    weekly: i64,
    monthly: i64,
    yearly: i64,
) -> Result<(), ()> {
    println!("Prune borg backup: \"{}\"", dest);
    match pw {
        Some(pw) => env::set_var("BORG_PASSPHRASE", OsStr::new(pw)),
        None => println!("no borg password given"),
    }
    let mut cmd = Command::new(BORG);
    cmd.arg("prune")
        .arg("-s")
        .arg("--list")
        .arg("-H")
        .arg(hourly.to_string())
        .arg("-d")
        .arg(daily.to_string())
        .arg("-w")
        .arg(weekly.to_string())
        .arg("-m")
        .arg(monthly.to_string())
        .arg("-y")
        .arg(yearly.to_string())
        .arg(dest);
    let output = cmd.output().expect("borg - failed to execute process");
    log_output(&logfile, &output);
    println!("End borg pruning: {}", output.status);
    if !output.status.success() {
        return Err(());
    }
    // TODO: verify backup or rely on TDD test <13-12-20, Heiko Riemer> //
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
