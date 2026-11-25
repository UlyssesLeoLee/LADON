//! Integration tests for the document-task workflow (WP07): the transitions of design section 4.2
//! with their events, illegal moves, the acceptance rules, the proposal and agent limits,
//! dependencies, links, evidence, quotas, the schedule and verification (IT-052, IT-053, IT-057,
//! TC-880 to TC-899).
use async_trait::async_trait;
use ladon_explorer::{
    doctask_flow::{DocFlow, EditPlanSupport, EditSpec, EditSubject, PlanSpec},
    doctask_model::{
        evidence_hash, sha256_hex, ActorKind, DocError, EditKind, ErrorCode, ProposalState,
        SourceKind, Visibility,
    },
    doctask_service::{Caller, DocTaskService, Shown, TaskSpec},
    doctask_store::{DocStore, NewEvent, NewTask, RowWrite, StatusChange, StepEvent},
    engine::Engine,
    model::FileRecord,
};
use std::{fs, path::Path};
use turso::Value;

/// Writes `name` with `text` under the engine root, records it, and returns the file and version ids.
async fn write_capture(e: &Engine, root: &Path, name: &str, text: &str) -> (String, String) {
    fs::write(root.join(name), text).unwrap();
    let r = e.capture(&root.join(name), "human", "test").await.unwrap();
    (r.file_id, r.version_id.unwrap())
}

/// The file `a.md` with its baseline, and the file `b.md`.
async fn setup() -> (tempfile::TempDir, Engine, String, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = write_capture(&e, tmp.path(), "a.md", "# A\nbody\n").await;
    let (_other_file, other_v) = write_capture(&e, tmp.path(), "b.md", "# B\nbody\n").await;
    let content_hash = e.store.version(&v1).await.unwrap().unwrap().content_hash;
    let svc = DocTaskService::open(&e, owner()).await.unwrap();
    svc.register_baseline(
        &file_id,
        None,
        "",
        &Shown {
            version_id: v1.clone(),
            hash: content_hash,
        },
    )
    .await
    .unwrap();
    drop(svc);
    (tmp, e, file_id, v1, other_v)
}

fn owner() -> Caller {
    Caller::human("owner", SourceKind::Cli)
}

/// The error of a failed call, awaited. Panics when the call succeeded.
macro_rules! err {
    ($call:expr) => {
        match $call.await {
            Ok(_) => panic!("expected a DocError, the call succeeded"),
            Err(err) => err
                .downcast::<DocError>()
                .expect("the error must be a DocError"),
        }
    };
}

/// Creates a task on `a.md` section `H1:A#1` as the owner and returns its handle.
async fn make_task(e: &Engine, file_id: &str, title: &str) -> String {
    let svc = DocTaskService::open(e, owner()).await.unwrap();
    svc.create_task(file_id, "key:H1:A#1", &TaskSpec::new(title))
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

/// `(kind, from, to, actor kind)` of every event of the chain, in `seq` order.
async fn chain(e: &Engine) -> Vec<(String, String, String, String)> {
    DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .map(|ev| (ev.kind, ev.from_status, ev.to_status, ev.actor_kind))
        .collect()
}

/// The `(kind, from, to)` triples written after the first `before` events.
fn written(
    all: &[(String, String, String, String)],
    before: usize,
) -> Vec<(String, String, String)> {
    all[before..]
        .iter()
        .map(|(kind, from, to, _)| (kind.clone(), from.clone(), to.clone()))
        .collect()
}

fn triple(kind: &str, from: &str, to: &str) -> (String, String, String) {
    (kind.to_string(), from.to_string(), to.to_string())
}

async fn task_id_of(e: &Engine, task_no: i64) -> String {
    DocStore::new(e.store.clone())
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .find(|task| task.task_no == task_no)
        .unwrap()
        .id
}

/// The evidence hash of design 4.5 for a task: the plan hash and every evidence id of the task.
async fn subject(e: &Engine, task_no: i64, plan_hash: &str) -> String {
    let ds = DocStore::new(e.store.clone());
    let id = task_id_of(e, task_no).await;
    let ids: Vec<String> = ds
        .evidence()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.task_id == id)
        .map(|row| row.id)
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    evidence_hash(plan_hash, &refs)
}

