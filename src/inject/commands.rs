use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;

use crate::error::{BackmaticError, Result};
use crate::inject::process::ProcessRegistry;

fn sensitive_env_key(key: &str) -> bool {
    let k = key.to_uppercase();
    k.contains("PASSWORD")
        || k.contains("PASSPHRASE")
        || k.contains("SECRET")
        || k.contains("TOKEN")
}

fn redact_inline_secrets(input: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"-p\S+").expect("valid regex"));
    re.replace_all(input, "-p***").to_string()
}

#[derive(Debug)]
pub struct CommandRequest {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<Stdio>,
    pub stdout: Option<Stdio>,
    pub stderr: Option<Stdio>,
    /// When set, `run` monitors the child's `/proc/<pid>/io` byte counters and aborts the child
    /// (SIGTERM then SIGKILL) if they make no progress for this long — catches backups that hang
    /// forever (e.g. a wedged sshfs read returning I/O errors). `None` disables the watchdog.
    pub stall_timeout: Option<Duration>,
}

impl CommandRequest {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            stdin: None,
            stdout: None,
            stderr: None,
            stall_timeout: None,
        }
    }

    /// Enable the I/O-progress stall watchdog for this command (see [`CommandRequest::stall_timeout`]).
    pub fn stall_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.stall_timeout = timeout;
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = Some(stdin);
        self
    }

    pub fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = Some(stdout);
        self
    }

    pub fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = Some(stderr);
        self
    }

    /// Shell-safe representation for logs; secrets in env and inline flags are redacted.
    pub fn display_redacted(&self) -> String {
        let args: Vec<String> = self
            .args
            .iter()
            .map(|a| redact_inline_secrets(a))
            .collect();
        let mut cmd = self.program.clone();
        if !args.is_empty() {
            cmd.push(' ');
            cmd.push_str(&args.join(" "));
        }
        if self.env.is_empty() {
            return cmd;
        }
        let env: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| {
                if sensitive_env_key(k) {
                    format!("{k}=***")
                } else {
                    format!("{k}={v}")
                }
            })
            .collect();
        format!("{cmd} [env: {}]", env.join(" "))
    }
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandResult {
    pub fn from_output(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

pub trait CommandExecutor: Send + Sync {
    fn run(&self, request: &CommandRequest) -> Result<CommandResult>;
    fn spawn(&self, request: &CommandRequest) -> Result<std::process::Child>;
}

pub struct RealCommandExecutor {
    processes: Arc<ProcessRegistry>,
}

impl RealCommandExecutor {
    pub fn new(processes: Arc<ProcessRegistry>) -> Self {
        Self { processes }
    }
}

impl Default for RealCommandExecutor {
    fn default() -> Self {
        Self::new(Arc::new(ProcessRegistry::new()))
    }
}

/// How long to keep collecting a command's output after the direct child has already exited.
/// In the normal case the pipe is at EOF the moment the child dies, so this is never actually
/// waited out; it only bounds the pathological case where a daemonized descendant still holds
/// the pipe write end open (see `run`).
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

impl CommandExecutor for RealCommandExecutor {
    fn run(&self, request: &CommandRequest) -> Result<CommandResult> {
        log::debug!("exec: {}", request.display_redacted());
        let mut cmd = build_command(request);
        // No stdin: an unattended run must never block waiting for interactive input.
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|source| BackmaticError::Command {
            command: request.program.clone(),
            code: None,
            message: source.to_string(),
        })?;
        let pid = child.id();
        self.processes.register(pid);

        // Drain stdout/stderr concurrently (so a full pipe can't deadlock the child), but key
        // completion off the *direct* child exiting via `wait()` rather than pipe EOF. A child
        // that daemonizes — e.g. sshfs, whose `ssh` transport child inherits and holds the pipe
        // open for the whole mount lifetime — never lets the pipe reach EOF. `wait_with_output`
        // would block on that until unmount, wedging the job (and the whole process) forever.
        let stdout_rx = spawn_reader(child.stdout.take());
        let stderr_rx = spawn_reader(child.stderr.take());

        let status = match request.stall_timeout {
            Some(timeout) if !timeout.is_zero() => {
                wait_with_watchdog(&mut child, pid, timeout, &request.program)
            }
            _ => child.wait().map_err(|source| BackmaticError::Command {
                command: request.program.clone(),
                code: None,
                message: source.to_string(),
            }),
        };
        self.processes.unregister(pid);
        let status = status?;

        // Share one grace deadline across both streams so a daemonizing command costs at most
        // `OUTPUT_GRACE` total, not per-stream.
        let deadline = std::time::Instant::now() + OUTPUT_GRACE;
        let stdout = collect_output(stdout_rx, deadline);
        let stderr = collect_output(stderr_rx, deadline);

        log::debug!("exec finished: {} (exit {:?})", request.program, status.code());
        Ok(CommandResult {
            status,
            stdout,
            stderr,
        })
    }

    fn spawn(&self, request: &CommandRequest) -> Result<std::process::Child> {
        log::debug!("spawn: {}", request.display_redacted());
        build_command(request).spawn().map_err(|source| BackmaticError::Command {
            command: request.program.clone(),
            code: None,
            message: source.to_string(),
        })
    }
}

/// Drain a child pipe to end on a background thread, delivering the bytes over a channel. The
/// thread owns the pipe and exits on its own when the write end is finally closed (even if that
/// is long after we've stopped waiting).
fn spawn_reader<R: Read + Send + 'static>(pipe: Option<R>) -> Option<mpsc::Receiver<Vec<u8>>> {
    pipe.map(|mut pipe| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        rx
    })
}

