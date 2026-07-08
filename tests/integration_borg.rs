#![cfg(feature = "integration-tests")]

mod common;

use std::fs;

use backmatic::config::types::{BackupType, FileBackupJob, RetentionConfig};

use common::{
    assert_file_missing, assert_file_present, assert_trees_equal, borg_extract_latest,
    borg_list_archives, file_job, find_file_root, skip_without_tool, TestTree, Harness,
};

const PASS: &str = "test-borg-pass";

fn run_borg(h: &Harness, tree: &TestTree, repo: &std::path::Path, exclude: Vec<String>, retention: RetentionConfig) {
    let job = FileBackupJob {
        password: Some(PASS.into()),
        ..file_job(
            "integration-borg",
            vec![format!("{}/", tree.path().display())],
            vec![repo.to_string_lossy().to_string()],
            exclude,
            retention,
        )
    };
    h.run_job(BackupType::Borg, job);
}
#[test]
fn t01_initial_backup_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    assert_trees_equal(&tree.path(), &find_file_root(&restore, "file1.txt"));
}

#[test]
fn t02_incremental_unchanged_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let count1 = borg_list_archives(&repo, Some(PASS)).len();
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let count2 = borg_list_archives(&repo, Some(PASS)).len();
    assert_eq!(count2, count1 + 1);
}

#[test]
fn t03_add_files_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    tree.write_relative("d/new.txt", b"new");
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    let root = find_file_root(&restore, "d/new.txt");
    assert_file_present(&root, "d/new.txt");
}

#[test]
fn t04_modify_files_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let archives_before = borg_list_archives(&repo, Some(PASS));
    tree.write_relative("file1.txt", b"v2");
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    let root = find_file_root(&restore, "file1.txt");
    assert_eq!(fs::read_to_string(root.join("file1.txt")).unwrap(), "v2");
    let archives_after = borg_list_archives(&repo, Some(PASS));
    assert!(archives_after.len() > archives_before.len());
}

#[test]
fn t05_remove_files_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let old_archive = borg_list_archives(&repo, Some(PASS))
        .last()
        .cloned()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    tree.remove_relative("dir");
    run_borg(&h, &tree, &repo, vec![], RetentionConfig::default());
    let restore_latest = h.dest_dir("restore_latest");
    borg_extract_latest(&repo, Some(PASS), &restore_latest);
    let latest_root = find_file_root(&restore_latest, "file1.txt");
    assert_file_missing(&latest_root, "dir/file2.bin");
    let restore_old = h.dest_dir("restore_old");
    extract_borg_archive(&repo, &old_archive, &restore_old);
    let old_root = find_file_root(&restore_old, "dir/file2.bin");
    assert_file_present(&old_root, "dir/file2.bin");
}

fn extract_borg_archive(repo: &std::path::Path, archive: &str, target: &std::path::Path) {
    fs::create_dir_all(target).unwrap();
    let spec = format!("{}::{}", repo.display(), archive);
    let out = std::process::Command::new("borg")
        .arg("extract")
        .arg(spec)
        .current_dir(target)
        .env("BORG_PASSPHRASE", PASS)
        .env("BORG_RELOCATED_REPO_ACCESS_IS_OK", "yes")
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn t09_pruning_borg() {
    if !skip_without_tool("borg") {
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
        run_borg(&h, &tree, &repo, vec![], retention.clone());
        if i < 4 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    let count = borg_list_archives(&repo, Some(PASS)).len();
    assert!(count <= 2, "expected prune to keep at most 2 archives, got {count}");
}

#[test]
fn t10_exclude_simple_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(
        &h,
        &tree,
        &repo,
        vec!["**/skip/skip.txt".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    let root = find_file_root(&restore, "file1.txt");
    assert_file_missing(&root, "skip/skip.txt");
}

#[test]
fn t11_exclude_wildcard_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(
        &h,
        &tree,
        &repo,
        vec!["*.tmp".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    let root = find_file_root(&restore, "file1.txt");
    assert_file_missing(&root, "nested/file.tmp");
}

#[test]
fn t12_exclude_glob_path_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    run_borg(
        &h,
        &tree,
        &repo,
        vec!["nested/*.log".into()],
        RetentionConfig::default(),
    );
    let restore = h.dest_dir("restore");
    borg_extract_latest(&repo, Some(PASS), &restore);
    assert_file_missing(&restore, "nested/deep.log");
}

#[test]
fn t23_multi_origin_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    let h = Harness::new();
    let host1 = h.dest_dir("host1");
    let host2 = h.dest_dir("host2");
    fs::create_dir_all(host1.join("etc")).unwrap();
    fs::create_dir_all(host2.join("etc")).unwrap();
    fs::write(host1.join("etc/hosts"), "host1-content").unwrap();
    fs::write(host2.join("etc/hosts"), "host2-content").unwrap();
    let repo = h.dest_dir("repo");
    let job = FileBackupJob {
        comment: "multi-origin".into(),
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
        verify: Default::default(),
    };
    h.run_job(BackupType::Borg, job);
    let archives = borg_list_archives(&repo, Some(PASS));
    assert_eq!(archives.len(), 2, "expected one archive per origin");
    assert_ne!(archives[0], archives[1]);
}

#[test]
fn t25_restore_verify_borg() {
    if !skip_without_tool("borg") {
        return;
    }
    use backmatic::config::types::VerifyConfig;
    let h = Harness::new();
    let tree = TestTree::basic();
    let repo = h.dest_dir("repo");
    let job = FileBackupJob {
        password: Some(PASS.into()),
        verify: VerifyConfig {
            enabled: true,
            samples: 3,
            max_file_size: None,
        },
        ..file_job(
            "verify-borg",
            vec![format!("{}/", tree.path().display())],
            vec![repo.to_string_lossy().to_string()],
            vec![],
            RetentionConfig::default(),
        )
    };
    // A clean backup+verify round-trip must succeed (restored samples match originals).
    h.run_job(BackupType::Borg, job);
}
