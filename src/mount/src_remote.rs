use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::defaults::default_staging_base;
use crate::config::types::{FileBackupJob, SrcMountEntry};
use crate::error::{BackmaticError, Result};
use crate::inject::{BackmaticContext, CommandExecutor, CommandRequest};
use crate::mount::origin::{origin_slug_remote, SourcedPath};
use crate::mount::MountRegistry;

pub struct StagingGuard {
    mounts: Vec<PathBuf>,
    registry: Arc<MountRegistry>,
    commands: Arc<dyn CommandExecutor>,
    fusermount: PathBuf,
}

impl StagingGuard {
    fn new(
        registry: Arc<MountRegistry>,
        commands: Arc<dyn CommandExecutor>,
        fusermount: PathBuf,
    ) -> Self {
        Self {
            mounts: Vec::new(),
            registry,
            commands,
            fusermount,
        }
    }

    fn track(&mut self, mountpoint: PathBuf) {
        self.registry.register_sshfs(
            mountpoint.clone(),
            self.fusermount.clone(),
            Arc::clone(&self.commands),
        );
        self.mounts.push(mountpoint);
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.mounts.clone()
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        for mountpoint in self.mounts.drain(..) {
            log::info!("Unmounting srcmount at {}", mountpoint.display());
            unmount_sshfs(self.commands.as_ref(), &self.fusermount, &mountpoint);
            self.registry.unregister_sshfs(&mountpoint);
        }
    }
}

/// Per-job sshfs mount point: `{base}/{job_scope}/{origin_slug}/`.
pub fn srcmount_path(base: &Path, job_scope: &str, origin_slug: &str) -> PathBuf {
    base.join(job_scope).join(origin_slug)
}

/// SSH options enforced on every srcmount transport (sshfs for borg/restic, `ssh` for rsync) so
/// the tool can never block on an interactive prompt. `backmatic` runs unattended, so a missing
/// key/passphrase or an unknown/changed host key must fail fast instead of waiting forever on
/// stdin. These bound only *connection setup*: an authenticated session may run as long as the
/// transfer needs, so slow/large/long-running backups are unaffected.
///
/// `BatchMode=yes` and `ConnectTimeout` are prepended (ssh uses the first value seen for a
/// keyword, so they can't be accidentally weakened by `ssh_options`). The host-key policy
/// defaults to `accept-new` (TOFU: auto-add unknown hosts, reject changed keys) but is left to
/// the user if they pin their own `StrictHostKeyChecking`.
pub fn noninteractive_ssh_opts(user_opts: &[String]) -> Vec<String> {
    let mut opts = vec![
        "BatchMode=yes".to_string(),
        "ConnectTimeout=30".to_string(),
    ];
    let user_sets_policy = user_opts
        .iter()
        .any(|o| o.to_ascii_lowercase().starts_with("stricthostkeychecking"));
    if !user_sets_policy {
        opts.push("StrictHostKeyChecking=accept-new".to_string());
    }
    opts.extend(user_opts.iter().cloned());
    opts
}

fn staging_base_for(entry: &SrcMountEntry) -> PathBuf {
    entry
        .staging_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_staging_base)
}

pub fn prepare_sources(
    ctx: &BackmaticContext,
    job_scope: &str,
    job: &FileBackupJob,
) -> Result<(Vec<SourcedPath>, StagingGuard)> {
    let mut sources = Vec::new();
    let mut guard = StagingGuard::new(
        Arc::clone(&ctx.mount_registry),
        Arc::clone(&ctx.commands),
        ctx.paths.fusermount.clone(),
    );

    if !job.srcmount.is_empty() {
        log::info!(
            "Start srcmount for job '{job_scope}' ({} remote source(s))",
            job.srcmount.len()
        );
    }

    for sm in &job.srcmount {
        let slug = origin_slug_remote(sm);
        let base = staging_base_for(sm);
        let mountpoint = srcmount_path(&base, job_scope, &slug);
        prepare_mountpoint_dir(ctx, &mountpoint)?;
        mount_remote(ctx, sm, &mountpoint)?;
        guard.track(mountpoint.clone());
        sources.push(SourcedPath {
            origin_slug: slug,
            local_path: mountpoint,
            protocol: Some("sshfs".to_string()),
            host: Some(sm.host.clone()),
            remote_path: Some(sm.path.clone()),
        });
    }

    for src in &job.src {
        sources.push(SourcedPath {
            origin_slug: crate::mount::origin::origin_slug_local(src),
            local_path: PathBuf::from(src),
            protocol: None,
            host: None,
            remote_path: None,
        });
    }

    if !guard.mounts.is_empty() {
        let paths: Vec<String> = guard
            .paths()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        log::info!(
            "Finished srcmount for job '{job_scope}' ({} mount(s): {})",
            paths.len(),
            paths.join(", ")
        );
    }

    Ok((sources, guard))
}

