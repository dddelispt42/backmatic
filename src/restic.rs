use crate::config::{BackupConfig, Config};
use crate::mount::Mounter;
use threadpool::ThreadPool;

static BUPTYPE: &str = "restic";
static BUPCMD: &str = "/usr/bin/restic";

pub fn run(cfg: &Config) {
    let pool = ThreadPool::new(cfg.threadpool_size);
    for item in cfg.doc[BUPTYPE].as_vec().unwrap_or(&Vec::new()) {
        let my_item = item.clone();
        let my_cfg = cfg.clone();
        pool.execute(move || {
            let bupcfg = BackupConfig::new(&my_item, BUPTYPE);
            log::debug!("BackupConfig: {:?}", bupcfg);
            if Config::command_existing(BUPCMD) {
                let mut mounter = Mounter::new(&bupcfg.destmount);
                match mounter.mount() {
                    Ok(_) => {
                        run_restic_backup(&my_cfg, &bupcfg);
                    }
                    Err(_) => {},
                }
            } else {
                log::error!("{} not installed on machine!", BUPCMD);
            }
        });
    }
    pool.join();
}

fn run_restic_backup(cfg: &Config, bup: &BackupConfig) {
    if !Config::command_existing(BUPCMD) {
        log::error!("{} not installed on machine!", BUPCMD);
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
