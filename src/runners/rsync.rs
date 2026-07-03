use regex::Regex;

use crate::config::types::{FileBackupJob, JobId, SrcMountEntry};
use crate::config::{generate_log_path, logdir_for_job};
use crate::error::{BackmaticError, Result};
use crate::healthcheck;
use crate::inject::{log_command_output, BackmaticContext, CommandRequest};
use crate::mount::origin::{origin_slug_local, origin_slug_remote};
use crate::mount::resolve_destinations;
use crate::retention::rsync_retention;

/// A single rsync source: a local path, or a remote host pulled directly over SSH.
///
/// Unlike borg/restic (which can only read local paths and therefore need an sshfs mount),
/// rsync speaks SSH natively, so remote `srcmount` entries are fetched with `rsync -e ssh
/// user@host:path` — no FUSE staging, no local mirror.
///
/// When a job has **multiple** sources, each one syncs into its own slug-named subdirectory of
/// the destination (`{dest}/{slug}/`), mirroring how borg/restic keep origins separate via
/// archive names/tags. This is essential because each source runs as an independent
/// `rsync --delete` pass: sharing one destination root would let a later source delete an
/// earlier source's files. A single-source job keeps the classic flat layout (mirror straight
/// into `dest`) for backward compatibility.
struct RsyncSource {
    /// rsync source argument, always with a trailing slash so *contents* are copied into the
    /// slug subdirectory (`/local/path/` or `user@host:/remote/path/`).
    spec: String,
    /// Unique per-origin directory name used under the destination root.
    slug: String,
    /// `-e` remote-shell value for remote sources; `None` for local paths.
    remote_shell: Option<String>,
}

fn rsync_sources(job: &FileBackupJob) -> Vec<RsyncSource> {
    let mut sources = Vec::new();
    for src in &job.src {
        sources.push(RsyncSource {
            spec: format!("{}/", src.trim_end_matches('/')),
            slug: origin_slug_local(src),
            remote_shell: None,
        });
    }
    for sm in &job.srcmount {
        sources.push(RsyncSource {
            spec: format!("{}@{}:{}/", sm.user, sm.host, sm.path.trim_end_matches('/')),
            slug: origin_slug_remote(sm),
            remote_shell: Some(remote_shell(sm)),
        });
    }
    sources
}

fn remote_shell(sm: &SrcMountEntry) -> String {
    let mut ssh = format!("ssh -p {}", sm.port);
    if let Some(identity) = &sm.identity_file {
        ssh.push_str(&format!(" -i {identity}"));
    }
    // Strictly non-interactive: never block on a passphrase or host-key prompt (backmatic runs
    // unattended). Only connection setup is bounded; live transfers may run arbitrarily long.
    for opt in crate::mount::src_remote::noninteractive_ssh_opts(&sm.ssh_options) {
        ssh.push_str(&format!(" -o {opt}"));
    }
    ssh
}

/// A destination string that rsync treats as remote (`[user@]host:path` or an `rsync://` URL).
/// Local subdirectories are created ahead of time; remote ones are left to rsync/the remote base.
fn is_remote_dest(dest: &str) -> bool {
    if dest.starts_with("rsync://") {
        return true;
    }
    let re = Regex::new(r"^(([a-zA-Z][a-zA-Z_-]*)@)?([a-zA-Z][.a-zA-Z0-9_-]*):").unwrap();
    re.is_match(dest)
}

fn source_target(dest: &str, slug: &str) -> String {
    format!("{}/{}", dest.trim_end_matches('/'), slug)
}

