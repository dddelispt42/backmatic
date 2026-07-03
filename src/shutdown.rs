use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::Result;
use crate::inject::ProcessRegistry;
use crate::mount::MountRegistry;

/// Coordinates graceful shutdown on SIGINT/SIGTERM.
pub struct Shutdown {
    requested: AtomicBool,
    mount_registry: Arc<MountRegistry>,
    process_registry: Arc<ProcessRegistry>,
    lock_path: PathBuf,
}

impl Shutdown {
    pub fn new(
        mount_registry: Arc<MountRegistry>,
        process_registry: Arc<ProcessRegistry>,
        lock_path: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            requested: AtomicBool::new(false),
            mount_registry,
            process_registry,
            lock_path,
        })
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Mark shutdown requested, then clean up: kill running child processes *before* tearing
    /// down mounts (so nothing is still writing into a mountpoint when it is unmounted).
    /// Idempotent.
    pub fn request(&self, reason: &str) {
        if self.requested.swap(true, Ordering::SeqCst) {
            return;
        }
        log::warn!("Shutdown requested ({reason}), cleaning up...");
        self.process_registry.kill_all();
        self.mount_registry.unmount_all();
    }

    pub fn lock_path(&self) -> &PathBuf {
        &self.lock_path
    }

    #[cfg(unix)]
    pub fn install_handlers(self: &Arc<Self>) -> Result<()> {
        use signal_hook::consts::signal::{SIGINT, SIGTERM};
        use signal_hook::iterator::Signals;

        let state = Arc::clone(self);
        let mut signals =
            Signals::new([SIGTERM, SIGINT]).map_err(|e| crate::error::BackmaticError::Other(e.into()))?;

        std::thread::Builder::new()
            .name("backmatic-signals".into())
            .spawn(move || {
                if let Some(sig) = signals.forever().next() {
                    let reason = match sig {
                        SIGTERM => "SIGTERM",
                        SIGINT => "SIGINT",
                        _ => "signal",
                    };
                    state.request(reason);
                }
            })
            .map_err(|e| crate::error::BackmaticError::Other(e.into()))?;

        Ok(())
    }

    #[cfg(not(unix))]
    pub fn install_handlers(_self: &Arc<Self>) -> Result<()> {
        Ok(())
    }
}

/// Remove the lock file after the flock has been released.
pub fn cleanup_lock_file(lock_path: &std::path::Path) {
    if lock_path.exists() {
        match std::fs::remove_file(lock_path) {
            Ok(()) => log::info!("Removed lock file {}", lock_path.display()),
            Err(e) => log::warn!("could not remove lock file {}: {e}", lock_path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::inject::commands::{CommandExecutor, CommandRequest, CommandResult};
    use std::os::unix::process::ExitStatusExt;

    struct NoopExecutor;
    impl CommandExecutor for NoopExecutor {
        fn run(&self, _request: &CommandRequest) -> Result<CommandResult> {
            Ok(CommandResult {
                status: std::process::ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
        fn spawn(&self, _request: &CommandRequest) -> Result<std::process::Child> {
            unimplemented!()
        }
    }

    #[test]
    fn request_sets_flag_and_unmounts_then_is_idempotent() {
        let mounts = Arc::new(MountRegistry::new());
        let processes = Arc::new(ProcessRegistry::new());
        let dir = std::env::temp_dir().join(format!("backmatic-shutdown-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        mounts.register_sshfs(dir.clone(), PathBuf::from("fusermount"), Arc::new(NoopExecutor));
        assert_eq!(mounts.active_count(), 1);

        let shutdown = Shutdown::new(Arc::clone(&mounts), Arc::clone(&processes), PathBuf::from("/tmp/x.lock"));
        assert!(!shutdown.is_requested());

        shutdown.request("SIGTERM");
        assert!(shutdown.is_requested());
        assert_eq!(mounts.active_count(), 0, "mounts should be cleaned up on shutdown");

        // Second call is a no-op (does not panic / re-run cleanup).
        shutdown.request("SIGINT");
        assert!(shutdown.is_requested());
    }
}
