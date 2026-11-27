//! Integration tests for the external transfer gate of `engine::Engine` (IT-054, TC-850 to TC-857).
//!
//! Each test sets or removes the process-wide variable `LADON_GLOBAL_IGNORE`, so every test holds
//! one lock for its whole run. Async steps run on a current-thread runtime built inside the locked
//! scope, so the lock is never held across an await point.
use ladon_explorer::engine::Engine;
use std::{
    fs,
    future::Future,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{Duration, SystemTime},
};

static GATE_LOCK: Mutex<()> = Mutex::new(());

/// Holds the gate lock for one test. The global rule variable is removed on entry and on exit.
struct GateScope {
    _lock: MutexGuard<'static, ()>,
}

fn gate_scope() -> GateScope {
    let lock = GATE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::remove_var("LADON_GLOBAL_IGNORE");
    GateScope { _lock: lock }
}

impl Drop for GateScope {
    fn drop(&mut self) {
        std::env::remove_var("LADON_GLOBAL_IGNORE");
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

/// Sets the modification time of `path`, so that an edit can keep the time it had.
fn set_mtime(path: &Path, when: SystemTime) {
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(when).unwrap();
}

async fn version_of(e: &Engine, id: Option<String>) -> ladon_explorer::model::VersionRecord {
    let id = id.unwrap();
    e.store.version(&id).await.unwrap().unwrap()
}

fn fixed_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

#[test]
fn it_054_file_level_rules_follow_root_nested_global_and_negation() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let rules = tempfile::tempdir().unwrap();
    let global = rules.path().join("global.ignore");
    write(&global, "*.secret\n");
    std::env::set_var("LADON_GLOBAL_IGNORE", &global);
    write(&root.join(".LADONignore"), "*.pem\nprivate/\n");
    write(&root.join("keys/.LADONignore"), "!public.pem\n");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        let files = [
            ("top.txt", "top text"),
            ("top.pem", "top key"),
            ("keys/private.pem", "private key"),
            ("keys/public.pem", "public key"),
            ("private/a.md", "private doc"),
            ("docs/notes.secret", "notes secret"),
            ("docs/notes.md", "notes doc"),
        ];
        for (rel, text) in files {
            write(&root.join(rel), text);
            e.capture(&root.join(rel), "human", "tc850").await.unwrap();
        }
        let expected = [
            ("top.txt", true),
            ("top.pem", false),
            ("keys/private.pem", false),
            ("keys/public.pem", true),
            ("private/a.md", false),
            ("docs/notes.secret", false),
            ("docs/notes.md", true),
        ];
        for (rel, allowed) in expected {
            let file = e.file_by_path(rel).await.unwrap();
            assert_eq!(e.remote_read_allowed(&file).await.unwrap(), allowed, "{rel}");
        }
    });
}

#[test]
fn it_054_excluded_content_copy_and_sensitive_file_denied() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("contracts/a.txt"), "alpha");
        e.capture(&root.join("contracts/a.txt"), "human", "tc851").await.unwrap();

        write(&root.join(".LADONignore"), "/contracts/\n/flagged.md\n");
        e.reload_policy().unwrap();
        write(&root.join("shared/a.txt"), "alpha");
        e.capture(&root.join("shared/a.txt"), "human", "tc851").await.unwrap();
        write(&root.join("notes.md"), "beta");
        e.capture(&root.join("notes.md"), "human", "tc851").await.unwrap();
        write(&root.join("flagged.md"), "gamma");
        let flagged = e.capture(&root.join("flagged.md"), "human", "tc851").await.unwrap();
        assert!(flagged.sensitive, "the file is flagged while its path is excluded");

        // The rule for flagged.md is removed; its stored flag and its content still refuse it.
        write(&root.join(".LADONignore"), "/contracts/\n");
        e.reload_policy().unwrap();
        let contracts = e.file_by_path("contracts/a.txt").await.unwrap();
        assert!(!e.remote_read_allowed(&contracts).await.unwrap(), "an excluded path is refused");
        let copy = e.file_by_path("shared/a.txt").await.unwrap();
        assert!(!e.remote_read_allowed(&copy).await.unwrap(), "a copy of excluded content is refused");
        let notes = e.file_by_path("notes.md").await.unwrap();
        assert!(e.remote_read_allowed(&notes).await.unwrap(), "unrelated content is allowed");
        let flagged = e.file_by_path("flagged.md").await.unwrap();
        assert!(!e.remote_read_allowed(&flagged).await.unwrap(), "a sensitive file is refused");
    });
}