async fn total(e: &Engine) -> usize {
    chain(e).await.len()
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

/// A fixed edit component: every plan gets the same subject fields.
struct Fixed {
    version: String,
    expected: String,
    result: String,
}

#[async_trait]
impl EditPlanSupport for Fixed {
    async fn prepare(
        &self,
        _target: &FileRecord,
        _section: &str,
        _kind: EditKind,
        _new_text: &str,
    ) -> anyhow::Result<EditSubject> {
        Ok(EditSubject {
            expected_version_id: self.version.clone(),
            expected_hash: self.expected.clone(),
            result_hash: self.result.clone(),
        })
    }
}

#[tokio::test]
async fn tc880_every_legal_edge_writes_its_events() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();

    let before = total(&e).await;
    let step = flow.start_analysis(&t1).await.unwrap();
    assert_eq!((step.from.as_str(), step.to.as_str()), ("open", "analysis"));
    assert_eq!(step.events.len(), 1);
    assert_eq!(
        written(&chain(&e).await, before),
        vec![triple("dt.task.transitioned", "open", "analysis")]
    );

    let before = total(&e).await;
    let submitted = flow.submit_plan(&t1, &plan("change it")).await.unwrap();
    assert!(!submitted.existing);
    assert_eq!(
        written(&chain(&e).await, before),
        vec![
            triple("dt.plan.submitted", "", ""),
            triple("dt.task.transitioned", "analysis", "approval"),
        ]
    );

    let before = total(&e).await;
    flow.approve(&t1, &submitted.plan_hash, None, "ok")
        .await
        .unwrap();
    assert_eq!(
        written(&chain(&e).await, before),
        vec![
            triple("dt.approval.recorded", "", ""),
            triple("dt.task.transitioned", "approval", "working"),
        ]
    );

    let before = total(&e).await;
    flow.report_work(&t1, "implemented").await.unwrap();
    assert_eq!(
        written(&chain(&e).await, before),
        vec![triple("dt.task.transitioned", "working", "verification")]
    );

    flow.add_evidence(&t1, "test_run", "pass", "ran the tests", None)
        .await
        .unwrap();
    let before = total(&e).await;
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    flow.close(&t1, &hash, &[], None).await.unwrap();
    assert_eq!(
        written(&chain(&e).await, before),
        vec![
            triple("dt.approval.recorded", "", ""),
            triple("dt.task.transitioned", "verification", "done"),
        ]
    );
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc881_illegal_edges_are_refused_without_writes() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    let before = total(&e).await;
    assert_eq!(
        err!(flow.approve(&t1, "x", None, "")).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(
        err!(flow.reopen(&t1, "why")).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(
        err!(flow.rework(&t1, "why")).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(
        err!(flow.report_work(&t1, "done")).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(
        err!(flow.submit_plan(&t1, &plan("x"))).code,
        ErrorCode::TransitionDenied
    );
    flow.start_analysis(&t1).await.unwrap();
    let after_start = total(&e).await;
    assert_eq!(
        err!(flow.start_analysis(&t1)).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(
        err!(flow.close(&t1, "x", &[], None)).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(total(&e).await, after_start);
    assert_eq!(after_start, before + 1);
}

#[tokio::test]
async fn tc882_a_failed_step_writes_nothing() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let ds = DocStore::new(e.store.clone());
    let task_id = task_id_of(&e, 1).await;
    let before = total(&e).await;
    let stale = StatusChange {
        task_id: task_id.clone(),
        from: ladon_explorer::doctask_model::TaskState::Approval,
        to: ladon_explorer::doctask_model::TaskState::Working,
    };
    let event = StepEvent {
        id: "00000000-0000-0000-0000-000000000001".to_string(),
        event: NewEvent {
            task_id: task_id.clone(),
            file_id: file_id.clone(),
            subject_kind: "task".to_string(),
            subject_id: task_id.clone(),
            kind: "dt.task.transitioned".to_string(),
            from_status: "approval".to_string(),
            to_status: "working".to_string(),
            actor: "owner".to_string(),
            actor_kind: ActorKind::Human,
            session: String::new(),
            claimed_session: String::new(),
            payload_json: "{}".to_string(),
        },
    };
    assert_eq!(
        err!(ds.commit_step(Some(&stale), &[], std::slice::from_ref(&event))).code,
        ErrorCode::Conflict
    );
    let missing = RowWrite::Update {
        table: "doc_task_plans",
        key: "id",
        key_value: Value::Text("missing".to_string()),
        values: vec![("status", Value::Text("approved".to_string()))],
    };
    let change = StatusChange {
        task_id: task_id.clone(),
        from: ladon_explorer::doctask_model::TaskState::Analysis,
        to: ladon_explorer::doctask_model::TaskState::Approval,
    };
    assert_eq!(
        err!(ds.commit_step(Some(&change), &[missing], &[event])).code,
        ErrorCode::NotFound
    );
    assert_eq!(total(&e).await, before);
    assert_eq!(ds.tasks().await.unwrap()[0].status, "analysis");
}

#[tokio::test]
async fn tc883_close_requires_evidence_and_dependencies() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let t2 = make_task(&e, &file_id, "Second").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    flow.approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    flow.report_work(&t1, "done").await.unwrap();
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    assert_eq!(
        err!(flow.close(&t1, &hash, &[], None)).code,
        ErrorCode::GuardFailed
    );
    flow.add_evidence(&t1, "note", "pass", "checked by hand", None)
        .await
        .unwrap();
    flow.add_dependency(&t1, &t2).await.unwrap();
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    assert_eq!(
        err!(flow.close(&t1, &hash, &[], None)).code,
        ErrorCode::GuardFailed
    );
    flow.remove_dependency(&t1, &t2).await.unwrap();
    flow.close(&t1, &hash, &[], None).await.unwrap();
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc884_agent_pass_needs_explicit_acceptance() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let owner_flow = DocFlow::open(&e, owner()).await.unwrap();
    owner_flow.start_analysis(&t1).await.unwrap();
    let submitted = owner_flow.submit_plan(&t1, &plan("p")).await.unwrap();
    owner_flow
        .approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    owner_flow.report_work(&t1, "done").await.unwrap();
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    agent
        .add_evidence(&t1, "test_run", "pass", "ran in the sandbox", None)
        .await
        .unwrap();
    let rows = DocStore::new(e.store.clone()).evidence().await.unwrap();
    let agent_row = rows
        .iter()
        .find(|row| row.actor_kind == ActorKind::Agent)
        .unwrap()
        .id
        .clone();
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    assert_eq!(
        err!(owner_flow.close(&t1, &hash, &[], None)).code,
        ErrorCode::GuardFailed
    );
    assert_eq!(
        err!(owner_flow.close(&t1, &hash, &["missing".to_string()], None)).code,
        ErrorCode::InvalidArg
    );
    owner_flow
        .close(&t1, &hash, std::slice::from_ref(&agent_row), None)
        .await
        .unwrap();
    let approval = chain(&e).await;
    assert!(approval
        .iter()
        .any(|(kind, _, _, _)| kind == "dt.approval.recorded"));
    let payloads: Vec<String> = DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "dt.approval.recorded")
        .map(|ev| ev.payload_json)
        .collect();
    assert!(payloads.last().unwrap().contains(&agent_row));
}

#[tokio::test]
async fn tc885_written_waiver_and_chain_order() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    flow.approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    flow.report_work(&t1, "done").await.unwrap();
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    assert_eq!(
        err!(flow.close(&t1, &hash, &[], Some("   "))).code,
        ErrorCode::InvalidArg
    );
    flow.close(&t1, &hash, &[], Some("accepted risk"))
        .await
        .unwrap();
    let waivers: Vec<_> = DocStore::new(e.store.clone())
        .evidence()
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == ladon_explorer::doctask_model::EvidenceKind::Waiver)
        .collect();
    assert_eq!(waivers.len(), 1);
    assert_eq!(waivers[0].actor_kind, ActorKind::Human);
    let last_transition = DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "dt.task.transitioned")
        .last()
        .unwrap();
    assert!(last_transition
        .payload_json
        .contains("\"evidence_waived\":true"));
}