fn prepare_mountpoint_dir(ctx: &BackmaticContext, mountpoint: &Path) -> Result<()> {
    if mountpoint.exists() {
        unmount_sshfs(ctx.commands.as_ref(), &ctx.paths.fusermount, mountpoint);
    }
    if mountpoint.exists() {
        return Err(BackmaticError::Mount(format!(
            "mountpoint {} is not empty after unmount",
            mountpoint.display()
        )));
    }
    if let Some(parent) = mountpoint.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BackmaticError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::create_dir_all(mountpoint).map_err(|source| BackmaticError::Io {
        path: mountpoint.display().to_string(),
        source,
    })?;
    Ok(())
}

/// Unmount a sshfs mountpoint (via the configured `fusermount` binary, routed through the
/// injected command executor), then remove the now-empty mountpoint directory and any empty
/// parent scope directory it leaves behind.
pub(crate) fn unmount_sshfs(
    commands: &dyn CommandExecutor,
    fusermount: &Path,
    mountpoint: &Path,
) {
    log::debug!("Unmounting srcmount at {}", mountpoint.display());
    let req = CommandRequest::new(fusermount.to_string_lossy().to_string())
        .arg("-uz")
        .arg(mountpoint.to_string_lossy().to_string());
    match commands.run(&req) {
        Ok(result) if result.status.success() => {
            log::debug!("Unmounted {}", mountpoint.display());
        }
        Ok(result) => {
            log::warn!(
                "fusermount -uz {} failed (exit {:?})",
                mountpoint.display(),
                result.status.code()
            );
        }
        Err(e) => {
            log::warn!("fusermount -uz {} failed: {e}", mountpoint.display());
        }
    }
    remove_mountpoint_dirs(mountpoint);
}

/// Remove the leaf mountpoint directory and its now-empty parent scope directory so staging
/// dirs do not accumulate across runs. `remove_dir` only succeeds on empty dirs, so shared
/// parents that still hold other mounts are left intact.
fn remove_mountpoint_dirs(mountpoint: &Path) {
    if mountpoint.exists() {
        if let Err(e) = std::fs::remove_dir(mountpoint) {
            log::warn!("could not remove mountpoint {}: {e}", mountpoint.display());
            return;
        }
    }
    if let Some(parent) = mountpoint.parent() {
        // Best-effort: succeeds only once the last mount under this job scope is gone.
        let _ = std::fs::remove_dir(parent);
    }
}

