//! Integration tests for the document-task reconciliation (WP08): reconciliation of anchors with
//! the latest version, the owner's decisions on moved anchors with their expected subjects, the
//! deleted and cold states, idempotence, legacy fingerprints, rebase, and file-move lineage
//! (IT-055, IT-056, TC-900 to TC-909).
use ladon_explorer::{
    doctask_flow::{DocFlow, PlanSpec},
    doctask_model::{evidence_hash, DocError, ErrorCode, SourceKind},
    doctask_outline,
    doctask_reconcile::Reconciler,
    doctask_service::{Caller, DocTaskService, Shown, TaskSpec},
    doctask_store::{AnchorRecord, DocStore, LineageRow},
    engine::Engine,
};
use std::{fs, path::Path};

/// A body of two non-empty lines, long enough for the relocation floor.
const PARA: &str = "Alpha clause body sentence one is long enough.\nAlpha clause body sentence two follows here.\n";

fn owner() -> Caller {
    Caller::human("owner", SourceKind::Cli)
}

/// Writes `name` with `text` under the engine root, records it, and returns the file and version ids.
async fn capture(e: &Engine, root: &Path, name: &str, text: &str) -> (String, String) {
    fs::write(root.join(name), text).unwrap();
    let r = e.capture(&root.join(name), "human", "test").await.unwrap();
    (r.file_id, r.version_id.unwrap())
}

/// Registers the given version as the baseline of its file.
async fn baseline(e: &Engine, file_id: &str, version_id: &str) {
    let hash = e
        .store
        .version(version_id)
        .await
        .unwrap()
        .unwrap()
        .content_hash;
    let svc = DocTaskService::open(e, owner()).await.unwrap();
    svc.register_baseline(
        file_id,
        None,
        "",
        &Shown {
            version_id: version_id.to_string(),
            hash,
        },
    )
    .await
    .unwrap();
}

/// Creates a task on `key` of `file_id` as the owner and returns its handle.
async fn make_task(e: &Engine, file_id: &str, key: &str, title: &str) -> String {
    let svc = DocTaskService::open(e, owner()).await.unwrap();
    svc.create_task(file_id, key, &TaskSpec::new(title))
        .await
        .unwrap()
        .handle
}

fn plan(summary: &str) -> PlanSpec {
    PlanSpec {
        summary: summary.to_string(),
        rationale: "why".to_string(),
        impact: "none".to_string(),
        ..PlanSpec::default()
    }
}

/// The error code of a failed call, awaited. Panics when the call succeeded.
macro_rules! code_of {
    ($call:expr) => {
        match $call.await {
            Ok(_) => panic!("expected a DocError, the call succeeded"),
            Err(err) => {
                err.downcast::<DocError>()
                    .expect("the error must be a DocError")
                    .code
            }
        }
    };
}

/// The anchor of the task with the given number.
async fn anchor_of_task(e: &Engine, task_no: i64) -> AnchorRecord {
    let ds = DocStore::new(e.store.clone());
    let task = ds
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .find(|task| task.task_no == task_no)
        .unwrap();
    ds.anchors()
        .await
        .unwrap()
        .into_iter()
        .find(|anchor| anchor.id == task.anchor_id)
        .unwrap()
}

/// `(kind, payload)` of every event of the chain, in `seq` order.
async fn chain(e: &Engine) -> Vec<(String, String)> {
    DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .map(|ev| (ev.kind, ev.payload_json))
        .collect()
}

async fn total(e: &Engine) -> usize {
    chain(e).await.len()
}

/// Events of one kind, with the payload parsed as JSON.
async fn events_of(e: &Engine, kind: &str) -> Vec<serde_json::Value> {
    chain(e)
        .await
        .into_iter()
        .filter(|(event_kind, _)| event_kind == kind)
        .map(|(_, payload)| serde_json::from_str(&payload).unwrap())
        .collect()
}

/// Raw statement on the database file, used to reach states that no public operation writes.
async fn raw(e: &Engine, sql: &str) {
    let db = turso::Builder::new_local(&e.store.path.to_string_lossy())
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    conn.execute(sql, ()).await.unwrap();
}

