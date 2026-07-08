use std::fs;
use std::path::Path;

use log::LevelFilter;
use regex::Regex;

use crate::cli::CliArgs;
use crate::config::defaults::{default_config_path, default_lock_path, default_thread_count};
use crate::config::types::{AppConfig, DatabaseJob, FileBackupJob, FileConfig};
use crate::config::validate::validate_file_config;
use crate::config::schema::validate_yaml_against_schema;
use crate::error::{BackmaticError, Result};

static ENV_PATTERN: &str = r"\$\{([A-Z0-9_]+)(?::-([^}]*))?\}";

pub fn load_app_config(cli: &CliArgs) -> Result<AppConfig> {
    init_logging(cli.verbose)?;

    if cli.retries < 1 {
        return Err(BackmaticError::Config(
            "retries must be at least 1 (a job must run at least once)".into(),
        ));
    }

    let config_path = cli
        .configfile
        .clone()
        .unwrap_or_else(default_config_path);

    let contents = fs::read_to_string(&config_path).map_err(|source| BackmaticError::Io {
        path: config_path.display().to_string(),
        source,
    })?;

    let resolved = resolve_env_vars(&contents)?;
    validate_yaml_against_schema(&resolved)?;
    let mut file: FileConfig = serde_yaml::from_str(&resolved).map_err(|e| {
        BackmaticError::Config(format!("failed to parse {}: {e}", config_path.display()))
    })?;

    resolve_job_secrets(&mut file);
    validate_file_config(&file)?;
    ensure_destmount_privileges(&file)?;

    let threadpool_size = cli.threads.unwrap_or_else(default_thread_count);

    Ok(AppConfig {
        lock_file: default_lock_path(),
        config_path,
        threadpool_size,
        retry_interval_sec: cli.retryinterval,
        retry_count: cli.retries,
        continuous_hours: cli.continuous,
        dry_run: cli.dry_run,
        file,
    })
}

fn resolve_env_vars(input: &str) -> Result<String> {
    let re = Regex::new(ENV_PATTERN).map_err(|e| BackmaticError::Config(e.to_string()))?;
    let mut out = input.to_string();
    for cap in re.captures_iter(input) {
        let full = cap.get(0).unwrap().as_str();
        let var = cap.get(1).unwrap().as_str();
        let default = cap.get(2).map(|m| m.as_str());
        let value = std::env::var(var)
            .ok()
            .or_else(|| default.map(str::to_string))
            .ok_or_else(|| {
                BackmaticError::Config(format!("environment variable {var} is not set"))
            })?;
        out = out.replace(full, &value);
    }
    Ok(out)
}

fn resolve_job_secrets(file: &mut FileConfig) {
    for job in file
        .rsync
        .iter_mut()
        .chain(file.borg.iter_mut())
        .chain(file.restic.iter_mut())
    {
        resolve_file_job(job);
    }
    for job in file.database.iter_mut() {
        resolve_db_job(job);
    }
}

fn resolve_file_job(job: &mut FileBackupJob) {
    if let Some(pw) = &job.password {
        job.password = Some(pw.clone());
    }
    for dm in &mut job.destmount {
        if let Some(pw) = &dm.password {
            dm.password = Some(pw.clone());
        }
    }
}

/// `destmount` mounts block devices / opens LUKS containers, which requires root. Fail fast
/// with a clear message rather than deep inside a runner as a permission error.
fn ensure_destmount_privileges(file: &FileConfig) -> Result<()> {
    if crate::config::defaults::is_root() {
        return Ok(());
    }
    let has_destmount = file
        .rsync
        .iter()
        .chain(file.borg.iter())
        .chain(file.restic.iter())
        .any(|j| !j.destmount.is_empty())
        || file.database.iter().any(|j| !j.destmount.is_empty());
    if has_destmount {
        return Err(BackmaticError::RootRequired {
            reason: "'destmount' mounts block devices and opens LUKS containers".into(),
        });
    }
    Ok(())
}

