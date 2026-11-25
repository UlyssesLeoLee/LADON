//! Integration tests for the document-task store (WP05): immediate transactions, counters,
//! compare-and-set transitions, the governance chain and the legacy backfill (IT-049, IT-050).
use ladon_explorer::{
    doctask_model::{
        sha256_hex, ActorKind, AnchorState, DocError, ErrorCode, ProposalState, SourceKind,
        Visibility, GENESIS_PREV_HASH,
    },
    doctask_store::{BackfillReport, DocStore, NewEvent, NewTask},
    store::{PendingVersion, Store},
};
use serde_json::{json, Value as Json};
use turso::Value;

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

/// A library migrated to 004 with one file and one version, ready for document-task writes.
struct Fresh {
    _tmp: tempfile::TempDir,
    store: Store,
    ds: DocStore,
    file_id: String,
    version_id: String,
}

async fn fresh() -> Fresh {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.ensure_schema(false).await.unwrap();
    let (file_id, version_id, _) = store.append_version(pending("a.md", "h1")).await.unwrap();
    let ds = DocStore::new(store.clone());
    Fresh {
        _tmp: tmp,
        store,
        ds,
        file_id,
        version_id,
    }
}

/// A library in the 0.0.3 shape with two legacy tasks (the second created first, by raw
/// `created_at`), two annotations (one on `H1:A#1`, one on `ROOT`) and one governance event for
/// each. Returns the store, the file id, the task ids `(title one, title two)`.
async fn legacy_fixture() -> (tempfile::TempDir, Store, String, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    let (file_id, version_id, _) = store.append_version(pending("a.md", "h1")).await.unwrap();
    store
        .add_annotation(&file_id, "H1:A#1", &version_id, "Secret Note Beta")
        .await
        .unwrap();
    store
        .add_annotation(&file_id, "ROOT", &version_id, "root note")
        .await
        .unwrap();
    let t1 = store
        .create_task(&file_id, "H1:A#1", &version_id, "Secret Title Alpha", "d")
        .await
        .unwrap();
    let t2 = store
        .create_task(&file_id, "H1:A#1", &version_id, "Title two", "d")
        .await
        .unwrap();
    let conn = raw(&store).await;
    conn.execute(
        &format!("UPDATE doc_tasks SET created_at = '2026-03-02T00:00:00+00:00' WHERE id = '{t1}'"),
        (),
    )
    .await
    .unwrap();
    conn.execute(
        &format!("UPDATE doc_tasks SET created_at = '2026-03-01T00:00:00+00:00' WHERE id = '{t2}'"),
        (),
    )
    .await
    .unwrap();
    (tmp, store, file_id, t1, t2)
}

/// A separate raw connection, used to simulate what the store API must reject or detect.
async fn raw(store: &Store) -> turso::Connection {
    let db = turso::Builder::new_local(&store.path.to_string_lossy())
        .build()
        .await
        .unwrap();
    db.connect().unwrap()
}

