//! Integration tests for the document-task service core (WP06): authorization, the transfer gate
//! and the agent view, baselines with their expected-subject guards, annotations and tasks,
//! visibility, read views and the legacy path (IT-051, IT-054, TC-870 to TC-879).
use async_trait::async_trait;
use ladon_explorer::{
    doctask_model::{sha256_hex, DocError, ErrorCode, SourceKind, Visibility},
    doctask_service::{
        confirm_yes, hash_prefix_matches, task_content_hash, Caller, DocTaskService, EngineGate,
        Shown, TaskEdit, TaskSpec, TransferGate,
    },
    doctask_store::DocStore,
    engine::Engine,
    model::{FileRecord, VersionRecord},
};
use std::{collections::HashSet, fs, path::Path};

/// Gate for tests: refuses the named files and versions, and reports the rule set as unusable
/// when `policy` is false.
struct Deny {
    files: HashSet<String>,
    versions: HashSet<String>,
    policy: bool,
}

#[async_trait]
impl TransferGate for Deny {
    async fn policy_usable(&self) -> bool {
        self.policy
    }
    async fn file_allowed(&self, file: &FileRecord) -> bool {
        !self.files.contains(&file.id)
    }
    async fn version_allowed(&self, version: &VersionRecord) -> bool {
        !self.versions.contains(&version.id)
    }
}

/// Writes `name` with `text` under the engine root and records it; returns the file and version ids.
async fn write_capture(e: &Engine, root: &Path, name: &str, text: &str) -> (String, String) {
    fs::write(root.join(name), text).unwrap();
    let r = e.capture(&root.join(name), "human", "test").await.unwrap();
    (r.file_id, r.version_id.unwrap())
}

async fn version_hash(e: &Engine, version_id: &str) -> String {
    e.store
        .version(version_id)
        .await
        .unwrap()
        .unwrap()
        .content_hash
}

fn shown(version_id: &str, hash: &str) -> Shown {
    Shown {
        version_id: version_id.to_string(),
        hash: hash.to_string(),
    }
}

fn owner() -> Caller {
    Caller::human("owner", SourceKind::Cli)
}

/// The error of a failed call. Panics when the call succeeded.
fn err_of<T>(result: anyhow::Result<T>) -> DocError {
    match result {
        Ok(_) => panic!("expected a DocError, the call succeeded"),
        Err(err) => err
            .downcast::<DocError>()
            .expect("the error must be a DocError"),
    }
}

/// Counts the rows of a table through a second connection.
async fn count(e: &Engine, table: &str) -> i64 {
    let db = turso::Builder::new_local(&e.store.path.to_string_lossy())
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(&format!("SELECT COUNT(*) FROM {table}"), ())
        .await
        .unwrap();
    rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
}

async fn event_count(e: &Engine) -> usize {
    DocStore::new(e.store.clone()).events().await.unwrap().len()
}

/// A library with one captured file `a.md` holding `# A` and `body`, and a human service.
async fn fixture_a() -> (tempfile::TempDir, Engine, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = write_capture(&e, tmp.path(), "a.md", "# A\nbody\n").await;
    (tmp, e, file_id, v1)
}

#[tokio::test]
async fn tc870_create_requires_baseline_and_pins_base_version() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = write_capture(&e, tmp.path(), "a.md", "# REQ-001\nApproved\n").await;
    let svc = DocTaskService::open(&e, owner()).await.unwrap();
    let section = "key:H1:REQ-001#1";
    let spec = TaskSpec::new("Review");

    let no_task = err_of(svc.create_task(&file_id, section, &spec).await);
    assert_eq!(no_task.code, ErrorCode::NoBaseline);
    let no_note = err_of(
        svc.add_annotation(&file_id, section, "add review", "", "", false)
            .await,
    );
    assert_eq!(no_note.code, ErrorCode::NoBaseline);

    let h1 = version_hash(&e, &v1).await;
    svc.register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let v2 = {
        fs::write(tmp.path().join("a.md"), "# REQ-001\nChanged\n").unwrap();
        e.capture(&tmp.path().join("a.md"), "human", "edit")
            .await
            .unwrap()
            .version_id
            .unwrap()
    };

    let task = svc.create_task(&file_id, section, &spec).await.unwrap();
    let card = svc.show_task(&task.handle).await.unwrap();
    assert_eq!(card.task.base_version_id, v1);
    assert_ne!(card.task.base_version_id, v2);
    assert_eq!(card.task.section, "H1:REQ-001#1");
    assert!(card.task.base_stale);

    let note = svc
        .add_annotation(&file_id, section, "add review", "", "", false)
        .await
        .unwrap();
    let notes = svc.list_annotations(Some(&file_id)).await.unwrap();
    assert!(notes
        .items
        .iter()
        .any(|n| n.handle == note.handle && n.text == "add review" && n.base_version_id == v1));
}

