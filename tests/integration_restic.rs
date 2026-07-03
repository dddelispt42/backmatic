#![cfg(feature = "integration-tests")]

mod common;

use std::fs;

use backmatic::config::types::{BackupType, FileBackupJob, RetentionConfig};

use common::{
    assert_file_missing, assert_file_present, assert_trees_equal, file_job, find_file_root,
    restic_restore_latest, restic_snapshot_count, skip_without_tool, TestTree, Harness,
};

const PASS: &str = "test-restic-pass";

fn run_restic(
    h: &Harness,
    tree: &TestTree,
    repo: &std::path::Path,
    exclude: Vec<String>,
    retention: RetentionConfig,
) {
    let job = FileBackupJob {
        password: Some(PASS.into()),
        ..file_job(
            "integration-restic",
            vec![format!("{}/", tree.path().display())],
            vec![repo.to_string_lossy().to_string()],
            exclude,
            retention,
        )
    };
    h.run_job(BackupType::Restic, job);
}

#[test]
fn t01_initial_backup_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore = h.dest_dir("restore");
    restic_restore_latest(&repo, PASS, &restore);
    assert_trees_equal(&tree.path(), &find_file_root(&restore, "file1.txt"));
}

#[test]
fn t02_incremental_unchanged_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(&h, &tree, &repo, vec![], RetentionConfig::default());
    let c1 = restic_snapshot_count(&repo, PASS);
    run_restic(&h, &tree, &repo, vec![], RetentionConfig::default());
    assert_eq!(restic_snapshot_count(&repo, PASS), c1 + 1);
}

#[test]
fn t03_add_files_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(&h, &tree, &repo, vec![], RetentionConfig::default());
    tree.write_relative("d/new.txt", b"new");
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_restic(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore = h.dest_dir("restore");
    restic_restore_latest(&repo, PASS, &restore);
    assert_file_present(&find_file_root(&restore, "d/new.txt"), "d/new.txt");
}

#[test]
fn t10_exclude_simple_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(
        &h,
        &tree,
        &repo,
        vec!["**/skip/skip.txt".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    restic_restore_latest(&repo, PASS, &restore);
    assert_file_missing(&find_file_root(&restore, "file1.txt"), "skip/skip.txt");
}

#[test]
fn t11_exclude_wildcard_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(
        &h,
        &tree,
        &repo,
        vec!["*.tmp".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    restic_restore_latest(&repo, PASS, &restore);
    assert_file_missing(&find_file_root(&restore, "file1.txt"), "nested/file.tmp");
}

#[test]
fn t12_exclude_glob_path_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_restic(
        &h,
        &tree,
        &repo,
        vec!["nested/*.log".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    restic_restore_latest(&repo, PASS, &restore);
    let root = find_file_root(&restore, "file1.txt");
    assert_file_missing(&root, "nested/deep.log");
    assert_file_present(&root, "nested/file.tmp");
}

#[test]
fn t22_restic_forget_and_prune() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    let retention = RetentionConfig {
        keep_hourly: 2,
        ..Default::default()
    };
    for i in 0..5 {
        run_restic(&h, &tree, &repo, vec![], retention.clone());
        if i < 4 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    let count = restic_snapshot_count(&repo, PASS);
    assert!(count <= 2, "forget+prune should keep at most 2 snapshots, got {count}");
}

#[test]
fn t23_multi_origin_restic() {
    if !skip_without_tool("restic") {
        return;
    }
    let h = Harness::new();
    let host1 = h.dest_dir("host1");
    let host2 = h.dest_dir("host2");
    fs::create_dir_all(host1.join("etc")).unwrap();
    fs::create_dir_all(host2.join("etc")).unwrap();
    fs::write(host1.join("etc/hosts"), "host1").unwrap();
    fs::write(host2.join("etc/hosts"), "host2").unwrap();
    let repo = h.dest_dir("repo");
    let job = FileBackupJob {
        comment: "multi".into(),
        logdir: None,
        src: vec![
            format!("{}/", host1.display()),
            format!("{}/", host2.display()),
        ],
        dest: vec![repo.to_string_lossy().to_string()],
        exclude: vec![],
        password: Some(PASS.into()),
        destmount: vec![],
        srcmount: vec![],
        retention: RetentionConfig::default(),
        healthcheck: None,
    };
    h.run_job(BackupType::Restic, job);
    assert_eq!(restic_snapshot_count(&repo, PASS), 2);
}
