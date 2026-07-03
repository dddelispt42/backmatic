//! Shared helpers for integration tests (T1–T24).
//!
//! Each integration binary uses a different subset of these helpers, so unused-code lints
//! are expected here for the ones a given binary does not touch.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use backmatic::config::types::{
    AppConfig, BackupType, FileBackupJob, FileConfig, JobId, JobSpec, RetentionConfig,
};
use backmatic::config::validate::validate_file_config;
use backmatic::inject::{BackmaticContext, Clock, FakeClock};
use backmatic::runners::execute_job;

/// Return false when an external tool is missing (caller should return early).
pub fn skip_without_tool(name: &str) -> bool {
    if tool_available(name) {
        true
    } else {
        eprintln!("skipping: {name} not installed");
        false
    }
}

pub fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct TestTree {
    pub root: TempDir,
}

impl TestTree {
    pub fn basic() -> Self {
        let root = TempDir::new().expect("tempdir");
        write_file(&root, "file1.txt", b"version 1");
        write_file(&root, "dir/file2.bin", &[0u8, 1, 2, 3]);
        write_file(&root, "skip/skip.txt", b"excluded");
        write_file(&root, "nested/deep.log", b"log content");
        write_file(&root, "nested/file.tmp", b"tmp content");
        Self { root }
    }

    pub fn path(&self) -> PathBuf {
        self.root.path().to_path_buf()
    }

    pub fn write_relative(&self, rel: &str, content: &[u8]) {
        write_file(&self.root, rel, content);
    }

    pub fn remove_relative(&self, rel: &str) {
        let p = self.root.path().join(rel);
        if p.is_dir() {
            fs::remove_dir_all(&p).expect("remove dir");
        } else if p.exists() {
            fs::remove_file(&p).expect("remove file");
        }
    }
}

fn write_file(base: &TempDir, rel: &str, content: &[u8]) {
    let path = base.path().join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(path, content).expect("write");
}

pub fn checksums(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    walk_checksums(root, root, &mut out);
    out
}

fn walk_checksums(base: &Path, dir: &Path, out: &mut HashMap<String, String>) {
    let entries = fs::read_dir(dir).expect("read_dir");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_checksums(base, &path, out);
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .expect("strip")
                .to_string_lossy()
                .replace('\\', "/");
            let data = fs::read(&path).expect("read");
            let hash = Sha256::digest(&data);
            out.insert(rel, format!("{:x}", hash));
        }
    }
}

pub fn find_file_root(dir: &Path, rel: &str) -> PathBuf {
    fn walk(base: &Path, rel: &str) -> Option<PathBuf> {
        if base.join(rel).exists() {
            return Some(base.to_path_buf());
        }
        let entries = fs::read_dir(base).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, rel) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(dir, rel).unwrap_or_else(|| dir.to_path_buf())
}

pub fn assert_trees_equal(expected: &Path, actual: &Path) {
    let exp = checksums(expected);
    let act = checksums(actual);
    assert_eq!(exp, act, "tree mismatch between {} and {}", expected.display(), actual.display());
}

pub fn assert_file_missing(root: &Path, rel: &str) {
    assert!(
        !root.join(rel).exists(),
        "expected {} to be absent under {}",
        rel,
        root.display()
    );
}

pub fn assert_file_present(root: &Path, rel: &str) {
    assert!(
        root.join(rel).exists(),
        "expected {} under {}",
        rel,
        root.display()
    );
}

pub struct Harness {
    pub work: TempDir,
    pub logdir: PathBuf,
    pub ctx: BackmaticContext,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_clock(None)
    }

    pub fn with_clock(clock: Option<Arc<dyn Clock>>) -> Self {
        let work = TempDir::new().expect("work tempdir");
        let logdir = work.path().join("logs");
        fs::create_dir_all(&logdir).expect("logdir");
        let mut ctx = test_context(&logdir);
        if let Some(c) = clock {
            ctx = ctx.with_clock(c);
        }
        Self { work, logdir, ctx }
    }

    pub fn dest_dir(&self, name: &str) -> PathBuf {
        let p = self.work.path().join(name);
        fs::create_dir_all(&p).expect("dest");
        p
    }

    pub fn run_job(&self, job_type: BackupType, job: FileBackupJob) {
        let spec = JobSpec::File {
            id: JobId {
                backup_type: job_type,
                index: 0,
            },
            job_type,
            job,
        };
        execute_job(&self.ctx, &spec).expect("job should succeed");
    }
}