/// Mount a remote `srcmount` path via sshfs so backup tools read changed files over the
/// network instead of copying the full tree into local staging storage. Authentication is
/// always key-based (identity file or agent/`~/.ssh/config`).
fn mount_remote(ctx: &BackmaticContext, entry: &SrcMountEntry, mountpoint: &Path) -> Result<()> {
    let remote = format!(
        "{}@{}:{}",
        entry.user,
        entry.host,
        entry.path.trim_end_matches('/')
    );
    log::info!(
        "Mounting remote srcmount (sshfs) {} → {}",
        remote,
        mountpoint.display()
    );
    if !crate::mount::dest::tool_available(ctx, &ctx.paths.sshfs) {
        return Err(BackmaticError::Command {
            command: "sshfs".into(),
            code: None,
            message: "sshfs not found (required for srcmount)".into(),
        });
    }

    // NOTE: we deliberately do NOT pass `default_permissions`. With that option the local
    // kernel enforces access checks using the remote file's numeric uid/gid, which blocks a
    // non-root backmatic process from reading files owned by other (e.g. root) remote users —
    // even though the SSH session itself is allowed to read them. `idmap=user` remaps the
    // remote login user to the local mounting user so ownership displays sensibly. Reads are
    // authorised by the remote SSH server, not the local kernel.
    let mut cmd = CommandRequest::new(ctx.paths.sshfs.to_string_lossy().to_string())
        .arg(remote.clone())
        .arg(mountpoint.to_string_lossy().to_string())
        .arg("-p")
        .arg(entry.port.to_string())
        .arg("-o")
        .arg("idmap=user,reconnect,ServerAliveInterval=15,ServerAliveCountMax=3");
    if let Some(key) = &entry.identity_file {
        cmd = cmd.arg("-o").arg(format!("IdentityFile={key}"));
    }
    // Force strictly non-interactive SSH so a hung host-key/passphrase prompt can never wedge
    // an unattended run (also folds in any user-supplied ssh_options).
    for opt in noninteractive_ssh_opts(&entry.ssh_options) {
        cmd = cmd.arg("-o").arg(opt);
    }
    let result = ctx.commands.run(&cmd)?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(BackmaticError::Command {
            command: "sshfs".into(),
            code: result.status.code(),
            message: format!(
                "sshfs mount {} → {} failed (exit {:?}): {} (srcmount is key-only and \
                 non-interactive: ensure the host key is known and a usable key/agent is available)",
                remote,
                mountpoint.display(),
                result.status.code(),
                stderr.trim()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::config::types::{
        AppConfig, FileBackupJob, FileConfig, JobId, RetentionConfig, SrcMountEntry,
    };
    use crate::error::{BackmaticError, Result};
    use crate::inject::commands::{CommandExecutor, CommandRequest, CommandResult, RecordingExecutor};
    use crate::inject::BackmaticContext;

    use super::*;

    fn test_ctx(executor: Arc<dyn CommandExecutor>) -> BackmaticContext {
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
        BackmaticContext::new(config).with_commands(executor)
    }

    fn srcmount_job(host: &str) -> FileBackupJob {
        FileBackupJob {
            comment: "remote".into(),
            logdir: None,
            src: vec![],
            dest: vec!["/backup".into()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![SrcMountEntry {
                host: host.into(),
                port: 22,
                user: "backup".into(),
                path: "/data".into(),
                identity_file: Some("/keys/id".into()),
                staging_dir: None,
                ssh_options: vec![],
            }],
            retention: RetentionConfig::default(),
            healthcheck: None,
        }
    }

    #[test]
    fn mount_paths_are_unique_per_job_scope() {
        let base = default_staging_base();
        let a = srcmount_path(&base, "borg-0", "sshfs_host1_data");
        let b = srcmount_path(&base, "borg-1", "sshfs_host1_data");
        assert_ne!(a, b);
    }

    #[test]
    fn prepare_sources_uses_job_scope_in_paths() {
        let executor = Arc::new(RecordingExecutor::new());
        let ctx = test_ctx(executor);
        let job = srcmount_job("host1");
        let (sources, _guard) = prepare_sources(&ctx, "borg-0", &job).unwrap();
        let path = sources[0].local_path.to_string_lossy();
        assert!(
            path.contains("borg-0"),
            "mount path should include job scope: {path}"
        );
    }

    #[test]
    fn different_jobs_with_same_srcmount_get_distinct_paths() {
        let executor = Arc::new(RecordingExecutor::new());
        let ctx = test_ctx(executor);
        let job = srcmount_job("host1");
        let (a, _) = prepare_sources(&ctx, "borg-0", &job).unwrap();
        let (b, _) = prepare_sources(&ctx, "borg-1", &job).unwrap();
        assert_ne!(a[0].local_path, b[0].local_path);
    }

    struct FailSecondSshfs {
        calls: AtomicUsize,
    }

    impl CommandExecutor for FailSecondSshfs {
        fn run(&self, request: &CommandRequest) -> Result<CommandResult> {
            use std::os::unix::process::ExitStatusExt;
            if request.program.contains("sshfs") {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n >= 1 {
                    return Err(BackmaticError::Command {
                        command: "sshfs".into(),
                        code: Some(1),
                        message: "injected second mount failure".into(),
                    });
                }
            }
            Ok(CommandResult {
                status: std::process::ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }

        fn spawn(&self, request: &CommandRequest) -> Result<std::process::Child> {
            Err(BackmaticError::Command {
                command: request.program.clone(),
                code: None,
                message: "spawn unsupported".into(),
            })
        }
    }

    #[test]
    fn partial_srcmount_failure_unmounts_prior_mounts() {
        let job = FileBackupJob {
            srcmount: vec![
                SrcMountEntry {
                    host: "host1".into(),
                    port: 22,
                    user: "backup".into(),
                    path: "/a".into(),
                    identity_file: None,
                    staging_dir: None,
                    ssh_options: vec![],
                },
                SrcMountEntry {
                    host: "host2".into(),
                    port: 22,
                    user: "backup".into(),
                    path: "/b".into(),
                    identity_file: None,
                    staging_dir: None,
                    ssh_options: vec![],
                },
            ],
            ..srcmount_job("unused")
        };
        let executor = Arc::new(FailSecondSshfs {
            calls: AtomicUsize::new(0),
        });
        let ctx = test_ctx(executor);
        let scope = JobId {
            backup_type: crate::config::types::BackupType::Borg,
            index: 0,
        }
        .scope_key();
        let first_mount = srcmount_path(&default_staging_base(), &scope, "sshfs_host1_a");
        match prepare_sources(&ctx, &scope, &job) {
            Err(e) => assert!(
                e.to_string().contains("injected second mount failure"),
                "unexpected error: {e}"
            ),
            Ok(_) => panic!("expected partial mount failure"),
        }
        assert!(
            !first_mount.exists(),
            "first mountpoint should be cleaned up after partial failure"
        );
    }

    #[test]
    fn remove_mountpoint_dirs_removes_leaf_and_empty_scope() {
        let base = tempfile::tempdir().unwrap();
        let scope = base.path().join("borg-0");
        let leaf = scope.join("sshfs_host_data");
        std::fs::create_dir_all(&leaf).unwrap();

        remove_mountpoint_dirs(&leaf);
        assert!(!leaf.exists(), "leaf mountpoint should be removed");
        assert!(!scope.exists(), "empty scope dir should be removed");
    }

    #[test]
    fn remove_mountpoint_dirs_keeps_shared_scope_with_other_mounts() {
        let base = tempfile::tempdir().unwrap();
        let scope = base.path().join("borg-0");
        let leaf = scope.join("mount_a");
        let sibling = scope.join("mount_b");
        std::fs::create_dir_all(&leaf).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        remove_mountpoint_dirs(&leaf);
        assert!(!leaf.exists());
        assert!(scope.exists(), "scope must remain while sibling mount exists");
        assert!(sibling.exists());
    }

    #[test]
    fn unmount_sshfs_routes_through_executor_and_removes_dir() {
        let dir = tempfile::tempdir().unwrap();
        let leaf = dir.path().join("scope").join("mnt");
        std::fs::create_dir_all(&leaf).unwrap();

        let rec = RecordingExecutor::new();
        unmount_sshfs(&rec, Path::new("/usr/bin/fusermount"), &leaf);

        let calls = rec.calls.lock().unwrap();
        let (program, args) = calls.last().expect("a command was run");
        assert!(program.contains("fusermount"), "should call fusermount: {program}");
        assert_eq!(args[0], "-uz");
        assert!(!leaf.exists(), "mountpoint dir should be removed after unmount");
    }

    #[test]
    fn noninteractive_opts_enforce_batchmode_and_default_hostkey_policy() {
        let opts = noninteractive_ssh_opts(&[]);
        assert!(opts.contains(&"BatchMode=yes".to_string()));
        assert!(opts.contains(&"ConnectTimeout=30".to_string()));
        assert!(opts.contains(&"StrictHostKeyChecking=accept-new".to_string()));
        // BatchMode must come first so ssh's first-value-wins can't be weakened by user opts.
        assert_eq!(opts.first().map(String::as_str), Some("BatchMode=yes"));
    }

    #[test]
    fn noninteractive_opts_respect_user_hostkey_policy() {
        let opts = noninteractive_ssh_opts(&["StrictHostKeyChecking=yes".to_string()]);
        assert!(opts.contains(&"BatchMode=yes".to_string()));
        assert!(opts.contains(&"StrictHostKeyChecking=yes".to_string()));
        assert!(
            !opts.contains(&"StrictHostKeyChecking=accept-new".to_string()),
            "user host-key policy must not be overridden by the default"
        );
    }

    #[test]
    fn sshfs_mount_is_noninteractive() {
        let rec = Arc::new(RecordingExecutor::new());
        let ctx = test_ctx(Arc::clone(&rec) as Arc<dyn CommandExecutor>);
        let staging = tempfile::tempdir().unwrap();
        let mut job = srcmount_job("host1");
        job.srcmount[0].staging_dir = Some(staging.path().to_string_lossy().to_string());

        let (_sources, guard) = prepare_sources(&ctx, "borg-0", &job).unwrap();
        drop(guard);

        let calls = rec.calls.lock().unwrap();
        let sshfs_call = calls
            .iter()
            .find(|(prog, _)| prog.contains("sshfs"))
            .expect("sshfs was invoked");
        let opts = sshfs_call.1.join(" ");
        assert!(opts.contains("BatchMode=yes"), "sshfs must be non-interactive: {opts}");
        assert!(opts.contains("ConnectTimeout=30"), "expected ConnectTimeout: {opts}");
        assert!(
            opts.contains("StrictHostKeyChecking=accept-new"),
            "expected host-key policy: {opts}"
        );
    }

    #[test]
    fn sshfs_mount_uses_idmap_and_not_default_permissions() {
        let rec = Arc::new(RecordingExecutor::new());
        let ctx = test_ctx(Arc::clone(&rec) as Arc<dyn CommandExecutor>);
        let staging = tempfile::tempdir().unwrap();
        let mut job = srcmount_job("host1");
        job.srcmount[0].staging_dir = Some(staging.path().to_string_lossy().to_string());

        let (_sources, guard) = prepare_sources(&ctx, "borg-0", &job).unwrap();
        drop(guard); // trigger unmount before assertions

        let calls = rec.calls.lock().unwrap();
        let sshfs_call = calls
            .iter()
            .find(|(prog, _)| prog.contains("sshfs"))
            .expect("sshfs was invoked");
        let opts = sshfs_call.1.join(" ");
        assert!(opts.contains("idmap=user"), "expected idmap=user: {opts}");
        assert!(
            !opts.contains("default_permissions"),
            "default_permissions must not be set: {opts}"
        );
    }
}
