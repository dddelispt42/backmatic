use crate::config::Config;
use std::process::Command;
use std::{thread, time};
use threadpool::ThreadPool;
use yaml_rust::Yaml;

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["restic"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            run_restic_backup(
                &my_cfg,
                my_item["comment"].as_str().unwrap_or(""),
                my_item["uuid"]
                    .as_str()
                    .expect("unable to find UUID in backup config"),
                my_item["src"]
                    .as_str()
                    .expect("unable to find src key in backup config"),
                my_item["dest"]
                    .as_str()
                    .expect("unable to find dest key in backup config"),
                my_item["exclude"].as_vec().unwrap_or(&Vec::new()),
            );
        });
    }
    pool.join();
}

fn run_restic_backup(
    cfg: &Config,
    comment: &str,
    uuid: &str,
    src: &str,
    dest: &str,
    excludelist: &Vec<Yaml>,
) {
    Command::new("test")
        .arg("-x")
        .arg("/usr/bin/restic")
        .output()
        .expect("unable to locate restic command");
    // TODO: check disk and mount if there (/mnt/backup/uuid) <13-12-20, Heiko Riemer> //
    if std::path::Path::new(&format!("/dev/disk/by-uuid/{}", uuid)).exists() {
        println!("UUID {} disk is existing.", uuid);
        // TODO: new mounting class with Drop trait <16-12-20, Heiko Riemer> //
        let logfile = Config::generate_logfilename(&cfg.log_dir, "restic", src, dest);
        println!(
            "Run restic backup ({}) to disk={}: \"{}\" --> \"{}\"",
            comment, uuid, src, dest,
        );
        // TODO: set pw as env variable <13-12-20, Heiko Riemer> //
        // TODO: init if repo does not exit <13-12-20, Heiko Riemer> //
        // TODO: run backup <13-12-20, Heiko Riemer> //
        let output = Command::new("ls").output().expect("unable to run ls");
        // TODO: check backup <13-12-20, Heiko Riemer> //
        // TODO: prune backups <13-12-20, Heiko Riemer> //
        // TODO: verify backup or rely on TDD test <13-12-20, Heiko Riemer> //
        println!("End restic backup ({}): {}", comment, output.status,);
        if !output.status.success() {
            std::fs::rename(
                &logfile,
                &format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
            )
            .expect("logfile cannot be renamed");
            thread::sleep(time::Duration::from_secs(cfg.retry_interval_sec));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_restic_config() {
        assert!(false)
    }
}