async fn scalar_i64(conn: &turso::Connection, sql: &str) -> i64 {
    let mut rows = conn.query(sql, ()).await.unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

async fn scalar_text(conn: &turso::Connection, sql: &str) -> String {
    let mut rows = conn.query(sql, ()).await.unwrap();
    rows.next()
        .await
        .unwrap()
        .unwrap()
        .get::<String>(0)
        .unwrap()
}

fn code_of<T>(result: anyhow::Result<T>) -> Option<ErrorCode> {
    result
        .err()
        .and_then(|err| err.downcast_ref::<DocError>().map(|doc| doc.code))
}

fn task(file_id: &str, anchor_id: &str, title: &str) -> NewTask {
    NewTask {
        file_id: file_id.into(),
        anchor_id: anchor_id.into(),
        title: title.into(),
        description: "description".into(),
        owner: String::new(),
        priority: 2,
        proposal_state: ProposalState::Accepted,
        visibility: Visibility::Shared,
        source_kind: SourceKind::Cli,
        created_by: "owner".into(),
        created_by_kind: ActorKind::Human,
        session: String::new(),
    }
}

fn event(
    kind: &str,
    task_id: &str,
    file_id: &str,
    from: &str,
    to: &str,
    payload: &str,
) -> NewEvent {
    NewEvent {
        task_id: task_id.into(),
        file_id: file_id.into(),
        subject_kind: "task".into(),
        subject_id: task_id.into(),
        kind: kind.into(),
        from_status: from.into(),
        to_status: to.into(),
        actor: "tester".into(),
        actor_kind: ActorKind::Human,
        session: String::new(),
        claimed_session: String::new(),
        payload_json: payload.into(),
    }
}

#[tokio::test]
async fn it_049_second_statement_failure_rolls_back_first() {
    let f = fresh().await;
    let anchor =
        f.ds.ensure_anchor(&f.file_id, &f.version_id, "H1:A#1", AnchorState::Valid)
            .await
            .unwrap();
    f.ds.create_task(&task(&f.file_id, &anchor, "Title one"))
        .await
        .unwrap();

    // The next task number is taken again, so the meta insert violates UNIQUE(task_no) after the
    // doc_tasks insert has already succeeded inside the same transaction.
    let conn = raw(&f.store).await;
    conn.execute(
        "UPDATE doc_counters SET value = 0 WHERE name = 'task_no'",
        (),
    )
    .await
    .unwrap();
    let err =
        f.ds.create_task(&task(&f.file_id, &anchor, "Title two"))
            .await
            .unwrap_err();
    assert!(
        err.to_string().to_uppercase().contains("UNIQUE"),
        "unexpected error: {err}"
    );

    assert_eq!(scalar_i64(&conn, "SELECT COUNT(*) FROM doc_tasks").await, 1);
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM doc_tasks WHERE title = 'Title two'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT value FROM doc_counters WHERE name = 'task_no'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM doc_task_events WHERE kind = 'dt.task.created'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn it_049_cas_admits_one_winner() {
    let f = fresh().await;
    let anchor =
        f.ds.ensure_anchor(&f.file_id, &f.version_id, "H1:A#1", AnchorState::Valid)
            .await
            .unwrap();
    let created =
        f.ds.create_task(&task(&f.file_id, &anchor, "Title"))
            .await
            .unwrap();
    let first = f.ds.clone();
    let second = f.ds.clone();
    let open_to_analysis = event(
        "dt.task.transitioned",
        &created.task_id,
        &f.file_id,
        "open",
        "analysis",
        "{}",
    );

    let (a, b) = tokio::join!(
        first.transition(&open_to_analysis),
        second.transition(&open_to_analysis)
    );
    let outcomes = [a, b];
    assert_eq!(
        outcomes.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one concurrent transition wins"
    );
    let loser = outcomes.into_iter().find_map(|r| r.err()).unwrap();
    assert_eq!(code_of::<()>(Err(loser)), Some(ErrorCode::Conflict));

    let stale = event(
        "dt.task.transitioned",
        &created.task_id,
        &f.file_id,
        "open",
        "working",
        "{}",
    );
    assert_eq!(
        code_of(f.ds.transition(&stale).await),
        Some(ErrorCode::Conflict)
    );

    let next = event(
        "dt.task.transitioned",
        &created.task_id,
        &f.file_id,
        "analysis",
        "working",
        "{}",
    );
    assert!(f.ds.transition(&next).await.is_ok());

    let missing = event(
        "dt.task.transitioned",
        "no-such-task",
        &f.file_id,
        "open",
        "analysis",
        "{}",
    );
    assert_eq!(
        code_of(f.ds.transition(&missing).await),
        Some(ErrorCode::NotFound)
    );
    let bogus = event(
        "dt.task.transitioned",
        &created.task_id,
        &f.file_id,
        "working",
        "bogus",
        "{}",
    );
    assert_eq!(
        code_of(f.ds.transition(&bogus).await),
        Some(ErrorCode::InvalidArg)
    );

    let conn = raw(&f.store).await;
    assert_eq!(
        scalar_text(&conn, "SELECT status FROM doc_tasks").await,
        "working"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM doc_task_events WHERE kind = 'dt.task.transitioned'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn it_049_chain_is_linear_from_genesis() {
    let f = fresh().await;
    let before = f.ds.events().await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].kind, "dt.migration.applied");
    assert_eq!(before[0].prev_hash, GENESIS_PREV_HASH);

    for kind in ["t.a", "t.b", "t.c"] {
        f.ds.append_event(&event(kind, "", &f.file_id, "", "", "{}"))
            .await
            .unwrap();
    }
    let events = f.ds.events().await.unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].prev_hash, "0".repeat(64));
    for pair in events.windows(2) {
        assert_eq!(pair[1].prev_hash, pair[0].hash);
    }
    assert_eq!(f.ds.verify_chain().await.unwrap(), None);
}

