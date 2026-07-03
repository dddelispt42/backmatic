use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::config::defaults::default_staging_base;
use crate::config::types::{DatabaseJob, FileBackupJob, FileConfig};
use crate::error::{BackmaticError, Result};
use crate::mount::origin::{origin_slug_local, origin_slug_remote};

pub fn validate_file_config(file: &FileConfig) -> Result<()> {
    for (label, jobs) in [
        ("rsync", file.rsync.iter().collect::<Vec<_>>()),
        ("borg", file.borg.iter().collect()),
        ("restic", file.restic.iter().collect()),
    ] {
        for (idx, job) in jobs.into_iter().enumerate() {
            validate_file_job(label, idx, job)?;
        }
    }
    for (idx, job) in file.database.iter().enumerate() {
        validate_database_job(idx, job)?;
    }
    Ok(())
}

fn validate_file_job(kind: &str, index: usize, job: &FileBackupJob) -> Result<()> {
    if job.src.is_empty() && job.srcmount.is_empty() {
        return Err(BackmaticError::Validation(format!(
            "{kind}[{index}]: at least one of 'src' or 'srcmount' is required"
        )));
    }
    if job.dest.is_empty() && job.destmount.is_empty() {
        return Err(BackmaticError::Validation(format!(
            "{kind}[{index}]: at least one of 'dest' or 'destmount' is required"
        )));
    }

    let mut slugs = HashSet::new();
    for sm in &job.srcmount {
        let slug = origin_slug_remote(sm);
        if !slugs.insert(slug.clone()) {
            return Err(BackmaticError::Validation(format!(
                "{kind}[{index}]: duplicate origin_slug '{slug}'"
            )));
        }
    }
    for src in &job.src {
        let slug = origin_slug_local(src);
        if !slugs.insert(slug.clone()) {
            return Err(BackmaticError::Validation(format!(
                "{kind}[{index}]: duplicate origin_slug '{slug}' for src '{src}'"
            )));
        }
    }

    let effective_dests = effective_dest_paths(kind, index, job);
    let effective_srcs = effective_src_paths(kind, index, job);

    for dest in &effective_dests {
        for src in &effective_srcs {
            if path_contains(src, dest) {
                return Err(BackmaticError::Validation(format!(
                    "{kind}[{index}]: destination '{}' must not be inside source '{}'",
                    dest.display(),
                    src.display()
                )));
            }
        }
    }

    Ok(())
}

fn validate_database_job(index: usize, job: &DatabaseJob) -> Result<()> {
    if job.dest.is_empty() && job.destmount.is_empty() {
        return Err(BackmaticError::Validation(format!(
            "database[{index}]: at least one of 'dest' or 'destmount' is required"
        )));
    }
    Ok(())
}

fn effective_dest_paths(kind: &str, index: usize, job: &FileBackupJob) -> Vec<PathBuf> {
    let scope = format!("{kind}-{index}");
    let mut paths: Vec<PathBuf> = job.dest.iter().map(PathBuf::from).collect();
    for dm in &job.destmount {
        let mountpoint = dm
            .mountpoint
            .clone()
            .unwrap_or_else(|| format!("/mnt/backapp/{scope}/{}", dm.uuid));
        let sub = if dm.path == "." {
            PathBuf::new()
        } else {
            PathBuf::from(&dm.path)
        };
        paths.push(Path::new(&mountpoint).join(sub));
    }
    paths
}

fn effective_src_paths(kind: &str, index: usize, job: &FileBackupJob) -> Vec<PathBuf> {
    let scope = format!("{kind}-{index}");
    let mut paths: Vec<PathBuf> = job.src.iter().map(PathBuf::from).collect();
    for sm in &job.srcmount {
        let base = sm
            .staging_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_staging_base);
        paths.push(crate::mount::src_remote::srcmount_path(
            &base,
            &scope,
            &origin_slug_remote(sm),
        ));
    }
    paths
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    let parent = normalize_path(parent);
    let child = normalize_path(child);
    parent == child || child.starts_with(&parent)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{DestMountEntry, RetentionConfig, SrcMountEntry};

    fn minimal_job() -> FileBackupJob {
        FileBackupJob {
            comment: "test".into(),
            logdir: None,
            src: vec!["/data".into()],
            dest: vec!["/backup".into()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![],
            retention: RetentionConfig::default(),
            healthcheck: None,
        }
    }

    #[test]
    fn rejects_dest_inside_src() {
        let job = FileBackupJob {
            dest: vec!["/data/backup".into()],
            ..minimal_job()
        };
        let err = validate_file_job("rsync", 0, &job).unwrap_err();
        assert!(err.to_string().contains("must not be inside"));
    }

    #[test]
    fn allows_destmount_only() {
        let job = FileBackupJob {
            dest: vec![],
            destmount: vec![DestMountEntry {
                uuid: "abc".into(),
                mountpoint: Some("/mnt/usb".into()),
                password: None,
                path: ".".into(),
            }],
            ..minimal_job()
        };
        validate_file_job("borg", 0, &job).unwrap();
    }

    #[test]
    fn rejects_duplicate_origin_slug() {
        let job = FileBackupJob {
            srcmount: vec![
                SrcMountEntry {
                    host: "host1".into(),
                    port: 22,
                    user: "u".into(),
                    path: "/".into(),
                    identity_file: None,
                    staging_dir: None,
                    ssh_options: vec![],
                },
                SrcMountEntry {
                    host: "host1".into(),
                    port: 22,
                    user: "u".into(),
                    path: "/".into(),
                    identity_file: None,
                    staging_dir: None,
                    ssh_options: vec![],
                },
            ],
            ..minimal_job()
        };
        let err = validate_file_job("restic", 0, &job).unwrap_err();
        assert!(err.to_string().contains("duplicate origin_slug"));
    }
}