pub fn test_context(logdir: &Path) -> BackmaticContext {
    let file = FileConfig {
        defaults: backmatic::config::types::DefaultsConfig {
            logdir: logdir.to_string_lossy().to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    validate_file_config(&file).expect("empty file config");
    let config = AppConfig {
        lock_file: logdir.join("lock"),
        config_path: logdir.join("config.yml"),
        threadpool_size: 2,
        retry_interval_sec: 0,
        retry_count: 1,
        continuous_hours: 0,
        dry_run: false,
        file,
    };
    BackmaticContext::new(config)
}

pub fn file_job(
    comment: &str,
    src: Vec<String>,
    dest: Vec<String>,
    exclude: Vec<String>,
    retention: RetentionConfig,
) -> FileBackupJob {
    FileBackupJob {
        comment: comment.into(),
        logdir: None,
        src,
        dest,
        exclude,
        password: None,
        destmount: vec![],
        srcmount: vec![],
        retention,
        healthcheck: None,
    }
}

pub fn fake_clock_at(y: i32, m: u32, d: u32, h: u32, min: u32) -> Arc<dyn Clock> {
    use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
    let naive = NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_time(NaiveTime::from_hms_opt(h, min, 0).unwrap());
    let dt = Local.from_local_datetime(&naive).unwrap();
    Arc::new(FakeClock::new(dt))
}

/// List rsync retention snapshot directories for `dest`.
pub fn rsync_snapshots(dest: &Path) -> Vec<PathBuf> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("/"));
    let prefix = format!("{}.hourly", dest.to_string_lossy().trim_end_matches('/'));
    let mut snaps: Vec<PathBuf> = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_dir() && p.to_string_lossy().starts_with(&prefix) {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    snaps.sort();
    snaps
}

pub fn borg_list_archives(repo: &Path, password: Option<&str>) -> Vec<String> {
    let mut cmd = Command::new("borg");
    cmd.arg("list").arg("--short").arg(repo);
    cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes");
    if let Some(pw) = password {
        cmd.env("BORG_PASSPHRASE", pw);
    }
    let out = cmd.output().expect("borg list");
    assert!(out.status.success(), "borg list failed");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn borg_extract_latest(repo: &Path, password: Option<&str>, target: &Path) {
    let archives = borg_list_archives(repo, password);
    let latest = archives.last().expect("archive");
    let spec = format!("{}::{}", repo.display(), latest);
    fs::create_dir_all(target).expect("extract target");
    let mut cmd = Command::new("borg");
    cmd.arg("extract").arg(spec).current_dir(target);
    cmd.env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes");
    if let Some(pw) = password {
        cmd.env("BORG_PASSPHRASE", pw);
    }
    let out = cmd.output().expect("borg extract");
    assert!(out.status.success(), "borg extract: {}", String::from_utf8_lossy(&out.stderr));
}

pub fn restic_snapshot_count(repo: &Path, password: &str) -> usize {
    let out = Command::new("restic")
        .arg("-r")
        .arg(repo)
        .env("RESTIC_PASSWORD", password)
        .arg("snapshots")
        .arg("--json")
        .output()
        .expect("restic snapshots");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    v.as_array().map(|a| a.len()).unwrap_or(0)
}

pub fn restic_restore_latest(repo: &Path, password: &str, target: &Path) {
    fs::create_dir_all(target).expect("restore target");
    let out = Command::new("restic")
        .arg("-r")
        .arg(repo)
        .env("RESTIC_PASSWORD", password)
        .arg("restore")
        .arg("latest")
        .arg("--target")
        .arg(target)
        .output()
        .expect("restic restore");
    assert!(out.status.success(), "restic restore: {}", String::from_utf8_lossy(&out.stderr));
}