/// The evidence hash of design 4.5 for the task with the given number.
async fn evidence_subject(e: &Engine, task_no: i64, plan_hash: &str) -> String {
    let ds = DocStore::new(e.store.clone());
    let task_id = ds
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .find(|task| task.task_no == task_no)
        .unwrap()
        .id;
    let ids: Vec<String> = ds
        .evidence()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.task_id == task_id)
        .map(|row| row.id)
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    evidence_hash(plan_hash, &refs)
}

/// The section hash of `key` in `text`, as the outline of design 5.1 computes it.
fn section_hash(text: &str, key: &str) -> String {
    doctask_outline::outline(text.as_bytes(), "md")
        .find_key(key)
        .unwrap()
        .full_hash
        .clone()
}

/// Four sections `# S1` to `# S5` with distinct bodies; each index in `changed` gets another body.
fn sections(changed: &[usize]) -> String {
    let mut text = String::new();
    for index in 1..=5 {
        text.push_str(&format!("# S{index}\n"));
        if changed.contains(&index) {
            text.push_str(&format!(
                "Changed body of section {index} is another long sentence.\nChanged second line of section {index} here.\n"
            ));
        } else {
            text.push_str(&format!(
                "Body of section {index} sentence one is long enough.\nBody of section {index} sentence two follows.\n"
            ));
        }
    }
    text
}

fn lineage_of(rows: &[LineageRow], old: &str, new: &str) -> LineageRow {
    rows.iter()
        .find(|row| row.old_file_id == old && row.new_file_id == new)
        .cloned()
        .unwrap()
}

/// A renamed heading with the same body relocates by body hash, is recorded, and blocks the
/// approval until the owner acknowledges it (TC-900, IT-055).
#[tokio::test]
async fn tc900_renamed_heading_relocates_and_gates_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&handle).await.unwrap();
    let plan_hash = flow
        .submit_plan(&handle, &plan("change it"))
        .await
        .unwrap()
        .plan_hash;
    let (_, v2) = capture(&e, tmp.path(), "a.md", &format!("# Renamed\n{PARA}")).await;

    assert_eq!(
        code_of!(flow.approve(&handle, &plan_hash, None, "ok")),
        ErrorCode::AnchorBroken
    );
    let anchor = anchor_of_task(&e, 1).await;
    assert_eq!(anchor.state, "relocated");
    assert_eq!(anchor.current_key, "H1:Renamed#1");
    assert_eq!(anchor.current_version_id, v2);
    assert_eq!(anchor.reviewed_version_id, "");
    let changes = events_of(&e, "dt.anchor.reconciled").await;
    assert!(changes
        .iter()
        .any(|p| p["to_state"] == "relocated" && p["method"] == "body_hash"));

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    let step = reviewer
        .ack(&handle, &anchor.current_version_id, &anchor.current_hash)
        .await
        .unwrap();
    assert_eq!(step.state, "relocated");
    assert!(step.event_seq.is_some());

    flow.approve(&handle, &plan_hash, None, "ok").await.unwrap();
    let task = DocStore::new(e.store.clone())
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(task.status, "working");
    assert_eq!(
        task.base_version_id, v1,
        "acknowledgement never moves the base"
    );
}

/// A duplicate heading relocates by full hash; a body below the relocation floor is ambiguous
/// and is not relocated automatically (TC-901, IT-055).
#[tokio::test]
async fn tc901_duplicate_heading_uses_full_hash_and_short_body_is_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (a_id, a_v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &a_id, &a_v1).await;
    make_task(&e, &a_id, "key:H1:A#1", "Review A").await;
    let (s_id, s_v1) = capture(&e, tmp.path(), "s.md", "# Old\nShort\n").await;
    baseline(&e, &s_id, &s_v1).await;
    make_task(&e, &s_id, "key:H1:Old#1", "Review short").await;

    capture(
        &e,
        tmp.path(),
        "a.md",
        &format!("# A\nInserted text that differs from the original section here.\n# A\n{PARA}"),
    )
    .await;
    capture(&e, tmp.path(), "s.md", "# New\nShort\n").await;

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    reviewer.reconcile_all().await.unwrap();

    let duplicate = anchor_of_task(&e, 1).await;
    assert_eq!(duplicate.state, "relocated");
    assert_eq!(duplicate.current_key, "H1:A#2");
    let changed = events_of(&e, "dt.anchor.reconciled").await;
    assert!(changed
        .iter()
        .any(|p| p["to_key"] == "H1:A#2" && p["method"] == "full_hash"));

    let short = anchor_of_task(&e, 2).await;
    assert_eq!(short.state, "ambiguous");
    assert_eq!(short.current_key, "H1:Old#1");
    assert!(changed.iter().all(|p| p["method"] != "body_hash"
        || p["anchor_id"] != serde_json::Value::String(short.id.clone())));
}

