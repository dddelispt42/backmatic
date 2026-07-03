use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize a YAML field that may be a string or a list of strings (nested lists flattened).
pub fn deserialize_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrList {
        One(String),
        Many(Vec<serde_yaml::Value>),
    }

    fn flatten(value: serde_yaml::Value, out: &mut Vec<String>) {
        match value {
            serde_yaml::Value::String(s) => out.push(s),
            serde_yaml::Value::Sequence(seq) => {
                for item in seq {
                    flatten(item, out);
                }
            }
            serde_yaml::Value::Null => {}
            other => out.push(format!("{other:?}")),
        }
    }

    let raw = Option::<StringOrList>::deserialize(deserializer)?;
    let mut result = Vec::new();
    match raw {
        None => {}
        Some(StringOrList::One(s)) => result.push(s),
        Some(StringOrList::Many(values)) => {
            for v in values {
                flatten(v, &mut result);
            }
        }
    }
    Ok(result)
}

/// Deserialize a single struct or a list of structs.
pub fn deserialize_entry_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum EntryOneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }
    match Option::<EntryOneOrMany<T>>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(EntryOneOrMany::One(item)) => Ok(vec![item]),
        Some(EntryOneOrMany::Many(items)) => Ok(items),
    }
}

/// A single item or a list of items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(item) => vec![item],
            OneOrMany::Many(items) => items,
        }
    }

    pub fn as_vec(&self) -> Vec<&T> {
        match self {
            OneOrMany::One(item) => vec![item],
            OneOrMany::Many(items) => items.iter().collect(),
        }
    }
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseEngine {
    #[default]
    Mysql,
    Postgres,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPathsConfig {
    pub rsync: Option<String>,
    pub borg: Option<String>,
    pub restic: Option<String>,
    pub mysqldump: Option<String>,
    pub pg_dump: Option<String>,
    pub gzip: Option<String>,
    pub mount: Option<String>,
    pub umount: Option<String>,
    pub cryptsetup: Option<String>,
    pub cp: Option<String>,
    pub rm: Option<String>,
    pub sshfs: Option<String>,
    pub fusermount: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    #[serde(default = "default_logdir")]
    pub logdir: String,
    /// Scratch directory exported as `TMPDIR` to backup tools (restic/borg) so they don't
    /// exhaust a small `/tmp`. Defaults to a disk-backed path (see `resolve_tmp_dir`).
    #[serde(default)]
    pub tmp_dir: Option<String>,
    #[serde(default)]
    pub tools: ToolPathsConfig,
}

fn default_logdir() -> String {
    crate::config::defaults::default_logdir()
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            logdir: default_logdir(),
            tmp_dir: None,
            tools: ToolPathsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthcheckConfig {
    pub url: String,
    pub uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestMountEntry {
    pub uuid: String,
    pub mountpoint: Option<String>,
    pub password: Option<String>,
    #[serde(default = "default_dot_path")]
    pub path: String,
}

fn default_dot_path() -> String {
    ".".to_string()
}

/// Remote source mounted read-only via sshfs. Authentication is always key-based
/// (`identity_file` or the agent/`~/.ssh/config`); passwords are intentionally unsupported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrcMountEntry {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    pub path: String,
    pub identity_file: Option<String>,
    pub staging_dir: Option<String>,
    #[serde(default)]
    pub ssh_options: Vec<String>,
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default)]
    pub keep_hourly: i64,
    #[serde(default)]
    pub keep_daily: i64,
    #[serde(default)]
    pub keep_weekly: i64,
    #[serde(default)]
    pub keep_monthly: i64,
    #[serde(default)]
    pub keep_yearly: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBackupJob {
    #[serde(default)]
    pub comment: String,
    pub logdir: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub src: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub dest: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub exclude: Vec<String>,
    pub password: Option<String>,
    #[serde(default, deserialize_with = "deserialize_entry_list")]
    pub destmount: Vec<DestMountEntry>,
    #[serde(default, deserialize_with = "deserialize_entry_list")]
    pub srcmount: Vec<SrcMountEntry>,
    #[serde(flatten)]
    pub retention: RetentionConfig,
    pub healthcheck: Option<HealthcheckConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseJob {
    #[serde(default)]
    pub comment: String,
    pub logdir: Option<String>,
    #[serde(default)]
    pub engine: DatabaseEngine,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub src: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_list")]
    pub dest: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_entry_list")]
    pub destmount: Vec<DestMountEntry>,
    pub healthcheck: Option<HealthcheckConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub rsync: Vec<FileBackupJob>,
    #[serde(default)]
    pub borg: Vec<FileBackupJob>,
    #[serde(default)]
    pub restic: Vec<FileBackupJob>,
    #[serde(default)]
    pub database: Vec<DatabaseJob>,
}

fn default_version() -> u32 {
    1
}

/// Resolved runtime configuration combining CLI and file.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub lock_file: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub threadpool_size: usize,
    pub retry_interval_sec: u64,
    pub retry_count: u32,
    pub continuous_hours: u64,
    pub dry_run: bool,
    pub file: FileConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackupType {
    Rsync,
    Borg,
    Restic,
    Database,
}

impl BackupType {
    pub fn as_str(self) -> &'static str {
        match self {
            BackupType::Rsync => "rsync",
            BackupType::Borg => "borg",
            BackupType::Restic => "restic",
            BackupType::Database => "database",
        }
    }

    pub fn priority(self) -> u8 {
        match self {
            BackupType::Rsync => 0,
            BackupType::Borg => 1,
            BackupType::Restic => 2,
            BackupType::Database => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId {
    pub backup_type: BackupType,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub enum JobSpec {
    File {
        id: JobId,
        job_type: BackupType,
        job: FileBackupJob,
    },
    Database {
        id: JobId,
        job: DatabaseJob,
    },
}

impl JobId {
    pub fn scope_key(&self) -> String {
        format!("{}-{}", self.backup_type.as_str(), self.index)
    }
}

impl JobSpec {
    pub fn id(&self) -> &JobId {
        match self {
            JobSpec::File { id, .. } => id,
            JobSpec::Database { id, .. } => id,
        }
    }

    pub fn comment(&self) -> &str {
        match self {
            JobSpec::File { job, .. } => &job.comment,
            JobSpec::Database { job, .. } => &job.comment,
        }
    }

    pub fn healthcheck(&self) -> Option<&HealthcheckConfig> {
        match self {
            JobSpec::File { job, .. } => job.healthcheck.as_ref(),
            JobSpec::Database { job, .. } => job.healthcheck.as_ref(),
        }
    }

    pub fn dest_summary(&self) -> String {
        match self {
            JobSpec::File { id, job, .. } => {
                let scope = id.scope_key();
                let mut dests = job.dest.clone();
                for dm in &job.destmount {
                    let mp = dm
                        .mountpoint
                        .clone()
                        .unwrap_or_else(|| format!("/mnt/backapp/{scope}/{}", dm.uuid));
                    dests.push(format!("{mp}/{}", dm.path));
                }
                if dests.is_empty() {
                    "none".into()
                } else {
                    dests.join(",")
                }
            }
            JobSpec::Database { job, .. } => {
                if job.dest.is_empty() {
                    job.destmount
                        .iter()
                        .map(|d| d.uuid.clone())
                        .collect::<Vec<_>>()
                        .join(",")
                } else {
                    job.dest.join(",")
                }
            }
        }
    }

    pub fn logdir(&self) -> Option<String> {
        match self {
            JobSpec::File { job, .. } => job.logdir.clone(),
            JobSpec::Database { job, .. } => job.logdir.clone(),
        }
    }
}

impl FileConfig {
    pub fn all_jobs(&self) -> Vec<JobSpec> {
        let mut jobs = Vec::new();
        for (index, job) in self.rsync.iter().cloned().enumerate() {
            jobs.push(JobSpec::File {
                id: JobId {
                    backup_type: BackupType::Rsync,
                    index,
                },
                job_type: BackupType::Rsync,
                job,
            });
        }
        for (index, job) in self.borg.iter().cloned().enumerate() {
            jobs.push(JobSpec::File {
                id: JobId {
                    backup_type: BackupType::Borg,
                    index,
                },
                job_type: BackupType::Borg,
                job,
            });
        }
        for (index, job) in self.restic.iter().cloned().enumerate() {
            jobs.push(JobSpec::File {
                id: JobId {
                    backup_type: BackupType::Restic,
                    index,
                },
                job_type: BackupType::Restic,
                job,
            });
        }
        for (index, job) in self.database.iter().cloned().enumerate() {
            jobs.push(JobSpec::Database {
                id: JobId {
                    backup_type: BackupType::Database,
                    index,
                },
                job,
            });
        }
        jobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_string_list_scalar() {
        let yaml = r#"
src: /home/user
dest:
  - /backup/a
  - /backup/b
"#;
        #[derive(Deserialize)]
        struct T {
            #[serde(deserialize_with = "deserialize_string_list")]
            src: Vec<String>,
            #[serde(deserialize_with = "deserialize_string_list")]
            dest: Vec<String>,
        }
        let t: T = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(t.src, vec!["/home/user"]);
        assert_eq!(t.dest, vec!["/backup/a", "/backup/b"]);
    }
}