fn resolve_db_job(job: &mut DatabaseJob) {
    if let Some(pw) = &job.password {
        job.password = Some(pw.clone());
    }
    for dm in &mut job.destmount {
        if let Some(pw) = &dm.password {
            dm.password = Some(pw.clone());
        }
    }
}

pub fn logdir_for_job(file: &FileConfig, job_logdir: Option<&str>) -> String {
    job_logdir
        .map(str::to_string)
        .unwrap_or_else(|| file.defaults.logdir.clone())
}

pub fn filenamify(input: &str) -> String {
    let re = Regex::new("[!<> :'\"/\\\\|?*+]").expect("valid regex");
    re.replace_all(input, "_").to_string()
}

pub fn generate_log_path(log_dir: &str, buptype: &str, comment: &str) -> Result<String> {
    let dir = Path::new(log_dir);
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|source| BackmaticError::Io {
            path: log_dir.to_string(),
            source,
        })?;
    }
    let name = format!(
        "backup-{}_{}_{}.log",
        buptype,
        filenamify(comment),
        chrono::Local::now().format("%Y%m%d%H%M")
    );
    Ok(format!("{}/{}", log_dir.trim_end_matches('/'), name))
}

fn init_logging(verbose: u8) -> Result<()> {
    let level = match verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        _ => LevelFilter::Debug,
    };
    let config = simplelog::ConfigBuilder::new()
        .set_time_offset_to_local()
        .map_err(|_| BackmaticError::Config("no locale for logger".into()))?
        .set_thread_mode(simplelog::ThreadLogMode::Both)
        .set_time_format_custom(time::macros::format_description!(
            "[year]:[month]:[day] [hour]:[minute]:[second]"
        ))
        .build();
    simplelog::TermLogger::init(
        level,
        config,
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )
    .map_err(|e| BackmaticError::Other(anyhow::anyhow!("logger init: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_resolution() {
        std::env::set_var("BACKMATIC_TEST_VAR", "secret");
        let out = resolve_env_vars("password: ${BACKMATIC_TEST_VAR}").unwrap();
        assert!(out.contains("secret"));
    }

    #[test]
    fn filenamify_replaces_special_chars() {
        assert_eq!(filenamify("a/b:c"), "a_b_c");
    }

    #[test]
    fn destmount_requires_root() {
        use crate::config::types::{DestMountEntry, FileBackupJob, FileConfig, RetentionConfig};

        let job = FileBackupJob {
            comment: "d".into(),
            logdir: None,
            src: vec!["/data".into()],
            dest: vec![],
            exclude: vec![],
            password: None,
            destmount: vec![DestMountEntry {
                uuid: "abc".into(),
                mountpoint: Some("/mnt/x".into()),
                password: None,
                path: ".".into(),
            }],
            srcmount: vec![],
            retention: RetentionConfig::default(),
            healthcheck: None,
            verify: Default::default(),
        };
        let file = FileConfig {
            rsync: vec![job],
            ..Default::default()
        };

        let result = ensure_destmount_privileges(&file);
        if crate::config::defaults::is_root() {
            assert!(result.is_ok(), "root may use destmount");
        } else {
            let err = result.unwrap_err();
            assert!(matches!(err, BackmaticError::RootRequired { .. }));
        }
    }

    #[test]
    fn no_destmount_never_requires_root() {
        use crate::config::types::{FileBackupJob, FileConfig, RetentionConfig};
        let job = FileBackupJob {
            comment: "d".into(),
            logdir: None,
            src: vec!["/data".into()],
            dest: vec!["/backup".into()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![],
            retention: RetentionConfig::default(),
            healthcheck: None,
            verify: Default::default(),
        };
        let file = FileConfig {
            rsync: vec![job],
            ..Default::default()
        };
        assert!(ensure_destmount_privileges(&file).is_ok());
    }
}
