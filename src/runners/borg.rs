use std::path::Path;

use crate::config::{filenamify, generate_log_path, logdir_for_job};
use crate::config::types::{FileBackupJob, JobId, VerifyConfig};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{log_command_output, BackmaticContext, CommandRequest};
use crate::mount::{enumerate_sources, mount_source, resolve_destinations, SourcedPath};

pub fn run(ctx: &BackmaticContext, job_id: &JobId, job: &FileBackupJob) -> Result<()> {
    let scope = job_id.scope_key();
    let logdir = logdir_for_job(&ctx.config.file, job.logdir.as_deref());
    let logfile = generate_log_path(&logdir, "borg", &job.comment)?;

    if !crate::mount::dest::tool_available(ctx, &ctx.paths.borg) {
        return Err(BackmaticError::Command {
            command: "borg".into(),
            code: None,
            message: "borg not found".into(),
        });
    }

    let tmp_dir = crate::runners::ensure_tmp_dir(ctx)?;
    let stall = crate::runners::stall_timeout(ctx);

    let specs = enumerate_sources(&scope, job);
    let mut failures = Vec::new();
    let (dests, _mount_session) =
        resolve_destinations(ctx, &scope, &job.comment, &job.dest, &job.destmount)?;

    // Ensure every repository exists up front so an init failure surfaces before mounting sources.
    for dest in &dests {
        ensure_repo(ctx, &logfile, dest, &job.password, &tmp_dir)?;
    }

    // Lazy per-source flow: mount just before backup, unmount immediately after (see restic).
    for spec in &specs {
        let (source, guard) = match mount_source(ctx, spec) {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("srcmount {}: {e}", spec.describe());
                log::warn!("borg ({}) source failed to mount, skipping: {msg}", job.comment);
                failures.push(msg);
                continue;
            }
        };

        for dest in &dests {
            log::info!(
                "Start borg ({}): {} --> {}",
                job.comment,
                source.local_path.display(),
                dest
            );
            let archive = format!(
                "{}::{}-{} {{user}}@{{hostname}}_{{now:%Y-%m-%d_%H-%M-%S}}",
                dest,
                filenamify(&job.comment),
                source.origin_slug
            );
            let mut req = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
                .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
                .env("TMPDIR", tmp_dir.clone())
                .stall_timeout(stall)
                .arg("create")
                .arg("--exclude-caches")
                .arg(archive)
                .arg(source.local_path.to_string_lossy().to_string());
            if let Some(pw) = &job.password {
                req = req.env("BORG_PASSPHRASE", pw.clone());
            }
            for ex in &job.exclude {
                req = req.arg(format!("--exclude={ex}"));
            }
            let created = match run_source(ctx, &logfile, &req) {
                Ok(()) => {
                    log::info!(
                        "Finished borg ({}): {} --> {}",
                        job.comment,
                        source.local_path.display(),
                        dest
                    );
                    true
                }
                Err(msg) => {
                    let full = format!("'{}' --> '{}': {msg}", source.local_path.display(), dest);
                    log::warn!("borg ({}) source failed, skipping: {full}", job.comment);
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
                    log::error!("borg ({}) {full}", job.comment);
                    failures.push(full);
                }
            }
        }
        // Unmount this source immediately before moving on to the next one.
        drop(guard);
    }

    // Retention runs once per repository after all sources have been written and unmounted.
    for dest in &dests {
        prune_repo(ctx, &logfile, dest, job, &tmp_dir)?;
    }

    if !failures.is_empty() {
        return Err(BackmaticError::JobFailed {
            job_type: "borg".into(),
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

/// Run a single borg `create` and classify the outcome. Returns `Err(message)` for a per-source
/// failure the caller should record and skip (rather than aborting the whole job).
fn run_source(ctx: &BackmaticContext, logfile: &str, req: &CommandRequest) -> std::result::Result<(), String> {
    match ctx.commands.run(req) {
        Ok(result) => {
            let _ = log_command_output(logfile, &result);
            if result.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "exit {:?}: {}",
                    result.status.code(),
                    String::from_utf8_lossy(&result.stderr).trim()
                ))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Restore-verify a freshly written source against the repository: hash a random sample of the
/// (still mounted) source files, `borg extract --stdout` the same paths from the archive we just
/// created for this origin, and compare. Returns `Err` on the first mismatch or restore failure.
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
            "borg verify: no eligible files to sample under '{}'; skipping",
            source.local_path.display()
        );
        return Ok(());
    }
    // The archive we just created is the most recent one in the repo (per-source loop creates then
    // verifies immediately), so `--last 1` identifies it.
    let archive = latest_archive(ctx, dest, password, tmp_dir)?;
    log::info!(
        "borg verify: restoring {} sampled file(s) for '{}' from {dest}::{archive}",
        samples.len(),
        source.local_path.display()
    );
    for sample in &samples {
        let stored = borg_stored_path(&source.local_path, &sample.rel_path);
        let mut req = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
            .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
            .env("TMPDIR", tmp_dir)
            .arg("extract")
            .arg("--stdout")
            .arg(format!("{dest}::{archive}"))
            .arg(stored.clone());
        if let Some(pw) = password {
            req = req.env("BORG_PASSPHRASE", pw.clone());
        }
        let result = ctx.commands.run(&req).map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(format!(
                "borg extract {stored} failed (exit {:?}): {}",
                result.status.code(),
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
        crate::verify::compare(sample, &result.stdout)?;
    }
    log::info!(
        "borg verify OK: {} file(s) match for '{}'",
        samples.len(),
        source.local_path.display()
    );
    Ok(())
}

/// Name of the most recently created archive in `dest`.
fn latest_archive(
    ctx: &BackmaticContext,
    dest: &str,
    password: &Option<String>,
    tmp_dir: &str,
) -> std::result::Result<String, String> {
    let mut req = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
        .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
        .env("TMPDIR", tmp_dir)
        .arg("list")
        .arg("--short")
        .arg("--last")
        .arg("1")
        .arg(dest);
    if let Some(pw) = password {
        req = req.env("BORG_PASSPHRASE", pw.clone());
    }
    let result = ctx.commands.run(&req).map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "borg list failed (exit {:?}): {}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&result.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "borg list returned no archive".to_string())
}

/// Path as borg stores it: absolute source paths are recorded with the leading `/` stripped.
fn borg_stored_path(root: &Path, rel: &Path) -> String {
    root.join(rel)
        .to_string_lossy()
        .trim_start_matches('/')
        .to_string()
}

fn ensure_repo(
    ctx: &BackmaticContext,
    logfile: &str,
    dest: &str,
    password: &Option<String>,
    tmp_dir: &str,
) -> Result<()> {
    let mut list = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
        .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
        .env("TMPDIR", tmp_dir)
        .arg("list")
        .arg(dest);
    if let Some(pw) = password {
        list = list.env("BORG_PASSPHRASE", pw.clone());
    }
    if ctx.commands.run(&list)?.status.success() {
        return Ok(());
    }
    let mut init = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
        .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
        .env("TMPDIR", tmp_dir)
        .arg("init")
        .arg("-e")
        .arg(if password.is_some() {
            "repokey"
        } else {
            "none"
        })
        .arg(dest);
    if let Some(pw) = password {
        init = init.env("BORG_PASSPHRASE", pw.clone());
    }
    let result = ctx.commands.run(&init)?;
    log_command_output(logfile, &result)?;
    if !result.status.success() {
        return Err(BackmaticError::Command {
            command: "borg init".into(),
            code: result.status.code(),
            message: String::from_utf8_lossy(&result.stderr).to_string(),
        });
    }
    Ok(())
}

fn prune_repo(
    ctx: &BackmaticContext,
    logfile: &str,
    dest: &str,
    job: &FileBackupJob,
    tmp_dir: &str,
) -> Result<()> {
    if !retention_enabled(&job.retention) {
        return Ok(());
    }
    let mut req = CommandRequest::new(ctx.paths.borg.to_string_lossy().to_string())
        .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
        .env("TMPDIR", tmp_dir)
        .arg("prune")
        .arg("-s")
        .arg("--list")
        .arg("-H")
        .arg(job.retention.keep_hourly.to_string())
        .arg("-d")
        .arg(job.retention.keep_daily.to_string())
        .arg("-w")
        .arg(job.retention.keep_weekly.to_string())
        .arg("-m")
        .arg(job.retention.keep_monthly.to_string())
        .arg("-y")
        .arg(job.retention.keep_yearly.to_string())
        .arg(dest);
    if let Some(pw) = &job.password {
        req = req.env("BORG_PASSPHRASE", pw.clone());
    }
    let result = ctx.commands.run(&req)?;
    log_command_output(logfile, &result)?;
    if !result.status.success() {
        return Err(BackmaticError::JobFailed {
            job_type: "borg".into(),
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