#[tokio::test]
async fn tc886_agent_is_refused_every_human_operation() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let owner_flow = DocFlow::open(&e, owner()).await.unwrap();
    owner_flow.start_analysis(&t1).await.unwrap();
    let submitted = owner_flow.submit_plan(&t1, &plan("p")).await.unwrap();
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    let before = total(&e).await;
    let human_only = |code: ErrorCode| code == ErrorCode::HumanOnly;
    assert!(human_only(
        err!(agent.approve(&t1, &submitted.plan_hash, None, "")).code
    ));
    assert!(human_only(err!(agent.reject(&t1, "no")).code));
    assert!(human_only(err!(agent.close(&t1, "x", &[], None)).code));
    assert!(human_only(err!(agent.cancel(&t1, "no")).code));
    assert!(human_only(err!(agent.reopen(&t1, "no")).code));
    assert!(human_only(err!(agent.adopt(&t1)).code));
    assert!(human_only(err!(agent.dismiss(&t1, "no")).code));
    assert!(human_only(err!(agent.remove_dependency(&t1, &t1)).code));
    assert!(human_only(err!(agent.remove_link(&t1, "x")).code));
    assert!(human_only(err!(agent.inbox()).code));
    assert!(human_only(err!(agent.verify(None)).code));
    assert_eq!(
        err!(agent.start_analysis(&t1)).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(total(&e).await, before);
}

