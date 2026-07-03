use crate::config::{filenamify, generate_log_path, logdir_for_job, types::DatabaseJob};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{BackmaticContext, CommandRequest};

pub fn run(ctx: &BackmaticContext, job: &DatabaseJob, dests: &[String]) -> Result<()> {
    let logdir = logdir_for_job(&ctx.config.file, job.logdir.as_deref());
    let _logfile = generate_log_path(&logdir, "database", &job.comment)?;

    if !crate::mount::dest::tool_available(ctx, &ctx.paths.pg_dump) {
        return Err(BackmaticError::Command {
            command: "pg_dump".into(),
            code: None,
            message: "pg_dump not found".into(),
        });
    }

    let host = job.host.as_deref().unwrap_or("localhost");
    let port = job.port.unwrap_or(5432).to_string();
    let user = job.user.as_deref().unwrap_or("postgres");

    let databases: Vec<Option<&str>> = if job.src.is_empty() {
        vec![None]
    } else {
        job.src.iter().map(|s| Some(s.as_str())).collect()
    };

    log::info!(
        "Start database postgres ('{}'): {} database(s) -> {:?}",
        job.comment,
        databases.len(),
        dests
    );

    for dest in dests {
        std::fs::create_dir_all(dest).ok();
        for db in &databases {
            let db_label = db.unwrap_or("all");
            let outfile = format!(
                "{}/{}_{}.dump",
                dest.trim_end_matches('/'),
                filenamify(&job.comment),
                db_label
            );
            let mut req = CommandRequest::new(ctx.paths.pg_dump.to_string_lossy().to_string())
                .arg("-h")
                .arg(host)
                .arg("-p")
                .arg(port.clone())
                .arg("-U")
                .arg(user)
                .arg("-Fc")
                .arg("-f")
                .arg(outfile.clone());
            if let Some(name) = db {
                req = req.arg("-d").arg(*name);
            }
            if let Some(pw) = &job.password {
                req = req.env("PGPASSWORD", pw.clone());
            }
            let result = ctx.commands.run(&req)?;
            if !result.status.success() {
                return Err(BackmaticError::JobFailed {
                    job_type: "database".into(),
                    comment: job.comment.clone(),
                    message: format!(
                        "pg_dump of '{db_label}' on {host}:{port} --> '{dest}' failed (exit {:?}): {}",
                        result.status.code(),
                        String::from_utf8_lossy(&result.stderr).trim()
                    ),
                });
            }
        }
    }

    log::debug!("Finished database postgres ('{}')", job.comment);

    if let Some(hc) = &job.healthcheck {
        healthcheck::ping_success(ctx, hc)?;
    }
    Ok(())
}