#[tokio::test]
async fn tc871_baseline_register_revoke_and_guards() {
    let (tmp, e, file_id, v1) = fixture_a().await;
    let svc = DocTaskService::open(&e, owner()).await.unwrap();
    let h1 = version_hash(&e, &v1).await;

    let wrong = err_of(
        svc.register_baseline(&file_id, None, "", &shown(&v1, &"0".repeat(64)))
            .await,
    );
    assert_eq!(wrong.code, ErrorCode::StaleSubject);
    assert!(DocStore::new(e.store.clone())
        .duplicate_baselines()
        .await
        .unwrap()
        .is_empty());

    let first = svc
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let before = event_count(&e).await;
    let again = svc
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(event_count(&e).await, before);

    // The file changes on disk without a capture: the ledger no longer describes it.
    fs::write(tmp.path().join("a.md"), "# A\nedited\n").unwrap();
    let changed = err_of(
        svc.register_baseline(&file_id, None, "", &shown(&v1, &h1))
            .await,
    );
    assert_eq!(changed.code, ErrorCode::FileChanged);
    fs::write(tmp.path().join("a.md"), "# A\nbody\n").unwrap();

    let reason_empty = err_of(svc.revoke_baseline(&first.id, "", &shown(&v1, &h1)).await);
    assert_eq!(reason_empty.code, ErrorCode::InvalidArg);
    let reason_long = "r".repeat(501);
    let reason_too_long = err_of(
        svc.revoke_baseline(&first.id, &reason_long, &shown(&v1, &h1))
            .await,
    );
    assert_eq!(reason_too_long.code, ErrorCode::InvalidArg);
    let unknown = err_of(
        svc.revoke_baseline("no-such-baseline", "reason", &shown(&v1, &h1))
            .await,
    );
    assert_eq!(unknown.code, ErrorCode::NotFound);

    let reason = "r".repeat(500);
    let revoked = svc
        .revoke_baseline(&first.id, &reason, &shown(&v1, &h1))
        .await
        .unwrap();
    assert!(revoked.revoked_at.is_some());

    let second = svc
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    assert_ne!(second.id, first.id);
    assert!(second.revoked_at.is_none());
    let registered: Vec<_> = DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "dt.baseline.registered")
        .collect();
    let last = registered.last().unwrap();
    assert!(last.payload_json.contains(&first.id));
    assert!(last.payload_json.contains("replaces_revoked"));
}