#[tokio::test]
async fn it_049_tampered_payload_and_deleted_event_are_detected() {
    let altered = fresh().await;
    for kind in ["t.a", "t.b", "t.c"] {
        altered
            .ds
            .append_event(&event(kind, "", &altered.file_id, "", "", "{}"))
            .await
            .unwrap();
    }
    let events = altered.ds.events().await.unwrap();
    let second = events[2].seq;
    let conn = raw(&altered.store).await;
    conn.execute(
        &format!("UPDATE doc_task_events SET payload_json = '{{\"x\":1}}' WHERE seq = {second}"),
        (),
    )
    .await
    .unwrap();
    assert_eq!(altered.ds.verify_chain().await.unwrap(), Some(second));

    let deleted = fresh().await;
    for kind in ["t.a", "t.b", "t.c"] {
        deleted
            .ds
            .append_event(&event(kind, "", &deleted.file_id, "", "", "{}"))
            .await
            .unwrap();
    }
    let events = deleted.ds.events().await.unwrap();
    let removed = events[2].seq;
    let successor = events[3].seq;
    let conn = raw(&deleted.store).await;
    conn.execute(
        &format!("DELETE FROM doc_task_events WHERE seq = {removed}"),
        (),
    )
    .await
    .unwrap();
    assert_eq!(deleted.ds.verify_chain().await.unwrap(), Some(successor));
}

#[tokio::test]
async fn it_049_two_connections_append_an_intact_chain() {
    let f = fresh().await;
    let first = f.ds.clone();
    let second = f.ds.clone();
    let file_id = f.file_id.clone();
    let writer_a = async {
        for i in 0..5 {
            first
                .append_event(&event(
                    "t.a",
                    "",
                    &file_id,
                    "",
                    "",
                    &format!("{{\"i\":{i}}}"),
                ))
                .await
                .unwrap();
        }
    };
    let writer_b = async {
        for i in 0..5 {
            second
                .append_event(&event(
                    "t.b",
                    "",
                    &file_id,
                    "",
                    "",
                    &format!("{{\"i\":{i}}}"),
                ))
                .await
                .unwrap();
        }
    };
    tokio::join!(writer_a, writer_b);

    let events = f.ds.events().await.unwrap();
    assert_eq!(events.len(), 11);
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    assert_eq!(f.ds.verify_chain().await.unwrap(), None);
}

#[tokio::test]
async fn it_049_prev_hash_unique_index_rejects_a_second_successor() {
    let f = fresh().await;
    let conn = raw(&f.store).await;
    let insert = format!(
        "INSERT INTO doc_task_events (id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp, prev_hash, hash) \
         VALUES ('dup-successor', '', '', 'schema', '4', 'dt.test', '', '', 'x', 'system', '', '', '{{}}', '2026-01-01T00:00:00+00:00', '{GENESIS_PREV_HASH}', 'h')"
    );
    let err = conn
        .execute(&insert, ())
        .await
        .expect_err("a second successor of the genesis must be rejected");
    let text = err.to_string();
    assert!(
        text.contains("UNIQUE constraint failed"),
        "unexpected error: {text}"
    );
    assert!(text.contains("prev_hash"), "unexpected error: {text}");
    assert_eq!(
        scalar_i64(&conn, "SELECT COUNT(*) FROM doc_task_events").await,
        1
    );
}

