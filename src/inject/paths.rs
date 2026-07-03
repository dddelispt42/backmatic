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
            fusermount: resolve_tool(
                t.fusermount.as_deref(),
                "BACKMATIC_FUSERMOUNT_PATH",
                "fusermount",
            ),
        }
    }
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
