use crate::config::types::{BackupType, JobSpec};
use crate::error::{BackmaticError, Result};
use crate::inject::BackmaticContext;

pub mod borg;
pub mod database;
pub mod restic;
pub mod rsync;

/// Resolve and create the scratch directory used as `TMPDIR` for backup tools, so tools like
/// restic (which buffer `restic-temp-pack-*` files) don't exhaust a small `/tmp`. Returns the
/// path as a string suitable for a `TMPDIR` env value.
pub(crate) fn ensure_tmp_dir(ctx: &BackmaticContext) -> Result<String> {
    let dir = crate::config::defaults::resolve_tmp_dir(ctx.config.file.defaults.tmp_dir.as_deref());
    std::fs::create_dir_all(&dir).map_err(|source| BackmaticError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    Ok(dir.to_string_lossy().to_string())
}

/// Resolve the stall-watchdog window for backup transfer commands from job defaults (see
/// [`crate::config::defaults::resolve_stall_timeout`]). `None` disables the watchdog.
pub(crate) fn stall_timeout(ctx: &BackmaticContext) -> Option<std::time::Duration> {
    crate::config::defaults::resolve_stall_timeout(ctx.config.file.defaults.stall_timeout)
}

pub fn execute_job(ctx: &BackmaticContext, spec: &JobSpec) -> Result<()> {
    if ctx.config.dry_run {
        log::info!(
            "[dry-run] would run {} job '{}'",
            spec.id().backup_type.as_str(),
            spec.comment()
        );
        return Ok(());
    }
    match spec {
        JobSpec::File { id, job_type, job, .. } => match job_type {
            BackupType::Rsync => rsync::run(ctx, id, job),
            BackupType::Borg => borg::run(ctx, id, job),
            BackupType::Restic => restic::run(ctx, id, job),
            BackupType::Database => unreachable!(),
        },
        JobSpec::Database { id, job, .. } => database::run(ctx, id, job),
    }
}