/// Collect a reader's bytes now that the child has exited. Returns immediately in the common
/// case (pipe already at EOF); otherwise waits until `deadline` before giving up, so a lingering
/// descendant holding the pipe can't block us. Any output produced after the timeout is dropped,
/// which is acceptable for the daemonizing case (its useful output/errors arrive before the
/// fork).
fn collect_output(rx: Option<mpsc::Receiver<Vec<u8>>>, deadline: std::time::Instant) -> Vec<u8> {
    match rx {
        Some(rx) => {
            let timeout = deadline.saturating_duration_since(std::time::Instant::now());
            rx.recv_timeout(timeout).unwrap_or_default()
        }
        None => Vec::new(),
    }
}

/// Longest we sleep between watchdog polls. Kept short so shutdown/exit is noticed promptly, but
/// capped below `stall_timeout` so a small configured timeout still polls at least a few times.
const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Grace period after SIGTERM before escalating to SIGKILL for a stalled child.
const WATCHDOG_KILL_GRACE: Duration = Duration::from_secs(5);

/// Wait for `child` while enforcing the I/O-progress stall watchdog. Polls `/proc/<pid>/io`; if
/// the cumulative read+write byte counters do not advance for `timeout`, the child is aborted
/// (SIGTERM, then SIGKILL after a grace period) and its final status returned.
fn wait_with_watchdog(
    child: &mut std::process::Child,
    pid: u32,
    timeout: Duration,
    program: &str,
) -> Result<std::process::ExitStatus> {
    let poll = std::cmp::min(timeout, WATCHDOG_POLL_INTERVAL).max(Duration::from_millis(100));
    let mut last_bytes = read_proc_io_bytes(pid);
    let mut last_progress = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(source) => {
                return Err(BackmaticError::Command {
                    command: program.to_string(),
                    code: None,
                    message: source.to_string(),
                })
            }
        }

        std::thread::sleep(poll);

        match (read_proc_io_bytes(pid), last_bytes) {
            // Counters advanced (or first successful reading): reset the stall clock.
            (Some(now), Some(prev)) if now != prev => {
                last_bytes = Some(now);
                last_progress = std::time::Instant::now();
            }
            (Some(now), None) => {
                last_bytes = Some(now);
                last_progress = std::time::Instant::now();
            }
            // Couldn't read counters (kernel without per-proc io, or process gone): don't treat
            // an unreadable /proc as a stall — let `try_wait` decide when it actually exits.
            (None, _) => {
                last_progress = std::time::Instant::now();
            }
            // Readable but unchanged: leave `last_progress` alone so the stall clock keeps running.
            _ => {}
        }

        if last_progress.elapsed() >= timeout {
            log::error!(
                "stall watchdog: {program} (pid {pid}) made no I/O progress for {}s; aborting",
                timeout.as_secs()
            );
            return Ok(abort_stalled_child(child, pid));
        }
    }
}

/// Terminate a stalled child: SIGTERM, wait up to [`WATCHDOG_KILL_GRACE`], then SIGKILL. Returns
/// the reaped exit status.
fn abort_stalled_child(child: &mut std::process::Child, pid: u32) -> std::process::ExitStatus {
    send_signal(pid, term_signal());
    let deadline = std::time::Instant::now() + WATCHDOG_KILL_GRACE;
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    child.wait().unwrap_or_else(|_| default_killed_status())
}

#[cfg(unix)]
fn term_signal() -> i32 {
    libc::SIGTERM
}

#[cfg(not(unix))]
fn term_signal() -> i32 {
    0
}

#[cfg(unix)]
fn send_signal(pid: u32, sig: i32) {
    // SAFETY: kill() with a valid pid is safe; failures (already-exited) are ignored.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: u32, _sig: i32) {}

#[cfg(unix)]
fn default_killed_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(libc::SIGKILL)
}

#[cfg(not(unix))]
fn default_killed_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(1)
}

/// Read cumulative I/O for a pid as `rchar + wchar` from `/proc/<pid>/io`. Returns `None` when the
/// file is missing/unreadable (non-Linux, permission, or the process already exited).
fn read_proc_io_bytes(pid: u32) -> Option<u64> {
    let contents = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    let mut total: u64 = 0;
    let mut seen = false;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("rchar:").or_else(|| line.strip_prefix("wchar:")) {
            if let Ok(v) = rest.trim().parse::<u64>() {
                total = total.saturating_add(v);
                seen = true;
            }
        }
    }
    seen.then_some(total)
}