#[tokio::test]
async fn tc887_proposals_wait_for_the_owner() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let ds = DocStore::new(e.store.clone());
    let anchor = ds.tasks().await.unwrap()[0].anchor_id.clone();
    let proposal = |title: &str| NewTask {
        file_id: file_id.clone(),
        anchor_id: anchor.clone(),
        title: title.to_string(),
        description: String::new(),
        owner: String::new(),
        priority: 2,
        proposal_state: ProposalState::Proposed,
        visibility: Visibility::Shared,
        source_kind: SourceKind::Hermes,
        created_by: "hermes".to_string(),
        created_by_kind: ActorKind::Agent,
        session: "hermes:L1".to_string(),
    };
    ds.create_task(&proposal("First idea")).await.unwrap();
    ds.create_task(&proposal("Second idea")).await.unwrap();
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    let owner_flow = DocFlow::open(&e, owner()).await.unwrap();
    let before = total(&e).await;
    let refused = err!(agent.start_analysis("T-1"));
    assert_eq!(refused.code, ErrorCode::TransitionDenied);
    assert!(refused.hint_zh.contains("owner must adopt"));
    assert_eq!(total(&e).await, before);
    owner_flow.adopt("T-1").await.unwrap();
    agent.start_analysis("T-1").await.unwrap();
    let owner_task = owner_flow.inbox().await.unwrap();
    assert_eq!(owner_task.proposals.len(), 1);
    assert_eq!(owner_task.proposals[0].handle, "T-2");
    let dismissed = owner_flow.dismiss("T-2", "not now").await.unwrap();
    assert_eq!(dismissed.to, "cancelled");
    assert_eq!(
        err!(owner_flow.adopt("T-1")).code,
        ErrorCode::TransitionDenied
    );
    let task = ds
        .tasks()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.task_no == 2)
        .unwrap();
    assert_eq!(task.proposal_state, "dismissed");
}

#[tokio::test]
async fn tc888_uat05_one_conflict_group_and_one_parallel_task() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    let t1 = make_task(&e, &file_id, "One").await;
    let t2 = make_task(&e, &file_id, "Two").await;
    let t3 = make_task(&e, &file_id, "Three").await;
    for (handle, path) in [(&t1, "a.md"), (&t2, "b.md"), (&t3, "a.md")] {
        flow.start_analysis(handle).await.unwrap();
        flow.add_link(handle, "target", "file", path, None, "")
            .await
            .unwrap();
    }
    let view = flow.schedule().await.unwrap();
    assert_eq!(view.schedule.batches, vec![vec!["T-1", "T-2", "T-3"]]);
    assert_eq!(view.schedule.conflicts.len(), 1);
    assert_eq!(view.schedule.conflicts[0].tasks, vec!["T-1", "T-3"]);
    assert!(view.schedule.blocked.is_empty());
    assert!(view.schedule.unknown_targets.is_empty());
    assert_eq!(view.hidden, 0);
}