#[tokio::test]
async fn tc872_agent_refused_for_human_only_operations() {
    let (_tmp, e, file_id, v1) = fixture_a().await;
    let h1 = version_hash(&e, &v1).await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    let baseline = human
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let task = human
        .create_task(&file_id, "key:H1:A#1", &TaskSpec::new("Owner task"))
        .await
        .unwrap();
    let note = human
        .add_annotation(&file_id, "key:H1:A#1", "owner note", "", "", false)
        .await
        .unwrap();

    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap();
    let events = event_count(&e).await;
    let baselines = count(&e, "doc_baselines").await;
    let tasks = count(&e, "doc_task_meta").await;

    assert_eq!(
        err_of(
            agent
                .register_baseline(&file_id, None, "", &shown(&v1, &h1))
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );
    assert_eq!(
        err_of(
            agent
                .revoke_baseline(&baseline.id, "reason", &shown(&v1, &h1))
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );
    assert_eq!(
        err_of(
            agent
                .edit_task(
                    &task.handle,
                    &TaskEdit {
                        title: Some("Renamed".into()),
                        ..Default::default()
                    },
                    &"0".repeat(64),
                )
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );
    assert_eq!(
        err_of(
            agent
                .set_annotation_visibility(
                    &note.handle,
                    Visibility::Shared,
                    &sha256_hex(b"owner note")
                )
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );
    assert_eq!(
        err_of(
            agent
                .legacy_task(&file_id, "key:H1:A#1", "Legacy", "d")
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );
    assert_eq!(
        err_of(
            agent
                .legacy_annotate(&file_id, "key:H1:A#1", "legacy")
                .await
        )
        .code,
        ErrorCode::HumanOnly
    );

    assert_eq!(event_count(&e).await, events);
    assert_eq!(count(&e, "doc_baselines").await, baselines);
    assert_eq!(count(&e, "doc_task_meta").await, tasks);
}

#[tokio::test]
async fn tc873_agent_proposals_and_natural_keys() {
    let (_tmp, e, file_id, v1) = fixture_a().await;
    let h1 = version_hash(&e, &v1).await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    human
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap();

    let mut spec = TaskSpec::new("Agent idea");
    spec.priority = 1;
    let first = agent
        .create_task(&file_id, "key:H1:A#1", &spec)
        .await
        .unwrap();
    assert!(!first.existing);
    let second = agent
        .create_task(&file_id, "key:H1:A#1", &spec)
        .await
        .unwrap();
    assert!(second.existing);
    assert_eq!(second.id, first.id);

    let card = agent.show_task(&first.handle).await.unwrap();
    assert_eq!(card.task.status, "open");
    assert_eq!(card.task.proposal_state, "proposed");
    assert_eq!(card.task.visibility, "shared");
    assert_eq!(card.task.source_kind, "hermes");
    assert_eq!(card.task.created_by, "hermes");

    let note = agent
        .add_annotation(&file_id, "key:H1:A#1", "agent note", "", "", false)
        .await
        .unwrap();
    let note_again = agent
        .add_annotation(&file_id, "key:H1:A#1", "agent note", "", "", false)
        .await
        .unwrap();
    assert!(!note.existing);
    assert!(note_again.existing);
    assert_eq!(note_again.handle, note.handle);

    let mut too_high = TaskSpec::new("Priority four");
    too_high.priority = 4;
    assert_eq!(
        err_of(agent.create_task(&file_id, "key:H1:A#1", &too_high).await).code,
        ErrorCode::InvalidArg
    );
    let long_title = "t".repeat(201);
    assert_eq!(
        err_of(
            agent
                .create_task(&file_id, "key:H1:A#1", &TaskSpec::new(&long_title))
                .await
        )
        .code,
        ErrorCode::InvalidArg
    );

    let mut private = TaskSpec::new("Private idea");
    private.private = true;
    human
        .create_task(&file_id, "key:H1:A#1", &private)
        .await
        .unwrap();

    let list = agent.list_tasks().await.unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].id, first.id);
    assert_eq!(list.hidden, 1);
}

#[tokio::test]
async fn tc874_agent_view_hides_denied_files_uniformly() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (ok_id, ok_v) = write_capture(&e, tmp.path(), "ok.md", "# OK\nbody\n").await;
    let (secret_id, secret_v) = write_capture(&e, tmp.path(), "secret.md", "# SEC\nbody\n").await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    let ok_h = version_hash(&e, &ok_v).await;
    let sec_h = version_hash(&e, &secret_v).await;
    human
        .register_baseline(&ok_id, None, "", &shown(&ok_v, &ok_h))
        .await
        .unwrap();
    human
        .register_baseline(&secret_id, None, "", &shown(&secret_v, &sec_h))
        .await
        .unwrap();
    let ok_task = human
        .create_task(&ok_id, "key:H1:OK#1", &TaskSpec::new("Ok task"))
        .await
        .unwrap();
    let secret_task = human
        .create_task(&secret_id, "key:H1:SEC#1", &TaskSpec::new("Secret task"))
        .await
        .unwrap();
    let mut private = TaskSpec::new("Hidden task");
    private.private = true;
    human
        .create_task(&ok_id, "key:H1:OK#1", &private)
        .await
        .unwrap();
    human
        .add_annotation(&secret_id, "key:H1:SEC#1", "secret note", "", "", false)
        .await
        .unwrap();
    assert_eq!(ok_task.handle, "T-1");
    assert_eq!(secret_task.handle, "T-2");

    let gate = Deny {
        files: HashSet::from([secret_id.clone()]),
        versions: HashSet::new(),
        policy: true,
    };
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap()
        .with_gate(Box::new(gate));

    let list = agent.list_tasks().await.unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].title, "Ok task");
    assert_eq!(list.hidden, 2);
    let notes = agent.list_annotations(None).await.unwrap();
    assert!(notes.items.is_empty());
    assert_eq!(notes.hidden, 1);

    let refused = err_of(agent.show_task("T-2").await);
    let local_only = err_of(agent.show_task("T-3").await);
    let missing = err_of(agent.show_task("T-999").await);
    assert_eq!(refused.code, ErrorCode::NotFound);
    assert_eq!(refused, local_only);
    assert_eq!(refused, missing);
    assert!(refused.detail.is_empty());
    let on_refused_file = err_of(
        agent
            .create_task(&secret_id, "key:H1:SEC#1", &TaskSpec::new("Sneak"))
            .await,
    );
    assert_eq!(on_refused_file, refused);

    let ok_record = human.resolve_file(&ok_id).await.unwrap();
    assert_eq!(ok_record.relative_path, "ok.md");
    assert_eq!(agent.show_task("T-1").await.unwrap().task.title, "Ok task");
}

