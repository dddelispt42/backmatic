use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::types::{JobId, JobSpec};
use crate::config::{generate_log_path, logdir_for_job};
use crate::error::Result;
use crate::healthcheck;
use crate::inject::BackmaticContext;
use crate::runners::execute_job;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Pending,
    Running,
    Retrying,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
struct TrackedJob {
    spec: JobSpec,
    state: JobState,
    /// Number of execution attempts made so far this cycle.
    attempts: u32,
    /// Earliest instant this job may be (re)tried; `None` means "immediately".
    next_attempt: Option<Instant>,
}

impl TrackedJob {
    fn new(spec: JobSpec) -> Self {
        Self {
            spec,
            state: JobState::Pending,
            attempts: 0,
            next_attempt: None,
        }
    }

    fn is_schedulable(&self) -> bool {
        matches!(self.state, JobState::Pending | JobState::Retrying)
    }

    fn is_ready(&self, now: Instant) -> bool {
        self.is_schedulable() && self.next_attempt.map(|t| t <= now).unwrap_or(true)
    }
}

/// What a worker should do next, decided under the jobs lock.
enum NextAction {
    /// A job is ready to run now (already marked `Running`).
    Run(String, Box<JobSpec>),
    /// No job is ready yet, but one becomes eligible at this instant (a pending retry).
    WaitUntil(Instant),
    /// No schedulable jobs remain.
    Done,
}

pub struct Scheduler {
    ctx: Arc<BackmaticContext>,
    jobs: Arc<Mutex<HashMap<String, TrackedJob>>>,
    any_failed: Arc<Mutex<bool>>,
}

impl Scheduler {
    pub fn new(ctx: BackmaticContext, specs: Vec<JobSpec>) -> Self {
        let mut map = HashMap::new();
        for spec in specs {
            let key = job_key(spec.id());
            map.insert(key, TrackedJob::new(spec));
        }
        Self {
            ctx: Arc::new(ctx),
            jobs: Arc::new(Mutex::new(map)),
            any_failed: Arc::new(Mutex::new(false)),
        }
    }

    pub fn context(&self) -> BackmaticContext {
        BackmaticContext {
            config: self.ctx.config.clone(),
            paths: self.ctx.paths.clone(),
            commands: Arc::clone(&self.ctx.commands),
            clock: Arc::clone(&self.ctx.clock),
            http: Arc::clone(&self.ctx.http),
            mount_registry: Arc::clone(&self.ctx.mount_registry),
            process_registry: Arc::clone(&self.ctx.process_registry),
            shutdown: Arc::clone(&self.ctx.shutdown),
        }
    }

    /// Start a new backup cycle. Re-enqueues `Failed` jobs from the prior cycle and resets
    /// per-cycle retry bookkeeping.
    pub fn run_cycle(&self) -> Result<bool> {
        *self.any_failed.lock().unwrap() = false;

        {
            let mut jobs = self.jobs.lock().unwrap();
            for job in jobs.values_mut() {
                if job.state == JobState::Failed {
                    job.state = JobState::Pending;
                }
                job.attempts = 0;
                job.next_attempt = None;
            }
        }

        let pool = threadpool::ThreadPool::new(self.ctx.config.threadpool_size);
        let workers = self.ctx.config.threadpool_size;

        for _ in 0..workers {
            let ctx = Arc::clone(&self.ctx);
            let jobs = Arc::clone(&self.jobs);
            let any_failed = Arc::clone(&self.any_failed);
            pool.execute(move || worker_loop(&ctx, &jobs, &any_failed));
        }

        pool.join();
        let ok = !*self.any_failed.lock().unwrap();
        if ok {
            log::info!("Finished backup cycle");
        } else {
            log::error!("Backup cycle completed with failures");
        }
        Ok(ok)
    }

    /// On continuous-mode overrun tick: only re-enqueue jobs that completed successfully.
    /// `Running` and `Retrying` jobs are left unchanged (no duplicate instances).
    pub fn on_overrun_tick(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        for job in jobs.values_mut() {
            if job.state == JobState::Completed {
                job.state = JobState::Pending;
                job.attempts = 0;
                job.next_attempt = None;
            }
        }
    }

