//! Integration tests for `engine::Engine` over a real workspace directory.
use ladon_explorer::engine::Engine;
use std::{fs, path::Path};

/// Fixed 32-byte virtual test key (64 hex digits). Never a documented example key.
const TEST_KEY: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

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

    let plan = e.restore_plan(&version_id).await.unwrap();
    assert!(plan.contains("-> docs/a.md\n"), "{plan}");
    assert!(plan.contains("Target exists: true"), "{plan}");

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

    fs::write(tmp.path().join(".ladon").join("probe.txt"), "x").unwrap();
    let err = e.resolve(Path::new(".ladon/probe.txt")).unwrap_err();
    assert!(err.to_string().contains("internal file cannot be tracked"), "{err}");

    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("x.txt"), "x").unwrap();
    let err = e.resolve(&outside.path().join("x.txt")).unwrap_err();
    assert!(err.to_string().contains("escapes the workspace"), "{err}");
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
async fn tc314_archive_blocks_hash_shared_with_sensitive_file() {
    // The block happens before any key lookup, so no archive key is needed here.
    let (tmp, e) = workspace().await;
    let remote = tempfile::tempdir().unwrap();
    let provider = ladon_explorer::provider::FilesystemProvider::new("nas", remote.path().to_path_buf());
    fs::write(tmp.path().join("secret.pem"), "same bytes").unwrap();
    fs::write(tmp.path().join("x.md"), "same bytes").unwrap();
    assert!(e.capture(&tmp.path().join("secret.pem"), "human", "init").await.unwrap().sensitive);
    let plain = e.capture(&tmp.path().join("x.md"), "human", "init").await.unwrap();
    assert!(!plain.sensitive, "x.md itself is not covered by any ignore rule");

    let err = e.archive_version(plain.version_id.as_deref().unwrap(), &provider, false).await.unwrap_err();
    assert!(err.to_string().contains("sensitive version blocked from external storage"), "{err}");
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

#[tokio::test]
async fn tc_moved_file_keeps_sensitivity_after_rule() {
    // Set so that the archive reaches the sensitivity check rather than failing on a missing key.
    std::env::set_var("LADON_ARCHIVE_KEY_HEX", TEST_KEY);
    let (tmp, e) = workspace().await;
    let remote = tempfile::tempdir().unwrap();
    let provider = ladon_explorer::provider::FilesystemProvider::new("nas", remote.path().to_path_buf());

    fs::create_dir_all(tmp.path().join("restricted")).unwrap();
    let before = tmp.path().join("restricted").join("a.md");
    fs::write(&before, "# Restricted\n").unwrap();
    assert!(!e.capture(&before, "human", "before rule").await.unwrap().sensitive);

    let mut rules = fs::read_to_string(tmp.path().join(".LADONignore")).unwrap();
    rules.push_str("\n/restricted/\n");
    fs::write(tmp.path().join(".LADONignore"), rules).unwrap();
    e.reload_policy().unwrap();
    assert!(e.capture(&before, "human", "after rule").await.unwrap().sensitive, "the rule must cover the old path");
    e.scan("scanner").await.unwrap();

    fs::create_dir_all(tmp.path().join("plain")).unwrap();
    fs::rename(&before, tmp.path().join("plain").join("a.md")).unwrap();
    e.scan("scanner").await.unwrap();
    let moved = e.file_by_path("plain/a.md").await.unwrap();
    let version = e.store.latest_version(&moved.id).await.unwrap().unwrap();
    assert!(!version.sensitive, "the new path is not covered by any rule");

    let err = e.archive_version(&version.id, &provider, false).await.expect_err("a moved copy must not upload");
    assert!(err.to_string().contains("sensitive version blocked"), "{err}");
    assert!(fs::read_dir(remote.path()).unwrap().next().is_none(), "nothing may reach the remote");
}

#[tokio::test]
async fn tc315_open_defers_nested_rule_errors() {
    let (tmp, e) = workspace().await;
    let root = tmp.path().canonicalize().unwrap();
    fs::write(root.join("top.txt"), "top").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/.LADONignore"), "[z-a]\n").unwrap();
    drop(e);
    let reopened = Engine::open(&root).await.expect("open must not read the nested rule file");
    let first = reopened.capture(&root.join("top.txt"), "human", "gui-capture").await;
    assert!(first.is_err(), "a capture under a failed scan must fail closed");
    assert!(reopened.store.find_path("top.txt").await.unwrap().is_none(), "nothing may be recorded");
    fs::write(root.join("sub/.LADONignore"), "# fixed\n").unwrap();
    reopened.reload_policy().unwrap();
    let second = reopened.capture(&root.join("top.txt"), "human", "gui-capture").await.unwrap();
    assert_eq!(second.status, "captured");
}

#[tokio::test]
async fn tc316_open_rejects_invalid_root_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    drop(e);
    fs::write(tmp.path().join(".LADONignore"), "[z-a]\n").unwrap();
    assert!(Engine::open(tmp.path()).await.is_err());
}

#[tokio::test]
async fn tc318_relative_for_input_accepts_plain_absolute_path() {
    let (tmp, e) = workspace().await;
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    fs::write(tmp.path().join("docs").join("a.md"), "# A\n").unwrap();
    e.capture(&tmp.path().join("docs").join("a.md"), "human", "init").await.unwrap();

    let rel = e.relative_for_input(&tmp.path().join("docs/a.md")).unwrap();
    assert_eq!(rel, "docs/a.md");
}