pub fn run(ctx: &BackmaticContext, job_id: &JobId, job: &FileBackupJob) -> Result<()> {
    let scope = job_id.scope_key();
    let logdir = logdir_for_job(&ctx.config.file, job.logdir.as_deref());
    let logfile = generate_log_path(&logdir, "rsync", &job.comment)?;

    if !crate::mount::dest::tool_available(ctx, &ctx.paths.rsync) {
        return Err(BackmaticError::Command {
            command: "rsync".into(),
            code: None,
            message: "rsync not found".into(),
        });
    }

    let sources = rsync_sources(job);
    // With multiple sources, isolate each into its own slug subdir so per-source `--delete`
    // passes can't clobber a sibling source. A single source keeps the flat mirror layout.
    let use_subdirs = sources.len() > 1;
    let (dests, _mount_session) =
        resolve_destinations(ctx, &scope, &job.comment, &job.dest, &job.destmount)?;

    // A source that can't be synced (e.g. its remote host is down) is skipped and recorded; the
    // remaining sources still complete, and the job is reported as failed at the end.
    let mut failures: Vec<String> = Vec::new();

    for dest in &dests {
        for source in &sources {
            let target = if use_subdirs {
                source_target(dest, &source.slug)
            } else {
                dest.clone()
            };
            // Pre-create per-origin subdirectories for local destinations so rsync has a base.
            if use_subdirs && !is_remote_dest(dest) {
                if let Err(e) = std::fs::create_dir_all(&target) {
                    let full = format!("'{}' --> '{target}': {e}", source.spec);
                    log::warn!("rsync ({}) source failed, skipping: {full}", job.comment);
                    failures.push(full);
                    continue;
                }
            }

            log::info!(
                "Start rsync ({}): {} --> {}",
                job.comment,
                source.spec,
                target
            );
            let mut req = CommandRequest::new(ctx.paths.rsync.to_string_lossy().to_string())
                .arg("-avHAXhE")
                .arg("--delete")
                .arg("--delete-excluded")
                .arg(format!("--log-file={logfile}"));
            if let Some(shell) = &source.remote_shell {
                req = req.arg("-e").arg(shell.clone());
            }
            for ex in &job.exclude {
                req = req.arg(format!("--exclude={ex}"));
            }
            req = req.arg(source.spec.clone()).arg(target.clone());

            match ctx.commands.run(&req) {
                Ok(result) => {
                    let _ = log_command_output(&logfile, &result);
                    let code = result.status.code();
                    if !result.status.success() && code != Some(23) && code != Some(24) {
                        let full = format!(
                            "'{}' --> '{target}': exit {code:?}; see {logfile}",
                            source.spec
                        );
                        log::warn!("rsync ({}) source failed, skipping: {full}", job.comment);
                        failures.push(full);
                        continue;
                    }
                    // 23/24 are partial-transfer warnings (e.g. a file vanished mid-copy).
                    if code == Some(23) || code == Some(24) {
                        log::warn!(
                            "rsync ({}) partial transfer for source '{}' --> '{}' (exit {code:?}); see {logfile}",
                            job.comment,
                            source.spec,
                            target
                        );
                    }
                    log::info!(
                        "Finished rsync ({}): {} --> {}",
                        job.comment,
                        source.spec,
                        target
                    );
                }
                Err(e) => {
                    let full = format!("'{}' --> '{target}': {e}", source.spec);
                    log::warn!("rsync ({}) source failed, skipping: {full}", job.comment);
                    failures.push(full);
                }
            }
        }
        rsync_retention(ctx, dest, &job.retention)?;
    }

    if !failures.is_empty() {
        return Err(BackmaticError::JobFailed {
            job_type: "rsync".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{
        AppConfig, BackupType, FileConfig, RetentionConfig,
    };
    use crate::inject::commands::RecordingExecutor;
    use crate::inject::{BackmaticContext, CommandExecutor};
    use std::sync::Arc;

    fn ctx_with(rec: Arc<RecordingExecutor>, logdir: &std::path::Path) -> BackmaticContext {
        let file = FileConfig {
            defaults: crate::config::types::DefaultsConfig {
                logdir: logdir.to_string_lossy().to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let config = AppConfig {
            lock_file: "/tmp/lock".into(),
            config_path: "/tmp/cfg".into(),
            threadpool_size: 1,
            retry_interval_sec: 0,
            retry_count: 1,
            continuous_hours: 0,
            dry_run: false,
            file,
        };
        BackmaticContext::new(config).with_commands(rec as Arc<dyn CommandExecutor>)
    }

    fn remote_job(dest: &str) -> FileBackupJob {
        FileBackupJob {
            comment: "remote".into(),
            logdir: None,
            src: vec![],
            dest: vec![dest.into()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![SrcMountEntry {
                host: "100.113.81.74".into(),
                port: 2222,
                user: "backup".into(),
                path: "/home/HRiemer".into(),
                identity_file: Some("/keys/id".into()),
                staging_dir: None,
                ssh_options: vec!["StrictHostKeyChecking=accept-new".into()],
            }],
            retention: RetentionConfig::default(),
            healthcheck: None,
        }
    }

    #[test]
    fn remote_source_builds_ssh_spec_without_sshfs() {
        let sources = rsync_sources(&remote_job("/backup"));
        assert_eq!(sources.len(), 1);
        // trailing slash so the remote directory's *contents* land in the slug subdir
        assert_eq!(sources[0].spec, "backup@100.113.81.74:/home/HRiemer/");
        assert_eq!(sources[0].slug, "sshfs_100.113.81.74_home_HRiemer");
        let shell = sources[0].remote_shell.as_deref().unwrap();
        assert!(shell.contains("ssh -p 2222"));
        assert!(shell.contains("-i /keys/id"));
        assert!(shell.contains("-o StrictHostKeyChecking=accept-new"));
    }

    #[test]
    fn remote_shell_is_noninteractive() {
        let job = remote_job("/backup");
        let shell = remote_shell(&job.srcmount[0]);
        // must never block on a prompt while running unattended
        assert!(shell.contains("-o BatchMode=yes"), "shell: {shell}");
        assert!(shell.contains("-o ConnectTimeout=30"), "shell: {shell}");
        assert!(
            shell.contains("-o StrictHostKeyChecking=accept-new"),
            "shell: {shell}"
        );
    }

    #[test]
    fn rsync_remote_job_does_not_mount_sshfs() {
        let logdir = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let rec = Arc::new(RecordingExecutor::new());
        let ctx = ctx_with(Arc::clone(&rec), logdir.path());

        let id = JobId { backup_type: BackupType::Rsync, index: 0 };
        run(&ctx, &id, &remote_job(&dest.path().to_string_lossy())).expect("run ok");

        let calls = rec.calls.lock().unwrap();
        assert!(
            !calls.iter().any(|(prog, _)| prog.contains("sshfs")),
            "rsync must not invoke sshfs"
        );
        assert!(
            !calls.iter().any(|(prog, _)| prog.contains("fusermount")),
            "rsync must not invoke fusermount"
        );
        let rsync_call = calls
            .iter()
            .find(|(prog, args)| prog.contains("rsync") && args.iter().any(|a| a == "-e"))
            .expect("an rsync command with -e was issued");
        assert!(
            rsync_call
                .1
                .iter()
                .any(|a| a == "backup@100.113.81.74:/home/HRiemer/"),
            "remote source spec should be passed to rsync: {:?}",
            rsync_call.1
        );
        // single source keeps the flat layout: target is the bare dest root
        assert!(
            rsync_call.1.last() == Some(&dest.path().to_string_lossy().to_string()),
            "single-source rsync target should be the dest root: {:?}",
            rsync_call.1
        );
    }

    #[test]
    fn local_source_has_no_remote_shell() {
        let mut job = remote_job("/backup");
        job.srcmount.clear();
        job.src = vec!["/data/docs".into()];
        let sources = rsync_sources(&job);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].spec, "/data/docs/");
        assert_eq!(sources[0].slug, "local_data_docs");
        assert!(sources[0].remote_shell.is_none());
    }

    #[test]
    fn multiple_sources_use_distinct_slug_subdirs() {
        let logdir = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let dest_str = dest.path().to_string_lossy().to_string();
        let rec = Arc::new(RecordingExecutor::new());
        let ctx = ctx_with(Arc::clone(&rec), logdir.path());

        let mut job = remote_job(&dest_str);
        // two remote origins with the *same* basename would collide in a shared root
        job.srcmount = vec![
            SrcMountEntry {
                host: "hostA".into(),
                port: 22,
                user: "backup".into(),
                path: "/srv/etc".into(),
                identity_file: None,
                staging_dir: None,
                ssh_options: vec![],
            },
            SrcMountEntry {
                host: "hostB".into(),
                port: 22,
                user: "backup".into(),
                path: "/srv/etc".into(),
                identity_file: None,
                staging_dir: None,
                ssh_options: vec![],
            },
        ];

        let id = JobId { backup_type: BackupType::Rsync, index: 0 };
        run(&ctx, &id, &job).expect("run ok");

        let calls = rec.calls.lock().unwrap();
        let targets: Vec<String> = calls
            .iter()
            .filter(|(prog, _)| prog.contains("rsync"))
            .filter_map(|(_, args)| args.last().cloned())
            .collect();

        let target_a = source_target(&dest_str, "sshfs_hostA_srv_etc");
        let target_b = source_target(&dest_str, "sshfs_hostB_srv_etc");
        assert!(targets.contains(&target_a), "missing hostA target: {targets:?}");
        assert!(targets.contains(&target_b), "missing hostB target: {targets:?}");
        assert_ne!(
            target_a, target_b,
            "same-basename origins must not share a destination dir"
        );
        // the isolating subdirectories must actually be created on disk
        assert!(std::path::Path::new(&target_a).is_dir());
        assert!(std::path::Path::new(&target_b).is_dir());
    }

    #[test]
    fn remote_dest_detection() {
        assert!(is_remote_dest("backup@host:/data"));
        assert!(is_remote_dest("host:/data"));
        assert!(is_remote_dest("rsync://host/module"));
        assert!(!is_remote_dest("/local/path"));
        assert!(!is_remote_dest("./relative"));
    }
}
