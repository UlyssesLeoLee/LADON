//! Kernel-lane integration tests for the command handler (`ladon_ui_prototype::app`).
//!
//! Each test runs against a real temporary workspace and the real kernel. Workspace
//! paths are canonical, so no reparse point lies above a test workspace.

use std::fs;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use ladon_explorer::archive::CasStore;
use ladon_explorer::store::Store;
use ladon_ui_prototype::app::{self, Command, DiffBlock, FailKind, Reply, State, Worker};
use ladon_ui_prototype::model::{self, LineKind};

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).expect("canonical path")
}

/// Path text passed to the handler: the drive form without the verbatim prefix.
fn arg(path: &Path) -> String {
    model::strip_extended_prefix(&path.to_string_lossy()).to_string()
}

/// Display form of a path after canonicalisation, as replies report it.
fn display_of(path: &Path) -> String {
    arg(&canonical(path))
}

fn expect_failed(reply: Reply) -> (FailKind, String) {
    match reply {
        Reply::Failed { kind, message, detail } => {
            assert!(detail.is_none(), "failure detail must stay empty");
            (kind, message)
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn assert_failed(reply: Reply, kind: FailKind, message: &str) {
    let (actual_kind, actual_message) = expect_failed(reply);
    assert_eq!(actual_kind, kind);
    assert_eq!(actual_message, message);
}

fn folder(reply: Reply) -> (String, Vec<model::BrowseRow>, bool) {
    match reply {
        Reply::Folder { relative_dir, rows, truncated } => (relative_dir, rows, truncated),
        other => panic!("expected Folder, got {other:?}"),
    }
}

fn history(reply: Reply) -> (Vec<model::VersionRow>, bool, usize) {
    match reply {
        Reply::History { versions, chain_valid, event_count, .. } => {
            (versions, chain_valid, event_count)
        }
        other => panic!("expected History, got {other:?}"),
    }
}

fn row<'a>(rows: &'a [model::BrowseRow], name: &str) -> &'a model::BrowseRow {
    rows.iter()
        .find(|candidate| candidate.name == name)
        .unwrap_or_else(|| panic!("no row named {name}"))
}

async fn run_init(state: &mut State, dir: &Path) -> Reply {
    app::handle(state, Command::Init { path: arg(dir), confirmed: true }).await
}

async fn run_open(state: &mut State, dir: &Path) -> Reply {
    app::handle(state, Command::Open { path: arg(dir) }).await
}

async fn run_list(state: &mut State, relative_dir: &str) -> Reply {
    app::handle(state, Command::ListFolder { relative_dir: relative_dir.to_string() }).await
}

async fn run_capture(state: &mut State, relative_path: &str) -> Reply {
    app::handle(state, Command::Capture { relative_path: relative_path.to_string() }).await
}

async fn run_history(state: &mut State, relative_path: &str) -> Reply {
    app::handle(state, Command::LoadHistory { relative_path: relative_path.to_string() }).await
}

async fn run_diff(state: &mut State, older: &str, newer: &str) -> Reply {
    app::handle(
        state,
        Command::LoadDiff {
            older_version_id: older.to_string(),
            newer_version_id: newer.to_string(),
        },
    )
    .await
}

/// Version identifiers of a file, newest first.
async fn version_ids(state: &mut State, relative_path: &str) -> Vec<String> {
    let (versions, _, _) = history(run_history(state, relative_path).await);
    versions.into_iter().map(|version| version.version_id).collect()
}

#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(arg(link))
        .arg(arg(target))
        .status()
        .expect("start cmd");
    assert!(status.success(), "mklink /J failed for {}", arg(link));
}

/// Sorted names of the entries in a folder.
#[cfg(windows)]
fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Creates a file symbolic link with `mklink`, which needs SeCreateSymbolicLinkPrivilege.
#[cfg(windows)]
fn make_file_symlink(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg(arg(link))
        .arg(arg(target))
        .status()
        .expect("start cmd");
    assert!(
        status.success(),
        "mklink failed for {}: a file symbolic link needs SeCreateSymbolicLinkPrivilege (Developer Mode or an elevated shell)",
        arg(link)
    );
}

fn set_mtime(path: &Path, when: SystemTime) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[tokio::test]
async fn it_620_init_open_and_needs_init() {
    let ws_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let root = canonical(ws_dir.path());
    let second = canonical(second_dir.path());
    let mut state = State::new();

    match run_init(&mut state, &root).await {
        Reply::Opened { root: shown } => {
            assert_eq!(shown, display_of(&root));
            assert!(!shown.starts_with(r"\\?\"));
        }
        other => panic!("expected Opened, got {other:?}"),
    }
    assert!(root.join(".ladon").join("meta.turso").exists());
    assert!(root.join(".LADONignore").exists());

    match run_open(&mut state, &second).await {
        Reply::NeedsInit { path } => assert_eq!(path, display_of(&second)),
        other => panic!("expected NeedsInit, got {other:?}"),
    }
    assert!(!second.join(".ladon").exists());

    // The first workspace stays open after NeedsInit.
    assert!(matches!(run_list(&mut state, "").await, Reply::Folder { .. }));
}

#[tokio::test]
async fn it_621_init_requires_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical(dir.path());
    let mut state = State::new();

    let refused = app::handle(
        &mut state,
        Command::Init { path: arg(&root), confirmed: false },
    )
    .await;
    assert_failed(refused, FailKind::NotConfirmed, app::MSG_NOT_CONFIRMED);
    assert!(!root.join(".ladon").exists());
    assert!(!root.join(".LADONignore").exists());

    fs::write(root.join("notes.md"), "keep\n").unwrap();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 1, changed: true, .. }
    ));

    // Init on an initialised folder behaves as Open and keeps the recorded history.
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    let (versions, _, _) = history(run_history(&mut state, "notes.md").await);
    assert_eq!(versions.len(), 1);
}