#[tokio::test]
async fn tc889_verify_detects_bypass_and_forgery() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let task_id = task_id_of(&e, 1).await;
    e.store
        .set_task_status(&task_id, "approval", "bypass")
        .await
        .unwrap();
    let report = flow.verify(Some("T-1")).await.unwrap();
    assert!(!report.ok);
    assert!(report
        .findings
        .iter()
        .any(|f| f.code == "STATUS_DRIFT" && f.level == "error"));
    e.store
        .set_task_status(&task_id, "analysis", "repair")
        .await
        .unwrap();
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    let ds = DocStore::new(e.store.clone());
    let forged = StepEvent {
        id: "00000000-0000-0000-0000-000000000002".to_string(),
        event: NewEvent {
            task_id: task_id.clone(),
            file_id: file_id.clone(),
            subject_kind: "task".to_string(),
            subject_id: task_id.clone(),
            kind: "dt.task.transitioned".to_string(),
            from_status: "approval".to_string(),
            to_status: "working".to_string(),
            actor: "hermes".to_string(),
            actor_kind: ActorKind::Agent,
            session: "hermes:L1".to_string(),
            claimed_session: String::new(),
            payload_json: "{}".to_string(),
        },
    };
    let change = StatusChange {
        task_id: task_id.clone(),
        from: ladon_explorer::doctask_model::TaskState::Approval,
        to: ladon_explorer::doctask_model::TaskState::Working,
    };
    ds.commit_step(Some(&change), &[], &[forged]).await.unwrap();
    let report = flow.verify(None).await.unwrap();
    assert!(report
        .findings
        .iter()
        .any(|f| f.code == "HUMAN_ONLY_FORGED" && f.level == "error"));
    assert!(!report.ok);
    assert!(submitted.plan_no == 1);
}

