//! Integration tests for migration 004, the migration backup and the turso behaviour probe (WP01).
use ladon_explorer::{
    engine::Engine,
    store::{BackupState, MigrationBackupUnverified, PendingVersion, SchemaOutcome, Store},
};
use std::path::PathBuf;

const COUNTED_TABLES: [&str; 5] = ["resources", "versions", "doc_tasks", "annotations", "governance_events"];
const LEGACY_TABLES: [&str; 8] = [
    "resources",
    "versions",
    "events",
    "archive_objects",
    "file_tags",
    "annotations",
    "doc_tasks",
    "governance_events",
];

fn pending(path: &str, hash: &str) -> PendingVersion {
    PendingVersion {
        path: path.into(),
        hash: hash.into(),
        size: 3,
        sections: vec!["H1:A#1".into()],
        actor: "human".into(),
        reason: "init".into(),
        sensitive: false,
    }
}

/// A library in the 0.0.3 shape: migrations 1 and 2 only, with one row in every table that can hold data.
async fn legacy_library() -> (tempfile::TempDir, Store, String) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let (file_id, version_id, _) = store.append_version(pending("a.md", "h1")).await.unwrap();
    store.append_version(pending("a.md", "h2")).await.unwrap();
    store.append_version(pending("b.md", "hb")).await.unwrap();
    store.add_annotation(&file_id, "H1:A#1", &version_id, "note").await.unwrap();
    let task = store.create_task(&file_id, "H1:A#1", &version_id, "title", "description").await.unwrap();
    store.set_task_status(&task, "working", "human").await.unwrap();
    store.add_governance_event(&file_id, "custom.event", "subject", "payload", "human").await.unwrap();
    store.set_tags(&file_id, &["alpha".to_string()], "rule", "1").await.unwrap();
    (tmp, store, file_id)
}

/// A separate raw connection, used to observe what the Store API does not expose.
async fn raw(store: &Store) -> turso::Connection {
    let db = turso::Builder::new_local(&store.path.to_string_lossy()).build().await.unwrap();
    db.connect().unwrap()
}

async fn scalar_i64(conn: &turso::Connection, sql: &str) -> i64 {
    let mut rows = conn.query(sql, ()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

async fn scalar_text(conn: &turso::Connection, sql: &str) -> String {
    let mut rows = conn.query(sql, ()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<String>(0).unwrap()
}

async fn table_exists(conn: &turso::Connection, table: &str) -> bool {
    conn.query(&format!("SELECT COUNT(*) FROM {table}"), ()).await.is_ok()
}

async fn recorded_versions(conn: &turso::Connection) -> Vec<i64> {
    let mut rows = conn.query("SELECT version FROM schema_migrations ORDER BY version", ()).await.unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        out.push(row.get::<i64>(0).unwrap());
    }
    out
}

async fn counted_rows(store: &Store) -> Vec<i64> {
    let conn = raw(store).await;
    let mut out = Vec::new();
    for table in COUNTED_TABLES {
        out.push(scalar_i64(&conn, &format!("SELECT COUNT(*) FROM {table}")).await);
    }
    out
}

async fn migration_payload(store: &Store) -> serde_json::Value {
    let conn = raw(store).await;
    let text = scalar_text(
        &conn,
        "SELECT payload_json FROM doc_task_events WHERE kind='dt.migration.applied'",
    )
    .await;
    serde_json::from_str(&text).unwrap()
}

fn render(value: turso::Value) -> String {
    match value {
        turso::Value::Null => "NULL".to_string(),
        turso::Value::Integer(i) => format!("i:{i}"),
        turso::Value::Real(f) => format!("r:{f:?}"),
        turso::Value::Text(s) => format!("t:{s}"),
        turso::Value::Blob(b) => format!("b:{b:?}"),
    }
}

/// Every row of a table as text, in rowid order, one string per row with every column rendered.
async fn snapshot(store: &Store, table: &str) -> Vec<String> {
    let conn = raw(store).await;
    let mut rows = conn.query(&format!("SELECT * FROM {table} ORDER BY rowid"), ()).await.unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        let cells: Vec<String> = (0..row.column_count()).map(|i| render(row.get_value(i).unwrap())).collect();
        out.push(cells.join("\u{1f}"));
    }
    out
}

fn wal_path(store: &Store) -> PathBuf {
    let mut p = store.path.clone().into_os_string();
    p.push("-wal");
    PathBuf::from(p)
}

fn variant(err: &turso::Error) -> &'static str {
    match err {
        turso::Error::Busy(_) => "Busy",
        turso::Error::BusySnapshot(_) => "BusySnapshot",
        turso::Error::Constraint(_) => "Constraint",
        turso::Error::Corrupt(_) => "Corrupt",
        turso::Error::Misuse(_) => "Misuse",
        turso::Error::Error(_) => "Error",
        _ => "other",
    }
}