#[test]
fn it_054_version_level_flags_and_excluded_bytes_denied() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("plain.md"), "old bytes");
        let plain_v1 = e.capture(&root.join("plain.md"), "human", "tc852").await.unwrap();
        write(&root.join("plain.md"), "new bytes");
        let plain_v2 = e.capture(&root.join("plain.md"), "human", "tc852").await.unwrap();

        write(&root.join(".LADONignore"), "/draft.md\n");
        e.reload_policy().unwrap();
        write(&root.join("draft.md"), "v1 text");
        let draft_v1 = e.capture(&root.join("draft.md"), "human", "tc852").await.unwrap();
        write(&root.join(".LADONignore"), "");
        e.reload_policy().unwrap();
        write(&root.join("draft.md"), "v2 text");
        let draft_v2 = e.capture(&root.join("draft.md"), "human", "tc852").await.unwrap();

        // The vault file is captured while no rule covers it, so its own version is not flagged.
        write(&root.join("vault/key.txt"), "old bytes");
        e.capture(&root.join("vault/key.txt"), "human", "tc852").await.unwrap();
        write(&root.join(".LADONignore"), "/vault/\n");
        e.reload_policy().unwrap();

        let draft_v1_record = version_of(&e, draft_v1.version_id.clone()).await;
        assert!(draft_v1_record.sensitive, "version 1 was captured while its path was excluded");
        let draft_v2_record = version_of(&e, draft_v2.version_id.clone()).await;
        let plain_v1_record = version_of(&e, plain_v1.version_id.clone()).await;
        let plain_v2_record = version_of(&e, plain_v2.version_id.clone()).await;

        assert!(!e.remote_version_allowed(&draft_v1_record).await.unwrap(), "a version flagged sensitive is refused");
        assert!(e.remote_version_allowed(&draft_v2_record).await.unwrap(), "the unflagged later version is allowed");
        assert!(!e.remote_version_allowed(&plain_v1_record).await.unwrap(), "bytes held by an excluded file are refused");
        assert!(e.remote_version_allowed(&plain_v2_record).await.unwrap(), "the later bytes are allowed");

        let draft = e.file_by_path("draft.md").await.unwrap();
        assert!(e.remote_read_allowed(&draft).await.unwrap());
        let plain = e.file_by_path("plain.md").await.unwrap();
        assert!(e.remote_read_allowed(&plain).await.unwrap());
    });
}

#[test]
fn it_054_invalid_or_unreadable_rules_deny_every_transfer() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let rules = tempfile::tempdir().unwrap();
    write(&root.join(".LADONignore"), "");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("ok.md"), "ok bytes");
        let captured = e.capture(&root.join("ok.md"), "human", "tc853").await.unwrap();
        let file = e.file_by_path("ok.md").await.unwrap();
        let version = e.store.version(captured.version_id.as_deref().unwrap()).await.unwrap().unwrap();
        assert!(e.remote_read_allowed(&file).await.unwrap());
        assert!(e.remote_version_allowed(&version).await.unwrap());

        // (a) The root rule file does not parse.
        write(&root.join(".LADONignore"), "[z-a]\n");
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "an invalid root rule file refuses the file");
        assert!(!e.remote_version_allowed(&version).await.unwrap(), "an invalid root rule file refuses the version");
        write(&root.join(".LADONignore"), "");
        assert!(e.remote_read_allowed(&file).await.unwrap());
        assert!(e.remote_version_allowed(&version).await.unwrap());

        // (b) The global rule file named by the variable cannot be read.
        std::env::set_var("LADON_GLOBAL_IGNORE", rules.path().join("missing.ignore"));
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "an unreadable global rule file refuses the file");
        assert!(!e.remote_version_allowed(&version).await.unwrap(), "an unreadable global rule file refuses the version");
        std::env::remove_var("LADON_GLOBAL_IGNORE");
        assert!(e.remote_read_allowed(&file).await.unwrap());
        assert!(e.remote_version_allowed(&version).await.unwrap());

        // (c) The root rule file is missing.
        fs::remove_file(root.join(".LADONignore")).unwrap();
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "a missing root rule file refuses the file");
        assert!(!e.remote_version_allowed(&version).await.unwrap(), "a missing root rule file refuses the version");
        write(&root.join(".LADONignore"), "");
        assert!(e.remote_read_allowed(&file).await.unwrap());
        assert!(e.remote_version_allowed(&version).await.unwrap());
    });
}

#[test]
fn it_054_root_rule_edit_seen_by_next_call_with_one_traversal() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("report.md"), "report text");
        e.capture(&root.join("report.md"), "human", "tc854").await.unwrap();
        let file = e.file_by_path("report.md").await.unwrap();
        assert!(e.remote_read_allowed(&file).await.unwrap());
        let base = e.rule_traversals();

        write(&root.join(".LADONignore"), "/report.md\n");
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "the next call sees the new root rule");
        assert_eq!(e.rule_traversals(), base + 1, "one rule edit starts exactly one traversal");
        assert!(!e.remote_read_allowed(&file).await.unwrap());
        assert_eq!(e.rule_traversals(), base + 1, "an unchanged rule set starts no traversal");

        write(&root.join(".LADONignore"), "");
        assert!(e.remote_read_allowed(&file).await.unwrap(), "the next call sees the removed rule");
        assert_eq!(e.rule_traversals(), base + 2);
        assert!(e.remote_read_allowed(&file).await.unwrap());
        assert_eq!(e.rule_traversals(), base + 2);
    });
}