#[tokio::test]
async fn it_049_counters_are_sequential_and_inputs_are_validated() {
    let f = fresh().await;
    assert_eq!(f.ds.next_counter("task_no").await.unwrap(), 1);
    assert_eq!(f.ds.next_counter("task_no").await.unwrap(), 2);
    assert_eq!(f.ds.next_counter("annotation_no").await.unwrap(), 1);
    assert_eq!(f.ds.next_counter("backfill_v4").await.unwrap(), 1);
    assert_eq!(
        code_of(f.ds.next_counter("resources").await),
        Some(ErrorCode::InvalidArg)
    );

    f.ds.insert_row(
        "doc_counters",
        &[("name", Value::from("probe")), ("value", Value::from(5i64))],
    )
    .await
    .unwrap();
    let conn = raw(&f.store).await;
    assert_eq!(
        scalar_i64(&conn, "SELECT value FROM doc_counters WHERE name = 'probe'").await,
        5
    );

    let legacy =
        f.ds.insert_row("resources", &[("id", Value::from("x"))])
            .await;
    assert_eq!(code_of(legacy), Some(ErrorCode::InvalidArg));
    let injected =
        f.ds.insert_row(
            "doc_counters",
            &[
                ("name; DROP TABLE x", Value::from("p")),
                ("value", Value::from(1i64)),
            ],
        )
        .await;
    assert_eq!(code_of(injected), Some(ErrorCode::InvalidArg));
    let empty: &[(&str, Value)] = &[];
    assert_eq!(
        code_of(f.ds.insert_row("doc_counters", empty).await),
        Some(ErrorCode::InvalidArg)
    );
}

#[tokio::test]
async fn it_050_backfill_numbers_tasks_by_creation_and_marks_local_only() {
    let (_tmp, store, _file_id, t1, t2) = legacy_fixture().await;
    store.ensure_schema(false).await.unwrap();
    let ds = DocStore::new(store.clone());

    let report: BackfillReport = ds.backfill().await.unwrap();
    assert_eq!(report.anchors_created, 2, "H1:A#1 and ROOT");
    assert_eq!(report.tasks_imported, 2);
    assert_eq!(report.annotations_imported, 2);
    assert_eq!(
        report.events_imported, 4,
        "two annotation and two task governance events"
    );
    assert_eq!(
        report.status_events_added, 2,
        "one dt.task.imported per legacy task"
    );

    let conn = raw(&store).await;
    let number_one = scalar_i64(
        &conn,
        &format!("SELECT task_no FROM doc_task_meta WHERE task_id = '{t1}'"),
    )
    .await;
    let number_two = scalar_i64(
        &conn,
        &format!("SELECT task_no FROM doc_task_meta WHERE task_id = '{t2}'"),
    )
    .await;
    assert!(
        number_two < number_one,
        "the task created first in time gets the smaller number"
    );

    let legacy_tasks = "SELECT COUNT(*) FROM doc_task_meta WHERE visibility = 'local_only' AND source_kind = 'legacy' \
                        AND created_by = 'human' AND created_by_kind = 'unknown'";
    assert_eq!(scalar_i64(&conn, legacy_tasks).await, 2);
    let legacy_notes = "SELECT COUNT(*) FROM doc_annotation_meta WHERE visibility = 'local_only' \
                        AND author = 'human' AND author_kind = 'unknown'";
    assert_eq!(scalar_i64(&conn, legacy_notes).await, 2);
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT COUNT(*) FROM doc_tasks WHERE status = 'open'"
        )
        .await,
        2
    );

    assert_eq!(
        scalar_text(
            &conn,
            "SELECT state FROM doc_anchors WHERE section_key = 'H1:A#1'"
        )
        .await,
        "unchecked"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT level FROM doc_anchors WHERE section_key = 'H1:A#1'"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_text(
            &conn,
            "SELECT title FROM doc_anchors WHERE section_key = 'H1:A#1'"
        )
        .await,
        "A"
    );
    assert_eq!(
        scalar_i64(
            &conn,
            "SELECT level FROM doc_anchors WHERE section_key = 'ROOT'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_text(
            &conn,
            "SELECT title FROM doc_anchors WHERE section_key = 'ROOT'"
        )
        .await,
        "（全文）"
    );
    assert_eq!(
        scalar_text(
            &conn,
            "SELECT state FROM doc_anchors WHERE section_key = 'ROOT'"
        )
        .await,
        "unchecked"
    );
    let linked = "SELECT COUNT(*) FROM doc_task_meta m JOIN doc_anchors a ON a.id = m.anchor_id WHERE a.section_key = 'H1:A#1'";
    assert_eq!(scalar_i64(&conn, linked).await, 2);
}

