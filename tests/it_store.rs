//! Integration tests for `store::Store` against a real Turso database file.
use ladon_explorer::{
    model::StorageObject,
    store::{PendingVersion, Store},
};

async fn fixture() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::new(tmp.path().join(".ladon/meta.turso"));
    store.init().await.unwrap();
    (tmp, store)
}

fn pending(path: &str, hash: &str, sensitive: bool) -> PendingVersion {
    PendingVersion {
        path: path.into(),
        hash: hash.into(),
        size: 3,
        sections: vec!["H1:A#1".into()],
        actor: "human".into(),
        reason: "init".into(),
        sensitive,
    }
}

fn kinds(events: &[ladon_explorer::model::EventRecord]) -> Vec<&str> {
    events.iter().map(|e| e.kind.as_str()).collect()
}

#[tokio::test]
async fn tc101_append_version_round_trip() {
    let (_tmp, store) = fixture().await;
    let (file_id, version_id, event_id) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    assert!(!file_id.is_empty() && !version_id.is_empty() && !event_id.is_empty());

    let record = store.find_path("a.md").await.unwrap().unwrap();
    assert_eq!(record.id, file_id);
    assert_eq!(record.version_no, 1);
    assert_eq!(record.current_hash, "h1");
    assert!(!record.sensitive && !record.deleted);

    let versions = store.versions(&file_id).await.unwrap();
    assert_eq!(versions.len(), 1);
    let v = &versions[0];
    assert_eq!(v.id, version_id);
    assert_eq!((v.number, v.byte_size), (1, 3));
    assert_eq!((v.actor.as_str(), v.reason.as_str()), ("human", "init"));
    assert_eq!(v.changed_sections, vec!["H1:A#1".to_string()]);
    assert_eq!(store.latest_version(&file_id).await.unwrap().unwrap().id, version_id);
    assert_eq!(store.version(&version_id).await.unwrap().unwrap().content_hash, "h1");

    let events = store.events(&file_id).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "created");
    assert_eq!(events[0].id, event_id);
    assert_eq!(events[0].ledger_prev_hash, "");
    assert_eq!(store.list_files().await.unwrap().len(), 1);
}

#[tokio::test]
async fn tc102_second_version_and_no_change_rejected() {
    let (_tmp, store) = fixture().await;
    let (file_id, _, _) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    let (same_file, _, _) = store.append_version(pending("a.md", "h2", false)).await.unwrap();
    assert_eq!(same_file, file_id);

    let record = store.find_path("a.md").await.unwrap().unwrap();
    assert_eq!((record.version_no, record.current_hash.as_str()), (2, "h2"));
    let events = store.events(&file_id).await.unwrap();
    assert_eq!(kinds(&events), vec!["created", "modified"]);
    assert_eq!(events[1].ledger_prev_hash, events[0].ledger_hash);
    assert_eq!(events[1].previous_hash, "h1");

    let err = store.append_version(pending("a.md", "h2", false)).await.unwrap_err();
    assert!(err.to_string().contains("no content change"), "{err}");
    assert_eq!(store.versions(&file_id).await.unwrap().len(), 2);
    assert!(store.verify_chain(&file_id).await.unwrap());
}

#[tokio::test]
async fn tc103_sensitive_flag_round_trip() {
    let (_tmp, store) = fixture().await;
    let (secret_id, _, _) = store.append_version(pending("s.pem", "hs", true)).await.unwrap();
    store.append_version(pending("n.md", "hn", false)).await.unwrap();

    assert!(store.find_path("s.pem").await.unwrap().unwrap().sensitive);
    assert!(!store.find_path("n.md").await.unwrap().unwrap().sensitive);
    assert!(store.versions(&secret_id).await.unwrap()[0].sensitive);
    assert!(store.is_hash_sensitive("hs").await.unwrap());
    assert!(!store.is_hash_sensitive("hn").await.unwrap());
    assert!(!store.is_hash_sensitive("unknown").await.unwrap());
}