#[tokio::test]
async fn it_047_fresh_library_records_versions_2_and_4() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let (file_id, _, _) = store.append_version(pending("a.md", "h1")).await.unwrap();

    let outcome = store.ensure_schema(false).await.unwrap();
    assert!(matches!(outcome, SchemaOutcome::Applied { .. }), "{outcome:?}");

    let conn = raw(&store).await;
    assert_eq!(recorded_versions(&conn).await, vec![2, 4]);
    assert!(table_exists(&conn, "doc_task_events").await);
    assert_eq!(
        scalar_i64(&conn, "SELECT COUNT(*) FROM doc_task_events WHERE kind='dt.migration.applied'").await,
        1
    );
    drop(conn);

    let by_id = store.file_by_id(&file_id).await.unwrap().unwrap();
    let by_path = store.find_path("a.md").await.unwrap().unwrap();
    assert_eq!(by_id.id, by_path.id);
    assert_eq!(by_id.relative_path, "a.md");
    assert_eq!((by_id.current_hash.as_str(), by_id.version_no, by_id.deleted), ("h1", 1, false));
    assert!(store.file_by_id("no-such-id").await.unwrap().is_none());
}

#[tokio::test]
async fn it_047_init_and_engine_init_do_not_apply_004() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let conn = raw(&store).await;
    assert_eq!(recorded_versions(&conn).await, vec![2]);
    assert!(!table_exists(&conn, "doc_task_events").await);
    drop(conn);

    let engine_root = tempfile::tempdir().unwrap();
    let _engine = Engine::init(engine_root.path()).await.unwrap();
    let engine_store = Store::new(engine_root.path().join(".ladon/meta.turso"));
    let conn = raw(&engine_store).await;
    assert_eq!(recorded_versions(&conn).await, vec![2]);
    assert!(!table_exists(&conn, "doc_task_events").await);
}

