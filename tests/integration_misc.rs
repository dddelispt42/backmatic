#![cfg(feature = "integration-tests")]

mod common;

use std::sync::Arc;

use backmatic::config::types::{
    BackupType, FileBackupJob, HealthcheckConfig, JobId, JobSpec, RetentionConfig,
};
use backmatic::error::BackmaticError;
use backmatic::inject::{
    BackmaticContext, CommandExecutor, CommandRequest, CommandResult, MockHttpClient,
};
use backmatic::scheduler::{JobState, Scheduler};

struct FailingExecutor;

impl CommandExecutor for FailingExecutor {
    fn run(&self, request: &CommandRequest) -> backmatic::Result<CommandResult> {
        Err(BackmaticError::Command {
            command: request.program.clone(),
            code: Some(1),
            message: "injected failure".into(),
        })
    }

    fn spawn(
        &self,
        request: &CommandRequest,
    ) -> backmatic::Result<std::process::Child> {
        Err(BackmaticError::Command {
            command: request.program.clone(),
            code: None,
            message: "spawn not supported".into(),
        })
    }
}

#[test]
fn t18_healthcheck_success_and_failure_ping() {
    let mock = Arc::new(MockHttpClient::new());
    let h = common::Harness::new();
    let ctx = h.ctx.clone().with_http(Arc::clone(&mock) as Arc<dyn backmatic::inject::HttpClient>);

    // Success path via runner
    let job = FileBackupJob {
        healthcheck: Some(HealthcheckConfig {
            url: "https://hc.example.com".into(),
            uuid: "uuid-success".into(),
        }),
        ..common::file_job(
            "hc-test",
            vec![format!("{}/", common::TestTree::basic().path().display())],
            vec![format!("{}/", h.dest_dir("dest").display())],
            vec![],
            RetentionConfig::default(),
        )
    };
    if common::skip_without_tool("rsync") {
        backmatic::runners::execute_job(&ctx, &JobSpec::File {
            id: JobId {
                backup_type: BackupType::Rsync,
                index: 0,
            },
            job_type: BackupType::Rsync,
            job,
        })
        .unwrap();
        let pings = mock.pings.lock().unwrap();
        assert!(pings.iter().any(|p| p.url.contains("/ping/uuid-success")));
    }

    // Failure path via scheduler retries exhausted
    let mock2 = Arc::new(MockHttpClient::new());
    let mut config = h.ctx.config.clone();
    config.retry_count = 1;
    config.dry_run = false;
    let specs = vec![JobSpec::File {
        id: JobId {
            backup_type: BackupType::Rsync,
            index: 0,
        },
        job_type: BackupType::Rsync,
        job: FileBackupJob {
            comment: "fail-job".into(),
            logdir: Some(h.logdir.to_string_lossy().to_string()),
            src: vec!["/nonexistent/src/path".into()],
            dest: vec!["/tmp/dest".into()],
            exclude: vec![],
            password: None,
            destmount: vec![],
            srcmount: vec![],
            retention: RetentionConfig::default(),
            healthcheck: Some(HealthcheckConfig {
                url: "https://hc.example.com".into(),
                uuid: "uuid-fail".into(),
            }),
            verify: Default::default(),
        },
    }];
    let scheduler = Scheduler::new(
        BackmaticContext::new(config)
            .with_http(Arc::clone(&mock2) as Arc<dyn backmatic::inject::HttpClient>)
            .with_commands(Arc::new(FailingExecutor)),
        specs,
    );
    assert!(!scheduler.run_cycle().unwrap());
    let pings = mock2.pings.lock().unwrap();
    let fail = pings
        .iter()
        .find(|p| p.url.contains("/fail"))
        .expect("failure ping");
    let body = fail.body.as_ref().expect("body");
    assert!(body.contains("job_type=rsync"));
    assert!(body.contains("comment=fail-job"));
    assert!(body.contains("last_error"));
}

#[test]
fn t20_continuous_overrun_requeues_completed_only() {
    let config = backmatic::config::types::AppConfig {
        lock_file: "/tmp/lock".into(),
        config_path: "/tmp/cfg".into(),
        threadpool_size: 2,
        retry_interval_sec: 0,
        retry_count: 1,
        continuous_hours: 0,
        dry_run: true,
        file: backmatic::config::types::FileConfig::default(),
    };
    let specs = vec![JobSpec::File {
        id: JobId {
            backup_type: BackupType::Rsync,
            index: 0,
        },
        job_type: BackupType::Rsync,
        job: common::file_job("j1", vec![], vec![], vec![], RetentionConfig::default()),
    }];
    let scheduler = Scheduler::new(BackmaticContext::new(config), specs);
    let id = JobId {
        backup_type: BackupType::Rsync,
        index: 0,
    };
    scheduler.set_job_state(&id, JobState::Completed);
    scheduler.on_overrun_tick();
    assert_eq!(scheduler.job_state(&id), Some(JobState::Pending));
}

#[test]
#[ignore = "requires privileged LUKS setup (integration-luks)"]
fn t15_luks_destmount() {}

#[test]
#[ignore = "requires OpenSSH testcontainer (integration-ssh)"]
fn t14_srcmount_pull() {}

#[test]
#[ignore = "requires MySQL testcontainer (integration-db)"]
fn t16_database_mysql() {}

#[test]
#[ignore = "requires Postgres testcontainer (integration-db)"]
fn t17_database_postgres() {}

#[test]
#[ignore = "requires LUKS loop device (integration-luks)"]
fn t24_destmount_only_job() {}