#[tokio::test]
async fn tc874_resolve_file_for_agent_accepts_ids_only() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (ok_id, _) = write_capture(&e, tmp.path(), "ok.md", "# OK\nbody\n").await;
    let (secret_id, _) = write_capture(&e, tmp.path(), "secret.md", "# SEC\nbody\n").await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    assert_eq!(human.resolve_file("ok.md").await.unwrap().id, ok_id);

    let gate = Deny {
        files: HashSet::from([secret_id.clone()]),
        versions: HashSet::new(),
        policy: true,
    };
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap()
        .with_gate(Box::new(gate));
    let by_path = err_of(agent.resolve_file("ok.md").await);
    assert_eq!(by_path.code, ErrorCode::NotFound);
    assert_eq!(
        agent.resolve_file(&ok_id).await.unwrap().relative_path,
        "ok.md"
    );
    let by_refused_id = err_of(agent.resolve_file(&secret_id).await);
    assert_eq!(by_refused_id, by_path);
}

#[tokio::test]
async fn tc874_policy_unusable_is_policy_denied() {
    let (_tmp, e, _file_id, _v1) = fixture_a().await;
    DocTaskService::open(&e, owner()).await.unwrap();
    let gate = Deny {
        files: HashSet::new(),
        versions: HashSet::new(),
        policy: false,
    };
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap()
        .with_gate(Box::new(gate));
    assert_eq!(
        err_of(agent.list_tasks().await).code,
        ErrorCode::PolicyDenied
    );
}

#[tokio::test]
async fn tc874_engine_gate_refuses_excluded_path() {
    let tmp = tempfile::tempdir().unwrap();
    let rules = format!(
        "{}\nsecret.md\n",
        include_str!("../config/examples/.LADONignore")
    );
    fs::write(tmp.path().join(".LADONignore"), rules).unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (ok_id, _) = write_capture(&e, tmp.path(), "ok.md", "# OK\nbody\n").await;
    let (secret_id, _) = write_capture(&e, tmp.path(), "secret.md", "# SEC\nbody\n").await;
    let ok = e.store.file_by_id(&ok_id).await.unwrap().unwrap();
    let secret = e.store.file_by_id(&secret_id).await.unwrap().unwrap();
    let gate = EngineGate::new(&e);
    assert!(gate.policy_usable().await);
    assert!(gate.file_allowed(&ok).await);
    assert!(!gate.file_allowed(&secret).await);
}

#[tokio::test]
async fn tc875_historical_version_gate_hides_task_and_annotation() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, v1) = write_capture(&e, tmp.path(), "h.md", "# Alpha\nold body\n").await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    let h1 = version_hash(&e, &v1).await;
    let base1 = human
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    human
        .create_task(&file_id, "key:H1:Alpha#1", &TaskSpec::new("Old task"))
        .await
        .unwrap();
    human
        .add_annotation(&file_id, "key:H1:Alpha#1", "old note", "", "", false)
        .await
        .unwrap();
    human
        .revoke_baseline(&base1.id, "superseded", &shown(&v1, &h1))
        .await
        .unwrap();

    fs::write(tmp.path().join("h.md"), "# Alpha\nnew body\n").unwrap();
    let v2 = e
        .capture(&tmp.path().join("h.md"), "human", "edit")
        .await
        .unwrap()
        .version_id
        .unwrap();
    let h2 = version_hash(&e, &v2).await;
    human
        .register_baseline(&file_id, None, "", &shown(&v2, &h2))
        .await
        .unwrap();
    human
        .create_task(&file_id, "key:H1:Alpha#1", &TaskSpec::new("New task"))
        .await
        .unwrap();
    human
        .add_annotation(&file_id, "key:H1:Alpha#1", "new note", "", "", false)
        .await
        .unwrap();

    let gate = Deny {
        files: HashSet::new(),
        versions: HashSet::from([v1.clone()]),
        policy: true,
    };
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap()
        .with_gate(Box::new(gate));
    let tasks = agent.list_tasks().await.unwrap();
    assert_eq!(tasks.items.len(), 1);
    assert_eq!(tasks.items[0].title, "New task");
    assert_eq!(tasks.hidden, 1);
    let notes = agent.list_annotations(Some(&file_id)).await.unwrap();
    assert_eq!(notes.items.len(), 1);
    assert_eq!(notes.items[0].text, "new note");
    assert_eq!(notes.hidden, 1);
    assert_eq!(
        err_of(agent.show_task("T-1").await).code,
        ErrorCode::NotFound
    );
    assert_eq!(human.list_tasks().await.unwrap().items.len(), 2);
}

