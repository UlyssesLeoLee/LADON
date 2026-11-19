//! Integration tests for `engine::Engine` over a real workspace directory.
use ladon_explorer::engine::Engine;
use std::{fs, path::Path};

async fn workspace() -> (tempfile::TempDir, Engine) {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::init(tmp.path()).await.unwrap();
    (tmp, engine)
}

#[tokio::test]
async fn tc301_restore_plan_describes_target() {
    let (tmp, e) = workspace().await;
    fs::write(tmp.path().join("req.md"), "# R1\none\n").unwrap();
    let cap = e.capture(&tmp.path().join("req.md"), "human", "init").await.unwrap();
    let version_id = cap.version_id.clone().unwrap();
    let version = e.store.version(&version_id).await.unwrap().unwrap();

    let plan = e.restore_plan(&version_id).await.unwrap();
    let expected = format!("RESTORE PLAN: {} version 1 ({}) -> req.md\n", cap.file_id, version.content_hash);
    assert!(plan.starts_with(&expected), "{plan}");
    assert!(plan.contains("Target exists: true"), "{plan}");
    assert!(plan.contains("require --approve"), "{plan}");
}

#[tokio::test]
async fn tc302_restore_plan_unknown_version() {
    let (_tmp, e) = workspace().await;
    let err = e.restore_plan("no-such-version").await.unwrap_err();
    assert!(err.to_string().contains("unknown version"), "{err}");
}

#[tokio::test]
async fn tc303_restore_plan_nested_path_and_missing_target() {
    let (tmp, e) = workspace().await;
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    let file = tmp.path().join("docs").join("a.md");
    fs::write(&file, "# A\n").unwrap();
    let cap = e.capture(&file, "human", "init").await.unwrap();
    let version_id = cap.version_id.unwrap();

    fs::remove_file(&file).unwrap();
    let plan = e.restore_plan(&version_id).await.unwrap();
    assert!(plan.contains("-> docs/a.md\n"), "{plan}");
    assert!(plan.contains("Target exists: false"), "{plan}");
}

#[tokio::test]
async fn tc304_restore_refuses_diverged_live_file() {
    let (tmp, e) = workspace().await;
    let file = tmp.path().join("x.txt");
    fs::write(&file, "one").unwrap();
    let first = e.capture(&file, "human", "v1").await.unwrap();
    fs::write(&file, "two").unwrap();
    e.capture(&file, "human", "v2").await.unwrap();
    fs::write(&file, "three").unwrap();

    let err = e.restore(first.version_id.as_deref().unwrap(), true, None).await.unwrap_err();
    assert!(err.to_string().contains("live file changed outside ledger"), "{err}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "three");
}

#[tokio::test]
async fn tc305_restore_recreates_missing_file() {
    let (tmp, e) = workspace().await;
    let file = tmp.path().join("x.txt");
    fs::write(&file, "original").unwrap();
    let first = e.capture(&file, "human", "v1").await.unwrap();
    fs::remove_file(&file).unwrap();
    e.scan("scanner").await.unwrap();

    let restored = e.restore(first.version_id.as_deref().unwrap(), true, None).await.unwrap();
    assert_eq!(restored.status, "captured");
    assert_eq!(fs::read_to_string(&file).unwrap(), "original");
    let events = e.store.events(&first.file_id).await.unwrap();
    let kinds: Vec<&str> = events.iter().map(|ev| ev.kind.as_str()).collect();
    assert_eq!(kinds, vec!["created", "deleted", "restored"]);
    assert!(e.store.verify_chain(&first.file_id).await.unwrap());
}

#[tokio::test]
async fn tc306_resolve_rejects_internal_and_outside_paths() {
    let (tmp, e) = workspace().await;
    fs::create_dir_all(tmp.path().join(".git")).unwrap();
    fs::write(tmp.path().join(".git").join("config"), "[core]\n").unwrap();
    let err = e.resolve(Path::new(".git/config")).unwrap_err();
    assert!(err.to_string().contains("internal file cannot be tracked"), "{err}");

    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("x.txt"), "x").unwrap();
    assert!(e.resolve(&outside.path().join("x.txt")).is_err());
}

#[tokio::test]
async fn tc307_open_requires_initialized_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let err = Engine::open(tmp.path()).await.err().unwrap();
    assert!(err.to_string().contains("not initialized"), "{err}");

    let e = Engine::init(tmp.path()).await.unwrap();
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    fs::write(tmp.path().join("docs").join("a.md"), "# A\n").unwrap();
    e.capture(&tmp.path().join("docs").join("a.md"), "human", "init").await.unwrap();

    let reopened = Engine::open(tmp.path()).await.unwrap();
    let record = reopened.file_by_path("docs\\a.md").await.unwrap();
    assert_eq!(record.relative_path, "docs/a.md");
    let err = reopened.file_by_path("missing.md").await.unwrap_err();
    assert!(err.to_string().contains("file not found in ledger"), "{err}");
}

#[tokio::test]
async fn tc308_reload_policy_applies_new_rules() {
    let (tmp, e) = workspace().await;
    let file = tmp.path().join("a.md");
    fs::write(&file, "# A\n").unwrap();
    assert!(!e.capture(&file, "human", "first").await.unwrap().sensitive);

    let mut rules = fs::read_to_string(tmp.path().join(".LADONignore")).unwrap();
    rules.push_str("\n/a.md\n");
    fs::write(tmp.path().join(".LADONignore"), rules).unwrap();
    assert!(!e.capture(&file, "human", "before reload").await.unwrap().sensitive);

    e.reload_policy().unwrap();
    let after = e.capture(&file, "human", "after reload").await.unwrap();
    assert!(after.sensitive);
    assert_eq!(after.status, "unchanged");
}

#[tokio::test]
async fn tc309_scan_skips_internal_directories() {
    let (tmp, e) = workspace().await;
    fs::create_dir_all(tmp.path().join(".git")).unwrap();
    fs::write(tmp.path().join(".git").join("config"), "[core]\n").unwrap();
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    fs::write(tmp.path().join("docs").join("x.md"), "# X\n").unwrap();
    fs::write(tmp.path().join("y.md"), "# Y\n").unwrap();

    let results = e.scan("scanner").await.unwrap();
    assert_eq!(results.len(), 2);
    let mut paths: Vec<String> = e.store.list_files().await.unwrap().into_iter().map(|f| f.relative_path).collect();
    paths.sort();
    assert_eq!(paths, vec!["docs/x.md".to_string(), "y.md".to_string()]);
}

#[tokio::test]
async fn tc310_capture_large_file_defers_structured_diff() {
    let (tmp, e) = workspace().await;
    let file = tmp.path().join("big.bin");
    fs::write(&file, vec![b'a'; 9 * 1024 * 1024]).unwrap();
    let cap = e.capture(&file, "human", "big").await.unwrap();
    assert_eq!(cap.status, "captured");
    assert_eq!(cap.changed_sections, vec!["LARGE_BINARY_OR_TEXT".to_string()]);
}