#[tokio::test]
async fn it_047_upgrade_keeps_legacy_rows_unchanged() {
    let (_tmp, store, file_id) = legacy_library().await;
    let mut before = Vec::new();
    for table in LEGACY_TABLES {
        before.push((table, snapshot(&store, table).await));
    }
    let chain_before = store.verify_chain(&file_id).await.unwrap();

    store.ensure_schema(false).await.unwrap();

    for (table, rows) in before {
        assert_eq!(snapshot(&store, table).await, rows, "table {table} changed");
    }
    assert_eq!(store.verify_chain(&file_id).await.unwrap(), chain_before);
    assert!(chain_before);
    assert_eq!(store.versions(&file_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn it_047_recorded_version_not_reapplied() {
    let (_tmp, store, _) = legacy_library().await;
    store.ensure_schema(false).await.unwrap();

    let conn = raw(&store).await;
    conn.execute("DROP TABLE doc_counters", ()).await.unwrap();
    drop(conn);

    assert!(matches!(store.ensure_schema(false).await.unwrap(), SchemaOutcome::Current));
    assert!(matches!(store.ensure_schema(false).await.unwrap(), SchemaOutcome::Current));
    store.init().await.unwrap();

    let conn = raw(&store).await;
    assert!(!table_exists(&conn, "doc_counters").await);
    assert_eq!(
        scalar_i64(&conn, "SELECT COUNT(*) FROM doc_task_events WHERE kind='dt.migration.applied'").await,
        1
    );
}

#[test]
fn it_047_sql_004_uses_only_allowed_statements() {
    let sql = include_str!("../sql/004_document_tasks.sql");
    assert!(!sql.contains("--") && !sql.contains("/*"), "comments are not allowed");
    let upper = sql.to_ascii_uppercase();
    for banned in ["TRIGGER", "ALTER", "CHECK", "WITH", "WHERE"] {
        assert!(!upper.contains(banned), "{banned} is not allowed in migration 004");
    }

    let (mut tables, mut indexes) = (0, 0);
    for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let upper = statement.to_ascii_uppercase();
        if upper.starts_with("CREATE TABLE IF NOT EXISTS ") {
            tables += 1;
        } else if upper.starts_with("CREATE INDEX IF NOT EXISTS ")
            || upper.starts_with("CREATE UNIQUE INDEX IF NOT EXISTS ")
        {
            indexes += 1;
        } else {
            panic!("unexpected statement in migration 004: {statement}");
        }
    }
    assert_eq!((tables, indexes), (14, 16));
}

#[tokio::test]
async fn it_073_backup_pair_copied_and_counted() {
    let (tmp, store, _) = legacy_library().await;
    let counts_before = counted_rows(&store).await;
    let had_wal = wal_path(&store).exists();

    let outcome = store.ensure_schema(false).await.unwrap();
    let backup_dir = tmp.path().join(".ladon/backup");
    let expected = backup_dir.join("meta.turso.pre-004");
    match outcome {
        SchemaOutcome::Applied { backup: BackupState::Verified { path } } => assert_eq!(path, expected),
        other => panic!("unexpected outcome {other:?}"),
    }

    let copy = Store::new(expected);
    assert_eq!(counted_rows(&copy).await, counts_before);
    if had_wal {
        assert!(backup_dir.join("meta.turso.pre-004-wal").exists());
    }

    let payload = migration_payload(&store).await;
    assert_eq!(payload["version"], 4);
    assert_eq!(payload["backup"], "verified");
    assert_eq!(payload["backup_verified"], true);
    assert!(payload["backup_reason"].is_null());
}

#[tokio::test]
async fn it_073_unverified_backup_refuses_then_accepts_with_flag() {
    let (tmp, store, _) = legacy_library().await;
    // A directory at the copy target makes the copy fail, so the backup cannot be verified.
    std::fs::create_dir_all(tmp.path().join(".ladon/backup/meta.turso.pre-004")).unwrap();

    let err = store.ensure_schema(false).await.unwrap_err();
    assert!(err.to_string().starts_with("E_MIGRATION_BACKUP"), "{err}");
    assert!(err.downcast_ref::<MigrationBackupUnverified>().is_some());
    let conn = raw(&store).await;
    assert_eq!(recorded_versions(&conn).await, vec![2]);
    assert!(!table_exists(&conn, "doc_task_events").await);
    drop(conn);

    let reason = match store.ensure_schema(true).await.unwrap() {
        SchemaOutcome::Applied { backup: BackupState::Unverified { reason } } => reason,
        other => panic!("unexpected outcome {other:?}"),
    };
    assert!(!reason.is_empty());

    let conn = raw(&store).await;
    assert_eq!(recorded_versions(&conn).await, vec![2, 4]);
    drop(conn);
    let payload = migration_payload(&store).await;
    assert_eq!(payload["backup"], "unverified");
    assert_eq!(payload["backup_verified"], false);
    assert_eq!(payload["backup_reason"], reason.as_str());
}

#[tokio::test]
async fn it_073_empty_library_has_no_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();

    let outcome = store.ensure_schema(false).await.unwrap();
    assert!(matches!(outcome, SchemaOutcome::Applied { backup: BackupState::NotNeeded }), "{outcome:?}");
    assert!(!tmp.path().join(".ladon/backup").exists());

    let payload = migration_payload(&store).await;
    assert_eq!(payload["backup"], "not_needed");
    assert_eq!(payload["backup_verified"], true);
}

#[tokio::test]
async fn it_048_rows_affected_transactions_and_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let conn = raw(&store).await;
    conn.execute("CREATE TABLE probe_rows (id TEXT PRIMARY KEY, v INTEGER NOT NULL)", ()).await.unwrap();
    conn.execute("INSERT INTO probe_rows(id,v) VALUES('a',1)", ()).await.unwrap();

    let updated = conn.execute("UPDATE probe_rows SET v=2 WHERE id='a'", ()).await.unwrap();
    let not_matched = conn.execute("UPDATE probe_rows SET v=3 WHERE id='zz'", ()).await.unwrap();
    let ignored = conn.execute("INSERT OR IGNORE INTO probe_rows(id,v) VALUES('a',9)", ()).await.unwrap();
    eprintln!("IT-048 probe: rows_affected update_one={updated} update_none={not_matched} insert_or_ignore_dup={ignored}");

    conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    conn.execute("UPDATE probe_rows SET v=5 WHERE id='a'", ()).await.unwrap();
    conn.execute("ROLLBACK", ()).await.unwrap();
    let after_rollback = scalar_i64(&conn, "SELECT v FROM probe_rows WHERE id='a'").await;
    eprintln!("IT-048 probe: begin_immediate_rollback_value={after_rollback}");
    assert_eq!(after_rollback, 2, "ROLLBACK must restore the committed value");
}

