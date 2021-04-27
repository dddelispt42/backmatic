use single_instance::SingleInstance;

mod borg;
mod config;
mod database;
mod mount;
mod restic;
mod rsync;

pub struct BackApp {
    conf: config::Config,
    _instance: SingleInstance,
}

impl BackApp {
    pub fn new() -> BackApp {
        let cfg = config::Config::new();
        // println!("Settings: {:?}", cfg);
        let instance = SingleInstance::new(&cfg.lock_file)
            .expect("Cannot create a lockfile to guarantee single application use.");
        if !instance.is_single() {
            panic!("Another backup is running! Terminating...")
        }
        BackApp {
            conf: cfg,
            _instance: instance,
        }
    }
    pub fn run(&self) {
        rsync::run(&self.conf);
        borg::run(&self.conf);
        restic::run(&self.conf);
        database::run(&self.conf);
    }
}

impl Default for BackApp {
    fn default() -> Self {
        Self::new()
    }
}

fn main() {
    let app = BackApp::new();
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_multiple_instances() {
        let app = BackApp::new();
        assert!(app._instance.is_single());
        let app2 = BackApp::new();
        assert!(!app2._instance.is_single());
    }
}