#[tokio::test]
async fn tc876_agent_events_omit_payload() {
    let (_tmp, e, file_id, v1) = fixture_a().await;
    let h1 = version_hash(&e, &v1).await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    human
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let task = human
        .create_task(&file_id, "key:H1:A#1", &TaskSpec::new("Event probe"))
        .await
        .unwrap();
    let card = human.show_task(&task.handle).await.unwrap();
    let expected = task_content_hash(
        &card.task.title,
        &card.task.description,
        &card.task.anchor_id,
        card.task.priority,
        &[],
    );
    human
        .edit_task(
            &task.handle,
            &TaskEdit {
                priority: Some(3),
                ..Default::default()
            },
            &expected,
        )
        .await
        .unwrap();

    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap();
    let agent_card = agent.show_task(&task.handle).await.unwrap();
    assert!(agent_card.events.len() >= 2);
    assert!(agent_card.events.iter().all(|ev| ev.payload.is_none()));
    assert!(agent_card
        .events
        .iter()
        .any(|ev| ev.kind == "dt.task.created" && ev.actor_kind == "human"));
    assert!(agent_card
        .events
        .iter()
        .any(|ev| ev.kind == "dt.task.edited"));

    let owner_card = human.show_task(&task.handle).await.unwrap();
    assert!(owner_card.events.iter().all(|ev| ev.payload.is_some()));
}

#[tokio::test]
async fn tc877_attach_requires_schema_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let outdated = err_of(DocTaskService::attach(&e, Caller::agent("L1")).await);
    assert_eq!(outdated.code, ErrorCode::SchemaOutdated);
    assert!(!DocStore::new(e.store.clone())
        .schema_current()
        .await
        .unwrap());

    let (file_id, v1) = write_capture(&e, tmp.path(), "a.md", "# A\nbody\n").await;
    DocTaskService::open(&e, owner()).await.unwrap();
    assert!(DocStore::new(e.store.clone())
        .schema_current()
        .await
        .unwrap());
    e.store
        .add_annotation(&file_id, "H1:A#1", &v1, "raw legacy text")
        .await
        .unwrap();

    let meta = count(&e, "doc_task_meta").await;
    let annotation_meta = count(&e, "doc_annotation_meta").await;
    let events = event_count(&e).await;
    let agent = DocTaskService::attach(&e, Caller::agent("L1"))
        .await
        .unwrap();
    assert!(agent.list_tasks().await.unwrap().items.is_empty());
    assert!(agent
        .list_annotations(Some(&file_id))
        .await
        .unwrap()
        .items
        .is_empty());
    assert_eq!(count(&e, "doc_task_meta").await, meta);
    assert_eq!(count(&e, "doc_annotation_meta").await, annotation_meta);
    assert_eq!(event_count(&e).await, events);
}