/// A changed body needs review; acknowledgement with the shown subject allows the acceptance,
/// and the base version stays where it was (TC-902, IT-055).
#[tokio::test]
async fn tc902_changed_body_needs_review_ack_then_close() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&handle).await.unwrap();
    let plan_hash = flow
        .submit_plan(&handle, &plan("change it"))
        .await
        .unwrap()
        .plan_hash;
    flow.approve(&handle, &plan_hash, None, "ok").await.unwrap();
    flow.report_work(&handle, "implemented").await.unwrap();
    flow.add_evidence(&handle, "test_run", "pass", "ran the tests", None)
        .await
        .unwrap();

    let (_, v2) = capture(
        &e,
        tmp.path(),
        "a.md",
        "# A\nChanged alpha clause body text is long enough here.\nAnother changed sentence follows here.\n",
    )
    .await;
    let subject = evidence_subject(&e, 1, &plan_hash).await;
    assert_eq!(
        code_of!(flow.close(&handle, &subject, &[], None)),
        ErrorCode::AnchorBroken
    );
    let anchor = anchor_of_task(&e, 1).await;
    assert_eq!(anchor.state, "needs_review");
    assert_eq!(anchor.current_version_id, v2);

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    reviewer
        .ack(&handle, &anchor.current_version_id, &anchor.current_hash)
        .await
        .unwrap();
    flow.close(&handle, &subject, &[], None).await.unwrap();

    let after = anchor_of_task(&e, 1).await;
    assert_eq!(after.reviewed_version_id, v2);
    let task = DocStore::new(e.store.clone())
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(task.status, "done");
    assert_eq!(task.base_version_id, v1);
}

/// An orphaned anchor is reported without a write and relocates only with the shown version and
/// clause hash (TC-903, IT-055).
#[tokio::test]
async fn tc903_orphaned_relocates_only_with_matching_subject() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let both = format!("# A\n{PARA}# B\n{PARA}");
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &both).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;

    let b_text = format!("# B\n{PARA}");
    let (_, v2) = capture(&e, tmp.path(), "a.md", &b_text).await;
    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    reviewer.reconcile_all().await.unwrap();
    let anchor = anchor_of_task(&e, 1).await;
    assert_eq!(anchor.state, "orphaned");

    let before = total(&e).await;
    let lines = reviewer.report(None).await.unwrap();
    assert_eq!(total(&e).await, before, "the report writes nothing");
    let line = lines
        .iter()
        .find(|line| line.anchor_id == anchor.id)
        .unwrap();
    assert_eq!(line.state, "orphaned");
    assert_eq!(line.verdict_state, "orphaned");
    assert!(!line.would_change);

    let target_hash = section_hash(&b_text, "H1:B#1");
    assert_eq!(
        code_of!(reviewer.relocate(&handle, "key:H1:B#1", &v1, &target_hash)),
        ErrorCode::StaleSubject,
        "the version shown was the first one"
    );
    assert_eq!(
        code_of!(reviewer.relocate(&handle, "key:H1:B#1", &v2, "0000000000")),
        ErrorCode::StaleSubject
    );
    let step = reviewer
        .relocate(&handle, "key:H1:B#1", &v2, &target_hash)
        .await
        .unwrap();
    assert_eq!(step.state, "relocated");

    let after = anchor_of_task(&e, 1).await;
    assert_eq!(after.state, "relocated");
    assert_eq!(after.current_key, "H1:B#1");
    assert_eq!(after.reviewed_version_id, v2);
    assert_eq!(after.full_hash, target_hash);
    assert_eq!(events_of(&e, "dt.anchor.relocated").await.len(), 1);
}