fn build_command(request: &CommandRequest) -> Command {
    let mut cmd = Command::new(&request.program);
    cmd.args(&request.args);
    for (k, v) in &request.env {
        cmd.env(k, v);
    }
    if request.stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    if request.stdout.is_some() {
        cmd.stdout(Stdio::piped());
    }
    if request.stderr.is_some() {
        cmd.stderr(Stdio::piped());
    }
    cmd
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn display_redacted_hides_env_secrets() {
        let req = CommandRequest::new("borg")
            .arg("create")
            .env("BORG_PASSPHRASE", "secret");
        let shown = req.display_redacted();
        assert!(shown.contains("BORG_PASSPHRASE=***"));
        assert!(!shown.contains("secret"));
    }

    #[test]
    fn display_redacted_hides_inline_password_flags() {
        let req = CommandRequest::new("bash")
            .arg("-c")
            .arg("mysqldump -u root -pS3cret -h localhost");
        let shown = req.display_redacted();
        assert!(shown.contains("-p***"));
        assert!(!shown.contains("S3cret"));
    }

    // Regression: a command that backgrounds a long-lived descendant (which inherits the
    // stdout/stderr pipe, like sshfs' ssh transport) must not wedge `run`. The direct child
    // exits immediately; `wait_with_output` would block on the sleeper until it dies.
    #[cfg(unix)]
    #[test]
    fn run_returns_when_descendant_holds_pipe_open() {
        use std::time::Instant;

        let exec = RealCommandExecutor::default();
        // `sleep 30 &` inherits the shell's stdout/stderr (our pipe); `sh` then prints and exits.
        let req = CommandRequest::new("sh")
            .arg("-c")
            .arg("sleep 30 & echo hi");

        let start = Instant::now();
        let result = exec.run(&req).expect("run should not error");
        let elapsed = start.elapsed();

        assert!(result.status.success(), "direct child (sh) exited 0");
        assert!(
            elapsed < Duration::from_secs(10),
            "run must return promptly despite the lingering descendant (took {elapsed:?})"
        );
    }

    // The stall watchdog must abort a child that makes no I/O progress within the timeout.
    #[cfg(unix)]
    #[test]
    fn watchdog_aborts_stalled_child() {
        use std::time::Instant;

        let exec = RealCommandExecutor::default();
        let req = CommandRequest::new("sleep")
            .arg("60")
            .stall_timeout(Some(Duration::from_secs(1)));

        let start = Instant::now();
        let result = exec.run(&req).expect("run should not error");
        let elapsed = start.elapsed();

        assert!(!result.status.success(), "stalled child should be aborted, not succeed");
        assert!(
            elapsed < Duration::from_secs(15),
            "watchdog should abort the stalled child promptly (took {elapsed:?})"
        );
    }

    // A command that keeps doing I/O must not be aborted by the watchdog.
    #[cfg(unix)]
    #[test]
    fn watchdog_allows_progressing_child() {
        let exec = RealCommandExecutor::default();
        // Continuously writes for ~3s: /proc/<pid>/io wchar advances every poll.
        let req = CommandRequest::new("sh")
            .arg("-c")
            .arg("for i in $(seq 1 30); do echo x; sleep 0.1; done")
            .stall_timeout(Some(Duration::from_secs(2)));

        let result = exec.run(&req).expect("run should not error");
        assert!(result.status.success(), "progressing child should finish normally");
    }

    #[cfg(unix)]
    #[test]
    fn read_proc_io_bytes_reads_self() {
        let pid = std::process::id();
        assert!(read_proc_io_bytes(pid).is_some(), "should read own /proc/<pid>/io on Linux");
    }
}

#[cfg(test)]
use std::os::unix::process::ExitStatusExt;

#[cfg(test)]
#[derive(Default)]
pub struct RecordingExecutor {
    pub calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// Env vars per recorded call, index-aligned with `calls`.
    pub envs: std::sync::Mutex<Vec<Vec<(String, String)>>>,
}

#[cfg(test)]
impl RecordingExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Value of env `key` for the first recorded call whose program contains `program_needle`.
    pub fn find_env(&self, program_needle: &str, key: &str) -> Option<String> {
        let calls = self.calls.lock().unwrap();
        let envs = self.envs.lock().unwrap();
        for (i, (prog, _)) in calls.iter().enumerate() {
            if prog.contains(program_needle) {
                if let Some(env) = envs.get(i) {
                    if let Some((_, v)) = env.iter().find(|(k, _)| k == key) {
                        return Some(v.clone());
                    }
                }
            }
        }
        None
    }

    fn record(&self, request: &CommandRequest) {
        self.calls
            .lock()
            .unwrap()
            .push((request.program.clone(), request.args.clone()));
        self.envs.lock().unwrap().push(request.env.clone());
    }
}

#[cfg(test)]
impl CommandExecutor for RecordingExecutor {
    fn run(&self, request: &CommandRequest) -> Result<CommandResult> {
        self.record(request);
        Ok(CommandResult {
            status: std::process::ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn spawn(&self, request: &CommandRequest) -> Result<std::process::Child> {
        self.record(request);
        Err(BackmaticError::Command {
            command: request.program.clone(),
            code: None,
            message: "spawn not supported in RecordingExecutor".into(),
        })
    }
}
