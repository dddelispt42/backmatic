use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

/// Tracks PIDs of child processes spawned via [`RealCommandExecutor`](super::RealCommandExecutor)
/// so they can be terminated on shutdown before mounts are torn down.
#[derive(Debug, Default)]
pub struct ProcessRegistry {
    pids: Mutex<HashSet<u32>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, pid: u32) {
        self.pids.lock().unwrap().insert(pid);
    }

    pub fn unregister(&self, pid: u32) {
        self.pids.lock().unwrap().remove(&pid);
    }

    pub fn active_count(&self) -> usize {
        self.pids.lock().unwrap().len()
    }

    fn snapshot(&self) -> Vec<u32> {
        self.pids.lock().unwrap().iter().copied().collect()
    }

    /// Terminate all tracked child processes. Sends SIGTERM, waits briefly for them to
    /// exit (workers unregister as `wait` returns), then SIGKILLs any survivors.
    pub fn kill_all(&self) {
        let pids = self.snapshot();
        if pids.is_empty() {
            return;
        }
        log::warn!("Terminating {} active child process(es)", pids.len());
        signal_all(&pids, term_signal());

        // Give children a moment to exit cleanly; workers remove themselves as they reap.
        for _ in 0..30 {
            if self.active_count() == 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let survivors = self.snapshot();
        if !survivors.is_empty() {
            log::warn!("Force-killing {} unresponsive child process(es)", survivors.len());
            signal_all(&survivors, kill_signal());
        }
    }
}

#[cfg(unix)]
fn term_signal() -> i32 {
    libc::SIGTERM
}

#[cfg(unix)]
fn kill_signal() -> i32 {
    libc::SIGKILL
}

#[cfg(not(unix))]
fn term_signal() -> i32 {
    0
}

#[cfg(not(unix))]
fn kill_signal() -> i32 {
    0
}

#[cfg(unix)]
fn signal_all(pids: &[u32], sig: i32) {
    for &pid in pids {
        // SAFETY: kill() with a valid pid is safe; failures (e.g. already-exited) are ignored.
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
    }
}

#[cfg(not(unix))]
fn signal_all(_pids: &[u32], _sig: i32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_unregister_track_pids() {
        let reg = ProcessRegistry::new();
        reg.register(1234);
        reg.register(5678);
        assert_eq!(reg.active_count(), 2);
        reg.unregister(1234);
        assert_eq!(reg.active_count(), 1);
    }

    #[test]
    fn kill_all_on_empty_is_noop() {
        let reg = ProcessRegistry::new();
        reg.kill_all();
        assert_eq!(reg.active_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn kill_all_terminates_real_child() {
        use std::sync::Arc;

        let reg = Arc::new(ProcessRegistry::new());
        let worker = Arc::clone(&reg);
        // Spawn a long-lived child and wait on it in a background thread.
        let handle = std::thread::spawn(move || {
            let mut child = std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn sleep");
            let pid = child.id();
            worker.register(pid);
            let status = child.wait().expect("wait");
            worker.unregister(pid);
            status.success()
        });

        // Wait for the child to be registered.
        for _ in 0..50 {
            if reg.active_count() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(reg.active_count(), 1, "child should be registered");

        reg.kill_all();
        let succeeded = handle.join().expect("join");
        assert!(!succeeded, "killed sleep should not exit successfully");
        assert_eq!(reg.active_count(), 0);
    }
}
