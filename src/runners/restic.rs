use crate::config::{generate_log_path, logdir_for_job};
use crate::config::types::{FileBackupJob, JobId, VerifyConfig};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{log_command_output, BackmaticContext, CommandRequest};
use crate::mount::{enumerate_sources, mount_source, resolve_destinations, SourcedPath};

pub fn run(ctx: &BackmaticContext, job_id: &JobId, job: &FileBackupJob) -> Result<()> {
    let scope = job_id.scope_key();
    let logdir = logdir_for_job(&ctx.config.file, job.logdir.as_deref());
    let logfile = generate_log_path(&logdir, "restic", &job.comment)?;

    if !crate::mount::dest::tool_available(ctx, &ctx.paths.restic) {
        return Err(BackmaticError::Command {
            command: "restic".into(),
            code: None,
            message: "restic not found".into(),
        });
    }

    let tmp_dir = crate::runners::ensure_tmp_dir(ctx)?;
    let stall = crate::runners::stall_timeout(ctx);

    let specs = enumerate_sources(&scope, job);
    let mut failures = Vec::new();
    let (dests, _mount_session) =
        resolve_destinations(ctx, &scope, &job.comment, &job.dest, &job.destmount)?;

    // Ensure every repository exists before we start mounting sources, so a repo-init failure is
    // surfaced up front rather than after bringing mounts up.
    for dest in &dests {
        ensure_repo(ctx, &logfile, dest, &job.password, &tmp_dir)?;
    }

    // Lazy per-source flow: mount a source just before its backup and unmount immediately after,
    // so a mount only exists while a backup is actively using it (minimizing hang exposure and
    // avoiding pointless mounts of hosts that are down).
    for spec in &specs {
        let (source, guard) = match mount_source(ctx, spec) {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("srcmount {}: {e}", spec.describe());
                log::warn!("restic ({}) source failed to mount, skipping: {msg}", job.comment);
                failures.push(msg);
                continue;
            }
        };

        for dest in &dests {
            log::info!(
                "Start restic ({}): {} --> {}",
                job.comment,
                source.local_path.display(),
                dest
            );
            let mut req = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
                .env("TMPDIR", tmp_dir.clone())
                .stall_timeout(stall)
                .arg("-r")
                .arg(dest.clone())
                .arg("backup")
                .arg(source.local_path.to_string_lossy().to_string())
                .arg("--tag")
                .arg(format!("origin:{}", source.origin_slug));
            if let Some(pw) = &job.password {
                req = req.env("RESTIC_PASSWORD", pw.clone());
            }
            for ex in &job.exclude {
                req = req.arg(format!("--exclude={ex}"));
            }
            let created = match run_source(ctx, &logfile, &req) {
                SourceOutcome::Ok => {
                    log::info!(
                        "Finished restic ({}): {} --> {}",
                        job.comment,
                        source.local_path.display(),
                        dest
                    );
                    true
                }
                SourceOutcome::Partial => {
                    // Exit 3 = some files could not be read; the snapshot was still created.
                    log::warn!(
                        "restic ({}) could not read all files for source '{}' --> '{}' (exit 3); see {logfile}",
                        job.comment,
                        source.local_path.display(),
                        dest
                    );
                    true
                }
                SourceOutcome::Failed(msg) => {
                    let full = format!("'{}' --> '{}': {msg}", source.local_path.display(), dest);
                    log::warn!("restic ({}) source failed, skipping: {full}", job.comment);
                    failures.push(full);
                    false
                }
            };

            // Verify while the source is still mounted (restore compares against the live source).
            if created && job.verify.enabled {
                if let Err(msg) =
                    verify_source(ctx, dest, &job.password, &source, &tmp_dir, &job.verify)
                {
                    let full = format!("verify '{}' --> '{}': {msg}", source.local_path.display(), dest);
                    log::error!("restic ({}) {full}", job.comment);
                    failures.push(full);
                }
            }
        }
        // Unmount this source immediately; the next source is mounted on its own iteration.
        drop(guard);
    }

    // Retention runs once per repository after all sources have been written and unmounted.
    for dest in &dests {
        forget_and_prune(ctx, &logfile, dest, job, &tmp_dir)?;
    }

    if !failures.is_empty() {
        return Err(BackmaticError::JobFailed {
            job_type: "restic".into(),
            comment: job.comment.clone(),
            message: format!(
                "{} of source(s) failed (the rest were backed up): {}",
                failures.len(),
                failures.join("; ")
            ),
        });
    }

    if let Some(hc) = &job.healthcheck {
        healthcheck::ping_success(ctx, hc)?;
    }
    Ok(())
}

