pub mod clock;
pub mod commands;
pub mod http;
pub mod paths;
pub mod process;

pub use clock::{Clock, FakeClock, SystemClock};
pub use commands::{CommandExecutor, CommandRequest, CommandResult, RealCommandExecutor};
pub use http::{HttpClient, PingResult, RealHttpClient};
#[cfg(any(test, feature = "integration-tests"))]
pub use http::MockHttpClient;
pub use paths::ToolPaths;
pub use process::ProcessRegistry;

use std::sync::Arc;

use crate::config::types::AppConfig;
use crate::mount::MountRegistry;
use crate::shutdown::Shutdown;

#[derive(Clone)]
pub struct BackmaticContext {
    pub config: AppConfig,
    pub paths: ToolPaths,
    pub commands: Arc<dyn CommandExecutor>,
    pub clock: Arc<dyn Clock>,
    pub http: Arc<dyn HttpClient>,
    pub mount_registry: Arc<MountRegistry>,
    pub process_registry: Arc<ProcessRegistry>,
    pub shutdown: Arc<Shutdown>,
}

impl BackmaticContext {
    pub fn new(config: AppConfig) -> Self {
        let paths = ToolPaths::from_config(&config);
        let mount_registry = Arc::new(MountRegistry::new());
        let process_registry = Arc::new(ProcessRegistry::new());
        let shutdown = Shutdown::new(
            Arc::clone(&mount_registry),
            Arc::clone(&process_registry),
            config.lock_file.clone(),
        );
        Self {
            config,
            paths,
            commands: Arc::new(RealCommandExecutor::new(Arc::clone(&process_registry))),
            clock: Arc::new(SystemClock),
            http: Arc::new(RealHttpClient::new()),
            mount_registry,
            process_registry,
            shutdown,
        }
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_commands(mut self, commands: Arc<dyn CommandExecutor>) -> Self {
        self.commands = commands;
        self
    }

    pub fn with_http(mut self, http: Arc<dyn HttpClient>) -> Self {
        self.http = http;
        self
    }
}

pub fn log_command_output(logfile: &str, result: &CommandResult) -> crate::error::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logfile)
        .map_err(|source| crate::error::BackmaticError::Io {
            path: logfile.to_string(),
            source,
        })?;
    file.write_all(&result.stdout).map_err(|source| {
        crate::error::BackmaticError::Io {
            path: logfile.to_string(),
            source,
        }
    })?;
    file.write_all(&result.stderr).map_err(|source| {
        crate::error::BackmaticError::Io {
            path: logfile.to_string(),
            source,
        }
    })?;

    if !result.status.success() {
        let code = result.status.code().unwrap_or(-1);
        let err_path = format!("{logfile}.ERROR_{code}");
        std::fs::rename(logfile, &err_path).map_err(|source| crate::error::BackmaticError::Io {
            path: err_path,
            source,
        })?;
    }
    Ok(())
}

pub fn command_exists(executor: &dyn CommandExecutor, path: &str) -> bool {
    use std::process::Stdio;
    executor
        .run(
            &CommandRequest::new("test")
                .arg("-x")
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .map(|r| r.status.success())
        .unwrap_or(false)
}

pub fn stderr_tail(result: &CommandResult, max_lines: usize) -> String {
    let text = String::from_utf8_lossy(&result.stderr);
    text.lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}
