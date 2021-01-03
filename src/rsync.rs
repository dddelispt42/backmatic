use crate::config::Config;
use std::process::Command;
use std::{thread, time};
use threadpool::ThreadPool;
use yaml_rust::Yaml;

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["rsync"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
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
        println!("End rsync backup ({}): {}", comment, output.status,);
        if !output.status.success() {
            std::fs::rename(
                &logfile,
                &format!("{}.ERROR_{}", logfile, output.status.code().unwrap_or(0)),
            )
            .expect("logfile cannot be renamed");
            thread::sleep(time::Duration::from_secs(cfg.retry_interval_sec));
        } else {
            break;
        }
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