#[tokio::test]
async fn tc890_analysis_only_lifecycle_open_to_done() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let first = flow.submit_plan(&t1, &plan("first")).await.unwrap();
    flow.reject(&t1, "try again").await.unwrap();
    let second_spec = PlanSpec {
        options: vec!["alpha".to_string(), "beta".to_string()],
        ..plan("second")
    };
    let second = flow.submit_plan(&t1, &second_spec).await.unwrap();
    assert_eq!(second.plan_no, 2);
    flow.approve(&t1, &second.plan_hash, Some(1), "go")
        .await
        .unwrap();
    flow.report_work(&t1, "built").await.unwrap();
    flow.add_evidence(&t1, "report", "pass", "reviewed", None)
        .await
        .unwrap();
    let hash = subject(&e, 1, &second.plan_hash).await;
    let done = flow.close(&t1, &hash, &[], None).await.unwrap();
    assert_eq!(done.to, "done");
    let plans = DocStore::new(e.store.clone()).plans().await.unwrap();
    let rejected = plans.iter().find(|p| p.id == first.id).unwrap();
    assert_eq!(rejected.status.as_str(), "rejected");
    let approved = plans.iter().find(|p| p.id == second.id).unwrap();
    assert_eq!(approved.status.as_str(), "approved");
    assert_eq!(approved.chosen_option, "alpha");
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc891_stale_plan_commits_t11_and_returns_error() {
    let (tmp, e, file_id, v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let fixed = Fixed {
        version: v1.clone(),
        expected: sha256_hex(b"# A\nbody\n"),
        result: "f".repeat(64),
    };
    let flow = DocFlow::open(&e, owner())
        .await
        .unwrap()
        .with_edit_support(Box::new(fixed));
    flow.start_analysis(&t1).await.unwrap();
    let edit = PlanSpec {
        edit: Some(EditSpec {
            file_ref: "a.md".to_string(),
            section: "key:H1:A#1".to_string(),
            kind: "replace_section".to_string(),
            new_text: "# A\nnew\n".to_string(),
        }),
        ..plan("edit a")
    };
    let submitted = flow.submit_plan(&t1, &edit).await.unwrap();
    let bare = DocFlow::open(&e, owner()).await.unwrap();
    let before = total(&e).await;
    let refused = err!(bare.submit_plan(&t1, &edit));
    assert_eq!(refused.code, ErrorCode::InvalidArg);
    assert_eq!(total(&e).await, before);
    fs::write(tmp.path().join("a.md"), "# A\nchanged\n").unwrap();
    e.capture(&tmp.path().join("a.md"), "human", "edit")
        .await
        .unwrap();
    let before = total(&e).await;
    assert_eq!(
        err!(flow.approve(&t1, &submitted.plan_hash, None, "")).code,
        ErrorCode::StalePlan
    );
    let plans = DocStore::new(e.store.clone()).plans().await.unwrap();
    assert_eq!(
        plans
            .iter()
            .find(|p| p.id == submitted.id)
            .unwrap()
            .status
            .as_str(),
        "superseded"
    );
    assert_eq!(
        DocStore::new(e.store.clone()).tasks().await.unwrap()[0].status,
        "analysis"
    );
    let last = chain(&e).await;
    assert_eq!(total(&e).await, before + 2);
    assert_eq!(last.last().unwrap().3, "system");
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc892_relocated_anchor_blocks_approve_and_close() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    raw(&e, "UPDATE doc_anchors SET state = 'relocated'").await;
    let before = total(&e).await;
    assert_eq!(
        err!(flow.approve(&t1, &submitted.plan_hash, None, "")).code,
        ErrorCode::AnchorBroken
    );
    assert_eq!(total(&e).await, before);
    raw(&e, "UPDATE doc_anchors SET state = 'valid'").await;
    flow.approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    flow.report_work(&t1, "done").await.unwrap();
    flow.add_evidence(&t1, "note", "pass", "checked", None)
        .await
        .unwrap();
    raw(&e, "UPDATE doc_anchors SET state = 'relocated'").await;
    let hash = subject(&e, 1, &submitted.plan_hash).await;
    assert_eq!(
        err!(flow.close(&t1, &hash, &[], None)).code,
        ErrorCode::AnchorBroken
    );
    raw(&e, "UPDATE doc_anchors SET state = 'valid'").await;
    flow.close(&t1, &hash, &[], None).await.unwrap();
}

#[tokio::test]
async fn tc893_return_and_rework_paths() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    flow.approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    flow.report_work(&t1, "done").await.unwrap();
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    assert_eq!(
        err!(agent.return_to_work(&t1, "again")).code,
        ErrorCode::GuardFailed
    );
    agent
        .add_evidence(&t1, "report", "fail", "broken build", None)
        .await
        .unwrap();
    let before = DocStore::new(e.store.clone()).tasks().await.unwrap();
    assert_eq!(before[0].status, "verification");
    agent.return_to_work(&t1, "fix the build").await.unwrap();
    let approvals = DocStore::new(e.store.clone())
        .approvals()
        .await
        .unwrap()
        .len();
    assert_eq!(approvals, 1);
    flow.rework(&t1, "rethink").await.unwrap();
    let plans = DocStore::new(e.store.clone()).plans().await.unwrap();
    assert_eq!(plans[0].status.as_str(), "superseded");
    assert_eq!(
        DocStore::new(e.store.clone()).tasks().await.unwrap()[0].status,
        "analysis"
    );
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc894_cancel_and_reopen_and_cancelled_dependency() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "One").await;
    let t2 = make_task(&e, &file_id, "Two").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    flow.add_dependency(&t2, &t1).await.unwrap();
    flow.cancel(&t1, "no longer needed").await.unwrap();
    assert_eq!(
        err!(flow.cancel(&t1, "again")).code,
        ErrorCode::TransitionDenied
    );
    assert_eq!(err!(flow.reopen(&t1, "")).code, ErrorCode::InvalidArg);
    let reopened = flow.reopen(&t1, "needed after all").await.unwrap();
    assert_eq!(reopened.to, "open");
    let events: Vec<String> = chain(&e).await.into_iter().map(|c| c.0).collect();
    assert_eq!(events.last().unwrap(), "dt.task.reopened");
    flow.cancel(&t1, "stop again").await.unwrap();
    let view = flow.schedule().await.unwrap();
    assert!(view
        .schedule
        .blocked
        .iter()
        .any(|entry| entry.task == "T-2" && entry.reason.contains("cancelled")));
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc895_dependencies_cycles_and_ownership() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    make_task(&e, &file_id, "One").await;
    let t2 = make_task(&e, &file_id, "Two").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.add_dependency("T-2", "T-1").await.unwrap();
    assert_eq!(
        err!(flow.add_dependency("T-1", "T-1")).code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        err!(flow.add_dependency("T-1", "T-99")).code,
        ErrorCode::DepDangling
    );
    assert_eq!(
        err!(flow.add_dependency("T-1", "T-2")).code,
        ErrorCode::DepCycle
    );
    assert_eq!(
        err!(flow.add_dependency("T-2", "T-1")).code,
        ErrorCode::InvalidArg
    );
    let ds = DocStore::new(e.store.clone());
    assert_eq!(ds.dependency_edges().await.unwrap().len(), 1);
    flow.remove_dependency("T-2", "T-1").await.unwrap();
    assert!(ds.dependency_edges().await.unwrap().is_empty());
    let agent_task = {
        let anchor = ds.tasks().await.unwrap()[0].anchor_id.clone();
        ds.create_task(&NewTask {
            file_id: file_id.clone(),
            anchor_id: anchor,
            title: "Agent idea".to_string(),
            description: String::new(),
            owner: String::new(),
            priority: 2,
            proposal_state: ProposalState::Accepted,
            visibility: Visibility::Shared,
            source_kind: SourceKind::Hermes,
            created_by: "hermes".to_string(),
            created_by_kind: ActorKind::Agent,
            session: "hermes:L1".to_string(),
        })
        .await
        .unwrap()
        .task_no
    };
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    let own = format!("T-{agent_task}");
    agent.add_dependency(&own, "T-1").await.unwrap();
    assert_eq!(
        err!(agent.add_dependency("T-1", "T-2")).code,
        ErrorCode::HumanOnly
    );
    let _ = t2;
}