    pub fn job_state(&self, id: &JobId) -> Option<JobState> {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(&job_key(id)).map(|j| j.state)
    }

    #[cfg(any(test, feature = "integration-tests"))]
    pub fn set_job_state(&self, id: &JobId, state: JobState) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(&job_key(id)) {
            job.state = state;
        }
    }

    #[cfg(test)]
    fn job_attempts(&self, id: &JobId) -> Option<u32> {
        let jobs = self.jobs.lock().unwrap();
        jobs.get(&job_key(id)).map(|j| j.attempts)
    }
}

/// Worker loop: repeatedly claim a ready job and run one attempt. Failed jobs are re-enqueued
/// with a deferred `next_attempt`, so other ready jobs proceed instead of the pool blocking on
/// one job's retry backoff. Exits when no schedulable jobs remain or shutdown is requested.
fn worker_loop(
    ctx: &BackmaticContext,
    jobs: &Mutex<HashMap<String, TrackedJob>>,
    any_failed: &Mutex<bool>,
) {
    loop {
        if ctx.shutdown.is_requested() {
            break;
        }
        match take_ready_job(jobs) {
            NextAction::Run(key, spec) => {
                run_attempt(ctx, jobs, any_failed, &key, spec.as_ref());
            }
            NextAction::WaitUntil(deadline) => {
                interruptible_sleep_until(ctx, deadline);
            }
            NextAction::Done => break,
        }
    }
}

/// Claim the highest-priority ready job (marking it `Running`), or report the next retry
/// deadline, or that all jobs are terminal.
fn take_ready_job(jobs: &Mutex<HashMap<String, TrackedJob>>) -> NextAction {
    let now = Instant::now();
    let mut guard = jobs.lock().unwrap();

    let mut best: Option<(u8, String)> = None;
    let mut earliest_wait: Option<Instant> = None;

    for (key, job) in guard.iter() {
        if !job.is_schedulable() {
            continue;
        }
        if !job.is_ready(now) {
            if let Some(t) = job.next_attempt {
                earliest_wait = Some(earliest_wait.map_or(t, |cur| cur.min(t)));
            }
            continue;
        }
        let pri = job.spec.id().backup_type.priority();
        match &best {
            None => best = Some((pri, key.clone())),
            Some((best_pri, _)) if pri < *best_pri => best = Some((pri, key.clone())),
            _ => {}
        }
    }

    if let Some((_, key)) = best {
        let job = guard.get_mut(&key).expect("job present");
        job.state = JobState::Running;
        return NextAction::Run(key, Box::new(job.spec.clone()));
    }
    match earliest_wait {
        Some(t) => NextAction::WaitUntil(t),
        None => NextAction::Done,
    }
}

/// Execute a single attempt of a job and record the outcome (complete, deferred retry, or
/// final failure with a healthcheck failure ping).
fn run_attempt(
    ctx: &BackmaticContext,
    jobs: &Mutex<HashMap<String, TrackedJob>>,
    any_failed: &Mutex<bool>,
    key: &str,
    spec: &JobSpec,
) {
    let max = ctx.config.retry_count.max(1);
    let job_type = spec.id().backup_type.as_str();
    let comment = spec.comment();

    let attempt = {
        let mut guard = jobs.lock().unwrap();
        let job = guard.get_mut(key).expect("job present");
        job.attempts += 1;
        job.attempts
    };
    if attempt == 1 {
        log::info!("Start job {job_type} '{comment}'");
    }

    match execute_job(ctx, spec) {
        Ok(()) => {
            log::info!("Finished job {job_type} '{comment}'");
            let mut guard = jobs.lock().unwrap();
            let job = guard.get_mut(key).expect("job present");
            job.state = JobState::Completed;
            job.next_attempt = None;
        }
        Err(e) => {
            let can_retry = attempt < max && !ctx.shutdown.is_requested();
            if can_retry {
                log::warn!("{job_type} ('{comment}') attempt {attempt}/{max} failed: {e}");
                let deadline =
                    Instant::now() + Duration::from_secs(ctx.config.retry_interval_sec);
                let mut guard = jobs.lock().unwrap();
                let job = guard.get_mut(key).expect("job present");
                job.state = JobState::Retrying;
                job.next_attempt = Some(deadline);
            } else {
                log::error!("Failed job {job_type} '{comment}' after {attempt} attempt(s): {e}");
                {
                    let mut guard = jobs.lock().unwrap();
                    let job = guard.get_mut(key).expect("job present");
                    job.state = JobState::Failed;
                    job.next_attempt = None;
                }
                *any_failed.lock().unwrap() = true;
                notify_failure(ctx, spec, attempt, max, &e.to_string());
            }
        }
    }
}