#[test]
fn it_054_global_rule_edit_seen_by_next_call() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let rules = tempfile::tempdir().unwrap();
    let global = rules.path().join("global.ignore");
    write(&global, "");
    write(&root.join(".LADONignore"), "");
    std::env::set_var("LADON_GLOBAL_IGNORE", &global);

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("x.dat"), "dat bytes");
        e.capture(&root.join("x.dat"), "human", "tc855").await.unwrap();
        let file = e.file_by_path("x.dat").await.unwrap();
        assert!(e.remote_read_allowed(&file).await.unwrap());
        let base = e.rule_traversals();

        write(&global, "*.dat\n");
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "the next call sees the new global rule");
        assert_eq!(e.rule_traversals(), base + 1);

        std::env::remove_var("LADON_GLOBAL_IGNORE");
        assert!(e.remote_read_allowed(&file).await.unwrap(), "removing the variable drops the global rule");
        assert_eq!(e.rule_traversals(), base + 2);
    });
}

#[test]
fn it_054_unchanged_rules_start_no_traversal() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("a.md"), "a text");
        let captured = e.capture(&root.join("a.md"), "human", "tc856").await.unwrap();
        let file = e.file_by_path("a.md").await.unwrap();
        let version = e.store.version(captured.version_id.as_deref().unwrap()).await.unwrap().unwrap();
        let base = e.rule_traversals();

        for _ in 0..20 {
            assert!(e.remote_read_allowed(&file).await.unwrap());
            assert!(e.remote_version_allowed(&version).await.unwrap());
        }
        assert_eq!(e.rule_traversals(), base, "consecutive calls with unchanged rules start no traversal");
    });
}

#[test]
fn it_054_same_size_same_mtime_edit_is_detected() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "*.bbb\n");
    set_mtime(&root.join(".LADONignore"), fixed_time());

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("x.aaa"), "x one");
        e.capture(&root.join("x.aaa"), "human", "tc857").await.unwrap();
        let file = e.file_by_path("x.aaa").await.unwrap();
        assert!(e.remote_read_allowed(&file).await.unwrap(), "*.bbb does not match x.aaa");
        let base = e.rule_traversals();

        // The rewritten rule has the same length and the same modification time as the one in force.
        write(&root.join(".LADONignore"), "*.aaa\n");
        set_mtime(&root.join(".LADONignore"), fixed_time());
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "the same-size, same-time edit is seen");
        assert_eq!(e.rule_traversals(), base + 1, "the edit starts exactly one traversal");
    });
}

#[test]
fn tc860_path_gate_matches_rules_for_untracked_paths() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "*.key\n");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        write(&root.join("a.md"), "tracked");
        e.capture(&root.join("a.md"), "human", "tc860").await.unwrap();
        assert!(!e.remote_path_allowed(Path::new("secret.key")).await.unwrap(), "an untracked path matched by the rules is refused");
        assert!(e.remote_path_allowed(Path::new("notes.txt")).await.unwrap(), "an untracked path no rule matches is allowed");
        assert!(e.remote_path_allowed(Path::new("a.md")).await.unwrap(), "a tracked path the rules allow is allowed");
    });
}

#[test]
fn tc861_path_gate_refuses_while_root_rule_is_missing() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "*.key\n");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        assert!(e.remote_path_allowed(Path::new("notes.txt")).await.unwrap());
        fs::remove_file(root.join(".LADONignore")).unwrap();
        assert!(!e.remote_path_allowed(Path::new("notes.txt")).await.unwrap(), "a missing root rule file refuses the path");
        write(&root.join(".LADONignore"), "*.key\n");
        assert!(e.remote_path_allowed(Path::new("notes.txt")).await.unwrap(), "the restored rules allow the path again");
    });
}
#[test]
fn tc1044_path_gate_refuses_dot_dot_components() {
    let _gate = gate_scope();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join(".LADONignore"), "/private/\n");

    block_on(async {
        let e = Engine::init(root).await.unwrap();
        assert!(
            !e.remote_path_allowed(Path::new("docs/../private/x.md")).await.unwrap(),
            "a path that reaches an excluded folder through `..` is refused"
        );
        assert!(!e.remote_path_allowed(Path::new("private/x.md")).await.unwrap());
        assert!(e.remote_path_allowed(Path::new("docs/ok.md")).await.unwrap());
    });
}