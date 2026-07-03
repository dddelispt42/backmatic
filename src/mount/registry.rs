use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::inject::{CommandExecutor, CommandRequest};

#[derive(Clone)]
struct DestMountCleanup {
    mountpoint: PathBuf,
    mapper: Option<String>,
    umount_bin: PathBuf,
    cryptsetup_bin: PathBuf,
    commands: Arc<dyn CommandExecutor>,
}

#[derive(Clone)]
struct SshfsCleanup {
    mountpoint: PathBuf,
    fusermount: PathBuf,
    commands: Arc<dyn CommandExecutor>,
}

#[derive(Clone)]
enum TrackedMount {
    Sshfs(SshfsCleanup),
    Dest(DestMountCleanup),
}

/// Tracks active staging and destination mounts for signal-driven cleanup.
#[derive(Default)]
pub struct MountRegistry {
    mounts: Mutex<Vec<TrackedMount>>,
}

impl MountRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_sshfs(
        &self,
        mountpoint: PathBuf,
        fusermount: PathBuf,
        commands: Arc<dyn CommandExecutor>,
    ) {
        self.mounts.lock().unwrap().push(TrackedMount::Sshfs(SshfsCleanup {
            mountpoint,
            fusermount,
            commands,
        }));
    }

    pub fn unregister_sshfs(&self, mountpoint: &Path) {
        self.mounts.lock().unwrap().retain(|m| match m {
            TrackedMount::Sshfs(c) => c.mountpoint != mountpoint,
            _ => true,
        });
    }

    pub fn register_dest(
        &self,
        mountpoint: PathBuf,
        mapper: Option<String>,
        umount_bin: PathBuf,
        cryptsetup_bin: PathBuf,
        commands: Arc<dyn CommandExecutor>,
    ) {
        self.mounts.lock().unwrap().push(TrackedMount::Dest(
            DestMountCleanup {
                mountpoint,
                mapper,
                umount_bin,
                cryptsetup_bin,
                commands,
            },
        ));
    }

    pub fn unregister_dest(&self, mountpoint: &Path) {
        self.mounts.lock().unwrap().retain(|m| match m {
            TrackedMount::Dest(c) => c.mountpoint != mountpoint,
            _ => true,
        });
    }

    pub fn active_count(&self) -> usize {
        self.mounts.lock().unwrap().len()
    }

    /// Unmount every tracked mount and clear the registry.
    pub fn unmount_all(&self) {
        let mounts: Vec<TrackedMount> = {
            let mut guard = self.mounts.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        for mount in mounts {
            match mount {
                TrackedMount::Sshfs(c) => {
                    log::info!("Unmounting srcmount at {}", c.mountpoint.display());
                    super::src_remote::unmount_sshfs(
                        c.commands.as_ref(),
                        &c.fusermount,
                        &c.mountpoint,
                    );
                }
                TrackedMount::Dest(c) => {
                    log::info!("Unmounting destmount at {}", c.mountpoint.display());
                    unmount_dest(&c);
                }
            }
        }
    }
}

fn unmount_dest(c: &DestMountCleanup) {
    let cmd = CommandRequest::new(c.umount_bin.to_string_lossy().to_string())
        .arg(c.mountpoint.to_string_lossy().to_string());
    if let Ok(r) = c.commands.run(&cmd) {
        if !r.status.success() {
            log::warn!("umount {} failed", c.mountpoint.display());
        }
    }
    if let Some(mapper) = &c.mapper {
        let close = CommandRequest::new(c.cryptsetup_bin.to_string_lossy().to_string())
            .arg("luksClose")
            .arg(mapper.clone());
        let _ = c.commands.run(&close);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    use super::*;
    use crate::inject::commands::{CommandExecutor, CommandRequest, CommandResult};
    use crate::error::Result;

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
    fn unregister_removes_tracked_mounts() {
        let registry = MountRegistry::new();
        let path = PathBuf::from("/tmp/test-mount");
        registry.register_sshfs(
            path.clone(),
            PathBuf::from("fusermount"),
            Arc::new(NoopExecutor),
        );
        assert_eq!(registry.active_count(), 1);
        registry.unregister_sshfs(&path);
        assert_eq!(registry.active_count(), 0);
    }

    #[test]
    fn unmount_all_clears_registry() {
        let registry = MountRegistry::new();
        registry.register_sshfs(
            PathBuf::from("/tmp/a"),
            PathBuf::from("fusermount"),
            Arc::new(NoopExecutor),
        );
        registry.register_dest(
            PathBuf::from("/mnt/b"),
            None,
            PathBuf::from("/bin/umount"),
            PathBuf::from("/sbin/cryptsetup"),
            Arc::new(NoopExecutor),
        );
        assert_eq!(registry.active_count(), 2);
        registry.unmount_all();
        assert_eq!(registry.active_count(), 0);
    }
}