#[tokio::test]
async fn it_048_check_foreign_key_and_unique_observations() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let conn = raw(&store).await;
    conn.execute("PRAGMA foreign_keys = ON", ()).await.unwrap();

    conn.execute("CREATE TABLE probe_check (n INTEGER CHECK (n > 0))", ()).await.unwrap();
    let check = conn.execute("INSERT INTO probe_check(n) VALUES(-1)", ()).await;
    eprintln!("IT-048 probe: check_violation={}", match &check {
        Ok(n) => format!("accepted rows={n}"),
        Err(e) => format!("rejected variant={} text={e}", variant(e)),
    });

    conn.execute("CREATE TABLE probe_parent (id TEXT PRIMARY KEY)", ()).await.unwrap();
    conn.execute("CREATE TABLE probe_child (p TEXT REFERENCES probe_parent(id))", ()).await.unwrap();
    let fk = conn.execute("INSERT INTO probe_child(p) VALUES('missing')", ()).await;
    eprintln!("IT-048 probe: foreign_key_violation={}", match &fk {
        Ok(n) => format!("accepted rows={n}"),
        Err(e) => format!("rejected variant={} text={e}", variant(e)),
    });

    conn.execute("CREATE TABLE probe_uniq (id TEXT PRIMARY KEY, k TEXT)", ()).await.unwrap();
    conn.execute("CREATE UNIQUE INDEX probe_uniq_k ON probe_uniq(k)", ()).await.unwrap();
    conn.execute("INSERT INTO probe_uniq(id,k) VALUES('1','x')", ()).await.unwrap();
    let dup = conn.execute("INSERT INTO probe_uniq(id,k) VALUES('2','x')", ()).await;
    let dup_text = match &dup {
        Ok(n) => format!("accepted rows={n}"),
        Err(e) => format!("rejected variant={} text={e}", variant(e)),
    };
    eprintln!("IT-048 probe: unique_index_violation={dup_text}");
    assert!(dup.is_err(), "a unique index violation must fail at insert time");
}

#[tokio::test]
async fn it_048_autoincrement_after_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let conn = raw(&store).await;
    conn.execute("CREATE TABLE probe_seq (seq INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT NOT NULL)", ()).await.unwrap();
    conn.execute("INSERT INTO probe_seq(v) VALUES('first')", ()).await.unwrap();
    conn.execute("BEGIN IMMEDIATE", ()).await.unwrap();
    conn.execute("INSERT INTO probe_seq(v) VALUES('rolled back')", ()).await.unwrap();
    conn.execute("ROLLBACK", ()).await.unwrap();
    conn.execute("INSERT INTO probe_seq(v) VALUES('second')", ()).await.unwrap();

    let seqs = scalar_text(&conn, "SELECT group_concat(seq) FROM (SELECT seq FROM probe_seq ORDER BY seq)").await;
    eprintln!("IT-048 probe: autoincrement_seq_after_rollback={seqs}");
}

#[tokio::test]
async fn it_048_cross_process_lock_observation() {
    // The holder keeps the database open; the child process is this test binary, re-run with an
    // environment variable that makes it attempt the open and report what it got.
    if let Ok(path) = std::env::var("LADON_IT048_CHILD_DB") {
        let outcome = async {
            let db = turso::Builder::new_local(&path).build().await?;
            db.connect()?;
            Ok::<(), turso::Error>(())
        }
        .await;
        match outcome {
            Ok(()) => eprintln!("IT-048 child: opened without error"),
            Err(e) => eprintln!("IT-048 child: variant={} text={e}", variant(&e)),
        }
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let holder = turso::Builder::new_local(&store.path.to_string_lossy()).build().await.unwrap();
    let _held = holder.connect().unwrap();

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "it_048_cross_process_lock_observation", "--nocapture", "--test-threads=1"])
        .env("LADON_IT048_CHILD_DB", store.path.to_string_lossy().to_string())
        .output()
        .unwrap();
    assert!(output.status.success(), "child process failed");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let line = text.lines().find(|l| l.contains("IT-048 child:")).unwrap_or("IT-048 child: no line reported");
    eprintln!("IT-048 probe: cross_process {line}");
}

#[tokio::test]
async fn it_1050_an_existing_backup_is_never_overwritten() {
    let (tmp, store, _) = legacy_library().await;
    let backup_dir = tmp.path().join(".ladon/backup");
    std::fs::create_dir_all(&backup_dir).unwrap();
    std::fs::write(backup_dir.join("meta.turso.pre-004"), b"earlier backup").unwrap();

    match store.ensure_schema(false).await.unwrap() {
        SchemaOutcome::Applied { backup: BackupState::Verified { path } } => {
            assert_eq!(path, backup_dir.join("meta.turso.pre-004-2"))
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    assert_eq!(std::fs::read(backup_dir.join("meta.turso.pre-004")).unwrap(), b"earlier backup");
}