#[tokio::test]
async fn tc896_links_validate_each_target_kind() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "One").await;
    let t2 = make_task(&e, &file_id, "Two").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.add_link(&t1, "design", "file", "a.md", None, "")
        .await
        .unwrap();
    flow.add_link(&t1, "code", "git_commit", "abcdef0123", None, "")
        .await
        .unwrap();
    flow.add_link(&t1, "doc", "url", "https://example.com/x", None, "")
        .await
        .unwrap();
    flow.add_link(&t1, "other", "task", &t2, None, "")
        .await
        .unwrap();
    let long_label = "x".repeat(121);
    let bad = [
        ("code", "git_commit", "abc", None, ""),
        ("doc", "url", "ftp://example.com", None, ""),
        (
            "doc",
            "url",
            "https://example.com",
            None,
            long_label.as_str(),
        ),
        ("design", "file_section", "a.md", Some(""), ""),
    ];
    for (kind, target_kind, target, section, label) in bad {
        let result = flow
            .add_link(&t1, kind, target_kind, target, section, label)
            .await;
        let code = result
            .err()
            .map(|err| err.downcast::<DocError>().unwrap().code);
        assert_eq!(code, Some(ErrorCode::InvalidArg), "{kind} {target}");
    }
    assert_eq!(
        err!(flow.add_link(&t1, "other", "task", "T-99", None, "")).code,
        ErrorCode::NotFound
    );
    let link_id = DocStore::new(e.store.clone())
        .links()
        .await
        .unwrap()
        .into_iter()
        .find(|link| link.target_kind == "file")
        .unwrap()
        .id;
    flow.remove_link(&t1, &link_id).await.unwrap();
    let removed = DocStore::new(e.store.clone())
        .links()
        .await
        .unwrap()
        .into_iter()
        .find(|link| link.id == link_id)
        .unwrap();
    assert!(removed.removed_at.is_some());
    assert!(flow.verify(None).await.unwrap().ok);
}

