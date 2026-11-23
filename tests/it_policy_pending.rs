//! Integration tests for `policy::PendingPolicy` and `LadonPolicy::check_root`.
//! These tests assume `LADON_GLOBAL_IGNORE` is unset.
use ladon_explorer::policy::{LadonPolicy, PendingPolicy};
use std::{fs, path::PathBuf};

fn workspace() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

#[test]
fn tc514_check_root_reads_only_the_root_rules() {
    let (_tmp, root) = workspace();
    fs::write(root.join(".LADONignore"), "*.pem\n").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/.LADONignore"), "[z-a]\n").unwrap();
    assert!(LadonPolicy::check_root(&root).is_ok(), "the invalid nested file must not be read");
    // The full load does read it, so the nested file is still an error for the scan.
    assert!(LadonPolicy::load(&root).is_err());
}

#[test]
fn tc515_check_root_rejects_invalid_root_rules() {
    let (_tmp, root) = workspace();
    fs::write(root.join(".LADONignore"), "[z-a]\n").unwrap();
    assert!(LadonPolicy::check_root(&root).is_err());
}

#[test]
fn tc516_pending_policy_applies_nested_rules_workspace_wide() {
    let (_tmp, root) = workspace();
    fs::write(root.join(".LADONignore"), "*.pem\n").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/.LADONignore"), "*.log\n").unwrap();
    let pending = PendingPolicy::start(&root);
    let top_log = root.join("top.log");
    let sub_pem = root.join("sub/a.pem");
    let top_log_excluded = pending.with(|policy| policy.excluded(&top_log)).unwrap();
    let sub_pem_excluded = pending.with(|policy| policy.excluded(&sub_pem)).unwrap();
    assert!(top_log_excluded, "a rule in sub/.LADONignore matches top.log because rules are workspace-wide");
    assert!(sub_pem_excluded);
}

#[test]
fn tc517_pending_policy_keeps_scan_error() {
    let (_tmp, root) = workspace();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/.LADONignore"), "[z-a]\n").unwrap();
    let pending = PendingPolicy::start(&root);
    let first = pending.with(|_| Ok(())).unwrap_err().to_string();
    let second = pending.with(|_| Ok(())).unwrap_err().to_string();
    assert_eq!(first, second);
    assert!(!first.is_empty());
}