#[tokio::test]
async fn tc104_mark_deleted_appends_event_and_is_idempotent() {
    let (_tmp, store) = fixture().await;
    let (file_id, _, _) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    let record = store.find_path("a.md").await.unwrap().unwrap();

    store.mark_deleted(&record, "scanner", "gone").await.unwrap();
    let deleted = store.find_path("a.md").await.unwrap().unwrap();
    assert!(deleted.deleted);
    let events = store.events(&file_id).await.unwrap();
    assert_eq!(kinds(&events), vec!["created", "deleted"]);
    assert_eq!((events[1].actor.as_str(), events[1].reason.as_str()), ("scanner", "gone"));
    assert_eq!((events[1].old_path.as_str(), events[1].new_path.as_str()), ("a.md", ""));
    assert!(store.verify_chain(&file_id).await.unwrap());

    store.mark_deleted(&deleted, "scanner", "again").await.unwrap();
    assert_eq!(store.events(&file_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn tc105_append_after_delete_is_restored_event() {
    let (_tmp, store) = fixture().await;
    let (file_id, _, _) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    let record = store.find_path("a.md").await.unwrap().unwrap();
    store.mark_deleted(&record, "scanner", "gone").await.unwrap();

    store.append_version(pending("a.md", "h2", false)).await.unwrap();
    let after = store.find_path("a.md").await.unwrap().unwrap();
    assert!(!after.deleted);
    assert_eq!(after.version_no, 2);
    let events = store.events(&file_id).await.unwrap();
    assert_eq!(kinds(&events), vec!["created", "deleted", "restored"]);
    assert!(store.verify_chain(&file_id).await.unwrap());

    // The same bytes are accepted again when the previous record was deleted.
    let (other_id, _, _) = store.append_version(pending("b.md", "hb", false)).await.unwrap();
    let other = store.find_path("b.md").await.unwrap().unwrap();
    store.mark_deleted(&other, "scanner", "gone").await.unwrap();
    store.append_version(pending("b.md", "hb", false)).await.unwrap();
    assert_eq!(kinds(&store.events(&other_id).await.unwrap()), vec!["created", "deleted", "restored"]);
}

#[tokio::test]
async fn tc106_archive_record_upsert() {
    let (_tmp, store) = fixture().await;
    let object = |provider: &str, locator: &str| StorageObject {
        content_hash: "h1".into(),
        provider: provider.into(),
        provider_locator: locator.into(),
        encrypted: true,
        verified_at: "2026-11-19T00:00:00Z".into(),
    };
    store.archive_record(&object("nas", "loc1")).await.unwrap();
    store.archive_record(&object("nas", "loc2")).await.unwrap();
    let records = store.archives("h1").await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_locator, "loc2");
    assert!(records[0].encrypted);

    store.archive_record(&object("s3", "locx")).await.unwrap();
    assert_eq!(store.archives("h1").await.unwrap().len(), 2);
}

#[tokio::test]
async fn tc107_set_tags_filters_and_sorts() {
    let (_tmp, store) = fixture().await;
    let (file_id, _, _) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    let too_long = "x".repeat(81);
    let exactly_80 = "y".repeat(80);
    let tags = vec!["b".to_string(), "a".to_string(), "   ".to_string(), too_long, exactly_80.clone()];
    store.set_tags(&file_id, &tags, "rule", "v1").await.unwrap();
    assert_eq!(store.tags(&file_id).await.unwrap(), vec!["a".to_string(), "b".to_string(), exactly_80.clone()]);

    store.set_tags(&file_id, &["a".to_string()], "ai", "v2").await.unwrap();
    assert_eq!(store.tags(&file_id).await.unwrap().len(), 3);
}

#[tokio::test]
async fn tc108_annotation_task_and_governance_flow() {
    let (_tmp, store) = fixture().await;
    let (file_id, version_id, _) = store.append_version(pending("a.md", "h1", false)).await.unwrap();
    let annotation = store.add_annotation(&file_id, "H1:A#1", &version_id, "note").await.unwrap();
    let task = store.create_task(&file_id, "H1:A#1", &version_id, "Review", "check impacts").await.unwrap();
    assert_eq!(store.list_tasks(&file_id).await.unwrap()[0]["status"], "open");
    store.set_task_status(&task, "done", "human").await.unwrap();

    let tasks = store.list_tasks(&file_id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["id"], task.as_str());
    assert_eq!(tasks[0]["title"], "Review");
    assert_eq!(tasks[0]["description"], "check impacts");
    assert_eq!(tasks[0]["section"], "H1:A#1");
    assert_eq!(tasks[0]["status"], "done");

    let events = store.governance_events(&file_id).await.unwrap();
    let event_kinds: Vec<&str> = events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(event_kinds, vec!["annotation.created", "task.created", "task.state_changed"]);
    assert_eq!(events[0]["subject_id"], annotation.as_str());
    assert_eq!(events[2]["payload"], "done");
}

#[tokio::test]
async fn tc109_invalid_task_state_and_unknown_task() {
    let (_tmp, store) = fixture().await;
    let err = store.set_task_status("any", "bogus", "human").await.unwrap_err();
    assert!(err.to_string().contains("invalid task state"), "{err}");
    let err = store.set_task_status("missing-id", "open", "human").await.unwrap_err();
    assert!(err.to_string().contains("task missing"), "{err}");
}

#[tokio::test]
async fn tc110_missing_records_return_none() {
    let (_tmp, store) = fixture().await;
    assert!(store.find_path("nope").await.unwrap().is_none());
    assert!(store.latest_version("nope").await.unwrap().is_none());
    assert!(store.version("nope").await.unwrap().is_none());
    assert!(store.versions("nope").await.unwrap().is_empty());
    assert!(store.events("nope").await.unwrap().is_empty());
    assert!(store.archives("nope").await.unwrap().is_empty());
    assert!(store.tags("nope").await.unwrap().is_empty());
    assert!(store.list_files().await.unwrap().is_empty());
    assert!(store.governance_events("nope").await.unwrap().is_empty());
    assert!(store.list_tasks("nope").await.unwrap().is_empty());
    assert!(!store.is_hash_sensitive("nope").await.unwrap());
    assert!(store.verify_chain("nope").await.unwrap());
}

#[tokio::test]
async fn tc111_special_characters_bound_verbatim() {
    let (_tmp, store) = fixture().await;
    let reason = "quote ' double \" newline\n結合テスト '); DROP TABLE resources;--";
    let mut p = pending("a.md", "h1", false);
    p.reason = reason.into();
    p.actor = "ac'tor".into();
    let (file_id, _, _) = store.append_version(p).await.unwrap();

    let versions = store.versions(&file_id).await.unwrap();
    assert_eq!(versions[0].reason, reason);
    assert_eq!(versions[0].actor, "ac'tor");
    assert_eq!(store.events(&file_id).await.unwrap()[0].reason, reason);
    store.append_version(pending("b.md", "h2", false)).await.unwrap();
    assert_eq!(store.list_files().await.unwrap().len(), 2);
}

#[tokio::test]
async fn tc112_init_is_idempotent() {
    let (_tmp, store) = fixture().await;
    store.append_version(pending("a.md", "h1", false)).await.unwrap();
    store.init().await.unwrap();
    store.init().await.unwrap();
    assert!(store.find_path("a.md").await.unwrap().is_some());
}