#[tokio::test]
async fn tc897_evidence_states_kinds_and_version_rules() {
    let (tmp, e, file_id, v1, other_version) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    assert_eq!(
        err!(flow.add_evidence(&t1, "test_run", "pass", "early", None)).code,
        ErrorCode::TransitionDenied
    );
    let submitted = flow.submit_plan(&t1, &plan("p")).await.unwrap();
    flow.approve(&t1, &submitted.plan_hash, None, "")
        .await
        .unwrap();
    flow.add_evidence(&t1, "test_run", "pass", "ran", None)
        .await
        .unwrap();
    assert_eq!(
        err!(flow.add_evidence(&t1, "commit", "info", "x", None)).code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        err!(flow.add_evidence(&t1, "version", "info", "x", None)).code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        err!(flow.add_evidence(&t1, "version", "info", "x", Some(&v1))).code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        err!(flow.add_evidence(&t1, "version", "info", "x", Some(&other_version))).code,
        ErrorCode::NotFound
    );
    fs::write(tmp.path().join("a.md"), "# A\nlater\n").unwrap();
    let v2 = e
        .capture(&tmp.path().join("a.md"), "human", "edit")
        .await
        .unwrap()
        .version_id
        .unwrap();
    assert_eq!(
        err!(flow.add_evidence(&t1, "version", "pass", "x", Some(&v2))).code,
        ErrorCode::InvalidArg
    );
    flow.add_evidence(&t1, "version", "info", "later version", Some(&v2))
        .await
        .unwrap();
    let rows = DocStore::new(e.store.clone()).evidence().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].source.as_str(), "ledger");
    assert_eq!(rows[0].source.as_str(), "reported");
}

#[tokio::test]
async fn tc898_proposal_quota_and_rate_window() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let ds = DocStore::new(e.store.clone());
    let anchor = ds.tasks().await.unwrap()[0].anchor_id.clone();
    let proposal = |n: usize| NewTask {
        file_id: file_id.clone(),
        anchor_id: anchor.clone(),
        title: format!("Idea {n}"),
        description: String::new(),
        owner: String::new(),
        priority: 2,
        proposal_state: ProposalState::Proposed,
        visibility: Visibility::Shared,
        source_kind: SourceKind::Hermes,
        created_by: "hermes".to_string(),
        created_by_kind: ActorKind::Agent,
        session: "hermes:L1".to_string(),
    };
    let agent = DocFlow::attach(&e, Caller::agent("L1")).await.unwrap();
    for n in 0..9 {
        agent.admit_proposal().await.unwrap();
        ds.create_task(&proposal(n)).await.unwrap();
    }
    agent.admit_proposal().await.unwrap();
    ds.create_task(&proposal(9)).await.unwrap();
    assert_eq!(err!(agent.admit_proposal()).code, ErrorCode::Quota);

    let filler = |session: &str| NewEvent {
        task_id: String::new(),
        file_id: String::new(),
        subject_kind: "test".to_string(),
        subject_id: String::new(),
        kind: "dt.session.started".to_string(),
        from_status: String::new(),
        to_status: String::new(),
        actor: "hermes".to_string(),
        actor_kind: ActorKind::Agent,
        session: session.to_string(),
        claimed_session: String::new(),
        payload_json: "{}".to_string(),
    };
    for _ in 0..39 {
        ds.append_event(&filler("hermes:L2")).await.unwrap();
    }
    let second = DocFlow::attach(&e, Caller::agent("L2")).await.unwrap();
    second.admit_proposal().await.unwrap();
    ds.append_event(&filler("hermes:L2")).await.unwrap();
    assert_eq!(err!(second.admit_proposal()).code, ErrorCode::RateLimit);

    for _ in 0..40 {
        ds.append_event(&filler("hermes:L3")).await.unwrap();
    }
    let third = DocFlow::attach(&e, Caller::agent("L3")).await.unwrap();
    assert_eq!(err!(third.start_analysis("T-1")).code, ErrorCode::RateLimit);
    assert!(third.schedule().await.is_ok());
}

#[tokio::test]
async fn tc899_fresh_library_verifies_clean_and_pure_rules_hold() {
    let (_tmp, e, file_id, _v1, _b) = setup().await;
    let t1 = make_task(&e, &file_id, "Review").await;
    let flow = DocFlow::open(&e, owner()).await.unwrap();
    flow.start_analysis(&t1).await.unwrap();
    let report = flow.verify(None).await.unwrap();
    assert!(report.ok);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    let inbox = flow.inbox().await.unwrap();
    assert!(inbox.approvals.is_empty() && inbox.verifications.is_empty());
    let view = flow.schedule().await.unwrap();
    assert_eq!(view.schedule.batches, vec![vec!["T-1"]]);
}