/// The decisions refuse a stale subject, a changed live file, a non-owner caller and a state
/// that does not accept the decision (TC-904, IT-055).
#[tokio::test]
async fn tc904_decision_guards_refuse_stale_subject_live_change_and_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let (_, v2) = capture(
        &e,
        tmp.path(),
        "a.md",
        "# A\nChanged alpha clause body text is long enough here.\nAnother changed sentence follows here.\n",
    )
    .await;
    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    reviewer.reconcile_all().await.unwrap();
    let anchor = anchor_of_task(&e, 1).await;
    assert_eq!(anchor.state, "needs_review");
    let base_hash = e.store.version(&v1).await.unwrap().unwrap().content_hash;
    let agent = Reconciler::attach(&e, Caller::agent("launch-1"))
        .await
        .unwrap();

    let before = total(&e).await;
    assert_eq!(
        code_of!(agent.ack(&handle, &anchor.current_version_id, &anchor.current_hash)),
        ErrorCode::HumanOnly
    );
    assert_eq!(
        code_of!(reviewer.ack(&handle, &v1, &anchor.current_hash)),
        ErrorCode::StaleSubject
    );
    assert_eq!(
        code_of!(reviewer.ack(&handle, &v2, "0000000000")),
        ErrorCode::StaleSubject
    );
    assert_eq!(
        code_of!(reviewer.rebase(&handle, &v1, "0000000000")),
        ErrorCode::StaleSubject
    );
    assert_eq!(
        code_of!(reviewer.relocate(&handle, "key:H1:A#1", &v2, &anchor.current_hash)),
        ErrorCode::GuardFailed,
        "a changed section is not relocated"
    );
    assert_eq!(total(&e).await, before, "refused decisions write nothing");

    fs::write(
        tmp.path().join("a.md"),
        "# A\nOther text written without a capture here.\n",
    )
    .unwrap();
    assert_eq!(
        code_of!(reviewer.ack(&handle, &v2, &anchor.current_hash)),
        ErrorCode::FileChanged
    );
    assert_eq!(
        code_of!(reviewer.rebase(&handle, &v1, &base_hash)),
        ErrorCode::FileChanged
    );
    assert_eq!(anchor_of_task(&e, 1).await.reviewed_version_id, "");
}

/// A deleted file gives `file_missing`; the approval and the acknowledgement are refused, and the
/// key stays (TC-905, IT-055).
#[tokio::test]
async fn tc905_deleted_file_is_file_missing_and_refuses_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&handle).await.unwrap();
    let plan_hash = flow
        .submit_plan(&handle, &plan("change it"))
        .await
        .unwrap()
        .plan_hash;

    let file = e.store.find_path("a.md").await.unwrap().unwrap();
    e.store
        .mark_deleted(&file, "human", "removed")
        .await
        .unwrap();
    fs::remove_file(tmp.path().join("a.md")).unwrap();

    assert_eq!(
        code_of!(flow.approve(&handle, &plan_hash, None, "ok")),
        ErrorCode::AnchorBroken
    );
    let anchor = anchor_of_task(&e, 1).await;
    assert_eq!(anchor.state, "file_missing");
    assert_eq!(anchor.current_key, "H1:A#1");
    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    assert_eq!(
        code_of!(reviewer.ack(&handle, &anchor.current_version_id, &anchor.current_hash)),
        ErrorCode::GuardFailed
    );
}

/// Cold latest bytes give `unknown`: the key and the hash stay as they were, and nothing is
/// fabricated (TC-906, IT-055).
#[tokio::test]
async fn tc906_cold_latest_bytes_are_unknown_without_fabrication() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let before = anchor_of_task(&e, 1).await;
    assert!(
        !before.current_hash.is_empty(),
        "a new anchor carries its hash"
    );

    let (_, v2) = capture(
        &e,
        tmp.path(),
        "a.md",
        "# A\nChanged alpha clause body text is long enough here.\nAnother changed sentence follows here.\n",
    )
    .await;
    let v2_hash = e.store.version(&v2).await.unwrap().unwrap().content_hash;
    fs::remove_file(e.cas.local_object_path(&v2_hash).unwrap()).unwrap();

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    let report = reviewer.reconcile_all().await.unwrap();
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].to_state, "unknown");

    let after = anchor_of_task(&e, 1).await;
    assert_eq!(after.state, "unknown");
    assert_eq!(after.current_version_id, v2);
    assert_eq!(after.current_key, before.current_key);
    assert_eq!(after.current_hash, before.current_hash);
    assert_eq!(after.full_hash, before.full_hash);
    let payload = events_of(&e, "dt.anchor.reconciled").await;
    assert_eq!(payload.last().unwrap()["method"], "none");
}