fn notify_failure(ctx: &BackmaticContext, spec: &JobSpec, attempt: u32, max: u32, err: &str) {
    if let Some(hc) = spec.healthcheck() {
        let logdir = logdir_for_job(&ctx.config.file, spec.logdir().as_deref());
        let logfile =
            generate_log_path(&logdir, spec.id().backup_type.as_str(), spec.comment())
                .unwrap_or_else(|_| format!("{logdir}/unknown.log"));
        let _ = healthcheck::ping_failure(
            ctx,
            hc,
            spec.id().backup_type.as_str(),
            spec.comment(),
            attempt,
            max,
            err,
            &logfile,
            &spec.dest_summary(),
        );
    }
}

/// Sleep until `deadline`, in short chunks, returning early if shutdown is requested.
fn interruptible_sleep_until(ctx: &BackmaticContext, deadline: Instant) {
    while Instant::now() < deadline {
        if ctx.shutdown.is_requested() {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        ctx.clock.sleep(remaining.min(Duration::from_millis(500)));
    }
}

fn job_key(id: &JobId) -> String {
    format!("{}:{}", id.backup_type.as_str(), id.index)
}

pub mod cycle;
pub use cycle::CycleScheduler;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{BackupType, FileBackupJob, JobId, RetentionConfig};

    fn test_job(comment: &str, job_type: BackupType) -> JobSpec {
        JobSpec::File {
            id: JobId {
                backup_type: job_type,
                index: 0,
            },
            job_type,
            job: FileBackupJob {
                comment: comment.into(),
                logdir: Some("/tmp".into()),
                src: vec!["/tmp/src".into()],
                dest: vec!["/tmp/dest".into()],
                exclude: vec![],
                password: None,
                destmount: vec![],
                srcmount: vec![],
                retention: RetentionConfig::default(),
                healthcheck: None,
                verify: Default::default(),
            },
        }
    }

    #[test]
    fn overrun_tick_only_requeues_completed() {
        let config = crate::config::types::AppConfig {
            lock_file: "/tmp/lock".into(),
            config_path: "/tmp/cfg".into(),
            threadpool_size: 2,
            retry_interval_sec: 0,
            retry_count: 1,
            continuous_hours: 0,
            dry_run: true,
            file: crate::config::types::FileConfig::default(),
        };
        let specs = vec![
            test_job("j1", BackupType::Rsync),
            test_job("j2", BackupType::Borg),
            test_job("j3", BackupType::Restic),
        ];
        let scheduler = Scheduler::new(BackmaticContext::new(config), specs);
        let id1 = JobId {
            backup_type: BackupType::Rsync,
            index: 0,
        };
        let id2 = JobId {
            backup_type: BackupType::Borg,
            index: 0,
        };
        let id3 = JobId {
            backup_type: BackupType::Restic,
            index: 0,
        };
        scheduler.set_job_state(&id1, JobState::Completed);
        scheduler.set_job_state(&id2, JobState::Running);
        scheduler.set_job_state(&id3, JobState::Retrying);
        scheduler.on_overrun_tick();
        assert_eq!(scheduler.job_state(&id1), Some(JobState::Pending));
        assert_eq!(scheduler.job_state(&id2), Some(JobState::Running));
        assert_eq!(scheduler.job_state(&id3), Some(JobState::Retrying));
    }

    fn typed_job(comment: &str, job_type: BackupType, index: usize) -> JobSpec {
        match test_job(comment, job_type) {
            JobSpec::File { job_type, job, .. } => JobSpec::File {
                id: JobId { backup_type: job_type, index },
                job_type,
                job,
            },
            other => other,
        }
    }

    fn config_with(retry_count: u32, dry_run: bool, bogus_tools: bool) -> crate::config::types::AppConfig {
        use crate::config::types::{DefaultsConfig, FileConfig, ToolPathsConfig};
        let mut tools = ToolPathsConfig::default();
        if bogus_tools {
            tools.rsync = Some("/nonexistent/bin/rsync-does-not-exist".to_string());
            tools.borg = Some("/nonexistent/bin/borg-does-not-exist".to_string());
        }
        let file = FileConfig {
            defaults: DefaultsConfig {
                logdir: std::env::temp_dir()
                    .join("backmatic-sched-test")
                    .to_string_lossy()
                    .to_string(),
                tools,
                ..Default::default()
            },
            ..Default::default()
        };
        crate::config::types::AppConfig {
            lock_file: "/tmp/lock".into(),
            config_path: "/tmp/cfg".into(),
            threadpool_size: 1,
            retry_interval_sec: 0,
            retry_count,
            continuous_hours: 0,
            dry_run,
            file,
        }
    }

    #[test]
    fn dry_run_job_completes() {
        let scheduler = Scheduler::new(
            BackmaticContext::new(config_with(3, true, false)),
            vec![test_job("dry", BackupType::Rsync)],
        );
        let ok = scheduler.run_cycle().unwrap();
        assert!(ok);
        let id = JobId { backup_type: BackupType::Rsync, index: 0 };
        assert_eq!(scheduler.job_state(&id), Some(JobState::Completed));
    }

    #[test]
    fn failing_job_retries_up_to_retry_count_then_fails() {
        // A bogus rsync path makes the runner fail fast (tool not found) on every attempt.
        let scheduler = Scheduler::new(
            BackmaticContext::new(config_with(3, false, true)),
            vec![test_job("boom", BackupType::Rsync)],
        );
        let ok = scheduler.run_cycle().unwrap();
        assert!(!ok, "cycle with a permanently failing job should report failure");
        let id = JobId { backup_type: BackupType::Rsync, index: 0 };
        assert_eq!(scheduler.job_state(&id), Some(JobState::Failed));
        assert_eq!(
            scheduler.job_attempts(&id),
            Some(3),
            "job should be attempted exactly retry_count times"
        );
    }

    #[test]
    fn retry_count_one_means_single_attempt() {
        let scheduler = Scheduler::new(
            BackmaticContext::new(config_with(1, false, true)),
            vec![test_job("once", BackupType::Rsync)],
        );
        let ok = scheduler.run_cycle().unwrap();
        assert!(!ok);
        let id = JobId { backup_type: BackupType::Rsync, index: 0 };
        assert_eq!(scheduler.job_attempts(&id), Some(1));
    }

    #[test]
    fn a_failing_job_does_not_starve_another_job() {
        // With a single worker and two independently-failing jobs, both must be attempted the
        // full retry_count. This proves the deferred-retry queue interleaves jobs rather than a
        // single failing job monopolising the worker.
        let scheduler = Scheduler::new(
            BackmaticContext::new(config_with(2, false, true)),
            vec![
                typed_job("a", BackupType::Rsync, 0),
                typed_job("b", BackupType::Borg, 0),
            ],
        );
        let ok = scheduler.run_cycle().unwrap();
        assert!(!ok);
        let rsync_id = JobId { backup_type: BackupType::Rsync, index: 0 };
        let borg_id = JobId { backup_type: BackupType::Borg, index: 0 };
        assert_eq!(scheduler.job_state(&rsync_id), Some(JobState::Failed));
        assert_eq!(scheduler.job_state(&borg_id), Some(JobState::Failed));
        assert_eq!(scheduler.job_attempts(&rsync_id), Some(2));
        assert_eq!(scheduler.job_attempts(&borg_id), Some(2));
    }
}
