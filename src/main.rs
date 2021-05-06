use single_instance::SingleInstance;
use std::{thread, time};

mod borg;
mod config;
mod database;
mod mount;
mod restic;
mod rsync;

pub struct Backmatic {
    conf: config::Config,
    _instance: SingleInstance,
}

impl Backmatic {
    pub fn new() -> Backmatic {
        let cfg = config::Config::new();
        log::debug!("Settings: {:?}", cfg);
        let instance = SingleInstance::new(&cfg.lock_file)
            .expect("Cannot create a lockfile to guarantee single application use.");
        if !instance.is_single() {
            panic!("Another backup is running! Terminating...")
        }
        Backmatic {
            conf: cfg,
            _instance: instance,
        }
    }
    pub fn run(&self) {
        loop {
            log::debug!("Running rsync backups...");
            rsync::run(&self.conf);
            log::debug!("Running borg backups...");
            borg::run(&self.conf);
            log::debug!("Running restic backups...");
            restic::run(&self.conf);
            log::debug!("Running database backups...");
            database::run(&self.conf);
            if self.conf.cycle_time == 0 {
                log::info!("... terminating!");
                break;
            } else {
                log::info!("Repeat after {}sec!", self.conf.cycle_time);
                thread::sleep(time::Duration::from_secs(self.conf.cycle_time));
            }
        }
    }
}

impl Default for Backmatic {
    fn default() -> Self {
        Self::new()
    }
}

fn main() {
    log::debug!("Starting application...");
    let app = Backmatic::new();
    log::debug!("Created application object");
    app.run();
    log::debug!("... terminating application.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_multiple_instances() {
        let app = Backmatic::new();
        assert!(app._instance.is_single());
        let app2 = Backmatic::new();
        assert!(!app2._instance.is_single());
    }
}