/// Reconciliation is idempotent, fills the fingerprint of a legacy anchor, and rebase re-pins a
/// task to the baseline version (TC-907, IT-055).
#[tokio::test]
async fn tc907_reconcile_is_idempotent_fills_legacy_fingerprints_and_rebases() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = capture(&e, tmp.path(), "a.md", &format!("# A\n{PARA}")).await;
    baseline(&e, &file_id, &v1).await;
    let handle = make_task(&e, &file_id, "key:H1:A#1", "Review A").await;
    let reviewer = Reconciler::open(&e, owner()).await.unwrap();

    assert!(reviewer.reconcile_all().await.unwrap().changes.is_empty());
    let settled = total(&e).await;
    assert!(reviewer.reconcile_all().await.unwrap().changes.is_empty());
    assert_eq!(total(&e).await, settled, "a second run writes no event");

    let anchor = anchor_of_task(&e, 1).await;
    raw(
        &e,
        &format!(
            "UPDATE doc_anchors SET full_hash = '', body_hash = '', state = 'unchecked', current_hash = '' WHERE id = '{}'",
            anchor.id
        ),
    )
    .await;
    let filled = reviewer.reconcile_all().await.unwrap();
    assert_eq!(filled.changes.len(), 1);
    let legacy = anchor_of_task(&e, 1).await;
    assert_eq!(legacy.state, "valid");
    assert_eq!(legacy.full_hash, anchor.full_hash);
    assert_eq!(legacy.current_hash, anchor.full_hash);

    let (_, v2) = capture(
        &e,
        tmp.path(),
        "a.md",
        "# A\nChanged alpha clause body text is long enough here.\nAnother changed sentence follows here.\n",
    )
    .await;
    let v2_hash = e.store.version(&v2).await.unwrap().unwrap().content_hash;
    let svc = DocTaskService::open(&e, owner()).await.unwrap();
    svc.register_baseline(
        &file_id,
        None,
        "second baseline",
        &Shown {
            version_id: v2.clone(),
            hash: v2_hash.clone(),
        },
    )
    .await
    .unwrap();
    drop(svc);

    let step = reviewer.rebase(&handle, &v2, &v2_hash).await.unwrap();
    assert_ne!(step.anchor_id, legacy.id);
    assert!(step.event_seq.is_some());
    let task = DocStore::new(e.store.clone())
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(task.base_version_id, v2);
    assert_eq!(task.anchor_id, step.anchor_id);
    let again = reviewer.rebase(&handle, &v2, &v2_hash).await.unwrap();
    assert_eq!(again.event_seq, None);
    assert_eq!(again.anchor_id, step.anchor_id);
}

