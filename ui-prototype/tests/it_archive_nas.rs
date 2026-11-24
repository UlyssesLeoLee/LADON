//! Archive and restore of versions through the window's commands. The tests set the archive key
//! and the Google token in this process, so the file runs in its own process and the tests are
//! serialised with a lock.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ladon_explorer::archive::CasStore;
use ladon_explorer::store::Store;
use ladon_ui_prototype::app::{self, ArchiveTarget, Command, FailKind, Reply, State};

static SERIAL: Mutex<()> = Mutex::new(());

const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

fn arg(path: &Path) -> String {
    canonical(path).to_string_lossy().into_owned()
}

async fn init(state: &mut State, dir: &Path) {
    let reply = app::handle(state, Command::OpenDrive { path: arg(dir) }).await;
    assert!(matches!(reply, Reply::Opened { .. }), "open the workspace: {reply:?}");
}

async fn capture(state: &mut State, relative: &str) {
    let reply = app::handle(state, Command::Capture { relative_path: relative.to_string() }).await;
    assert!(matches!(reply, Reply::Captured { .. }), "capture {relative}: {reply:?}");
}

async fn first_version(state: &mut State, relative: &str) -> String {
    match app::handle(state, Command::LoadHistory { relative_path: relative.to_string() }).await {
        Reply::History { versions, .. } => versions[0].version_id.clone(),
        other => panic!("expected History, got {other:?}"),
    }
}

fn assert_failed(reply: Reply, kind: FailKind, message: &str) {
    match reply {
        Reply::Failed { kind: got_kind, message: got_message, .. } => {
            assert_eq!(got_kind, kind);
            assert_eq!(got_message, message);
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn files_under(dir: &Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(fs::read(&path).unwrap());
        }
    }
    out
}

#[tokio::test]
async fn it_660_archive_version_targets() {
    let _guard = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let ws = tempfile::tempdir().unwrap();
    let dir = canonical(ws.path());
    let nas = tempfile::tempdir().unwrap();
    let nas_dir = canonical(nas.path());
    fs::write(dir.join("plain.txt"), "plain secret words").unwrap();
    fs::write(dir.join("secret.txt"), "classified").unwrap();
    fs::write(dir.join(".LADONignore"), "secret.txt\n").unwrap();
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
    let mut state = State::new();
    init(&mut state, &dir).await;
    capture(&mut state, "plain.txt").await;
    capture(&mut state, "secret.txt").await;
    let plain = first_version(&mut state, "plain.txt").await;
    let secret = first_version(&mut state, "secret.txt").await;
    let nas_target = ArchiveTarget::Nas { folder: nas_dir.to_string_lossy().into_owned() };

    assert_failed(
        app::handle(&mut state, Command::ArchiveVersion { version_id: plain.clone(), target: nas_target.clone() }).await,
        FailKind::Kernel,
        app::MSG_ARCHIVE_KEY_MISSING,
    );

    std::env::set_var("LADON_ARCHIVE_KEY_HEX", KEY);
    match app::handle(&mut state, Command::ArchiveVersion { version_id: plain, target: nas_target.clone() }).await {
        Reply::Archived { provider, .. } => assert_eq!(provider, "nas"),
        other => panic!("expected Archived, got {other:?}"),
    }
    let stored = files_under(&nas_dir);
    assert!(!stored.is_empty(), "the archive object is written to the NAS folder");
    assert!(
        stored.iter().all(|bytes| !bytes.windows(b"plain secret words".len()).any(|w| w == b"plain secret words")),
        "the archive object holds no plain text"
    );

    assert_failed(
        app::handle(&mut state, Command::ArchiveVersion { version_id: secret, target: nas_target }).await,
        FailKind::Refused,
        app::MSG_ARCHIVE_REFUSED,
    );

    let plain_again = first_version(&mut state, "plain.txt").await;
    assert_failed(
        app::handle(
            &mut state,
            Command::ArchiveVersion { version_id: plain_again, target: ArchiveTarget::GoogleDrive },
        )
        .await,
        FailKind::Kernel,
        app::MSG_TOKEN_MISSING,
    );
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
}

#[tokio::test]
async fn it_661_restore_version() {
    let _guard = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let ws = tempfile::tempdir().unwrap();
    let dir = canonical(ws.path());
    let nas = tempfile::tempdir().unwrap();
    let nas_dir = canonical(nas.path());
    fs::write(dir.join("plain.txt"), "version one").unwrap();
    std::env::set_var("LADON_ARCHIVE_KEY_HEX", KEY);
    let mut state = State::new();
    init(&mut state, &dir).await;
    capture(&mut state, "plain.txt").await;
    let version = first_version(&mut state, "plain.txt").await;
    let nas_target = ArchiveTarget::Nas { folder: nas_dir.to_string_lossy().into_owned() };
    assert!(matches!(
        app::handle(&mut state, Command::ArchiveVersion { version_id: version.clone(), target: nas_target.clone() }).await,
        Reply::Archived { .. }
    ));

    fs::write(dir.join("plain.txt"), "edited outside the ledger").unwrap();
    assert_failed(
        app::handle(&mut state, Command::RestoreVersion { version_id: version.clone(), target: None }).await,
        FailKind::Refused,
        app::MSG_RESTORE_DIVERGED,
    );

    fs::write(dir.join("plain.txt"), "version one").unwrap();
    let store = Store::new(dir.join(".ladon/meta.turso"));
    let content_hash = store.version(&version).await.unwrap().unwrap().content_hash;
    CasStore::new(dir.join(".ladon/cas")).evict(&content_hash).unwrap();
    assert_failed(
        app::handle(&mut state, Command::RestoreVersion { version_id: version.clone(), target: None }).await,
        FailKind::Refused,
        app::MSG_RESTORE_COLD,
    );

    match app::handle(&mut state, Command::RestoreVersion { version_id: version, target: Some(nas_target) }).await {
        Reply::Restored { relative_path, .. } => assert_eq!(relative_path, "plain.txt"),
        other => panic!("expected Restored, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(dir.join("plain.txt")).unwrap(), "version one");
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
}

#[tokio::test]
async fn it_662_archive_applies_rule_edited_after_open() {
    let _guard = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let ws = tempfile::tempdir().unwrap();
    let dir = canonical(ws.path());
    let nas = tempfile::tempdir().unwrap();
    let nas_dir = canonical(nas.path());
    fs::write(dir.join("note.txt"), "captured before the rule change").unwrap();
    fs::write(dir.join(".LADONignore"), "other.txt\n").unwrap();
    std::env::set_var("LADON_ARCHIVE_KEY_HEX", KEY);
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
    let mut state = State::new();
    init(&mut state, &dir).await;
    capture(&mut state, "note.txt").await;
    let version = first_version(&mut state, "note.txt").await;

    // The root rule file gains a rule after the workspace was opened and the last capture ran.
    fs::write(dir.join(".LADONignore"), "other.txt\nnote.txt\n").unwrap();
    let nas_target = ArchiveTarget::Nas { folder: nas_dir.to_string_lossy().into_owned() };
    assert_failed(
        app::handle(&mut state, Command::ArchiveVersion { version_id: version, target: nas_target }).await,
        FailKind::Refused,
        app::MSG_ARCHIVE_REFUSED,
    );
    assert!(files_under(&nas_dir).is_empty(), "no archive object is written for the excluded file");
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
}
