use std::path::{Path, PathBuf};

use crate::config::types::DestMountEntry;
use crate::error::{BackmaticError, Result};
use crate::inject::{command_exists, BackmaticContext, CommandRequest};
use std::process::Stdio;

#[derive(Debug, Clone)]
struct MountCleanup {
    mountpoint: PathBuf,
    mapper: Option<String>,
    umount_bin: PathBuf,
    cryptsetup_bin: PathBuf,
}

pub struct DestMountSession {
    cleanups: Vec<MountCleanup>,
    commands: std::sync::Arc<dyn crate::inject::CommandExecutor>,
    registry: std::sync::Arc<crate::mount::MountRegistry>,
}

impl DestMountSession {
    /// Mount every destmount device. A failure on any device is fatal: the partially
    /// established session is dropped (unmounting whatever already succeeded) and the error is
    /// propagated so the job fails and is retried, rather than silently backing up to a subset
    /// of destinations.
    pub fn mount_all(
        ctx: &BackmaticContext,
        job_scope: &str,
        entries: &[DestMountEntry],
    ) -> Result<(Vec<String>, Self)> {
        let mut resolved = Vec::new();
        let mut session = Self {
            cleanups: Vec::new(),
            commands: std::sync::Arc::clone(&ctx.commands),
            registry: std::sync::Arc::clone(&ctx.mount_registry),
        };
        for entry in entries {
            let (path, cleanup) = mount_one(ctx, job_scope, entry).map_err(|e| {
                BackmaticError::Mount(format!("destmount {} failed: {e}", entry.uuid))
            })?;
            ctx.mount_registry.register_dest(
                cleanup.mountpoint.clone(),
                cleanup.mapper.clone(),
                cleanup.umount_bin.clone(),
                cleanup.cryptsetup_bin.clone(),
                std::sync::Arc::clone(&ctx.commands),
            );
            resolved.push(path);
            session.cleanups.push(cleanup);
        }
        Ok((resolved, session))
    }
}

impl Drop for DestMountSession {
    fn drop(&mut self) {
        for c in self.cleanups.drain(..) {
            let mountpoint = c.mountpoint.clone();
            log::info!("Unmounting destmount at {}", mountpoint.display());
            let cmd = CommandRequest::new(c.umount_bin.to_string_lossy().to_string())
                .arg(mountpoint.to_string_lossy().to_string());
            if let Ok(r) = self.commands.run(&cmd) {
                if !r.status.success() {
                    log::warn!("umount {} failed", mountpoint.display());
                }
            }
            if let Some(mapper) = c.mapper {
                let close = CommandRequest::new(c.cryptsetup_bin.to_string_lossy().to_string())
                    .arg("luksClose")
                    .arg(mapper);
                let _ = self.commands.run(&close);
            }
            self.registry.unregister_dest(&mountpoint);
        }
    }
}