#[tokio::test]
async fn it_050_backfill_payloads_follow_the_allow_list() {
    let (_tmp, store, file_id, t1, t2) = legacy_fixture().await;
    store
        .add_governance_event(&file_id, "task.state_changed", &t1, "working", "human")
        .await
        .unwrap();
    store
        .add_governance_event(&file_id, "task.state_changed", &t2, "not-a-state", "human")
        .await
        .unwrap();
    store.ensure_schema(false).await.unwrap();
    let ds = DocStore::new(store.clone());
    ds.backfill().await.unwrap();

    let events = ds.events().await.unwrap();
    for e in &events {
        for field in [
            &e.task_id,
            &e.file_id,
            &e.subject_id,
            &e.kind,
            &e.actor,
            &e.payload_json,
            &e.from_status,
            &e.to_status,
        ] {
            assert!(
                !field.contains("Secret Title Alpha"),
                "title text leaked into {e:?}"
            );
            assert!(
                !field.contains("Secret Note Beta"),
                "annotation text leaked into {e:?}"
            );
        }
        if e.kind.starts_with("legacy.") {
            assert!(e.id.starts_with("legacy:"), "legacy id prefix: {}", e.id);
            assert_eq!(e.actor_kind, "unknown");
        }
    }

    let working = events
        .iter()
        .find(|e| e.kind == "legacy.task.state_changed" && e.task_id == t1)
        .unwrap();
    assert_eq!(
        working.payload_json,
        r#"{"legacy":true,"payload":"working"}"#
    );
    assert_eq!(working.to_status, "working");

    let bogus = events
        .iter()
        .find(|e| e.kind == "legacy.task.state_changed" && e.task_id == t2)
        .unwrap();
    let body: Json = serde_json::from_str(&bogus.payload_json).unwrap();
    assert_eq!(body["legacy"], json!(true));
    assert_eq!(body["sha256"], json!(sha256_hex(b"not-a-state")));
    assert_eq!(body["bytes"], json!(11));
    assert_eq!(bogus.to_status, "");

    let title = events
        .iter()
        .find(|e| e.kind == "legacy.task.created" && e.task_id == t1)
        .unwrap();
    let body: Json = serde_json::from_str(&title.payload_json).unwrap();
    assert_eq!(
        body["sha256"],
        json!(sha256_hex("Secret Title Alpha".as_bytes()))
    );
    assert_eq!(body["bytes"], json!("Secret Title Alpha".len()));
    let conn = raw(&store).await;
    let original = scalar_text(&conn, &format!("SELECT timestamp FROM governance_events WHERE kind = 'task.created' AND subject_id = '{t1}'")).await;
    assert_eq!(
        title.timestamp, original,
        "legacy events keep the original timestamp"
    );

    let imported = events
        .iter()
        .find(|e| e.kind == "dt.task.imported" && e.task_id == t2)
        .unwrap();
    assert_eq!(
        imported.payload_json,
        r#"{"source":"legacy","status":"open"}"#
    );
}

#[tokio::test]
async fn it_050_backfill_is_idempotent_and_reimports_status_drift() {
    let (_tmp, store, _file_id, _t1, t2) = legacy_fixture().await;
    store.ensure_schema(false).await.unwrap();
    let ds = DocStore::new(store.clone());

    let first = ds.backfill().await.unwrap();
    assert_eq!(first.status_events_added, 2);
    let count_after_first = ds.events().await.unwrap().len();

    let second = ds.backfill().await.unwrap();
    assert_eq!(
        second,
        BackfillReport::default(),
        "a repeated run imports nothing"
    );
    assert_eq!(ds.events().await.unwrap().len(), count_after_first);
    assert!(ds.status_drift().await.unwrap().is_empty());

    // An earlier binary changes a status and writes no event.
    let conn = raw(&store).await;
    conn.execute(
        &format!("UPDATE doc_tasks SET status = 'analysis' WHERE id = '{t2}'"),
        (),
    )
    .await
    .unwrap();
    let drift = ds.status_drift().await.unwrap();
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].task_id, t2);
    assert_eq!(drift[0].status, "analysis");
    assert_eq!(drift[0].replayed.as_deref(), Some("open"));

    let third = ds.backfill().await.unwrap();
    assert_eq!(third.status_events_added, 1);
    assert!(ds.status_drift().await.unwrap().is_empty());
    let events = ds.events().await.unwrap();
    let last = events.last().unwrap();
    assert_eq!(last.kind, "dt.task.imported");
    assert_eq!(last.to_status, "analysis");
    assert_eq!(ds.verify_chain().await.unwrap(), None);
}
