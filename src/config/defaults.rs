use std::path::PathBuf;

pub const DEFAULT_RETRY_INTERVAL_SEC: u64 = 3600;
pub const DEFAULT_RETRY_COUNT: u32 = 23;
pub const DEFAULT_CONTINUOUS_HOURS: u64 = 0;
pub const CONFIG_FILENAME: &str = "backmatic.yml";

/// True when the process is running with effective root privileges.
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid is always safe and never fails.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Default log directory: a system path when root, otherwise a per-user XDG state/cache path.
/// Avoids the world-writable `/tmp` (symlink-attack surface when running as root).
pub fn default_logdir() -> String {
    if is_root() {
        return "/var/log/backmatic".to_string();
    }
    if let Some(dir) = dirs::state_dir() {
        return dir
            .join("backmatic")
            .join("logs")
            .to_string_lossy()
            .to_string();
    }
    if let Some(dir) = dirs::cache_dir() {
        return dir
            .join("backmatic")
            .join("logs")
            .to_string_lossy()
            .to_string();
    }
    "/var/tmp/backmatic/logs".to_string()
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("backmatic")
        .join(CONFIG_FILENAME)
}

pub fn default_lock_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("backmatic.lock")
}

pub fn default_thread_count() -> usize {
    std::cmp::max(1, num_cpus::get() / 2)
}

/// Default scratch directory for backup tools that stage large temp files (e.g. restic's
/// `restic-temp-pack-*`, borg temp files). Uses disk-backed `/var/tmp` rather than the
/// frequently-tmpfs-backed (and small) `/tmp`, which is a common "no space left on device"
/// cause during large restic/borg backups.
pub fn default_tmp_dir() -> PathBuf {
    PathBuf::from("/var/tmp/backmatic-tmp")
}

/// Resolve the scratch/`TMPDIR` directory: explicit config, then `BACKMATIC_TMPDIR`, then the
/// disk-backed default.
pub fn resolve_tmp_dir(configured: Option<&str>) -> PathBuf {
    if let Some(dir) = configured {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("BACKMATIC_TMPDIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    default_tmp_dir()
}

/// Base directory for per-origin sshfs mount points (`{base}/{origin_slug}/`).
pub fn default_staging_base() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(|| PathBuf::from("/var/tmp"))
        .join("backmatic-staging")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_tmp_dir_takes_precedence() {
        assert_eq!(
            resolve_tmp_dir(Some("/data/scratch")),
            PathBuf::from("/data/scratch")
        );
    }

    #[test]
    fn default_tmp_dir_is_disk_backed() {
        // Not tmpfs `/tmp`; uses `/var/tmp` which is disk-backed on typical systems.
        assert_eq!(default_tmp_dir(), PathBuf::from("/var/tmp/backmatic-tmp"));
        assert!(!default_tmp_dir().starts_with("/tmp"));
    }
}
