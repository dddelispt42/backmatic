use crate::config::{filenamify, generate_log_path, logdir_for_job};
use crate::config::types::{FileBackupJob, JobId};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{log_command_output, BackmaticContext, CommandRequest};
use crate::mount::{prepare_sources, resolve_destinations};

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

    let (sources, _staging) = prepare_sources(ctx, &scope, job)?;
    let (dests, _mount_session) =
        resolve_destinations(ctx, &scope, &job.comment, &job.dest, &job.destmount)?;

    for dest in &dests {
        for source in &sources {
            ensure_repo(ctx, &logfile, dest, &job.password, &tmp_dir)?;
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
            let result = ctx.commands.run(&req)?;
            log_command_output(&logfile, &result)?;
            if !result.status.success() {
                return Err(BackmaticError::JobFailed {
                    job_type: "borg".into(),
                    comment: job.comment.clone(),
                    message: format!(
                        "source '{}' --> '{}' failed (exit {:?}): {}",
                        source.local_path.display(),
                        dest,
                        result.status.code(),
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                });
            }
            log::info!(
                "Finished borg ({}): {} --> {}",
                job.comment,
                source.local_path.display(),
                dest
            );
        }
        prune_repo(ctx, &logfile, dest, job, &tmp_dir)?;
    }

    if let Some(hc) = &job.healthcheck {
        healthcheck::ping_success(ctx, hc)?;
    }
    Ok(())
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
