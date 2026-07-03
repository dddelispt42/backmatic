#![cfg(feature = "integration-tests")]

mod common;

use std::fs;

use backmatic::config::types::{BackupType, RetentionConfig};

use common::{
    assert_file_missing, assert_file_present, assert_trees_equal, file_job, rsync_snapshots,
    skip_without_tool, TestTree, Harness,
};

fn run_rsync(h: &Harness, tree: &TestTree, dest: &std::path::Path, exclude: Vec<String>, retention: RetentionConfig) {
    let job = file_job(
        "integration-rsync",
        vec![format!("{}/", tree.path().display())],
        vec![format!("{}/", dest.display())],
        exclude,
        retention,
    );
    h.run_job(BackupType::Rsync, job);
}

// T1 — initial backup
#[test]
fn t01_initial_backup_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    assert_trees_equal(&tree.path(), &dest);
}

// T2 — incremental, no changes
#[test]
fn t02_incremental_unchanged_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    let after_first = common::checksums(&dest);
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    assert_eq!(after_first, common::checksums(&dest));
}

// T3 — add files
#[test]
fn t03_add_files_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    tree.write_relative("d/new.txt", b"new file");
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    assert_file_present(&dest, "d/new.txt");
}

// T4 — modify files
#[test]
fn t04_modify_files_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    tree.write_relative("file1.txt", b"version-two-longer");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    run_rsync(&h, &tree, &dest, vec![], RetentionConfig::default());
    let content = fs::read_to_string(dest.join("file1.txt")).unwrap();
    assert_eq!(content, "version-two-longer");
}

// T5 — remove files
#[test]
fn t05_remove_files_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    let retention = RetentionConfig {
        keep_hourly: 2,
        ..Default::default()
    };
    run_rsync(&h, &tree, &dest, vec![], retention.clone());
    tree.remove_relative("dir");
    run_rsync(&h, &tree, &dest, vec![], retention);
    assert_file_missing(&dest, "dir/file2.bin");
    let snaps = rsync_snapshots(&dest);
    if let Some(snap) = snaps.first() {
        assert_file_present(snap, "dir/file2.bin");
    }
}

// T6 — diff between backups
#[test]
fn t06_diff_snapshots_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    let retention = RetentionConfig {
        keep_hourly: 3,
        ..Default::default()
    };
    run_rsync(&h, &tree, &dest, vec![], retention.clone());
    tree.write_relative("file1.txt", b"changed");
    run_rsync(&h, &tree, &dest, vec![], retention);
    let snaps = rsync_snapshots(&dest);
    assert!(!snaps.is_empty());
    let live = common::checksums(&dest);
    if let Some(prev) = snaps.last() {
        let prev_cs = common::checksums(prev);
        assert_ne!(live, prev_cs);
    }
}

// T7 — restore from snapshot
#[test]
fn t07_restore_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    let retention = RetentionConfig {
        keep_hourly: 2,
        ..Default::default()
    };
    run_rsync(&h, &tree, &dest, vec![], retention.clone());
    tree.remove_relative("dir");
    run_rsync(&h, &tree, &dest, vec![], retention);
    let snaps = rsync_snapshots(&dest);
    let snap = snaps.first().expect("snapshot");
    let restore = h.dest_dir("restored");
    copy_dir_recursive(snap, &restore);
    assert_file_present(&restore, "dir/file2.bin");
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    if !dst.exists() {
        fs::create_dir_all(dst).unwrap();
    }
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

// T10 — exclude simple
#[test]
fn t10_exclude_simple_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(
        &h,
        &tree,
        &dest,
        vec!["skip/skip.txt".into()],
        RetentionConfig::default(),
    );
    assert_file_missing(&dest, "skip/skip.txt");
}

// T11 — exclude wildcard
#[test]
fn t11_exclude_wildcard_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(
        &h,
        &tree,
        &dest,
        vec!["*.tmp".into()],
        RetentionConfig::default(),
    );
    assert_file_missing(&dest, "nested/file.tmp");
}

// T12 — exclude glob path
#[test]
fn t12_exclude_glob_path_rsync() {
    if !skip_without_tool("rsync") {
        return;
    }
    let h = Harness::new();
    let tree = TestTree::basic();
    let dest = h.dest_dir("dest");
    run_rsync(
        &h,
        &tree,
        &dest,
        vec!["nested/*.log".into()],
        RetentionConfig::default(),
    );
    assert_file_missing(&dest, "nested/deep.log");
    assert_file_present(&dest, "nested/file.tmp");
}