#[tokio::test]
async fn it_622_commands_require_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let root = canonical(dir.path());
    fs::write(root.join("notes.md"), "text\n").unwrap();
    let mut state = State::new();

    assert_failed(
        run_list(&mut state, "").await,
        FailKind::NoWorkspace,
        app::MSG_NO_WORKSPACE,
    );
    assert_failed(
        run_capture(&mut state, "notes.md").await,
        FailKind::NoWorkspace,
        app::MSG_NO_WORKSPACE,
    );
    assert_failed(
        run_history(&mut state, "notes.md").await,
        FailKind::NoWorkspace,
        app::MSG_NO_WORKSPACE,
    );
    assert_failed(
        run_diff(&mut state, "a", "b").await,
        FailKind::NoWorkspace,
        app::MSG_NO_WORKSPACE,
    );
    assert!(!root.join(".ladon").exists());
    assert!(!root.join(".LADONignore").exists());
}

#[tokio::test]
async fn it_623_root_candidate_refusals() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("notes.md"), "text\n").unwrap();
    fs::create_dir_all(root.join("sub")).unwrap();
    let mut state = State::new();
    let base_arg = arg(&root);

    let relative = Command::Open { path: "relative/dir".to_string() };
    assert_failed(
        app::handle(&mut state, relative).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    let with_nul = Command::Open { path: format!("{base_arg}\0x") };
    assert_failed(
        app::handle(&mut state, with_nul).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_open(&mut state, &root.join("missing")).await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_open(&mut state, &root.join("notes.md")).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    let dotdot = Command::Open { path: format!("{base_arg}\\sub\\..\\sub") };
    assert_failed(
        app::handle(&mut state, dotdot).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_open(&mut state, &root.join("CON")).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );

    // Internal directories are refused wherever they appear in a root candidate.
    assert_failed(
        run_open(&mut state, &root.join(".ladon")).await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    assert_failed(
        run_init(&mut state, &root.join(".git")).await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    let device = Command::Open { path: r"\\.\C:\ws".to_string() };
    assert_failed(
        app::handle(&mut state, device).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );

    // The user profile directory is refused. A drive root has no refusal of its own: it
    // takes the same owned-entry checks as any folder, so Open answers NeedsInit or
    // Opened. It uses a separate state, and Init is never called on a real drive root.
    let profile = std::env::var("USERPROFILE").expect("USERPROFILE is set");
    let profile_root = Command::Open { path: display_of(Path::new(&profile)) };
    assert_failed(
        app::handle(&mut state, profile_root).await,
        FailKind::Refused,
        app::MSG_ROOT_REFUSED,
    );
    #[cfg(windows)]
    {
        let drive_text = format!("{}\\", &base_arg[..2]);
        let mut drive_state = State::new();
        let drive_root = Command::Open { path: drive_text.clone() };
        match app::handle(&mut drive_state, drive_root).await {
            Reply::NeedsInit { path } => assert_eq!(path, drive_text),
            Reply::Opened { root } => assert_eq!(root, drive_text),
            other => panic!("expected NeedsInit or Opened, got {other:?}"),
        }
    }
    assert!(!root.join(".ladon").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn it_624_root_candidate_links_refused() {
    // Case A: a junction in an ancestor of the candidate is refused by Open and by Init.
    {
        let base = tempfile::tempdir().unwrap();
        let base_root = canonical(base.path());
        let real = base_root.join("real");
        fs::create_dir_all(real.join("ws")).unwrap();
        make_junction(&base_root.join("alias"), &real);
        let through_alias = base_root.join("alias").join("ws");
        let mut state = State::new();
        assert_failed(
            run_open(&mut state, &through_alias).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert_failed(
            run_init(&mut state, &through_alias).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert!(!real.join("ws").join(".ladon").exists());
    }

    // Case B: a directory junction named .ladon.
    {
        let base = tempfile::tempdir().unwrap();
        let base_root = canonical(base.path());
        let ws = base_root.join("ws");
        let outside = base_root.join("outside");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        make_junction(&ws.join(".ladon"), &outside);
        let mut state = State::new();
        assert_failed(
            run_init(&mut state, &ws).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert!(entry_names(&outside).is_empty());
    }

    // Case C: a directory junction named .ladon/cas under a real .ladon folder.
    {
        let base = tempfile::tempdir().unwrap();
        let base_root = canonical(base.path());
        let ws = base_root.join("ws");
        let outside = base_root.join("outside");
        fs::create_dir_all(ws.join(".ladon")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        make_junction(&ws.join(".ladon").join("cas"), &outside);
        let mut state = State::new();
        assert_failed(
            run_init(&mut state, &ws).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert!(entry_names(&outside).is_empty());
    }

    // Case D: a file symbolic link named .ladon/meta.turso under a real .ladon folder.
    {
        let base = tempfile::tempdir().unwrap();
        let base_root = canonical(base.path());
        let ws = base_root.join("ws");
        let outside = base_root.join("outside");
        let target = outside.join("target.txt");
        fs::create_dir_all(ws.join(".ladon")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(&target, "outside\n").unwrap();
        make_file_symlink(&ws.join(".ladon").join("meta.turso"), &target);
        let mut state = State::new();
        assert_failed(
            run_init(&mut state, &ws).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert_eq!(entry_names(&outside), ["target.txt"]);
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    }

    // Case E: a file symbolic link named .LADONignore.
    {
        let base = tempfile::tempdir().unwrap();
        let base_root = canonical(base.path());
        let ws = base_root.join("ws");
        let outside = base_root.join("outside");
        let target = outside.join("target.txt");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(&target, "outside\n").unwrap();
        make_file_symlink(&ws.join(".LADONignore"), &target);
        let mut state = State::new();
        assert_failed(
            run_init(&mut state, &ws).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert_eq!(entry_names(&outside), ["target.txt"]);
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
        assert!(!ws.join(".ladon").exists());
    }
}

#[tokio::test]
async fn it_625_capture_and_list_statuses() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("notes.md"), "first\n").unwrap();
    fs::write(root.join("untracked.txt"), "plain\n").unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs").join("a.md"), "doc\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    match run_capture(&mut state, "notes.md").await {
        Reply::Captured { relative_path, version_no, changed } => {
            assert_eq!(relative_path, "notes.md");
            assert_eq!(version_no, 1);
            assert!(changed);
        }
        other => panic!("expected Captured, got {other:?}"),
    }
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 1, changed: false, .. }
    ));
    match run_capture(&mut state, "docs\\a.md").await {
        Reply::Captured { relative_path, .. } => assert_eq!(relative_path, "docs/a.md"),
        other => panic!("expected Captured, got {other:?}"),
    }

    let (relative_dir, rows, truncated) = folder(run_list(&mut state, "").await);
    assert_eq!(relative_dir, "");
    assert!(!truncated);
    let notes = row(&rows, "notes.md");
    assert_eq!(notes.status_text, "已记录 v1");
    assert!(notes.tracked);
    assert_eq!(notes.version_no, Some(1));
    assert!(notes.file_id.is_some());
    let untracked = row(&rows, "untracked.txt");
    assert_eq!(untracked.status_text, "未记录");
    assert!(!untracked.tracked);
    assert!(untracked.file_id.is_none());
}

#[tokio::test]
async fn it_626_listing_order_and_omission() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::create_dir_all(root.join("Zdir")).unwrap();
    fs::create_dir_all(root.join("adir").join(".git")).unwrap();
    fs::create_dir_all(root.join("adir").join(".Ladon")).unwrap();
    fs::create_dir_all(root.join(".GIT")).unwrap();
    fs::write(root.join("adir").join("inner.txt"), "i\n").unwrap();
    fs::write(root.join("zeta.txt"), "z\n").unwrap();
    fs::write(root.join("Alpha.txt"), "a\n").unwrap();
    fs::write(root.join("beta.txt"), "b\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    // Directories first, then names ASCII-case-insensitively; .ladon and .git are omitted.
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    let names: Vec<&str> = rows.iter().map(|candidate| candidate.name.as_str()).collect();
    assert_eq!(
        names,
        ["adir", "Zdir", ".LADONignore", "Alpha.txt", "beta.txt", "zeta.txt"]
    );

    let (relative_dir, rows, _) = folder(run_list(&mut state, "adir/").await);
    assert_eq!(relative_dir, "adir");
    let names: Vec<&str> = rows.iter().map(|candidate| candidate.name.as_str()).collect();
    assert_eq!(names, ["inner.txt"]);
    assert_eq!(rows[0].relative_path, "adir/inner.txt");

    assert_failed(
        run_list(&mut state, "missing").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_list(&mut state, "zeta.txt").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
}

#[tokio::test]
async fn it_627_list_limit_truncates() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    for index in 0..=app::LIST_LIMIT {
        fs::write(root.join(format!("f{index:05}.txt")), "x").unwrap();
    }
    // The root holds the rule file and LIST_LIMIT + 1 files: one row more than the limit.
    let (_, rows, truncated) = folder(run_list(&mut state, "").await);
    assert!(truncated);
    assert_eq!(rows.len(), app::LIST_LIMIT);
}

#[tokio::test]
async fn it_628_history_newest_first_chain_valid() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    fs::write(root.join("notes.md"), "one\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 1, changed: true, .. }
    ));
    fs::write(root.join("notes.md"), "two\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 2, changed: true, .. }
    ));

    let (versions, chain_valid, event_count) =
        history(run_history(&mut state, "notes.md").await);
    let labels: Vec<&str> = versions.iter().map(|version| version.label.as_str()).collect();
    assert_eq!(labels, ["v2", "v1"]);
    assert!(versions[0].current);
    assert!(!versions[1].current);
    assert!(versions
        .iter()
        .all(|version| version.actor == "human" && version.reason == "gui-capture"));
    assert!(versions.iter().all(|version| version.local));
    assert!(chain_valid);
    assert_eq!(event_count, 2);
}

#[tokio::test]
async fn it_629_history_not_found_and_deleted_rows() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("gone.md"), "bye\n").unwrap();
    fs::write(root.join("plain.txt"), "plain\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    assert!(matches!(
        run_capture(&mut state, "gone.md").await,
        Reply::Captured { .. }
    ));

    assert_failed(run_history(&mut state, "").await, FailKind::NotFound, app::MSG_NOT_FOUND);
    assert_failed(
        run_history(&mut state, ".LADONignore").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_history(&mut state, "plain.txt").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_history(&mut state, "missing.md").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );

    // A ledger row marked deleted is ignored by history and by the listing.
    let store = Store::new(root.join(".ladon").join("meta.turso"));
    let record = store.find_path("gone.md").await.unwrap().expect("ledger row");
    store.mark_deleted(&record, "human", "test").await.unwrap();
    assert_failed(
        run_history(&mut state, "gone.md").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    let gone = row(&rows, "gone.md");
    assert_eq!(gone.status_text, "未记录");
    assert!(!gone.tracked);
}

#[tokio::test]
async fn it_630_diff_between_text_versions() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    fs::write(root.join("doc.txt"), "a\nb\nc\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "doc.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    fs::write(root.join("doc.txt"), "a\nB\nc\nd\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "doc.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));

    let ids = version_ids(&mut state, "doc.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::Diff { older_version_id, newer_version_id, diff } => {
            assert_eq!(older_version_id, ids[1]);
            assert_eq!(newer_version_id, ids[0]);
            assert_eq!(diff.added, 2);
            assert_eq!(diff.removed, 1);
            assert!(!diff.truncated);
            assert!(!diff.approximate);
            let removed: Vec<&str> = diff
                .lines
                .iter()
                .filter(|line| matches!(line.kind, LineKind::Removed))
                .map(|line| line.text.as_str())
                .collect();
            let added: Vec<&str> = diff
                .lines
                .iter()
                .filter(|line| matches!(line.kind, LineKind::Added))
                .map(|line| line.text.as_str())
                .collect();
            assert_eq!(removed, ["b"]);
            assert_eq!(added, ["B", "d"]);
        }
        other => panic!("expected Diff, got {other:?}"),
    }
}

#[tokio::test]
async fn it_631_diff_max_lines_truncates() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    fs::write(root.join("long.txt"), "").unwrap();
    assert!(matches!(
        run_capture(&mut state, "long.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    let body: String = (0..5000).map(|index| format!("line {index}\n")).collect();
    fs::write(root.join("long.txt"), body).unwrap();
    assert!(matches!(
        run_capture(&mut state, "long.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));

    let ids = version_ids(&mut state, "long.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::Diff { diff, .. } => {
            assert_eq!(diff.added, 5000);
            assert_eq!(diff.removed, 0);
            assert!(diff.truncated);
            assert_eq!(diff.lines.len(), app::DIFF_MAX_LINES);
        }
        other => panic!("expected Diff, got {other:?}"),
    }
}

#[tokio::test]
async fn it_632_binary_version_not_diffed() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    // A zero byte at the start of the newer version.
    fs::write(root.join("data.bin"), "plain\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "data.bin").await,
        Reply::Captured { version_no: 1, .. }
    ));
    fs::write(root.join("data.bin"), [0u8, b'a']).unwrap();
    assert!(matches!(
        run_capture(&mut state, "data.bin").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "data.bin").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::DiffUnavailable { reason, .. } => assert_eq!(reason, DiffBlock::Binary),
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }

    // A zero byte after the first 8000 bytes also makes the version binary.
    let mut late = vec![b'a'; 9000];
    fs::write(root.join("late.txt"), &late).unwrap();
    assert!(matches!(
        run_capture(&mut state, "late.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    late[8500] = 0;
    fs::write(root.join("late.txt"), &late).unwrap();
    assert!(matches!(
        run_capture(&mut state, "late.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "late.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::DiffUnavailable { reason, .. } => assert_eq!(reason, DiffBlock::Binary),
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }
}

#[tokio::test]
async fn it_633_non_utf8_version_not_diffed() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    // Invalid UTF-8 without a zero byte.
    fs::write(root.join("bad.txt"), "text\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "bad.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    fs::write(root.join("bad.txt"), [0xFFu8, 0xFE, 0x41]).unwrap();
    assert!(matches!(
        run_capture(&mut state, "bad.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "bad.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::DiffUnavailable { reason, .. } => assert_eq!(reason, DiffBlock::NotUtf8),
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }

    // A zero byte takes precedence over invalid UTF-8.
    fs::write(root.join("both.txt"), "text\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "both.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    fs::write(root.join("both.txt"), [0xFFu8, 0x00]).unwrap();
    assert!(matches!(
        run_capture(&mut state, "both.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "both.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::DiffUnavailable { reason, .. } => assert_eq!(reason, DiffBlock::Binary),
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }
}

#[tokio::test]
async fn it_634_oversized_version_not_loaded() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    // One byte over the limit: the newer version is refused.
    fs::write(root.join("big.txt"), "a\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "big.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    let over = vec![b'a'; app::DIFF_LIMIT_BYTES as usize + 1];
    fs::write(root.join("big.txt"), &over).unwrap();
    assert!(matches!(
        run_capture(&mut state, "big.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "big.txt").await;
    match run_diff(&mut state, &ids[1], &ids[0]).await {
        Reply::DiffUnavailable { reason, .. } => assert_eq!(reason, DiffBlock::TooLarge),
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }

    // Exactly at the limit: the diff is computed.
    fs::write(root.join("exact.txt"), "a\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "exact.txt").await,
        Reply::Captured { version_no: 1, .. }
    ));
    let exact = vec![b'a'; app::DIFF_LIMIT_BYTES as usize];
    fs::write(root.join("exact.txt"), &exact).unwrap();
    assert!(matches!(
        run_capture(&mut state, "exact.txt").await,
        Reply::Captured { version_no: 2, .. }
    ));
    let ids = version_ids(&mut state, "exact.txt").await;
    assert!(matches!(
        run_diff(&mut state, &ids[1], &ids[0]).await,
        Reply::Diff { .. }
    ));
}

#[tokio::test]
async fn it_635_cold_version_diff_unavailable() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    fs::write(root.join("notes.md"), "old\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 1, .. }
    ));
    fs::write(root.join("notes.md"), "two\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 2, .. }
    ));

    let ids = version_ids(&mut state, "notes.md").await;
    let (older, newer) = (ids[1].clone(), ids[0].clone());
    // Remove the older version's content from the local content store.
    let store = Store::new(root.join(".ladon").join("meta.turso"));
    let older_hash = store
        .version(&older)
        .await
        .unwrap()
        .expect("older version")
        .content_hash;
    CasStore::new(root.join(".ladon").join("cas"))
        .evict(&older_hash)
        .unwrap();

    match run_diff(&mut state, &older, &newer).await {
        Reply::DiffUnavailable { older_version_id, newer_version_id, reason } => {
            assert_eq!(older_version_id, older);
            assert_eq!(newer_version_id, newer);
            assert_eq!(reason, DiffBlock::Cold);
        }
        other => panic!("expected DiffUnavailable, got {other:?}"),
    }

    let (versions, _, _) = history(run_history(&mut state, "notes.md").await);
    assert!(!versions[1].local);
    assert_eq!(versions[1].availability_text, "内容不在本地，暂不可比较");
    assert!(versions[0].local);
}

#[tokio::test]
async fn it_636_diff_rejects_unknown_or_foreign_versions() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));
    fs::write(root.join("a.txt"), "a\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "a.txt").await,
        Reply::Captured { .. }
    ));
    fs::write(root.join("b.txt"), "b\n").unwrap();
    assert!(matches!(
        run_capture(&mut state, "b.txt").await,
        Reply::Captured { .. }
    ));

    let a_ids = version_ids(&mut state, "a.txt").await;
    let b_ids = version_ids(&mut state, "b.txt").await;
    assert_failed(
        run_diff(&mut state, &a_ids[0], "no-such-version").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_diff(&mut state, "no-such-version", &a_ids[0]).await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_diff(&mut state, &a_ids[0], &b_ids[0]).await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
}

#[tokio::test]
async fn it_637_invalid_paths_refused() {
    let base = tempfile::tempdir().unwrap();
    let base_root = canonical(base.path());
    let root = base_root.join("ws");
    fs::create_dir_all(&root).unwrap();
    fs::write(base_root.join("outside.txt"), "outside\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    for relative in ["../outside.txt", "/abs.txt", "C:/x.txt", "docs//a.md", "docs\\..\\x"] {
        assert_failed(
            run_list(&mut state, relative).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert_failed(
            run_capture(&mut state, relative).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
        assert_failed(
            run_history(&mut state, relative).await,
            FailKind::InvalidPath,
            app::MSG_INVALID_PATH,
        );
    }

    // Kernel-owned folders are refused with the internal message, not as invalid.
    assert_failed(
        run_list(&mut state, ".ladon").await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    assert_failed(
        run_capture(&mut state, ".ladon/meta.turso").await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    assert_failed(
        run_history(&mut state, ".ladon/meta.turso").await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    assert_failed(
        run_capture(&mut state, ".git/config").await,
        FailKind::Refused,
        app::MSG_INTERNAL_REFUSED,
    );
    assert_eq!(
        fs::read_to_string(base_root.join("outside.txt")).unwrap(),
        "outside\n"
    );
    assert!(!root.join("docs").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn it_638_links_listed_not_followed() {
    let base = tempfile::tempdir().unwrap();
    let base_root = canonical(base.path());
    let root = base_root.join("ws");
    fs::create_dir_all(root.join("shared")).unwrap();
    let outside = base_root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    make_junction(&root.join("shared").join("link"), &outside);
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    let (relative_dir, rows, _) = folder(run_list(&mut state, "shared").await);
    assert_eq!(relative_dir, "shared");
    let link = row(&rows, "link");
    assert!(link.is_link);
    assert_eq!(link.status_text, "链接（不跟随）");
    assert!(rows.iter().all(|candidate| candidate.name != "secret.txt"));

    assert_failed(
        run_list(&mut state, "shared/link").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_capture(&mut state, "shared/link/secret.txt").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_capture(&mut state, "shared/link").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
}

#[tokio::test]
async fn it_639_capture_path_rules() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("notes.md"), "text\n").unwrap();
    fs::create_dir_all(root.join("sub")).unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    assert_failed(
        run_capture(&mut state, "").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_capture(&mut state, ".LADONignore").await,
        FailKind::Refused,
        app::MSG_RULE_FILE_REFUSED,
    );
    assert_failed(
        run_capture(&mut state, "sub/.ladonignore").await,
        FailKind::Refused,
        app::MSG_RULE_FILE_REFUSED,
    );
    assert_failed(
        run_capture(&mut state, "missing.txt").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_capture(&mut state, "missing/x.txt").await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    assert_failed(
        run_capture(&mut state, "sub").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        run_capture(&mut state, "notes.md/x.txt").await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );

    // The refused and failed captures recorded nothing.
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    assert_eq!(row(&rows, "notes.md").status_text, "未记录");
}

#[tokio::test]
async fn it_640_rule_file_reload_and_parse_failure() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("notes.md"), "rules\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    // A rule file that cannot be parsed stops the capture and records nothing.
    let rule = root.join(".LADONignore");
    // An unclosed alternate group is rejected by the glob parser; an unclosed
    // character class is accepted, as in Git, so it cannot serve as the failure input.
    fs::write(&rule, "{a,b\n").unwrap();
    set_mtime(&rule, SystemTime::now() + Duration::from_secs(120));
    assert_failed(
        run_capture(&mut state, "notes.md").await,
        FailKind::Refused,
        app::MSG_RULE_PARSE_FAILED,
    );
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    assert_eq!(row(&rows, "notes.md").status_text, "未记录");

    // A parsable rule file is loaded and the capture succeeds.
    fs::write(&rule, "# restored\n").unwrap();
    set_mtime(&rule, SystemTime::now() + Duration::from_secs(240));
    assert!(matches!(
        run_capture(&mut state, "notes.md").await,
        Reply::Captured { version_no: 1, changed: true, .. }
    ));

    // Opening a workspace whose rule file cannot be parsed fails and keeps the open one.
    let other_dir = tempfile::tempdir().unwrap();
    let other = canonical(other_dir.path());
    let mut scratch = State::new();
    assert!(matches!(run_init(&mut scratch, &other).await, Reply::Opened { .. }));
    fs::write(other.join(".LADONignore"), "{a,b\n").unwrap();
    assert_failed(
        run_open(&mut state, &other).await,
        FailKind::Kernel,
        app::MSG_OPEN_FAILED,
    );
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    assert_eq!(row(&rows, "notes.md").status_text, "已记录 v1");
}

#[cfg(windows)]
#[tokio::test]
async fn it_641_kernel_failures_report_messages() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    fs::write(root.join("locked.txt"), "held\n").unwrap();
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &root).await, Reply::Opened { .. }));

    {
        // An exclusive handle makes the kernel's read of the file fail.
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(root.join("locked.txt"))
            .unwrap();
        assert_failed(
            run_capture(&mut state, "locked.txt").await,
            FailKind::Kernel,
            app::MSG_CAPTURE_FAILED,
        );
    }
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    assert_eq!(row(&rows, "locked.txt").status_text, "未记录");
    assert!(matches!(
        run_capture(&mut state, "locked.txt").await,
        Reply::Captured { version_no: 1, changed: true, .. }
    ));

    // A regular file named .ladon blocks initialisation.
    let blocked_dir = tempfile::tempdir().unwrap();
    let blocked = canonical(blocked_dir.path());
    fs::write(blocked.join(".ladon"), "not a folder").unwrap();
    let mut fresh = State::new();
    assert_failed(
        run_init(&mut fresh, &blocked).await,
        FailKind::Kernel,
        app::MSG_INIT_FAILED,
    );
    assert!(!blocked.join(".LADONignore").exists());
}

#[test]
fn it_642_worker_one_reply_per_command_in_order() {
    let base = tempfile::tempdir().unwrap();
    let root = canonical(base.path());
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    let worker = Worker::spawn(move |reply| {
        let _ = reply_tx.send(reply);
    });
    assert!(worker.send(Command::Init { path: arg(&root), confirmed: true }));
    assert!(worker.send(Command::ListFolder { relative_dir: String::new() }));
    assert!(worker.send(Command::Capture { relative_path: "missing.txt".to_string() }));

    let wait = Duration::from_secs(60);
    assert!(matches!(reply_rx.recv_timeout(wait), Ok(Reply::Opened { .. })));
    assert!(matches!(reply_rx.recv_timeout(wait), Ok(Reply::Folder { .. })));
    assert!(matches!(
        reply_rx.recv_timeout(wait),
        Ok(Reply::Failed { kind: FailKind::NotFound, .. })
    ));

    drop(worker);
    // The callback owned the only sender, so the reply channel closes with the worker.
    assert!(matches!(
        reply_rx.recv_timeout(wait),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn it_643_send_reports_stopped_worker() {
    let (signal_tx, signal_rx) = mpsc::channel::<()>();
    let worker = Worker::spawn(move |_reply| {
        let _ = signal_tx.send(());
        panic!("reply callback fails on purpose");
    });
    assert!(worker.send(Command::ListFolder { relative_dir: String::new() }));
    signal_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("callback ran");

    let started = Instant::now();
    while worker.send(Command::ListFolder { relative_dir: String::new() }) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "worker still accepts commands after 5 seconds"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test]
async fn it_644_guard_and_diff_block_messages() {
    // A panic becomes an internal failure with the fixed message and no detail.
    let reply = app::guard(|| panic!("simulated fault"));
    assert_failed(reply, FailKind::Internal, app::MSG_INTERNAL);

    // A normal reply passes through unchanged.
    let passed = app::guard(|| Reply::Opened { root: "C:\\Work".to_string() });
    assert!(matches!(passed, Reply::Opened { .. }));

    // The handler keeps working after a guarded failure.
    let mut state = State::new();
    assert_failed(
        run_list(&mut state, "").await,
        FailKind::NoWorkspace,
        app::MSG_NO_WORKSPACE,
    );

    assert_eq!(app::diff_block_message(DiffBlock::Binary), app::MSG_DIFF_BINARY);
    assert_eq!(app::diff_block_message(DiffBlock::NotUtf8), app::MSG_DIFF_NOT_UTF8);
    assert_eq!(app::diff_block_message(DiffBlock::TooLarge), app::MSG_DIFF_TOO_LARGE);
    assert_eq!(app::diff_block_message(DiffBlock::Cold), app::MSG_DIFF_COLD);
}

#[test]
fn it_645_constants_and_message_text() {
    assert_eq!(app::DIFF_LIMIT_BYTES, 2 * 1024 * 1024);
    assert_eq!(app::DIFF_MAX_LINES, 4000);
    assert_eq!(app::DIFF_TIMEOUT, Duration::from_secs(2));
    assert_eq!(app::LIST_LIMIT, 5000);

    let messages: [(&str, &str); 21] = [
        (app::MSG_NO_WORKSPACE, "请先打开或初始化一个工作区。"),
        (
            app::MSG_INVALID_PATH,
            "该路径无效，或包含链接、重解析点，或位于工作区之外，已拒绝访问。",
        ),
        (app::MSG_NOT_FOUND, "找不到该文件、文件夹或版本。"),
        (app::MSG_NOT_CONFIRMED, "需要先确认才能初始化该文件夹。"),
        (
            app::MSG_ROOT_REFUSED,
            "不能把用户主目录本身作为工作区，请选择其中的子文件夹。",
        ),
        (app::MSG_RULE_FILE_REFUSED, "规则文件 .LADONignore 不作为历史记录对象。"),
        (app::MSG_INTERNAL_REFUSED, "内部目录 .ladon 和 .git 不作为历史记录对象。"),
        (
            app::MSG_RULE_PARSE_FAILED,
            "规则文件 .LADONignore 无法解析，已停止记录。请先修正该文件。",
        ),
        (
            app::MSG_OPEN_FAILED,
            "无法打开该工作区。请检查 .ladon 目录和 .LADONignore 规则文件是否完整。",
        ),
        (app::MSG_INIT_FAILED, "无法初始化该文件夹。请检查该文件夹的写入权限。"),
        (app::MSG_LIST_FAILED, "无法读取该文件夹。"),
        (app::MSG_CAPTURE_FAILED, "无法记录该文件，请稍后重试。"),
        (app::MSG_HISTORY_FAILED, "无法读取该文件的历史。"),
        (app::MSG_DIFF_FAILED, "无法读取这两个版本的内容。"),
        (
            app::MSG_INTERNAL,
            "内部错误，操作未能完成。请重试；若再次出现，请重新打开应用。",
        ),
        (app::MSG_DIFF_BINARY, "二进制文件，不显示文本差异。"),
        (app::MSG_DIFF_NOT_UTF8, "文件不是 UTF-8 文本，不显示文本差异。"),
        (app::MSG_DIFF_TOO_LARGE, "文件超过 2 MiB，不显示文本差异。"),
        (app::MSG_DIFF_COLD, "该版本的内容不在本地，暂不能比较。"),
        (app::MSG_DIFF_APPROXIMATE, "差异计算超过 2 秒，结果可能不是最小改动。"),
        (app::MSG_DIFF_TRUNCATED, "差异超过 4000 行，仅显示前 4000 行。"),
    ];
    for (actual, expected) in messages {
        assert_eq!(actual, expected);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn it_646_list_drives() {
    use std::os::windows::fs::MetadataExt;

    /// FILE_ATTRIBUTE_REPARSE_POINT from the Windows file attribute set.
    const REPARSE_POINT: u32 = 0x0000_0400;

    let ws_dir = tempfile::tempdir().unwrap();
    let workspace_text = display_of(ws_dir.path());
    let workspace_root = format!("{}\\", workspace_text[..2].to_ascii_uppercase());

    let mut state = State::new();
    let drives = match app::handle(&mut state, Command::ListDrives).await {
        Reply::Drives { drives } => drives,
        other => panic!("expected Drives, got {other:?}"),
    };
    assert!(!drives.is_empty());
    for row in &drives {
        assert!(row.root.ends_with(":\\"), "root {} must end with :\\", row.root);
    }
    assert!(
        drives.windows(2).all(|pair| pair[0].root < pair[1].root),
        "rows must be sorted by letter and must not repeat a letter"
    );

    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let system_root = format!("{}\\", system.to_ascii_uppercase());
    assert!(
        drives.iter().any(|row| row.root == system_root),
        "the system drive {system_root} must be listed"
    );
    assert!(
        drives.iter().any(|row| row.root == workspace_root),
        "the drive {workspace_root} of the test workspace must be listed"
    );

    for row in &drives {
        let meta_path = Path::new(&row.root).join(".ladon").join("meta.turso");
        let expected = fs::symlink_metadata(&meta_path)
            .map(|meta| meta.is_file() && (meta.file_attributes() & REPARSE_POINT) == 0)
            .unwrap_or(false);
        assert_eq!(row.initialised, expected, "initialised flag of {}", row.root);
    }
}

#[tokio::test]
async fn it_647_open_drive_initialises_and_opens() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    let mut state = State::new();
    let root = match app::handle(&mut state, Command::OpenDrive { path: arg(&dir) }).await {
        Reply::Opened { root } => root,
        other => panic!("expected Opened, got {other:?}"),
    };
    assert_eq!(root, display_of(&dir));
    assert!(dir.join(".ladon/meta.turso").is_file());
    assert!(dir.join(".LADONignore").is_file());
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, ".LADONignore");
    assert_eq!(rows[0].status_text, "未记录");
}

#[tokio::test]
async fn it_648_open_drive_keeps_existing_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &dir).await, Reply::Opened { .. }));
    fs::write(dir.join("notes.md"), "one\n").unwrap();
    assert!(matches!(run_capture(&mut state, "notes.md").await, Reply::Captured { .. }));
    drop(state);
    fs::write(dir.join(".LADONignore"), "# custom\n").unwrap();
    let mut second = State::new();
    assert!(matches!(
        app::handle(&mut second, Command::OpenDrive { path: arg(&dir) }).await,
        Reply::Opened { .. }
    ));
    assert_eq!(fs::read_to_string(dir.join(".LADONignore")).unwrap(), "# custom\n");
    let (versions, _, _) = history(run_history(&mut second, "notes.md").await);
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].label, "v1");
}

#[tokio::test]
async fn it_649_open_drive_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    fs::write(dir.join("notes.md"), "x").unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    let mut state = State::new();
    let open = |path: String| Command::OpenDrive { path };
    assert_failed(
        app::handle(&mut state, open("relative/dir".into())).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, open(format!("{}\0x", arg(&dir)))).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, open(arg(&dir.join("notes.md")))).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, open(arg(&dir.join("missing")))).await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = PathBuf::from(profile);
        if profile.is_dir() {
            assert_failed(
                app::handle(&mut state, open(arg(&canonical(&profile)))).await,
                FailKind::Refused,
                app::MSG_ROOT_REFUSED,
            );
        }
    }
    assert!(!dir.join(".ladon").exists(), "a refused drive root must not be initialised");
}

#[tokio::test]
async fn it_650_open_drive_init_failure_message() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    fs::write(dir.join(".ladon"), "not a folder").unwrap();
    let mut state = State::new();
    assert_failed(
        app::handle(&mut state, Command::OpenDrive { path: arg(&dir) }).await,
        FailKind::Kernel,
        app::MSG_DRIVE_INIT_FAILED,
    );
    assert!(!dir.join(".LADONignore").exists());
}

#[cfg(windows)]
fn write_with_attributes(path: &Path, attributes: u32) {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(attributes)
        .open(path)
        .unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn it_651_listing_omits_hidden_and_system() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    fs::write(dir.join("visible.txt"), "v").unwrap();
    write_with_attributes(&dir.join("hidden.txt"), 0x2);
    write_with_attributes(&dir.join("system.txt"), 0x4);
    let mut state = State::new();
    assert!(matches!(run_init(&mut state, &dir).await, Reply::Opened { .. }));
    let (_, rows, _) = folder(run_list(&mut state, "").await);
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, [".LADONignore", "visible.txt"]);
}

#[tokio::test]
async fn it_652_list_subfolders_folders_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    fs::create_dir(dir.join("beta")).unwrap();
    fs::create_dir(dir.join("Alpha")).unwrap();
    fs::create_dir(dir.join(".git")).unwrap();
    fs::write(dir.join("zeta.txt"), "z").unwrap();
    fs::write(dir.join(".LADONignore"), "").unwrap();
    let mut state = State::new();
    match app::handle(&mut state, Command::ListSubfolders { path: arg(&dir) }).await {
        Reply::Subfolders { path, names } => {
            assert_eq!(path, arg(&dir));
            assert_eq!(names, ["Alpha", "beta"]);
        }
        other => panic!("expected Subfolders, got {other:?}"),
    }
    assert!(!dir.join(".ladon").exists(), "listing subfolders writes nothing");
}

#[cfg(windows)]
#[tokio::test]
async fn it_653_list_subfolders_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = canonical(tmp.path());
    fs::create_dir(dir.join("real")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    make_junction(&dir.join("link"), &canonical(outside.path()));
    fs::write(dir.join("link2.txt"), "x").unwrap();
    let mut state = State::new();
    match app::handle(&mut state, Command::ListSubfolders { path: arg(&dir) }).await {
        Reply::Subfolders { names, .. } => assert_eq!(names, ["real"]),
        other => panic!("expected Subfolders, got {other:?}"),
    }
    let list = |path: String| Command::ListSubfolders { path };
    assert_failed(
        app::handle(&mut state, list("real".into())).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, list(format!("{}\0x", arg(&dir)))).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, list(arg(&dir.join("link2.txt")))).await,
        FailKind::InvalidPath,
        app::MSG_INVALID_PATH,
    );
    assert_failed(
        app::handle(&mut state, list(arg(&dir.join("missing")))).await,
        FailKind::NotFound,
        app::MSG_NOT_FOUND,
    );
}