fn mount_one(
    ctx: &BackmaticContext,
    job_scope: &str,
    entry: &DestMountEntry,
) -> Result<(String, MountCleanup)> {
    let mountpoint = entry
        .mountpoint
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/mnt/backapp/{job_scope}/{}", entry.uuid)));

    log::info!(
        "Start destmount {} at {}",
        entry.uuid,
        mountpoint.display()
    );

    let device = PathBuf::from(format!("/dev/disk/by-uuid/{}", entry.uuid));
    // Removable/hot-plugged devices can lag behind udev; wait briefly before giving up.
    wait_for_device(ctx, &device)?;

    std::fs::create_dir_all(&mountpoint).map_err(|source| BackmaticError::Io {
        path: mountpoint.display().to_string(),
        source,
    })?;

    let mut block_device = device.clone();
    let mut mapper_name = None;

    if let Some(pw) = &entry.password {
        let mapper = entry.uuid.clone();
        let open = CommandRequest::new(ctx.paths.cryptsetup.to_string_lossy().to_string())
            .arg("luksOpen")
            .arg(device.to_string_lossy().to_string())
            .arg(&mapper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = ctx.commands.spawn(&open)?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(pw.as_bytes())
                .map_err(|source| BackmaticError::Io {
                    path: "cryptsetup stdin".into(),
                    source,
                })?;
        }
        let output = child
            .wait_with_output()
            .map_err(|e| BackmaticError::Mount(format!("cryptsetup luksOpen: {e}")))?;
        if !output.status.success() {
            return Err(BackmaticError::Mount(format!(
                "cryptsetup luksOpen: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        block_device = PathBuf::from(format!("/dev/mapper/{mapper}"));
        mapper_name = Some(mapper);
    }

    let mount_cmd = CommandRequest::new(ctx.paths.mount.to_string_lossy().to_string())
        .arg(block_device.to_string_lossy().to_string())
        .arg(mountpoint.to_string_lossy().to_string());

    let result = ctx.commands.run(&mount_cmd)?;
    if !result.status.success() {
        // Roll back the LUKS mapping we just opened so we don't leak a mapper on failure.
        if let Some(mapper) = &mapper_name {
            let close = CommandRequest::new(ctx.paths.cryptsetup.to_string_lossy().to_string())
                .arg("luksClose")
                .arg(mapper.clone());
            let _ = ctx.commands.run(&close);
        }
        return Err(BackmaticError::Mount(format!(
            "mount failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )));
    }

    let sub = if entry.path == "." {
        PathBuf::new()
    } else {
        PathBuf::from(&entry.path)
    };
    let resolved_dest = mountpoint.join(sub);
    std::fs::create_dir_all(&resolved_dest).ok();

    log::info!(
        "Finished destmount {} at {} (backup dest {})",
        entry.uuid,
        mountpoint.display(),
        resolved_dest.display()
    );

    Ok((
        resolved_dest.to_string_lossy().to_string(),
        MountCleanup {
            mountpoint,
            mapper: mapper_name,
            umount_bin: ctx.paths.umount.clone(),
            cryptsetup_bin: ctx.paths.cryptsetup.clone(),
        },
    ))
}

/// Poll for a block device node to appear, tolerating udev latency for removable drives.
fn wait_for_device(ctx: &BackmaticContext, device: &Path) -> Result<()> {
    const MAX_ATTEMPTS: u32 = 10;
    for attempt in 1..=MAX_ATTEMPTS {
        if device.exists() {
            return Ok(());
        }
        if attempt < MAX_ATTEMPTS {
            log::debug!(
                "waiting for device {} (attempt {attempt}/{MAX_ATTEMPTS})",
                device.display()
            );
            ctx.clock.sleep(std::time::Duration::from_secs(1));
        }
    }
    Err(BackmaticError::Mount(format!(
        "device not found after {MAX_ATTEMPTS}s: {}",
        device.display()
    )))
}

pub fn resolve_destinations(
    ctx: &BackmaticContext,
    job_scope: &str,
    comment: &str,
    dest: &[String],
    destmount: &[DestMountEntry],
) -> Result<(Vec<String>, Option<DestMountSession>)> {
    let mut out: Vec<String> = dest.to_vec();
    let session = if destmount.is_empty() {
        None
    } else {
        log::info!(
            "Start destmount for job '{job_scope}' ({} device(s))",
            destmount.len()
        );
        let (mounted, session) = DestMountSession::mount_all(ctx, job_scope, destmount)?;
        log::info!(
            "Finished destmount for job '{job_scope}' ({} path(s))",
            mounted.len()
        );
        out.extend(mounted);
        Some(session)
    };
    if out.is_empty() {
        return Err(BackmaticError::NoDestination {
            comment: comment.to_string(),
        });
    }
    Ok((out, session))
}

pub fn tool_available(ctx: &BackmaticContext, path: &Path) -> bool {
    command_exists(ctx.commands.as_ref(), &path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{AppConfig, FileConfig};
    use crate::inject::{BackmaticContext, FakeClock};
    use std::sync::Arc;

    fn fast_ctx() -> BackmaticContext {
        let config = AppConfig {
            lock_file: "/tmp/lock".into(),
            config_path: "/tmp/cfg".into(),
            threadpool_size: 1,
            retry_interval_sec: 0,
            retry_count: 1,
            continuous_hours: 0,
            dry_run: false,
            file: FileConfig::default(),
        };
        // FakeClock makes wait_for_device's per-second sleeps instant.
        let clock = FakeClock::new(chrono::Local::now());
        BackmaticContext::new(config).with_clock(Arc::new(clock))
    }

    #[test]
    fn wait_for_device_errors_when_absent() {
        let ctx = fast_ctx();
        let missing = Path::new("/dev/disk/by-uuid/does-not-exist-backmatic-test");
        let err = wait_for_device(&ctx, missing).unwrap_err();
        assert!(err.to_string().contains("device not found"), "unexpected: {err}");
    }

    #[test]
    fn wait_for_device_ok_when_present() {
        let ctx = fast_ctx();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        wait_for_device(&ctx, tmp.path()).expect("existing path should resolve");
    }
}
