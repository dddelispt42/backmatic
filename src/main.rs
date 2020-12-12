// use cmd_lib::{run_cmd, CmdResult};
use chrono::Utc;
use regex::Regex;
use single_instance::SingleInstance;
use std::process::Command;
use std::{fs, thread, time};
use threadpool::ThreadPool;
use yaml_rust::{Yaml, YamlLoader};

static LOCK_FILE: &str = "/tmp/backup.lock";
static CONFIG_FILE: &str = "/home/heiko/backups.yml";
static LOGDIR: &str = "/tmp";
static THREADPOOLSIZE: usize = 4;
static RETRYINTERVAL: u64 = 3600;
static RETRYCOUNT: u32 = 23;

fn try_single_app_locking() -> SingleInstance {
    let instance = SingleInstance::new(LOCK_FILE)
        .expect("Cannot create a lockfile to guarantee single application use.");
    if !instance.is_single() {
        panic!("Another backup is running! Terminating...")
    }
    instance
}

fn check_preconditions() {
    // TODO: CLI commands <07-12-20, Heiko Riemer> //
}

fn get_config(filename: &str) -> Result<Vec<Yaml>, &str> {
    let s = fs::read_to_string(filename).expect("Cannot read backup configuration file.");
    // TODO: use yaml_validator and define schema to check against <07-12-20, Heiko Riemer> //
    let docs = YamlLoader::load_from_str(&s).expect("Cannot load deserialize yaml content.");
    Ok(docs)
}

fn filenamify(input: &str) -> String {
    let re = Regex::new("[!<>:\'\"/\\|?*+]").expect("Problem in RegEx.");
    return re.replace_all(input, "_").to_string();
}

fn generate_logfilename(buptype: &str, src: &str, dest: &str) -> String {
    let logstring: &str = &format!(
        "{}-{}---{}_{}.log",
        buptype,
        src,
        dest,
        Utc::now().format("%Y%m%d%H%M")
    );
    return String::from(&format!("{}/{}", LOGDIR, &filenamify(logstring)));
}

fn run_rsync_backup(comment: &str, src: &str, dest: &str, excludelist: &Vec<Yaml>) {
    for _ in 1..RETRYCOUNT {
        let logfile = generate_logfilename("rsync", src, dest);
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
            thread::sleep(time::Duration::from_secs(RETRYINTERVAL));
        } else {
            break;
        }
    }
}

fn run_borg_backup(comment: &str, src: &str, dest: &str, excludelist: &Vec<Yaml>) {
    println!(
        "Run borg backup ({}): \"{}\" --> \"{}\" - Excludes: {:?}",
        comment, src, dest, excludelist
    );
}

fn main() {
    let _instance = try_single_app_locking();
    check_preconditions();
    // TODO: unwrap --> expect <10-12-20, Heiko Riemer> //
    let docs = get_config(CONFIG_FILE).unwrap();
    let doc = &docs[0];

    let pool = ThreadPool::new(THREADPOOLSIZE);
    for item in doc["rsync"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        pool.execute(move || {
            run_rsync_backup(
                my_item["comment"].as_str().unwrap(),
                my_item["src"].as_str().unwrap(),
                my_item["dest"].as_str().unwrap(),
                my_item["exclude"].as_vec().unwrap(),
            );
        });
    }
    pool.join();
    for item in doc["borg"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        pool.execute(move || {
            run_borg_backup(
                my_item["comment"].as_str().unwrap(),
                my_item["src"].as_str().unwrap(),
                my_item["dest"].as_str().unwrap(),
                my_item["exclude"].as_vec().unwrap(),
            );
        });
    }
    pool.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_multiple_instances() {
        let _instance1 = try_single_app_locking();
        assert!(_instance1.is_single());
        let _instance2 = try_single_app_locking();
        assert!(!_instance1.is_single());
    }
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
    #[test]
    fn test_read_valid_borg_config() {
        assert!(false)
    }
}
