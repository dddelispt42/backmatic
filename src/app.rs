use std::sync::Arc;

use single_instance::SingleInstance;

use crate::cli::CliArgs;
use crate::config::load_app_config;
use crate::error::{BackmaticError, Result};
use crate::inject::BackmaticContext;
use crate::scheduler::{CycleScheduler, Scheduler};
use crate::shutdown::cleanup_lock_file;

pub struct Backmatic {
    ctx: BackmaticContext,
    scheduler: Scheduler,
    lock: Option<SingleInstance>,
}

impl Backmatic {
    pub fn from_cli(cli: CliArgs) -> Result<Self> {
        let config = load_app_config(&cli)?;
        let lock_path = config.lock_file.to_string_lossy().to_string();
        let lock = SingleInstance::new(&lock_path).map_err(|e| BackmaticError::Other(e.into()))?;
        if !lock.is_single() {
            return Err(BackmaticError::AlreadyRunning {
                lock_file: lock_path,
            });
        }
        let jobs = config.file.all_jobs();
        let ctx = BackmaticContext::new(config);
        ctx.shutdown.install_handlers()?;
        let scheduler = Scheduler::new(ctx.clone(), jobs);
        Ok(Self {
            ctx: scheduler.context(),
            scheduler,
            lock: Some(lock),
        })
    }

    pub fn run(&self) -> Result<i32> {
        let shutdown = Arc::clone(&self.ctx.shutdown);
        let mut cycle = if self.ctx.config.continuous_hours > 0 {
            Some(CycleScheduler::new(
                self.ctx.clock.as_ref(),
                self.ctx.config.continuous_hours,
            ))
        } else {
            None
        };

        loop {
            if shutdown.is_requested() {
                log::info!("Shutdown complete");
                return Ok(130);
            }

            log::info!("Starting backup cycle");
            let ok = self.scheduler.run_cycle()?;
            if shutdown.is_requested() {
                log::info!("Shutdown complete");
                return Ok(130);
            }
            if self.ctx.config.continuous_hours == 0 {
                return Ok(if ok { 0 } else { 1 });
            }
            if let Some(ref mut sched) = cycle {
                sched.advance();
                let overrun = sched.sleep_until_next(self.ctx.clock.as_ref(), shutdown.as_ref());
                if overrun {
                    self.scheduler.on_overrun_tick();
                }
            }
        }
    }

    fn release_lock(&mut self) {
        if let Some(lock) = self.lock.take() {
            drop(lock);
            cleanup_lock_file(&self.ctx.config.lock_file);
        }
    }
}

impl Drop for Backmatic {
    fn drop(&mut self) {
        self.release_lock();
    }
}
