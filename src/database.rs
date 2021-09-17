use crate::config::{BackupConfig, Config};
use std::fs::File;
use threadpool::ThreadPool;
use std::process::{Command, Stdio};

static BUPTYPE: &str = "database";
static BUPCMD: &str = "/usr/bin/mysqldump";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["database"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        log::debug!("YAML item: {:?}", my_item);
        let my_cfg = cfg.clone();
        pool.execute(move || {
            let bupcfg = BackupConfig::new(&my_item, BUPTYPE);
            log::debug!("BackupConfig: {:?}", bupcfg);
            if Config::command_existing(BUPCMD) {
                run_database_backup(&my_cfg, &bupcfg);
            } else {
                log::error!("{} not installed on machine!", BUPCMD);
            }
        });
    }
    pool.join();
}

fn run_database_backup(_cfg: &Config, bup: &BackupConfig) {
    let host = match &bup.host {
        None => "localhost".to_string(),
        Some(value) => value.clone(),
    };
    let user = match &bup.user {
        None => "root".to_string(),
        Some(value) => value.clone(),
    };
    let password = match &bup.password {
        None => "".to_string(),
        Some(value) => value.clone(),
    };
    for dest in &bup.dest {
        for src in &bup.src {
            log::info!(
                "Start {} ({}): \"{:?}\" --> \"{:?}\"",
                bup.buptype,
                bup.comment,
                src,
                dest,
            );
            let mut db:&str = "--all-databases";
            if src != "" {
                db = &src;
            }
            let filename:&str = &format!("{}/{}_{}.gz", dest, BackupConfig::filenamify(&bup.comment), src);
            log::debug!("Filename {}", filename);
            let bupfile = File::create(filename).expect("Cannot create the DB backup file.");
            let cmd1 = Command::new(BUPCMD).arg("-u").arg(&user).arg(&format!("-p{}", &password)).arg("-h").arg(&host).arg(db).arg("-E").stdout(Stdio::piped()).spawn().expect("failed to call db backup");
            log::debug!("{} backup starting: Command={:?}", BUPTYPE, cmd1);
            let output = Command::new("gzip").stdin(cmd1.stdout.unwrap()).stdout(Stdio::from(bupfile)).output().expect("Cannot run the gzip command.");

            Config::log_output(&bup.logfile, &output);
            if !output.status.success() {
                log::warn!("Failed {} backup ({}): {}", BUPTYPE, bup.comment, output.status);
                continue;
            }
            log::info!("End {} backup ({}): {}", BUPTYPE, bup.comment, output.status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_database_config() {
        assert!(false)
    }
}
