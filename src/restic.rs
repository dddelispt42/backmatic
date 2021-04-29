use crate::config::{BackupConfig, Config};
use threadpool::ThreadPool;

static BUPTYPE: &str = "restic";
static BUPCMD: &str = "/usr/bin/restic";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc[BUPTYPE].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            // TODO sanitize all inputs from the yaml files
            // TODO: mount if "mount" key exist (see borg)
            // TODO: check if user can mount - or skip
            // let mounter = Mounter::new(uuid, None);
            let bupcfg = BackupConfig::new(&my_item, BUPTYPE);
            run_restic_backup(&my_cfg, &bupcfg);
            // TODO: skip in case of Windows and for bad file system
        });
    }
    pool.join();
}

fn run_restic_backup(cfg: &Config, bup: &BackupConfig) {
    if !Config::command_existing(BUPCMD) {
        return;
    }
    for dest in &bup.dest {
        for _ in 1..cfg.retry_count {
            log::info!(
                "Start {} ({}): \"{:?}\" --> \"{:?}\"",
                bup.buptype,
                bup.comment,
                bup.src,
                dest,
            );
        }
    }
    // TODO: impl me
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_valid_restic_config() {
        assert!(false)
    }
}
