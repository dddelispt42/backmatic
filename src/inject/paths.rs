use std::path::PathBuf;

use crate::config::types::AppConfig;

#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub rsync: PathBuf,
    pub borg: PathBuf,
    pub restic: PathBuf,
    pub mysqldump: PathBuf,
    pub pg_dump: PathBuf,
    pub gzip: PathBuf,
    pub mount: PathBuf,
    pub umount: PathBuf,
    pub cryptsetup: PathBuf,
    pub cp: PathBuf,
    pub rm: PathBuf,
    pub sshfs: PathBuf,
    pub fusermount: PathBuf,
}

impl ToolPaths {
    pub fn from_config(config: &AppConfig) -> Self {
        let t = &config.file.defaults.tools;
        Self {
            rsync: resolve_tool(t.rsync.as_deref(), "BACKMATIC_RSYNC_PATH", "/usr/bin/rsync"),
            borg: resolve_tool(t.borg.as_deref(), "BACKMATIC_BORG_PATH", "/usr/bin/borg"),
            restic: resolve_tool(
                t.restic.as_deref(),
                "BACKMATIC_RESTIC_PATH",
                "/usr/bin/restic",
            ),
            mysqldump: resolve_tool(
                t.mysqldump.as_deref(),
                "BACKMATIC_MYSQLDUMP_PATH",
                "/usr/bin/mysqldump",
            ),
            pg_dump: resolve_tool(
                t.pg_dump.as_deref(),
                "BACKMATIC_PG_DUMP_PATH",
                "/usr/bin/pg_dump",
            ),
            gzip: resolve_tool(t.gzip.as_deref(), "BACKMATIC_GZIP_PATH", "gzip"),
            mount: resolve_tool(t.mount.as_deref(), "BACKMATIC_MOUNT_PATH", "/usr/bin/mount"),
            umount: resolve_tool(t.umount.as_deref(), "BACKMATIC_UMOUNT_PATH", "/usr/bin/umount"),
            cryptsetup: resolve_tool(
                t.cryptsetup.as_deref(),
                "BACKMATIC_CRYPTSETUP_PATH",
                "/usr/bin/cryptsetup",
            ),
            cp: resolve_tool(t.cp.as_deref(), "BACKMATIC_CP_PATH", "/usr/bin/cp"),
            rm: resolve_tool(t.rm.as_deref(), "BACKMATIC_RM_PATH", "/usr/bin/rm"),
            sshfs: resolve_tool(t.sshfs.as_deref(), "BACKMATIC_SSHFS_PATH", "sshfs"),
            fusermount: resolve_fusermount(t.fusermount.as_deref()),
        }
    }
}

/// Resolve the FUSE unmount helper. Explicit config / `BACKMATIC_FUSERMOUNT_PATH` win; otherwise
/// probe both the libfuse3 (`fusermount3`) and libfuse2 (`fusermount`) names on `PATH` and in the
/// usual install locations. A minimal service/cron `PATH` often lacks the directory holding it, so
/// falling back to well-known absolute paths avoids a spurious "No such file or directory" at
/// unmount time.
fn resolve_fusermount(config: Option<&str>) -> PathBuf {
    if let Some(path) = config {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("BACKMATIC_FUSERMOUNT_PATH") {
        return PathBuf::from(path);
    }
    const NAMES: [&str; 2] = ["fusermount3", "fusermount"];
    const DIRS: [&str; 6] = [
        "/usr/bin",
        "/bin",
        "/usr/local/bin",
        "/sbin",
        "/usr/sbin",
        "/run/wrappers/bin",
    ];
    for name in NAMES {
        if let Some(found) = which(name) {
            return found;
        }
    }
    for dir in DIRS {
        for name in NAMES {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("fusermount")
}

fn resolve_tool(config: Option<&str>, env_var: &str, default: &str) -> PathBuf {
    if let Some(path) = config {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var(env_var) {
        return PathBuf::from(path);
    }
    if default.contains('/') {
        return PathBuf::from(default);
    }
    which(default).unwrap_or_else(|| PathBuf::from(default))
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.is_file() {
                Some(full)
            } else {
                None
            }
        })
    })
}