#[tokio::test]
async fn tc319_copy_of_excluded_content_is_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join(".LADONignore"), "/contracts/\n").unwrap();
    Engine::init(tmp.path()).await.unwrap();
    let e = Engine::open(tmp.path()).await.unwrap();

    fs::create_dir_all(tmp.path().join("contracts")).unwrap();
    let original = tmp.path().join("contracts").join("a.txt");
    fs::write(&original, "contract terms v1").unwrap();
    e.capture(&original, "human", "init").await.unwrap();

    fs::create_dir_all(tmp.path().join("shared")).unwrap();
    let copy = tmp.path().join("shared").join("a.txt");
    fs::copy(&original, &copy).unwrap();
    e.capture(&copy, "human", "copy").await.unwrap();
    let copy_hash = e.file_by_path("shared/a.txt").await.unwrap().current_hash;
    assert!(
        e.content_is_excluded(&copy_hash).await.unwrap(),
        "a copy of excluded content at shared/a.txt must count as excluded"
    );

    fs::write(tmp.path().join("notes.md"), "meeting notes").unwrap();
    e.capture(&tmp.path().join("notes.md"), "human", "init").await.unwrap();
    let notes_hash = e.file_by_path("notes.md").await.unwrap().current_hash;
    assert!(
        !e.content_is_excluded(&notes_hash).await.unwrap(),
        "unrelated notes.md must not count as excluded"
    );
}

#[tokio::test]
async fn tc326_missing_root_rule_file_refuses_capture_and_archive() {
    let (tmp, e) = workspace().await;
    let notes = tmp.path().join("notes.md");
    fs::write(&notes, "# Notes\n").unwrap();
    let recorded = e.capture(&notes, "human", "before delete").await.unwrap();
    let version_id = recorded.version_id.clone().unwrap();
    drop(e);

    let e = Engine::open(tmp.path()).await.unwrap();
    fs::remove_file(tmp.path().join(".LADONignore")).unwrap();

    let err = e.reload_policy().expect_err("a missing root rule file must fail the reload");
    assert!(err.to_string().contains(".LADONignore"), "{err}");

    let draft = tmp.path().join("draft.md");
    fs::write(&draft, "# Draft\n").unwrap();
    let err = e
        .capture(&draft, "human", "after delete")
        .await
        .expect_err("capture must refuse rather than record the file as non-sensitive");
    assert!(err.to_string().contains(".LADONignore"), "{err}");
    assert!(e.store.find_path("draft.md").await.unwrap().is_none(), "nothing may be recorded");

    let remote = tempfile::tempdir().unwrap();
    let provider = ladon_explorer::provider::FilesystemProvider::new("nas", remote.path().to_path_buf());
    let err = e
        .archive_version(&version_id, &provider, false)
        .await
        .expect_err("archive must refuse without a root rule file");
    assert!(err.to_string().contains("policy reload failed"), "{err}");
    assert!(fs::read_dir(remote.path()).unwrap().next().is_none(), "nothing may reach the remote");
}

#[tokio::test]
async fn tc327_relative_for_input_accepts_deleted_tracked_file() {
    let (tmp, e) = workspace().await;
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    let file = tmp.path().join("docs").join("old.md");
    fs::write(&file, "# Old\n").unwrap();
    e.capture(&file, "human", "init").await.unwrap();
    fs::remove_file(&file).unwrap();

    let rel = e.relative_for_input(&tmp.path().join("docs/old.md")).unwrap();
    assert_eq!(rel, "docs/old.md");
}

#[tokio::test]
#[cfg_attr(not(windows), ignore = "needs a Windows share_mode(0) handle to lock one file")]
async fn tc328_scan_continues_after_one_file_fails() {
    let (tmp, e) = workspace().await;
    let dir = tmp.path().join("notes");
    fs::create_dir_all(&dir).unwrap();
    let locked = dir.join("file-10-locked.md");
    fs::write(&locked, "# Locked\n").unwrap();
    let mut readable = Vec::new();
    for i in 0..20 {
        let name = format!("file-{i:02}.md");
        fs::write(dir.join(&name), format!("# {i}\n")).unwrap();
        readable.push(format!("notes/{name}"));
    }

    let _hold = hold_unshared(&locked);
    assert!(fs::File::open(&locked).is_err(), "the locking handle must block other opens");
    let err = e.scan("scanner").await.err().expect("a file that cannot be read must make the scan report an error");

    let mut captured: Vec<String> = e.store.list_files().await.unwrap().into_iter().map(|f| f.relative_path).collect();
    captured.sort();
    assert_eq!(captured, readable, "every readable file must be captured although one file failed");
    assert!(err.to_string().contains("notes/file-10-locked.md"), "{err}");
}

/// Opens `path` with no sharing, so every other open of it fails until the returned handle is dropped.
#[cfg(windows)]
fn hold_unshared(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap()
}

#[cfg(not(windows))]
fn hold_unshared(_path: &Path) -> fs::File {
    unreachable!("tc328 is ignored off Windows")
}

#[cfg(windows)]
fn make_dir_junction(target: &Path, link: &Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
#[tokio::test]
async fn tc1043_restore_refuses_a_target_behind_a_junction() {
    let (tmp, e) = workspace().await;
    let root = tmp.path();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    let file = root.join("docs").join("a.md");
    fs::write(&file, "tracked text").unwrap();
    let captured = e.capture(&file, "human", "tc1043").await.unwrap();
    let version_id = captured.version_id.unwrap();

    fs::remove_dir_all(root.join("docs")).unwrap();
    if !make_dir_junction(outside.path(), &root.join("docs")) {
        assert!(
            std::env::var_os("LADON_REQUIRE_LINK_TESTS").is_none(),
            "directory junctions are required but unavailable on this host"
        );
        eprintln!("directory junctions cannot be created here; skipping the case");
        return;
    }
    let err = e.restore(&version_id, true, None).await.unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("link") || text.contains("escapes the workspace"), "{text}");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0, "nothing may be written outside the workspace");
}