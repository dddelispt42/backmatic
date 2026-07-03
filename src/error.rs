use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackmaticError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("IO error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("command failed: {command} (exit {code:?}): {message}")]
    Command {
        command: String,
        code: Option<i32>,
        message: String,
    },

    #[error("mount error: {0}")]
    Mount(String),

    #[error("backup job failed: {job_type} ({comment}): {message}")]
    JobFailed {
        job_type: String,
        comment: String,
        message: String,
    },

    #[error("another backup instance is already running (lock: {lock_file})")]
    AlreadyRunning { lock_file: String },

    #[error("root privileges required: {reason}. Re-run backmatic as root (e.g. via sudo).")]
    RootRequired { reason: String },

    #[error("no available destination for job '{comment}'")]
    NoDestination { comment: String },

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, BackmaticError>;
