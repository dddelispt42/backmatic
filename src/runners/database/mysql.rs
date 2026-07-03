use crate::config::{filenamify, generate_log_path, logdir_for_job, types::DatabaseJob};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{BackmaticContext, CommandRequest};

pub fn run(ctx: &BackmaticContext, job: &DatabaseJob, dests: &[String]) -> Result<()> {
    let logdir = logdir_for_job(&ctx.config.file, job.logdir.as_deref());
    let _logfile = generate_log_path(&logdir, "database", &job.comment)?;

    if !crate::mount::dest::tool_available(ctx, &ctx.paths.mysqldump) {
        return Err(BackmaticError::Command {
            command: "mysqldump".into(),
            code: None,
            message: "mysqldump not found".into(),
        });
    }

    let host = job.host.as_deref().unwrap_or("localhost");
    let user = job.user.as_deref().unwrap_or("root");
    let password = job.password.as_deref().unwrap_or("");

    let databases: Vec<&str> = if job.src.is_empty() {
        vec!["--all-databases"]
    } else {
        job.src.iter().map(String::as_str).collect()
    };

    log::info!(
        "Start database mysql ('{}'): {} database(s) -> {:?}",
        job.comment,
        databases.len(),
        dests
    );

    for dest in dests {
        std::fs::create_dir_all(dest).ok();
        for db in &databases {
            let db_label = if *db == "--all-databases" {
                "all"
            } else {
                db
            };
            let outfile = format!(
                "{}/{}_{}.sql.gz",
                dest.trim_end_matches('/'),
                filenamify(&job.comment),
                db_label
            );
            let shell = format!(
                "{} -u {} -p{} -h {} {} -E 2>/dev/null | {} > {}",
                ctx.paths.mysqldump.display(),
                user,
                password,
                host,
                db,
                ctx.paths.gzip.display(),
                outfile
            );
            let req = CommandRequest::new("bash").arg("-c").arg(shell);
            let result = ctx.commands.run(&req)?;
            if !result.status.success() {
                return Err(BackmaticError::JobFailed {
                    job_type: "database".into(),
                    comment: job.comment.clone(),
                    message: format!(
                        "mysqldump of '{db_label}' on {host} --> '{dest}' failed (exit {:?}): {}",
                        result.status.code(),
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                });
            }
        }
    }

    log::debug!("Finished database mysql ('{}')", job.comment);

    if let Some(hc) = &job.healthcheck {
        healthcheck::ping_success(ctx, hc)?;
    }
    Ok(())
}
