use crate::config::{BackupConfig, Config};
use threadpool::ThreadPool;

static BUPTYPE: &str = "database";
static BUPCMD: &str = "TODO";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc["database"].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            let bupcfg = BackupConfig::new(&my_item, BUPTYPE);
            run_database_backup(&my_cfg, &bupcfg);
        });
    }
    pool.join();
}

fn run_database_backup(cfg: &Config, bup: &BackupConfig) {
    if !Config::command_existing(BUPCMD) {
        return;
    }
    for dest in &bup.dest {
        for _ in 1..cfg.retry_count {
            log::info!(
                "Run {} backup ({}): \"{:?}\" --> \"{:?}\"",
                bup.buptype,
                bup.comment,
                bup.src,
                dest,
            );
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