enum SourceOutcome {
    Ok,
    /// restic exit 3: snapshot created, but some files were unreadable (not a source failure).
    Partial,
    /// A per-source failure to record and skip; the rest of the job still proceeds.
    Failed(String),
}

fn run_source(ctx: &BackmaticContext, logfile: &str, req: &CommandRequest) -> SourceOutcome {
    match ctx.commands.run(req) {
        Ok(result) => {
            let _ = log_command_output(logfile, &result);
            let code = result.status.code();
            if result.status.success() {
                SourceOutcome::Ok
            } else if code == Some(3) {
                SourceOutcome::Partial
            } else {
                SourceOutcome::Failed(format!(
                    "exit {code:?}: {}",
                    String::from_utf8_lossy(&result.stderr).trim()
                ))
            }
        }
        Err(e) => SourceOutcome::Failed(e.to_string()),
    }
}

/// Restore-verify a freshly written source against the repository: hash a random sample of the
/// (still mounted) source files, `restic dump` the same paths from the latest snapshot for this
/// origin, and compare. Returns `Err` describing the first mismatch or restore failure.
fn verify_source(
    ctx: &BackmaticContext,
    dest: &str,
    password: &Option<String>,
    source: &SourcedPath,
    tmp_dir: &str,
    cfg: &VerifyConfig,
) -> std::result::Result<(), String> {
    let samples = crate::verify::sample_files(&source.local_path, cfg.samples, cfg.max_file_size);
    if samples.is_empty() {
        log::info!(
            "restic verify: no eligible files to sample under '{}'; skipping",
            source.local_path.display()
        );
        return Ok(());
    }
    log::info!(
        "restic verify: restoring {} sampled file(s) for '{}' from {dest}",
        samples.len(),
        source.local_path.display()
    );
    for sample in &samples {
        let abs = source.local_path.join(&sample.rel_path);
        let mut req = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
            .env("TMPDIR", tmp_dir)
            .arg("-r")
            .arg(dest)
            .arg("dump")
            .arg("--tag")
            .arg(format!("origin:{}", source.origin_slug))
            .arg("latest")
            .arg(abs.to_string_lossy().to_string());
        if let Some(pw) = password {
            req = req.env("RESTIC_PASSWORD", pw.clone());
        }
        let result = ctx.commands.run(&req).map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(format!(
                "restic dump {} failed (exit {:?}): {}",
                abs.display(),
                result.status.code(),
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
        crate::verify::compare(sample, &result.stdout)?;
    }
    log::info!(
        "restic verify OK: {} file(s) match for '{}'",
        samples.len(),
        source.local_path.display()
    );
    Ok(())
}

fn ensure_repo(
    ctx: &BackmaticContext,
    logfile: &str,
    dest: &str,
    password: &Option<String>,
    tmp_dir: &str,
) -> Result<()> {
    let mut snap = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
        .env("TMPDIR", tmp_dir)
        .arg("-r")
        .arg(dest)
        .arg("snapshots");
    if let Some(pw) = password {
        snap = snap.env("RESTIC_PASSWORD", pw.clone());
    }
    if ctx.commands.run(&snap)?.status.success() {
        return Ok(());
    }
    let mut init = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
        .env("TMPDIR", tmp_dir)
        .arg("init")
        .arg("--repo")
        .arg(dest);
    if let Some(pw) = password {
        init = init.env("RESTIC_PASSWORD", pw.clone());
    }
    let result = ctx.commands.run(&init)?;
    log_command_output(logfile, &result)?;
    if !result.status.success() {
        return Err(BackmaticError::Command {
            command: "restic init".into(),
            code: result.status.code(),
            message: String::from_utf8_lossy(&result.stderr).to_string(),
        });
    }
    Ok(())
}

fn forget_and_prune(
    ctx: &BackmaticContext,
    logfile: &str,
    dest: &str,
    job: &FileBackupJob,
    tmp_dir: &str,
) -> Result<()> {
    if !retention_enabled(&job.retention) {
        return Ok(());
    }
    let mut forget = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
        .env("TMPDIR", tmp_dir)
        .arg("-r")
        .arg(dest)
        .arg("forget")
        .arg("--keep-hourly")
        .arg(job.retention.keep_hourly.to_string())
        .arg("--keep-daily")
        .arg(job.retention.keep_daily.to_string())
        .arg("--keep-weekly")
        .arg(job.retention.keep_weekly.to_string())
        .arg("--keep-monthly")
        .arg(job.retention.keep_monthly.to_string())
        .arg("--keep-yearly")
        .arg(job.retention.keep_yearly.to_string());
    if let Some(pw) = &job.password {
        forget = forget.env("RESTIC_PASSWORD", pw.clone());
    }
    let fresult = ctx.commands.run(&forget)?;
    log_command_output(logfile, &fresult)?;
    if !fresult.status.success() {
        return Err(BackmaticError::JobFailed {
            job_type: "restic".into(),
            comment: job.comment.clone(),
            message: "forget failed".into(),
        });
    }

    let mut prune = CommandRequest::new(ctx.paths.restic.to_string_lossy().to_string())
        .env("TMPDIR", tmp_dir)
        .arg("-r")
        .arg(dest)
        .arg("prune");
    if let Some(pw) = &job.password {
        prune = prune.env("RESTIC_PASSWORD", pw.clone());
    }
    let presult = ctx.commands.run(&prune)?;
    log_command_output(logfile, &presult)?;
    if !presult.status.success() {
        return Err(BackmaticError::JobFailed {
            job_type: "restic".into(),
            comment: job.comment.clone(),
            message: "prune failed".into(),
        });
    }
    Ok(())
}

fn retention_enabled(retention: &crate::config::types::RetentionConfig) -> bool {
    retention.keep_hourly > 0
        || retention.keep_daily > 0
        || retention.keep_weekly > 0
        || retention.keep_monthly > 0
        || retention.keep_yearly > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        AppConfig, BackupType, DefaultsConfig, FileConfig, RetentionConfig,
    };
    use crate::inject::commands::RecordingExecutor;
    use crate::inject::{BackmaticContext, CommandExecutor};
    use std::sync::Arc;

    #[test]
    fn restic_backup_sets_tmpdir_env() {
        let work = tempfile::tempdir().unwrap();
        let tmp = work.path().join("scratch");
        let logs = work.path().join("logs");
        let src = work.path().join("src");
        let dest = work.path().join("repo");
        std::fs::create_dir_all(&src).unwrap();

        let file = FileConfig {
            defaults: DefaultsConfig {
                logdir: logs.to_string_lossy().to_string(),
                tmp_dir: Some(tmp.to_string_lossy().to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let config = AppConfig {
            lock_file: work.path().join("lock"),
            config_path: work.path().join("cfg"),
            threadpool_size: 1,
            retry_interval_sec: 0,
            retry_count: 1,
            continuous_hours: 0,
            dry_run: false,
            file,
        };
        let rec = Arc::new(RecordingExecutor::new());
        let ctx =
            BackmaticContext::new(config).with_commands(Arc::clone(&rec) as Arc<dyn CommandExecutor>);

        let job = FileBackupJob {
            comment: "t".into(),
            logdir: None,
            src: vec![src.to_string_lossy().to_string()],
            dest: vec![dest.to_string_lossy().to_string()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![],
            retention: RetentionConfig::default(),
            healthcheck: None,
            verify: Default::default(),
        };
        let id = JobId { backup_type: BackupType::Restic, index: 0 };
        run(&ctx, &id, &job).expect("restic run (recording executor) should succeed");

        assert_eq!(
            rec.find_env("restic", "TMPDIR").as_deref(),
            Some(tmp.to_string_lossy().as_ref()),
            "restic commands must run with TMPDIR pointed at the configured scratch dir"
        );
        assert!(tmp.exists(), "scratch dir should be created");
    }
}