/// A move to a file with the same content is proposed, is confirmed only by the owner against
/// the shown ids and hash, and the task list then shows the new path (TC-908, IT-056).
#[tokio::test]
async fn tc908_same_content_move_needs_human_and_list_follows() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let text = format!("# A\n{PARA}");
    let (old_id, v1) = capture(&e, tmp.path(), "a.md", &text).await;
    baseline(&e, &old_id, &v1).await;
    make_task(&e, &old_id, "key:H1:A#1", "Review A").await;
    let file = e.store.find_path("a.md").await.unwrap().unwrap();
    e.store.mark_deleted(&file, "human", "moved").await.unwrap();
    fs::remove_file(tmp.path().join("a.md")).unwrap();
    let (new_id, _) = capture(&e, tmp.path(), "moved.md", &text).await;
    let new_hash = e
        .store
        .find_path("moved.md")
        .await
        .unwrap()
        .unwrap()
        .current_hash;

    let agent = Reconciler::attach(&e, Caller::agent("launch-1"))
        .await
        .unwrap();
    assert_eq!(code_of!(agent.scan_lineage()), ErrorCode::HumanOnly);
    assert_eq!(
        code_of!(agent.confirm_lineage("x", &old_id, &new_id, &new_hash)),
        ErrorCode::HumanOnly
    );

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    let proposed = reviewer.scan_lineage().await.unwrap();
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0].evidence, "same_hash");
    assert_eq!(proposed[0].score_permille, 1000);
    assert_eq!(proposed[0].state, "proposed");
    assert!(reviewer.scan_lineage().await.unwrap().is_empty());
    let row_id = proposed[0].id.clone();

    assert_eq!(
        code_of!(reviewer.confirm_lineage(&row_id, &old_id, &new_id, "0000000000")),
        ErrorCode::StaleSubject
    );
    let confirmed = reviewer
        .confirm_lineage(&row_id, &old_id, &new_id, &new_hash)
        .await
        .unwrap();
    assert_eq!(confirmed.state, "confirmed");
    assert_eq!(confirmed.decided_by, "owner");
    assert_eq!(
        code_of!(reviewer.confirm_lineage(&row_id, &old_id, &new_id, &new_hash)),
        ErrorCode::GuardFailed,
        "a confirmed move is not confirmed twice"
    );
    assert_eq!(events_of(&e, "dt.lineage.confirmed").await.len(), 1);

    let svc = DocTaskService::open(&e, owner()).await.unwrap();
    let listed = svc.list_tasks().await.unwrap();
    assert_eq!(listed.items[0].path, "moved.md");
    drop(svc);

    let report = reviewer.reconcile_all().await.unwrap();
    assert_eq!(report.changes.len(), 1);
    assert_eq!(anchor_of_task(&e, 1).await.state, "valid");
}

/// Section overlap proposes at 800 per mille and not at 600; a rejected pair is not proposed
/// again; only the owner rejects (TC-909, IT-056).
#[tokio::test]
async fn tc909_section_overlap_threshold_and_rejection_is_final() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (old_id, v1) = capture(&e, tmp.path(), "big.md", &sections(&[])).await;
    baseline(&e, &old_id, &v1).await;
    make_task(&e, &old_id, "key:H1:S1#1", "Review S1").await;
    let file = e.store.find_path("big.md").await.unwrap().unwrap();
    e.store.mark_deleted(&file, "human", "moved").await.unwrap();
    fs::remove_file(tmp.path().join("big.md")).unwrap();
    capture(&e, tmp.path(), "moved.md", &sections(&[5])).await;
    capture(&e, tmp.path(), "few.md", &sections(&[4, 5])).await;

    let reviewer = Reconciler::open(&e, owner()).await.unwrap();
    let proposed = reviewer.scan_lineage().await.unwrap();
    assert_eq!(
        proposed.len(),
        1,
        "only the four-of-five candidate is proposed"
    );
    assert_eq!(proposed[0].evidence, "section_overlap");
    assert_eq!(proposed[0].score_permille, 800);
    let moved_id = e.store.find_path("moved.md").await.unwrap().unwrap().id;
    assert_eq!(proposed[0].new_file_id, moved_id);

    let agent = Reconciler::attach(&e, Caller::agent("launch-1"))
        .await
        .unwrap();
    assert_eq!(
        code_of!(agent.reject_lineage(&proposed[0].id)),
        ErrorCode::HumanOnly
    );

    let rejected = reviewer.reject_lineage(&proposed[0].id).await.unwrap();
    assert_eq!(rejected.state, "rejected");
    assert_eq!(events_of(&e, "dt.lineage.rejected").await.len(), 1);
    let rows_after_reject = reviewer.lineage().await.unwrap().len();
    assert!(reviewer.scan_lineage().await.unwrap().is_empty());
    assert_eq!(reviewer.lineage().await.unwrap().len(), rows_after_reject);
    let rows = reviewer.lineage().await.unwrap();
    assert_eq!(lineage_of(&rows, &old_id, &moved_id).state, "rejected");
}