#[tokio::test]
async fn tc878_expected_subject_guards() {
    let (_tmp, e, file_id, v1) = fixture_a().await;
    let h1 = version_hash(&e, &v1).await;
    let human = DocTaskService::open(&e, owner()).await.unwrap();
    human
        .register_baseline(&file_id, None, "", &shown(&v1, &h1))
        .await
        .unwrap();
    let task = human
        .create_task(&file_id, "key:H1:A#1", &TaskSpec::new("Secret Title"))
        .await
        .unwrap();
    let card = human.show_task(&task.handle).await.unwrap();
    let edit = TaskEdit {
        title: Some("Renamed".into()),
        ..Default::default()
    };
    let stale = err_of(human.edit_task(&task.handle, &edit, &"0".repeat(64)).await);
    assert_eq!(stale.code, ErrorCode::StaleSubject);
    assert_eq!(
        human.show_task(&task.handle).await.unwrap().task.title,
        "Secret Title"
    );

    let current = task_content_hash(
        &card.task.title,
        &card.task.description,
        &card.task.anchor_id,
        card.task.priority,
        &[],
    );
    let edited = human
        .edit_task(&task.handle, &edit, &current)
        .await
        .unwrap();
    assert_eq!(edited.title, "Renamed");
    let edit_event = DocStore::new(e.store.clone())
        .events()
        .await
        .unwrap()
        .into_iter()
        .rfind(|ev| ev.kind == "dt.task.edited")
        .unwrap();
    assert!(edit_event
        .payload_json
        .contains(&sha256_hex("Secret Title".as_bytes())));
    assert!(!edit_event.payload_json.contains("Secret Title"));

    let note = human
        .add_annotation(&file_id, "key:H1:A#1", "note body", "", "", true)
        .await
        .unwrap();
    let hidden = human.list_annotations(Some(&file_id)).await.unwrap();
    assert_eq!(hidden.items[0].visibility, "local_only");
    let wrong_share = err_of(
        human
            .set_annotation_visibility(&note.handle, Visibility::Shared, &"0".repeat(64))
            .await,
    );
    assert_eq!(wrong_share.code, ErrorCode::StaleSubject);
    let shared = human
        .set_annotation_visibility(&note.handle, Visibility::Shared, &sha256_hex(b"note body"))
        .await
        .unwrap();
    assert_eq!(shared.visibility, "shared");

    assert_eq!(
        confirm_yes(true, None).unwrap_err().code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        confirm_yes(true, Some("abc12")).unwrap_err().code,
        ErrorCode::InvalidArg
    );
    assert_eq!(
        confirm_yes(true, Some("zz")).unwrap_err().code,
        ErrorCode::InvalidArg
    );
    assert!(confirm_yes(true, Some("deadbeef")).is_ok());
    assert!(confirm_yes(false, None).is_ok());
    assert!(hash_prefix_matches("abcdef12", "abcdef1234567890"));
    assert!(!hash_prefix_matches("abcdef12", "abcdef0000000000"));
}

#[tokio::test]
async fn tc879_legacy_path_keeps_unchecked_and_legacy_semantics() {
    let tmp = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let (file_id, _v1) = write_capture(&e, tmp.path(), "legacy.md", "# A\nbody\n").await;
    let svc = DocTaskService::open(&e, owner()).await.unwrap();

    let known_note = svc
        .legacy_annotate(&file_id, "H1:A#1", "note valid")
        .await
        .unwrap();
    let unknown_note = svc
        .legacy_annotate(&file_id, "H1:Missing", "note unchecked")
        .await
        .unwrap();
    assert!(!known_note.existing && !unknown_note.existing);
    let known_task = svc
        .legacy_task(&file_id, "H1:A#1", "Legacy valid", "d")
        .await
        .unwrap();
    let unknown_task = svc
        .legacy_task(&file_id, "H1:Missing", "Legacy unchecked", "d")
        .await
        .unwrap();

    let list = svc.list_tasks().await.unwrap();
    assert_eq!(list.hidden, 0);
    assert_eq!(list.items.len(), 2);
    let valid = list.items.iter().find(|t| t.id == known_task.id).unwrap();
    let unchecked = list.items.iter().find(|t| t.id == unknown_task.id).unwrap();
    assert_eq!(valid.anchor_state, "valid");
    assert_eq!(valid.section, "H1:A#1");
    assert_eq!(unchecked.anchor_state, "unchecked");
    assert_eq!(unchecked.section, "H1:Missing");
    assert!(unchecked.unbound);
    for task in [valid, unchecked] {
        assert_eq!(task.source_kind, "legacy");
        assert_eq!(task.proposal_state, "accepted");
        assert_eq!(task.visibility, "shared");
        assert_eq!(task.created_by_kind, "human");
    }
    let notes = svc.list_annotations(Some(&file_id)).await.unwrap();
    assert_eq!(notes.items.len(), 2);
    assert_eq!(notes.hidden, 0);
    let card = svc.show_task(&known_task.handle).await.unwrap();
    assert_eq!(card.task.title, "Legacy valid");
    assert_eq!(svc.list_baselines(None).await.unwrap().items.len(), 0);
}
