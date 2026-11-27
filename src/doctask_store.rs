//! Persistence layer of the document-task module. It owns the writes to the tables created by
//! migration 004 and the reads that verification and backfill need. Each write runs in one
//! `BEGIN IMMEDIATE` transaction together with the governance-chain append that records it.
//! Permission, transition rules and text validation belong to the service layer.

use crate::doctask_model::{
    event_hash, first_chain_break, sha256_hex, validate_priority, ActorKind, AnchorState,
    ApprovalDecision, ApprovalKind, ApprovalRecord, DocError, EditKind, ErrorCode, EventFields,
    EventRecord, EvidenceKind, EvidenceRecord, EvidenceResult, EvidenceSource, PlanRecord,
    PlanStatus, ProposalState, Risk, SourceKind, TaskState, Visibility, GENESIS_PREV_HASH,
};
use crate::store::{backoff_ms, Store, StoreBusy};
use anyhow::Result;
use rand::Rng;
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::time::Duration;
use turso::transaction::{Transaction, TransactionBehavior};
use turso::{params, Connection, IntoParams, Value};
use uuid::Uuid;

/// The fourteen tables created by migration 004.
pub const DOC_TABLES: [&str; 14] = [
    "doc_counters",
    "doc_baselines",
    "doc_anchors",
    "doc_task_meta",
    "doc_annotation_meta",
    "doc_task_deps",
    "doc_task_events",
    "doc_task_plans",
    "doc_task_approvals",
    "doc_task_links",
    "doc_task_evidence",
    "doc_file_lineage",
    "doc_agent_sessions",
    "doc_agent_disclosures",
];

/// Counter names accepted by `next_counter`.
pub const COUNTERS: [&str; 3] = ["task_no", "annotation_no", "backfill_v4"];

/// Lock errors: total attempts of one write, the first included.
const LOCK_ATTEMPTS: u32 = 6;
/// Re-runs of one write after a chain-append conflict, the first attempt excluded.
const CHAIN_RETRIES: u32 = 3;
const LEGACY_ID_PREFIX: &str = "legacy:";
const IMPORTED_KIND: &str = "dt.task.imported";
const ROOT_KEY: &str = "ROOT";
const ROOT_TITLE: &str = "（全文）";
const UNKNOWN_KIND: &str = "unknown";

/// One row write of an atomic change. Table and column names are checked against the
/// document-task tables before any statement runs.
#[derive(Clone)]
pub enum RowWrite {
    /// Inserts one row with the given columns.
    Insert {
        table: &'static str,
        values: Vec<(&'static str, Value)>,
    },
    /// Updates the row whose `key` column equals `key_value`. Exactly one row must change.
    Update {
        table: &'static str,
        key: &'static str,
        key_value: Value,
        values: Vec<(&'static str, Value)>,
    },
    /// Updates every row that matches all `filters`; zero matching rows is not an error.
    UpdateWhere {
        table: &'static str,
        filters: Vec<(&'static str, Value)>,
        values: Vec<(&'static str, Value)>,
    },
    /// Deletes the row that matches all `filters`. Exactly one row must match.
    Delete {
        table: &'static str,
        filters: Vec<(&'static str, Value)>,
    },
}

/// The compare-and-set of a task status performed by `DocStore::commit_step`.
#[derive(Debug, Clone)]
pub struct StatusChange {
    pub task_id: String,
    pub from: TaskState,
    pub to: TaskState,
}

/// One governance event of a step, with the identifier chosen by the caller.
#[derive(Debug, Clone)]
pub struct StepEvent {
    pub id: String,
    pub event: NewEvent,
}

/// One row of `doc_task_links`, removed rows included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    pub id: String,
    pub task_id: String,
    pub link_kind: String,
    pub target_kind: String,
    pub target_file_id: String,
    pub target_section: String,
    pub target_ref: String,
    pub label: String,
    pub created_by: String,
    pub created_by_kind: String,
    pub session: String,
    pub created_at: String,
    pub removed_at: Option<String>,
}

/// The identity, state and review columns of one `doc_anchors` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorReview {
    pub id: String,
    pub state: String,
    pub baseline_id: String,
    pub current_version_id: String,
    pub reviewed_version_id: String,
}

/// Clause fingerprint stored with an anchor created on a baseline version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorFingerprint {
    pub baseline_id: String,
    pub section_key: String,
    pub clause_number: String,
    pub title: String,
    pub level: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub full_hash: String,
    pub body_hash: String,
}

/// One row of `doc_baselines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineRow {
    pub id: String,
    pub file_id: String,
    pub version_id: String,
    pub version_no: i64,
    pub content_hash: String,
    pub approver: String,
    pub note: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
}

/// One task with its meta row and its anchor columns (`None` when no anchor row matches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: String,
    pub task_no: i64,
    pub file_id: String,
    pub anchor_id: String,
    pub anchor_section: String,
    pub base_version_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub owner: String,
    pub priority: i64,
    pub proposal_state: String,
    pub visibility: String,
    pub source_kind: String,
    pub created_by: String,
    pub created_by_kind: String,
    pub session: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub anchor_state: Option<String>,
    pub clause_number: Option<String>,
    pub baseline_id: Option<String>,
}

/// One annotation with its meta row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationRow {
    pub id: String,
    pub annotation_no: i64,
    pub file_id: String,
    pub anchor_id: String,
    pub section_key: String,
    pub base_version_id: String,
    pub text: String,
    pub author: String,
    pub author_kind: String,
    pub visibility: String,
    pub rationale: String,
    pub quote: String,
    pub created_at: String,
}

/// Every column of one `doc_anchors` row. `level`, `line_start` and `line_end` are integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorRecord {
    pub id: String,
    pub file_id: String,
    pub base_version_id: String,
    pub baseline_id: String,
    pub section_key: String,
    pub clause_id: String,
    pub title: String,
    pub level: i64,
    pub line_start: i64,
    pub line_end: i64,
    pub full_hash: String,
    pub body_hash: String,
    pub state: String,
    pub current_version_id: String,
    pub current_key: String,
    pub current_hash: String,
    pub reviewed_version_id: String,
    pub updated_at: String,
}

/// One row of `doc_file_lineage`: a proposed, confirmed or rejected move of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageRow {
    pub id: String,
    pub old_file_id: String,
    pub new_file_id: String,
    pub evidence: String,
    pub score_permille: i64,
    pub state: String,
    pub decided_by: String,
    pub decided_at: Option<String>,
    pub created_at: String,
}

/// Document-task persistence over one library file.
#[derive(Debug, Clone)]
pub struct DocStore {
    store: Store,
}

/// One governance event before storage. The id and timestamp are generated by the store.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub task_id: String,
    pub file_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub kind: String,
    pub from_status: String,
    pub to_status: String,
    pub actor: String,
    pub actor_kind: ActorKind,
    pub session: String,
    pub claimed_session: String,
    pub payload_json: String,
}

/// A task to create. Its anchor must belong to `file_id`.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub file_id: String,
    pub anchor_id: String,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub priority: u8,
    pub proposal_state: ProposalState,
    pub visibility: Visibility,
    pub source_kind: SourceKind,
    pub created_by: String,
    pub created_by_kind: ActorKind,
    pub session: String,
}

/// Identifiers of a created task and of its `dt.task.created` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTask {
    pub task_id: String,
    pub task_no: i64,
    pub event_seq: i64,
}

/// An annotation to add. Its anchor must belong to `file_id`.
#[derive(Debug, Clone)]
pub struct NewAnnotation {
    pub file_id: String,
    pub anchor_id: String,
    pub text: String,
    pub author: String,
    pub author_kind: ActorKind,
    pub rationale: String,
    pub visibility: Visibility,
    pub quote: String,
    pub line_start: i64,
    pub line_end: i64,
    pub session: String,
}

/// Identifiers of a created annotation and of its `dt.annotation.created` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedAnnotation {
    pub annotation_id: String,
    pub annotation_no: i64,
    pub event_seq: i64,
}

/// A task whose stored status differs from the last status recorded in the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusDrift {
    pub task_id: String,
    pub status: String,
    pub replayed: Option<String>,
}

/// Counts of one backfill run. A repeated run reports zeros.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    pub anchors_created: i64,
    pub tasks_imported: i64,
    pub annotations_imported: i64,
    pub events_imported: i64,
    pub status_events_added: i64,
}

impl DocStore {
    pub fn new(store: Store) -> DocStore {
        DocStore { store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Returns the id of the anchor for the triple, creating it in state `state` when absent.
    pub async fn ensure_anchor(
        &self,
        file_id: &str,
        base_version_id: &str,
        section_key: &str,
        state: AnchorState,
    ) -> Result<String> {
        let now = now();
        let store = &self.store;
        let (file_id, base_version_id, section_key, now) =
            (file_id, base_version_id, section_key, now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = ensure_anchor_tx(&tx, file_id, base_version_id, section_key, state, now)
                .await
                .map(|(id, _)| id);
            finish(tx, outcome).await
        })
        .await
    }

    /// Increments a named counter and returns the new value.
    pub async fn next_counter(&self, name: &str) -> Result<i64> {
        if !COUNTERS.contains(&name) {
            return Err(doc_error(
                ErrorCode::InvalidArg,
                "未知的计数器名称",
                "使用 task_no、annotation_no 或 backfill_v4",
            ));
        }
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = next_counter_tx(&tx, name).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Appends one event to the governance chain.
    pub async fn append_event(&self, ev: &NewEvent) -> Result<EventRecord> {
        validate_event(ev)?;
        let id = new_id();
        let now = now();
        let store = &self.store;
        let (ev, id, now) = (ev, id.as_str(), now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = append_draft(&tx, draft_from_event(ev, id, now)).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Creates a task in status `open` with its meta row and its `dt.task.created` event.
    pub async fn create_task(&self, new: &NewTask) -> Result<CreatedTask> {
        let priority = validate_priority(i64::from(new.priority))?;
        let task_id = new_id();
        let event_id = new_id();
        let now = now();
        let store = &self.store;
        let (new, task_id, event_id, now) =
            (new, task_id.as_str(), event_id.as_str(), now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = create_task_tx(&tx, new, priority, task_id, event_id, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Adds an annotation and its `dt.annotation.created` event.
    pub async fn add_annotation(&self, new: &NewAnnotation) -> Result<CreatedAnnotation> {
        let annotation_id = new_id();
        let event_id = new_id();
        let now = now();
        let store = &self.store;
        let (new, annotation_id, event_id, now) =
            (new, annotation_id.as_str(), event_id.as_str(), now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = add_annotation_tx(&tx, new, annotation_id, event_id, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Compare-and-set transition of a task status. The stored status must equal `from_status`.
    pub async fn transition(&self, ev: &NewEvent) -> Result<EventRecord> {
        validate_event(ev)?;
        let from = TaskState::parse(&ev.from_status).ok_or_else(|| {
            doc_error(ErrorCode::InvalidArg, "未知的任务状态", "使用合法的状态名")
        })?;
        let to = TaskState::parse(&ev.to_status).ok_or_else(|| {
            doc_error(ErrorCode::InvalidArg, "未知的任务状态", "使用合法的状态名")
        })?;
        let id = new_id();
        let now = now();
        let store = &self.store;
        let (ev, id, now) = (ev, id.as_str(), now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = transition_tx(&tx, ev, from, to, id, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Generic row writer for the document-task tables. It writes no event.
    pub async fn insert_row(&self, table: &str, values: &[(&str, Value)]) -> Result<()> {
        if !is_doc_table(table)
            || values.is_empty()
            || values.iter().any(|(column, _)| !is_identifier(column))
        {
            return Err(doc_error(
                ErrorCode::InvalidArg,
                "非法的表名或列名",
                "只能写入文档任务表的合法列",
            ));
        }
        let columns: Vec<&str> = values.iter().map(|(column, _)| *column).collect();
        let placeholders: Vec<String> = (1..=values.len())
            .map(|index| format!("?{index}"))
            .collect();
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            columns.join(", "),
            placeholders.join(", ")
        );
        let bound: Vec<Value> = values.iter().map(|(_, value)| value.clone()).collect();
        let store = &self.store;
        let (sql, bound) = (sql.as_str(), &bound);
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = tx
                .execute(sql, bound.clone())
                .await
                .map(|_| ())
                .map_err(anyhow::Error::from);
            finish(tx, outcome).await
        })
        .await
    }

    /// Imports the legacy rows and their governance events. Running it again imports nothing
    /// new unless a status changed without an event.
    pub async fn backfill(&self) -> Result<BackfillReport> {
        let now = now();
        let store = &self.store;
        let now = now.as_str();
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = backfill_tx(&tx, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Every event of the chain in `seq` order.
    pub async fn events(&self) -> Result<Vec<EventRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            read_events(&conn).await
        })
        .await
    }

    /// The `seq` of the first record that breaks the chain, or `None` when the chain is intact.
    pub async fn verify_chain(&self) -> Result<Option<i64>> {
        let events = self.events().await?;
        Ok(first_chain_break(&events))
    }

    /// Tasks whose stored status differs from the last status event of the chain.
    pub async fn status_drift(&self) -> Result<Vec<StatusDrift>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            status_drift_on(&conn).await
        })
        .await
    }

    /// `(task_id, depends_on)` pairs of the dependency table.
    pub async fn dependency_edges(&self) -> Result<Vec<(String, String)>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let rows = text_rows(
                &conn,
                "SELECT task_id, depends_on FROM doc_task_deps ORDER BY task_id, depends_on",
                (),
                2,
            )
            .await?;
            Ok(rows
                .into_iter()
                .map(|row| (row[0].clone(), row[1].clone()))
                .collect())
        })
        .await
    }

    /// `(file_id, version_id, count)` for every version with more than one active baseline.
    pub async fn duplicate_baselines(&self) -> Result<Vec<(String, String, i64)>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let rows = text_rows(
                &conn,
                "SELECT file_id, version_id FROM doc_baselines WHERE revoked_at IS NULL ORDER BY file_id, version_id",
                (),
                2,
            )
            .await?;
            let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();
            for row in rows {
                *counts.entry((row[0].clone(), row[1].clone())).or_insert(0) += 1;
            }
            Ok(counts
                .into_iter()
                .filter(|(_, count)| *count > 1)
                .map(|((file_id, version_id), count)| (file_id, version_id, count))
                .collect())
        })
        .await
    }

    /// Applies `writes` in order and appends `ev` to the chain, in one Immediate transaction.
    /// Every identifier is supplied by the caller, so a retried transaction writes the same rows.
    pub async fn write_with_event(
        &self,
        writes: &[RowWrite],
        ev: &NewEvent,
    ) -> Result<EventRecord> {
        validate_event(ev)?;
        for write in writes {
            validate_write(write)?;
        }
        let id = new_id();
        let now = now();
        let store = &self.store;
        let (writes, ev, id, now) = (writes, ev, id.as_str(), now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = write_with_event_tx(&tx, writes, ev, id, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// One Immediate transaction for a flow step: the optional compare-and-set of a task status,
    /// then the row writes in order, then the events in order. Every identifier comes from the
    /// caller, so a retried attempt writes the same rows; any error rolls the whole step back.
    /// Returns the stored event records in order. `events` must not be empty.
    pub async fn commit_step(
        &self,
        change: Option<&StatusChange>,
        writes: &[RowWrite],
        events: &[StepEvent],
    ) -> Result<Vec<EventRecord>> {
        if events.is_empty() {
            return Err(doc_error(
                ErrorCode::InvalidArg,
                "一次写入必须附带至少一条事件",
                "附带事件后重试",
            ));
        }
        for step in events {
            validate_event(&step.event)?;
        }
        for write in writes {
            validate_write(write)?;
        }
        let now = now();
        let store = &self.store;
        let (change, writes, events, now) = (change, writes, events, now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = commit_step_tx(&tx, change, writes, events, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// Returns the id of the anchor for the clause of `fp` on the base version, creating it in
    /// state `valid` with the fingerprint when it is absent. An existing anchor is unchanged.
    pub async fn ensure_anchor_clause(
        &self,
        file_id: &str,
        base_version_id: &str,
        fp: &AnchorFingerprint,
    ) -> Result<String> {
        let now = now();
        let store = &self.store;
        let (file_id, base_version_id, fp, now) = (file_id, base_version_id, fp, now.as_str());
        with_retry(move || async move {
            let conn = store.connect().await?;
            let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
            let outcome = ensure_anchor_clause_tx(&tx, file_id, base_version_id, fp, now).await;
            finish(tx, outcome).await
        })
        .await
    }

    /// True when migration 4 is recorded in `schema_migrations`. Writes nothing.
    pub async fn schema_current(&self) -> Result<bool> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let count = query_i64(
                &conn,
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 4",
                (),
            )
            .await?;
            Ok(count > 0)
        })
        .await
    }

    /// Baseline rows, of one file when `file_id` is given, ordered by creation time and id.
    pub async fn baselines(&self, file_id: Option<&str>) -> Result<Vec<BaselineRow>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = match file_id {
                Some(id) => conn.query(BASELINES_OF_FILE_SQL, params![id]).await?,
                None => conn.query(BASELINES_SQL, ()).await?,
            };
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(BaselineRow {
                    id: row.get::<String>(0)?,
                    file_id: row.get::<String>(1)?,
                    version_id: row.get::<String>(2)?,
                    version_no: row.get::<i64>(3)?,
                    content_hash: row.get::<String>(4)?,
                    approver: row.get::<String>(5)?,
                    note: row.get::<String>(6)?,
                    created_at: row.get::<String>(7)?,
                    revoked_at: row.get::<Option<String>>(8)?,
                    revoked_by: row.get::<Option<String>>(9)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every task with its meta row and anchor columns, ordered by task number.
    pub async fn tasks(&self) -> Result<Vec<TaskRow>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(TASKS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(TaskRow {
                    id: row.get::<String>(0)?,
                    task_no: row.get::<i64>(1)?,
                    file_id: row.get::<String>(2)?,
                    anchor_id: row.get::<String>(3)?,
                    anchor_section: row.get::<String>(4)?,
                    base_version_id: row.get::<String>(5)?,
                    title: row.get::<String>(6)?,
                    description: row.get::<String>(7)?,
                    status: row.get::<String>(8)?,
                    owner: row.get::<String>(9)?,
                    priority: row.get::<i64>(10)?,
                    proposal_state: row.get::<String>(11)?,
                    visibility: row.get::<String>(12)?,
                    source_kind: row.get::<String>(13)?,
                    created_by: row.get::<String>(14)?,
                    created_by_kind: row.get::<String>(15)?,
                    session: row.get::<String>(16)?,
                    created_at: row.get::<String>(17)?,
                    updated_at: row.get::<String>(18)?,
                    closed_at: row.get::<Option<String>>(19)?,
                    anchor_state: row.get::<Option<String>>(20)?,
                    clause_number: row.get::<Option<String>>(21)?,
                    baseline_id: row.get::<Option<String>>(22)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every annotation with its meta row, ordered by annotation number.
    pub async fn annotations(&self) -> Result<Vec<AnnotationRow>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(ANNOTATIONS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(AnnotationRow {
                    id: row.get::<String>(0)?,
                    annotation_no: row.get::<i64>(1)?,
                    file_id: row.get::<String>(2)?,
                    anchor_id: row.get::<String>(3)?,
                    section_key: row.get::<String>(4)?,
                    base_version_id: row.get::<String>(5)?,
                    text: row.get::<String>(6)?,
                    author: row.get::<String>(7)?,
                    author_kind: row.get::<String>(8)?,
                    visibility: row.get::<String>(9)?,
                    rationale: row.get::<String>(10)?,
                    quote: row.get::<String>(11)?,
                    created_at: row.get::<String>(12)?,
                });
            }
            Ok(out)
        })
        .await
    }
}

const PLANS_SQL: &str = "SELECT id, task_id, plan_no, summary, rationale, impact, risk, options_json, chosen_option, edit_kind, target_file_id, target_section, new_text, expected_version_id, expected_hash, result_hash, plan_hash, status, submitted_by_kind, created_at FROM doc_task_plans ORDER BY task_id, plan_no";
const APPROVALS_SQL: &str = "SELECT a.id, a.task_id, a.kind, a.decision, a.approver_kind, a.plan_id, a.subject_hash, a.base_version_id, a.comment, a.event_id, a.created_at FROM doc_task_approvals a ORDER BY (SELECT e.seq FROM doc_task_events e WHERE e.id = a.event_id)";
const EVIDENCE_SQL: &str = "SELECT v.id, v.task_id, v.kind, v.result, v.source, v.actor_kind, v.created_at FROM doc_task_evidence v ORDER BY (SELECT e.seq FROM doc_task_events e WHERE e.id = v.event_id)";
const LINKS_SQL: &str = "SELECT id, task_id, link_kind, target_kind, target_file_id, target_section, target_ref, label, created_by, created_by_kind, session, created_at, removed_at FROM doc_task_links ORDER BY created_at, id";
const ANCHORS_SQL: &str = "SELECT id, file_id, base_version_id, baseline_id, section_key, clause_id, title, level, line_start, line_end, full_hash, body_hash, state, current_version_id, current_key, current_hash, reviewed_version_id, updated_at FROM doc_anchors ORDER BY created_at, id";
const LINEAGE_SQL: &str = "SELECT id, old_file_id, new_file_id, evidence, score_permille, state, decided_by, decided_at, created_at FROM doc_file_lineage ORDER BY created_at, id";
const ANCHOR_REVIEWS_SQL: &str =
    "SELECT id, state, baseline_id, current_version_id, reviewed_version_id FROM doc_anchors ORDER BY id";

impl DocStore {
    /// Every plan row, ordered by task and plan number.
    pub async fn plans(&self) -> Result<Vec<PlanRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(PLANS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let options_json = row.get::<String>(7)?;
                let options: Vec<String> = serde_json::from_str(&options_json).map_err(|_| {
                    doc_error(
                        ErrorCode::Integrity,
                        "存储的方案选项不合法",
                        "运行 doc verify 检查数据库",
                    )
                })?;
                out.push(PlanRecord {
                    id: row.get::<String>(0)?,
                    task_id: row.get::<String>(1)?,
                    plan_no: row.get::<i64>(2)?,
                    summary: row.get::<String>(3)?,
                    rationale: row.get::<String>(4)?,
                    impact: row.get::<String>(5)?,
                    risk: parse_stored(Risk::parse, &row.get::<String>(6)?)?,
                    options,
                    chosen_option: row.get::<String>(8)?,
                    edit_kind: parse_stored(EditKind::parse, &row.get::<String>(9)?)?,
                    target_file_id: row.get::<String>(10)?,
                    target_section: row.get::<String>(11)?,
                    new_text: row.get::<String>(12)?,
                    expected_version_id: row.get::<String>(13)?,
                    expected_hash: row.get::<String>(14)?,
                    result_hash: row.get::<String>(15)?,
                    plan_hash: row.get::<String>(16)?,
                    status: parse_stored(PlanStatus::parse, &row.get::<String>(17)?)?,
                    submitted_by_kind: parse_stored(ActorKind::parse, &row.get::<String>(18)?)?,
                    created_at: row.get::<String>(19)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every approval row, in chain order of its event.
    pub async fn approvals(&self) -> Result<Vec<ApprovalRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(APPROVALS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(ApprovalRecord {
                    id: row.get::<String>(0)?,
                    task_id: row.get::<String>(1)?,
                    kind: parse_stored(ApprovalKind::parse, &row.get::<String>(2)?)?,
                    decision: parse_stored(ApprovalDecision::parse, &row.get::<String>(3)?)?,
                    approver_kind: parse_stored(ActorKind::parse, &row.get::<String>(4)?)?,
                    plan_id: row.get::<String>(5)?,
                    subject_hash: row.get::<String>(6)?,
                    base_version_id: row.get::<String>(7)?,
                    comment: row.get::<String>(8)?,
                    event_id: row.get::<String>(9)?,
                    created_at: row.get::<String>(10)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every evidence row, in chain order of its event.
    pub async fn evidence(&self) -> Result<Vec<EvidenceRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(EVIDENCE_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(EvidenceRecord {
                    id: row.get::<String>(0)?,
                    task_id: row.get::<String>(1)?,
                    kind: parse_stored(EvidenceKind::parse, &row.get::<String>(2)?)?,
                    result: parse_stored(EvidenceResult::parse, &row.get::<String>(3)?)?,
                    source: parse_stored(EvidenceSource::parse, &row.get::<String>(4)?)?,
                    actor_kind: parse_stored(ActorKind::parse, &row.get::<String>(5)?)?,
                    created_at: row.get::<String>(6)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every link row, removed rows included.
    pub async fn links(&self) -> Result<Vec<LinkRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(LINKS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(LinkRecord {
                    id: row.get::<String>(0)?,
                    task_id: row.get::<String>(1)?,
                    link_kind: row.get::<String>(2)?,
                    target_kind: row.get::<String>(3)?,
                    target_file_id: row.get::<String>(4)?,
                    target_section: row.get::<String>(5)?,
                    target_ref: row.get::<String>(6)?,
                    label: row.get::<String>(7)?,
                    created_by: row.get::<String>(8)?,
                    created_by_kind: row.get::<String>(9)?,
                    session: row.get::<String>(10)?,
                    created_at: row.get::<String>(11)?,
                    removed_at: row.get::<Option<String>>(12)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// The identity, state and review columns of every anchor row.
    pub async fn anchor_reviews(&self) -> Result<Vec<AnchorReview>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(ANCHOR_REVIEWS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(AnchorReview {
                    id: row.get::<String>(0)?,
                    state: row.get::<String>(1)?,
                    baseline_id: row.get::<String>(2)?,
                    current_version_id: row.get::<String>(3)?,
                    reviewed_version_id: row.get::<String>(4)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every anchor row with all its columns, ordered by creation time and id.
    pub async fn anchors(&self) -> Result<Vec<AnchorRecord>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(ANCHORS_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(AnchorRecord {
                    id: row.get::<String>(0)?,
                    file_id: row.get::<String>(1)?,
                    base_version_id: row.get::<String>(2)?,
                    baseline_id: row.get::<String>(3)?,
                    section_key: row.get::<String>(4)?,
                    clause_id: row.get::<String>(5)?,
                    title: row.get::<String>(6)?,
                    level: row.get::<i64>(7)?,
                    line_start: row.get::<i64>(8)?,
                    line_end: row.get::<i64>(9)?,
                    full_hash: row.get::<String>(10)?,
                    body_hash: row.get::<String>(11)?,
                    state: row.get::<String>(12)?,
                    current_version_id: row.get::<String>(13)?,
                    current_key: row.get::<String>(14)?,
                    current_hash: row.get::<String>(15)?,
                    reviewed_version_id: row.get::<String>(16)?,
                    updated_at: row.get::<String>(17)?,
                });
            }
            Ok(out)
        })
        .await
    }

    /// Every lineage row, ordered by creation time and id.
    pub async fn lineage(&self) -> Result<Vec<LineageRow>> {
        let store = &self.store;
        with_retry(move || async move {
            let conn = store.connect().await?;
            let mut rows = conn.query(LINEAGE_SQL, ()).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                out.push(LineageRow {
                    id: row.get::<String>(0)?,
                    old_file_id: row.get::<String>(1)?,
                    new_file_id: row.get::<String>(2)?,
                    evidence: row.get::<String>(3)?,
                    score_permille: row.get::<i64>(4)?,
                    state: row.get::<String>(5)?,
                    decided_by: row.get::<String>(6)?,
                    decided_at: row.get::<Option<String>>(7)?,
                    created_at: row.get::<String>(8)?,
                });
            }
            Ok(out)
        })
        .await
    }
}

/// Runs one write attempt until it succeeds or a retry limit is reached. A lock error is retried
/// with backoff up to `LOCK_ATTEMPTS` attempts. A chain-append conflict is retried up to
/// `CHAIN_RETRIES` times. Each attempt opens a new connection and runs a new transaction, so the
/// closure must generate no identifiers of its own.
async fn with_retry<T, F, Fut>(mut attempt: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut lock_attempts: u32 = 0;
    let mut chain_retries: u32 = 0;
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if err
                    .downcast_ref::<turso::Error>()
                    .is_some_and(is_lock_error)
                {
                    lock_attempts += 1;
                    if lock_attempts >= LOCK_ATTEMPTS {
                        return Err(StoreBusy {
                            attempts: lock_attempts,
                            detail: err.to_string(),
                        }
                        .into());
                    }
                    let jitter = rand::thread_rng().gen_range(0..=500u64);
                    tokio::time::sleep(Duration::from_millis(backoff_ms(lock_attempts, jitter)))
                        .await;
                } else if is_chain_conflict(&err) && chain_retries < CHAIN_RETRIES {
                    chain_retries += 1;
                } else {
                    return Err(err);
                }
            }
        }
    }
}

/// Commits when the body succeeded and rolls back otherwise.
async fn finish<T>(tx: Transaction<'_>, outcome: Result<T>) -> Result<T> {
    match outcome {
        Ok(value) => {
            tx.commit().await?;
            Ok(value)
        }
        Err(err) => {
            let _ = tx.rollback().await;
            Err(err)
        }
    }
}

struct Draft {
    id: String,
    task_id: String,
    file_id: String,
    subject_kind: String,
    subject_id: String,
    kind: String,
    from_status: String,
    to_status: String,
    actor: String,
    actor_kind: String,
    session: String,
    claimed_session: String,
    payload_json: String,
    timestamp: String,
}

fn draft_from_event(ev: &NewEvent, id: &str, timestamp: &str) -> Draft {
    Draft {
        id: id.to_string(),
        task_id: ev.task_id.clone(),
        file_id: ev.file_id.clone(),
        subject_kind: ev.subject_kind.clone(),
        subject_id: ev.subject_id.clone(),
        kind: ev.kind.clone(),
        from_status: ev.from_status.clone(),
        to_status: ev.to_status.clone(),
        actor: ev.actor.clone(),
        actor_kind: ev.actor_kind.as_str().to_string(),
        session: ev.session.clone(),
        claimed_session: ev.claimed_session.clone(),
        payload_json: ev.payload_json.clone(),
        timestamp: timestamp.to_string(),
    }
}

fn validate_event(ev: &NewEvent) -> Result<()> {
    if ev.kind.is_empty() {
        return Err(doc_error(
            ErrorCode::InvalidArg,
            "事件类型不能为空",
            "填写事件类型",
        ));
    }
    if serde_json::from_str::<Json>(&ev.payload_json).is_err() {
        return Err(doc_error(
            ErrorCode::InvalidArg,
            "事件载荷不是合法的 JSON",
            "以 JSON 文本提供载荷",
        ));
    }
    Ok(())
}

/// Appends a chain record: reads the tail hash, computes the hash and inserts the row.
async fn append_draft(conn: &Connection, d: Draft) -> Result<EventRecord> {
    let prev_hash = tail_hash(conn).await?;
    let hash = event_hash(&EventFields {
        prev_hash: &prev_hash,
        id: &d.id,
        task_id: &d.task_id,
        file_id: &d.file_id,
        subject_kind: &d.subject_kind,
        subject_id: &d.subject_id,
        kind: &d.kind,
        from_status: &d.from_status,
        to_status: &d.to_status,
        actor: &d.actor,
        actor_kind: &d.actor_kind,
        session: &d.session,
        claimed_session: &d.claimed_session,
        payload_json: &d.payload_json,
        timestamp: &d.timestamp,
    });
    conn.execute(
        "INSERT INTO doc_task_events (id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            d.id.as_str(),
            d.task_id.as_str(),
            d.file_id.as_str(),
            d.subject_kind.as_str(),
            d.subject_id.as_str(),
            d.kind.as_str(),
            d.from_status.as_str(),
            d.to_status.as_str(),
            d.actor.as_str(),
            d.actor_kind.as_str(),
            d.session.as_str(),
            d.claimed_session.as_str(),
            d.payload_json.as_str(),
            d.timestamp.as_str(),
            prev_hash.as_str(),
            hash.as_str()
        ],
    )
    .await?;
    let seq = query_i64(
        conn,
        "SELECT seq FROM doc_task_events WHERE id = ?1",
        params![d.id.as_str()],
    )
    .await?;
    Ok(EventRecord {
        seq,
        id: d.id,
        task_id: d.task_id,
        file_id: d.file_id,
        subject_kind: d.subject_kind,
        subject_id: d.subject_id,
        kind: d.kind,
        from_status: d.from_status,
        to_status: d.to_status,
        actor: d.actor,
        actor_kind: d.actor_kind,
        session: d.session,
        claimed_session: d.claimed_session,
        payload_json: d.payload_json,
        timestamp: d.timestamp,
        prev_hash,
        hash,
    })
}

async fn tail_hash(conn: &Connection) -> Result<String> {
    Ok(query_text(
        conn,
        "SELECT hash FROM doc_task_events ORDER BY seq DESC LIMIT 1",
        (),
    )
    .await?
    .unwrap_or_else(|| GENESIS_PREV_HASH.to_string()))
}

async fn ensure_anchor_tx(
    conn: &Connection,
    file_id: &str,
    base_version_id: &str,
    section_key: &str,
    state: AnchorState,
    now: &str,
) -> Result<(String, bool)> {
    let existing = query_text(
        conn,
        "SELECT id FROM doc_anchors WHERE file_id = ?1 AND base_version_id = ?2 AND section_key = ?3",
        params![file_id, base_version_id, section_key],
    )
    .await?;
    if let Some(id) = existing {
        return Ok((id, false));
    }
    let id = new_id();
    let (level, title) = parse_anchor_key(section_key);
    conn.execute(
        "INSERT INTO doc_anchors (id, file_id, base_version_id, section_key, title, level, state, current_version_id, current_key, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id.as_str(),
            file_id,
            base_version_id,
            section_key,
            title.as_str(),
            level,
            state.as_str(),
            base_version_id,
            section_key,
            now,
            now
        ],
    )
    .await?;
    Ok((id, true))
}

async fn next_counter_tx(conn: &Connection, name: &str) -> Result<i64> {
    let current = query_opt_i64(
        conn,
        "SELECT value FROM doc_counters WHERE name = ?1",
        params![name],
    )
    .await?;
    let next = current.unwrap_or(0) + 1;
    if current.is_some() {
        conn.execute(
            "UPDATE doc_counters SET value = ?2 WHERE name = ?1",
            params![name, next],
        )
        .await?;
    } else {
        conn.execute(
            "INSERT INTO doc_counters (name, value) VALUES (?1, ?2)",
            params![name, next],
        )
        .await?;
    }
    Ok(next)
}

struct AnchorRow {
    file_id: String,
    base_version_id: String,
    section_key: String,
}

async fn load_anchor(conn: &Connection, anchor_id: &str) -> Result<AnchorRow> {
    let rows = text_rows(
        conn,
        "SELECT file_id, base_version_id, section_key FROM doc_anchors WHERE id = ?1",
        params![anchor_id],
        3,
    )
    .await?;
    let Some(row) = rows.into_iter().next() else {
        return Err(doc_error(
            ErrorCode::NotFound,
            "未找到锚点",
            "先登记章节锚点",
        ));
    };
    Ok(AnchorRow {
        file_id: row[0].clone(),
        base_version_id: row[1].clone(),
        section_key: row[2].clone(),
    })
}

fn check_anchor_file(anchor: &AnchorRow, file_id: &str) -> Result<()> {
    if anchor.file_id != file_id {
        return Err(doc_error(
            ErrorCode::InvalidArg,
            "锚点不属于该文件",
            "使用该文件中的锚点",
        ));
    }
    Ok(())
}

async fn ensure_file_exists(conn: &Connection, file_id: &str) -> Result<()> {
    let count = query_i64(
        conn,
        "SELECT COUNT(*) FROM resources WHERE id = ?1",
        params![file_id],
    )
    .await?;
    if count == 0 {
        return Err(doc_error(
            ErrorCode::NotFound,
            "未找到文件",
            "确认文件已登记",
        ));
    }
    Ok(())
}

async fn anchor_id_for(
    conn: &Connection,
    file_id: &str,
    base_version_id: &str,
    section_key: &str,
) -> Result<String> {
    query_text(
        conn,
        "SELECT id FROM doc_anchors WHERE file_id = ?1 AND base_version_id = ?2 AND section_key = ?3",
        params![file_id, base_version_id, section_key],
    )
    .await?
    .ok_or_else(|| doc_error(ErrorCode::Integrity, "锚点缺失", "重新运行回填"))
}

async fn create_task_tx(
    conn: &Connection,
    new: &NewTask,
    priority: u8,
    task_id: &str,
    event_id: &str,
    now: &str,
) -> Result<CreatedTask> {
    let anchor = load_anchor(conn, &new.anchor_id).await?;
    check_anchor_file(&anchor, &new.file_id)?;
    ensure_file_exists(conn, &new.file_id).await?;
    let task_no = next_counter_tx(conn, "task_no").await?;
    conn.execute(
        "INSERT INTO doc_tasks (id, file_id, anchor_section, base_version_id, title, status, description, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6, ?7)",
        params![
            task_id,
            new.file_id.as_str(),
            anchor.section_key.as_str(),
            anchor.base_version_id.as_str(),
            new.title.as_str(),
            new.description.as_str(),
            now
        ],
    )
    .await?;
    conn.execute(
        "INSERT INTO doc_task_meta (task_id, task_no, anchor_id, owner, priority, proposal_state, visibility, source_kind, created_by, created_by_kind, session, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            task_id,
            task_no,
            new.anchor_id.as_str(),
            new.owner.as_str(),
            i64::from(priority),
            new.proposal_state.as_str(),
            new.visibility.as_str(),
            new.source_kind.as_str(),
            new.created_by.as_str(),
            new.created_by_kind.as_str(),
            new.session.as_str(),
            now
        ],
    )
    .await?;
    let payload = json!({
        "anchor_id": new.anchor_id,
        "proposal_state": new.proposal_state.as_str(),
        "source_kind": new.source_kind.as_str(),
        "task_no": task_no,
        "title_len": new.title.chars().count(),
        "title_sha256": sha256_hex(new.title.as_bytes()),
        "visibility": new.visibility.as_str(),
    })
    .to_string();
    let event = append_draft(
        conn,
        Draft {
            id: event_id.to_string(),
            task_id: task_id.to_string(),
            file_id: new.file_id.clone(),
            subject_kind: "task".to_string(),
            subject_id: task_id.to_string(),
            kind: "dt.task.created".to_string(),
            from_status: String::new(),
            to_status: TaskState::Open.as_str().to_string(),
            actor: new.created_by.clone(),
            actor_kind: new.created_by_kind.as_str().to_string(),
            session: new.session.clone(),
            claimed_session: String::new(),
            payload_json: payload,
            timestamp: now.to_string(),
        },
    )
    .await?;
    Ok(CreatedTask {
        task_id: task_id.to_string(),
        task_no,
        event_seq: event.seq,
    })
}

async fn add_annotation_tx(
    conn: &Connection,
    new: &NewAnnotation,
    annotation_id: &str,
    event_id: &str,
    now: &str,
) -> Result<CreatedAnnotation> {
    let anchor = load_anchor(conn, &new.anchor_id).await?;
    check_anchor_file(&anchor, &new.file_id)?;
    ensure_file_exists(conn, &new.file_id).await?;
    let annotation_no = next_counter_tx(conn, "annotation_no").await?;
    conn.execute(
        "INSERT INTO annotations (id, file_id, section_key, base_version_id, text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            annotation_id,
            new.file_id.as_str(),
            anchor.section_key.as_str(),
            anchor.base_version_id.as_str(),
            new.text.as_str(),
            now
        ],
    )
    .await?;
    conn.execute(
        "INSERT INTO doc_annotation_meta (annotation_id, annotation_no, anchor_id, author, author_kind, rationale, visibility, quote, line_start, line_end, session, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            annotation_id,
            annotation_no,
            new.anchor_id.as_str(),
            new.author.as_str(),
            new.author_kind.as_str(),
            new.rationale.as_str(),
            new.visibility.as_str(),
            new.quote.as_str(),
            new.line_start,
            new.line_end,
            new.session.as_str(),
            now
        ],
    )
    .await?;
    let payload = json!({
        "anchor_id": new.anchor_id,
        "annotation_no": annotation_no,
        "text_len": new.text.chars().count(),
        "text_sha256": sha256_hex(new.text.as_bytes()),
        "visibility": new.visibility.as_str(),
    })
    .to_string();
    let event = append_draft(
        conn,
        Draft {
            id: event_id.to_string(),
            task_id: String::new(),
            file_id: new.file_id.clone(),
            subject_kind: "annotation".to_string(),
            subject_id: annotation_id.to_string(),
            kind: "dt.annotation.created".to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: new.author.clone(),
            actor_kind: new.author_kind.as_str().to_string(),
            session: new.session.clone(),
            claimed_session: String::new(),
            payload_json: payload,
            timestamp: now.to_string(),
        },
    )
    .await?;
    Ok(CreatedAnnotation {
        annotation_id: annotation_id.to_string(),
        annotation_no,
        event_seq: event.seq,
    })
}

async fn transition_tx(
    conn: &Connection,
    ev: &NewEvent,
    from: TaskState,
    to: TaskState,
    event_id: &str,
    now: &str,
) -> Result<EventRecord> {
    let rows = text_rows(
        conn,
        "SELECT file_id, status FROM doc_tasks WHERE id = ?1",
        params![ev.task_id.as_str()],
        2,
    )
    .await?;
    let Some(task) = rows.into_iter().next() else {
        return Err(doc_error(ErrorCode::NotFound, "未找到任务", "确认任务 ID"));
    };
    if task[1] != from.as_str() {
        return Err(conflict());
    }
    let changed = conn
        .execute(
            "UPDATE doc_tasks SET status = ?1 WHERE id = ?2 AND status = ?3",
            params![to.as_str(), ev.task_id.as_str(), from.as_str()],
        )
        .await?;
    if changed != 1 {
        return Err(conflict());
    }
    let stored = query_text(
        conn,
        "SELECT status FROM doc_tasks WHERE id = ?1",
        params![ev.task_id.as_str()],
    )
    .await?;
    if stored.as_deref() != Some(to.as_str()) {
        return Err(conflict());
    }
    conn.execute(
        "UPDATE doc_task_meta SET updated_at = ?1 WHERE task_id = ?2",
        params![now, ev.task_id.as_str()],
    )
    .await?;
    let mut draft = draft_from_event(ev, event_id, now);
    draft.file_id = task[0].clone();
    draft.from_status = from.as_str().to_string();
    draft.to_status = to.as_str().to_string();
    append_draft(conn, draft).await
}

/// One Immediate transaction of a flow step: the optional compare-and-set of the task status,
/// the row writes in order, and the events in order.
async fn commit_step_tx(
    conn: &Connection,
    change: Option<&StatusChange>,
    writes: &[RowWrite],
    events: &[StepEvent],
    now: &str,
) -> Result<Vec<EventRecord>> {
    if let Some(change) = change {
        let stored = query_text(
            conn,
            "SELECT status FROM doc_tasks WHERE id = ?1",
            params![change.task_id.as_str()],
        )
        .await?;
        let Some(stored) = stored else {
            return Err(doc_error(ErrorCode::NotFound, "未找到任务", "确认任务 ID"));
        };
        if stored != change.from.as_str() {
            return Err(conflict());
        }
        let changed = conn
            .execute(
                "UPDATE doc_tasks SET status = ?1 WHERE id = ?2 AND status = ?3",
                params![
                    change.to.as_str(),
                    change.task_id.as_str(),
                    change.from.as_str()
                ],
            )
            .await?;
        if changed != 1 {
            return Err(conflict());
        }
        conn.execute(
            "UPDATE doc_task_meta SET updated_at = ?1 WHERE task_id = ?2",
            params![now, change.task_id.as_str()],
        )
        .await?;
    }
    for write in writes {
        apply_write(conn, write).await?;
    }
    let mut records = Vec::with_capacity(events.len());
    for step in events {
        let draft = draft_from_event(&step.event, &step.id, now);
        records.push(append_draft(conn, draft).await?);
    }
    Ok(records)
}

fn conflict() -> anyhow::Error {
    doc_error(
        ErrorCode::Conflict,
        "任务状态已变化",
        "重新读取任务后再操作",
    )
}

async fn backfill_tx(conn: &Connection, now: &str) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();

    // Anchors referenced by legacy tasks and annotations, in a fixed order.
    let mut triples: BTreeSet<(String, String, String)> = BTreeSet::new();
    for sql in [
        "SELECT file_id, base_version_id, anchor_section FROM doc_tasks",
        "SELECT file_id, base_version_id, section_key FROM annotations",
    ] {
        for row in text_rows(conn, sql, (), 3).await? {
            triples.insert((row[0].clone(), row[1].clone(), row[2].clone()));
        }
    }
    for (file_id, base_version_id, section_key) in &triples {
        let (_, created) = ensure_anchor_tx(
            conn,
            file_id,
            base_version_id,
            section_key,
            AnchorState::Unchecked,
            now,
        )
        .await?;
        if created {
            report.anchors_created += 1;
        }
    }

    // Legacy tasks, numbered by creation time. Their status is not changed.
    let legacy_tasks = text_rows(
        conn,
        "SELECT t.id, t.file_id, t.base_version_id, t.anchor_section FROM doc_tasks t WHERE NOT EXISTS (SELECT 1 FROM doc_task_meta m WHERE m.task_id = t.id) ORDER BY t.created_at, t.id",
        (),
        4,
    )
    .await?;
    for row in legacy_tasks {
        let task_no = next_counter_tx(conn, "task_no").await?;
        let anchor_id = anchor_id_for(conn, &row[1], &row[2], &row[3]).await?;
        conn.execute(
            "INSERT INTO doc_task_meta (task_id, task_no, anchor_id, owner, priority, proposal_state, proposal_decided_by, proposal_decided_at, visibility, source_kind, source_ref, created_by, created_by_kind, session, last_approval_id, cancel_reason, updated_at, closed_at) VALUES (?1, ?2, ?3, '', 2, 'accepted', '', NULL, 'local_only', 'legacy', '', 'human', 'unknown', '', '', '', ?4, NULL)",
            params![row[0].as_str(), task_no, anchor_id.as_str(), now],
        )
        .await?;
        report.tasks_imported += 1;
    }

    // Legacy annotations, numbered by creation time.
    let legacy_annotations = text_rows(
        conn,
        "SELECT a.id, a.file_id, a.section_key, a.base_version_id, a.created_at FROM annotations a WHERE NOT EXISTS (SELECT 1 FROM doc_annotation_meta m WHERE m.annotation_id = a.id) ORDER BY a.created_at, a.id",
        (),
        5,
    )
    .await?;
    for row in legacy_annotations {
        let annotation_no = next_counter_tx(conn, "annotation_no").await?;
        let anchor_id = anchor_id_for(conn, &row[1], &row[3], &row[2]).await?;
        conn.execute(
            "INSERT INTO doc_annotation_meta (annotation_id, annotation_no, anchor_id, author, author_kind, rationale, visibility, quote, line_start, line_end, session, created_at) VALUES (?1, ?2, ?3, 'human', 'unknown', '', 'local_only', '', 0, 0, '', ?4)",
            params![row[0].as_str(), annotation_no, anchor_id.as_str(), row[4].as_str()],
        )
        .await?;
        report.annotations_imported += 1;
    }

    // Governance events, appended in their original order with a legacy identifier. Titles and
    // note texts are replaced by their digest and length, except for the state strings.
    let governance = text_rows(
        conn,
        "SELECT id, file_id, kind, subject_id, payload, actor, timestamp FROM governance_events ORDER BY seq",
        (),
        7,
    )
    .await?;
    for row in governance {
        let legacy_id = format!("{LEGACY_ID_PREFIX}{}", row[0]);
        let present = query_i64(
            conn,
            "SELECT COUNT(*) FROM doc_task_events WHERE id = ?1",
            params![legacy_id.as_str()],
        )
        .await?;
        if present > 0 {
            continue;
        }
        let kind = row[2].as_str();
        let (payload_json, state) = legacy_payload(kind, &row[4]);
        let subject_kind = kind.split('.').next().unwrap_or(kind);
        let task_id = if kind.starts_with("task.") {
            row[3].clone()
        } else {
            String::new()
        };
        append_draft(
            conn,
            Draft {
                id: legacy_id,
                task_id,
                file_id: row[1].clone(),
                subject_kind: subject_kind.to_string(),
                subject_id: row[3].clone(),
                kind: format!("legacy.{kind}"),
                from_status: String::new(),
                to_status: state.unwrap_or_default(),
                actor: row[5].clone(),
                actor_kind: UNKNOWN_KIND.to_string(),
                session: String::new(),
                claimed_session: String::new(),
                payload_json,
                timestamp: row[6].clone(),
            },
        )
        .await?;
        report.events_imported += 1;
    }

    // Status import: one dt.task.imported event per legacy task, and another one when the stored
    // status no longer matches the last status recorded in the chain.
    let legacy_status = text_rows(
        conn,
        "SELECT t.id, t.file_id, t.status FROM doc_tasks t JOIN doc_task_meta m ON m.task_id = t.id WHERE m.source_kind = 'legacy' ORDER BY m.task_no",
        (),
        3,
    )
    .await?;
    for row in legacy_status {
        let imported = query_i64(
            conn,
            "SELECT COUNT(*) FROM doc_task_events WHERE task_id = ?1 AND kind = 'dt.task.imported'",
            params![row[0].as_str()],
        )
        .await?;
        let replayed = query_text(
            conn,
            "SELECT to_status FROM doc_task_events WHERE task_id = ?1 AND to_status <> '' ORDER BY seq DESC LIMIT 1",
            params![row[0].as_str()],
        )
        .await?;
        if imported == 0 || replayed.as_deref() != Some(row[2].as_str()) {
            append_draft(conn, imported_draft(&row[0], &row[1], &row[2], now)).await?;
            report.status_events_added += 1;
        }
    }

    next_counter_tx(conn, "backfill_v4").await?;
    Ok(report)
}

fn imported_draft(task_id: &str, file_id: &str, status: &str, now: &str) -> Draft {
    Draft {
        id: new_id(),
        task_id: task_id.to_string(),
        file_id: file_id.to_string(),
        subject_kind: "task".to_string(),
        subject_id: task_id.to_string(),
        kind: IMPORTED_KIND.to_string(),
        from_status: String::new(),
        to_status: status.to_string(),
        actor: "ladon".to_string(),
        actor_kind: UNKNOWN_KIND.to_string(),
        session: String::new(),
        claimed_session: String::new(),
        payload_json: json!({"source": "legacy", "status": status}).to_string(),
        timestamp: now.to_string(),
    }
}

async fn read_events(conn: &Connection) -> Result<Vec<EventRecord>> {
    let mut rows = conn
        .query(
            "SELECT seq, id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp, prev_hash, hash FROM doc_task_events ORDER BY seq",
            (),
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(EventRecord {
            seq: row.get::<i64>(0)?,
            id: row.get::<String>(1)?,
            task_id: row.get::<String>(2)?,
            file_id: row.get::<String>(3)?,
            subject_kind: row.get::<String>(4)?,
            subject_id: row.get::<String>(5)?,
            kind: row.get::<String>(6)?,
            from_status: row.get::<String>(7)?,
            to_status: row.get::<String>(8)?,
            actor: row.get::<String>(9)?,
            actor_kind: row.get::<String>(10)?,
            session: row.get::<String>(11)?,
            claimed_session: row.get::<String>(12)?,
            payload_json: row.get::<String>(13)?,
            timestamp: row.get::<String>(14)?,
            prev_hash: row.get::<String>(15)?,
            hash: row.get::<String>(16)?,
        });
    }
    Ok(out)
}

async fn status_drift_on(conn: &Connection) -> Result<Vec<StatusDrift>> {
    let tasks = text_rows(conn, "SELECT id, status FROM doc_tasks ORDER BY id", (), 2).await?;
    let mut last_status: HashMap<String, String> = HashMap::new();
    for row in text_rows(
        conn,
        "SELECT task_id, to_status FROM doc_task_events WHERE to_status <> '' ORDER BY seq",
        (),
        2,
    )
    .await?
    {
        last_status.insert(row[0].clone(), row[1].clone());
    }
    Ok(tasks
        .into_iter()
        .filter_map(|row| {
            let replayed = last_status.get(&row[0]).cloned();
            (replayed.as_deref() != Some(row[1].as_str())).then(|| StatusDrift {
                task_id: row[0].clone(),
                status: row[1].clone(),
                replayed,
            })
        })
        .collect())
}

/// Reads every row of a query whose columns are all text. Each row holds `width` values.
async fn text_rows<P: IntoParams>(
    conn: &Connection,
    sql: &str,
    p: P,
    width: usize,
) -> Result<Vec<Vec<String>>> {
    let mut rows = conn.query(sql, p).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        let mut values = Vec::with_capacity(width);
        for index in 0..width {
            values.push(row.get::<String>(index)?);
        }
        out.push(values);
    }
    Ok(out)
}

async fn query_text<P: IntoParams>(conn: &Connection, sql: &str, p: P) -> Result<Option<String>> {
    let mut rows = conn.query(sql, p).await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<String>(0)?)),
        None => Ok(None),
    }
}

async fn query_opt_i64<P: IntoParams>(conn: &Connection, sql: &str, p: P) -> Result<Option<i64>> {
    let mut rows = conn.query(sql, p).await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<i64>(0)?)),
        None => Ok(None),
    }
}

async fn query_i64<P: IntoParams>(conn: &Connection, sql: &str, p: P) -> Result<i64> {
    Ok(query_opt_i64(conn, sql, p).await?.unwrap_or(0))
}

const BASELINES_SQL: &str = "SELECT id, file_id, version_id, version_no, content_hash, approver, note, created_at, revoked_at, revoked_by FROM doc_baselines ORDER BY created_at, id";
const BASELINES_OF_FILE_SQL: &str = "SELECT id, file_id, version_id, version_no, content_hash, approver, note, created_at, revoked_at, revoked_by FROM doc_baselines WHERE file_id = ?1 ORDER BY created_at, id";
const TASKS_SQL: &str = "SELECT t.id, m.task_no, t.file_id, m.anchor_id, t.anchor_section, t.base_version_id, t.title, t.description, t.status, m.owner, m.priority, m.proposal_state, m.visibility, m.source_kind, m.created_by, m.created_by_kind, m.session, t.created_at, m.updated_at, m.closed_at, a.state, a.clause_id, a.baseline_id FROM doc_tasks t JOIN doc_task_meta m ON m.task_id = t.id LEFT JOIN doc_anchors a ON a.id = m.anchor_id ORDER BY m.task_no";
const ANNOTATIONS_SQL: &str = "SELECT a.id, m.annotation_no, a.file_id, m.anchor_id, a.section_key, a.base_version_id, a.text, m.author, m.author_kind, m.visibility, m.rationale, m.quote, m.created_at FROM annotations a JOIN doc_annotation_meta m ON m.annotation_id = a.id ORDER BY m.annotation_no";

/// Checks the table and column names of one row write, and that its shape is complete.
fn validate_write(write: &RowWrite) -> Result<()> {
    let none: &[(&'static str, Value)] = &[];
    let (table, values, filters, key, values_required, filters_required) = match write {
        RowWrite::Insert { table, values } => (*table, values.as_slice(), none, None, true, false),
        RowWrite::Update {
            table, key, values, ..
        } => (*table, values.as_slice(), none, Some(*key), true, false),
        RowWrite::UpdateWhere {
            table,
            filters,
            values,
        } => (
            *table,
            values.as_slice(),
            filters.as_slice(),
            None,
            true,
            true,
        ),
        RowWrite::Delete { table, filters } => {
            (*table, none, filters.as_slice(), None, false, true)
        }
    };
    let names_ok = values.iter().all(|(column, _)| is_identifier(column))
        && filters.iter().all(|(column, _)| is_identifier(column))
        && key.map_or(true, |name| is_identifier(name));
    let shape_ok =
        (!values_required || !values.is_empty()) && (!filters_required || !filters.is_empty());
    // A task edit changes the title and description, which live in the legacy `doc_tasks` table.
    let legacy_task_edit = matches!(
        write,
        RowWrite::Update { table: "doc_tasks", key: "id", values, .. }
            if values.iter().all(|(column, _)| matches!(*column, "title" | "description"))
    );
    if !(is_doc_table(table) || legacy_task_edit) || !shape_ok || !names_ok {
        return Err(doc_error(
            ErrorCode::InvalidArg,
            "非法的表名或列名",
            "只能写入文档任务表的合法列",
        ));
    }
    Ok(())
}

/// The `WHERE` clause of a filter list, numbering its placeholders from `first`, with the values
/// in the same order.
fn where_clause(filters: &[(&'static str, Value)], first: usize) -> (String, Vec<Value>) {
    let clause: Vec<String> = filters
        .iter()
        .enumerate()
        .map(|(index, (column, _))| format!("{column} = ?{}", first + index))
        .collect();
    let bound: Vec<Value> = filters.iter().map(|(_, value)| value.clone()).collect();
    (clause.join(" AND "), bound)
}

/// Parses a stored enumerated value. An unknown value is an integrity error.
fn parse_stored<T>(parse: fn(&str) -> Option<T>, text: &str) -> Result<T> {
    parse(text).ok_or_else(|| {
        doc_error(
            ErrorCode::Integrity,
            "存储的取值不合法",
            "运行 doc verify 检查数据库",
        )
    })
}

async fn write_with_event_tx(
    conn: &Connection,
    writes: &[RowWrite],
    ev: &NewEvent,
    id: &str,
    now: &str,
) -> Result<EventRecord> {
    for write in writes {
        apply_write(conn, write).await?;
    }
    append_draft(conn, draft_from_event(ev, id, now)).await
}

async fn apply_write(conn: &Connection, write: &RowWrite) -> Result<()> {
    match write {
        RowWrite::Insert { table, values } => {
            let columns: Vec<&str> = values.iter().map(|(column, _)| *column).collect();
            let placeholders: Vec<String> = (1..=values.len())
                .map(|index| format!("?{index}"))
                .collect();
            let sql = format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                columns.join(", "),
                placeholders.join(", ")
            );
            let bound: Vec<Value> = values.iter().map(|(_, value)| value.clone()).collect();
            conn.execute(sql.as_str(), bound).await?;
            Ok(())
        }
        RowWrite::Update {
            table,
            key,
            key_value,
            values,
        } => {
            let assignments: Vec<String> = values
                .iter()
                .enumerate()
                .map(|(index, (column, _))| format!("{column} = ?{}", index + 1))
                .collect();
            let sql = format!(
                "UPDATE {table} SET {} WHERE {key} = ?{}",
                assignments.join(", "),
                values.len() + 1
            );
            let mut bound: Vec<Value> = values.iter().map(|(_, value)| value.clone()).collect();
            bound.push(key_value.clone());
            let changed = conn.execute(sql.as_str(), bound).await?;
            if changed != 1 {
                return Err(doc_error(
                    ErrorCode::NotFound,
                    "未找到要更新的记录",
                    "刷新后重试",
                ));
            }
            Ok(())
        }
        RowWrite::UpdateWhere {
            table,
            filters,
            values,
        } => {
            let assignments: Vec<String> = values
                .iter()
                .enumerate()
                .map(|(index, (column, _))| format!("{column} = ?{}", index + 1))
                .collect();
            let (clause, filter_values) = where_clause(filters, values.len() + 1);
            let sql = format!(
                "UPDATE {table} SET {} WHERE {clause}",
                assignments.join(", ")
            );
            let mut bound: Vec<Value> = values.iter().map(|(_, value)| value.clone()).collect();
            bound.extend(filter_values);
            conn.execute(sql.as_str(), bound).await?;
            Ok(())
        }
        RowWrite::Delete { table, filters } => {
            let (clause, bound) = where_clause(filters, 1);
            let sql = format!("DELETE FROM {table} WHERE {clause}");
            let changed = conn.execute(sql.as_str(), bound).await?;
            if changed != 1 {
                return Err(doc_error(
                    ErrorCode::NotFound,
                    "未找到要删除的记录",
                    "刷新后重试",
                ));
            }
            Ok(())
        }
    }
}

async fn ensure_anchor_clause_tx(
    conn: &Connection,
    file_id: &str,
    base_version_id: &str,
    fp: &AnchorFingerprint,
    now: &str,
) -> Result<String> {
    let (id, created) = ensure_anchor_tx(
        conn,
        file_id,
        base_version_id,
        &fp.section_key,
        AnchorState::Valid,
        now,
    )
    .await?;
    if created {
        conn.execute(
            "UPDATE doc_anchors SET baseline_id = ?1, clause_id = ?2, title = ?3, level = ?4, line_start = ?5, line_end = ?6, full_hash = ?7, body_hash = ?8, current_hash = ?10 WHERE id = ?9",
            params![
                fp.baseline_id.as_str(),
                fp.clause_number.as_str(),
                fp.title.as_str(),
                fp.level,
                fp.line_start,
                fp.line_end,
                fp.full_hash.as_str(),
                fp.body_hash.as_str(),
                id.as_str(),
                fp.full_hash.as_str()
            ],
        )
        .await?;
    }
    Ok(id)
}

fn doc_error(code: ErrorCode, message: &str, hint: &str) -> anyhow::Error {
    DocError::new(code, message, hint).into()
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Anchor key `ROOT` is the whole document. `H<level>:<title>#<n>` gives the level and the title,
/// where the title runs from the first `:` after the level to the last `#`. Other keys give
/// level 0 and an empty title.
pub(crate) fn parse_anchor_key(key: &str) -> (i64, String) {
    if key == ROOT_KEY {
        return (0, ROOT_TITLE.to_string());
    }
    let unknown = (0, String::new());
    let Some(rest) = key.strip_prefix('H') else {
        return unknown;
    };
    let digits_len = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len == 0 {
        return unknown;
    }
    let (digits, after) = rest.split_at(digits_len);
    let Some(body) = after.strip_prefix(':') else {
        return unknown;
    };
    let Some(hash_at) = body.rfind('#') else {
        return unknown;
    };
    let suffix = &body[hash_at + 1..];
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return unknown;
    }
    match digits.parse::<i64>() {
        Ok(level) => (level, body[..hash_at].to_string()),
        Err(_) => unknown,
    }
}

/// Payload of an imported legacy event and the status it carries. Only `task.state_changed` with
/// one of the seven state strings keeps its text. Every other payload is replaced by its byte
/// length and SHA-256 digest.
pub(crate) fn legacy_payload(kind: &str, payload: &str) -> (String, Option<String>) {
    if kind == "task.state_changed" {
        if let Some(state) = TaskState::parse(payload) {
            let text = json!({"legacy": true, "payload": state.as_str()}).to_string();
            return (text, Some(state.as_str().to_string()));
        }
    }
    let text = json!({
        "bytes": payload.len(),
        "legacy": true,
        "sha256": sha256_hex(payload.as_bytes()),
    })
    .to_string();
    (text, None)
}

/// True for a write rejected because another connection appended the same predecessor.
pub(crate) fn is_chain_conflict(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<turso::Error>(),
        Some(turso::Error::Constraint(text))
            if text.contains("UNIQUE constraint failed") && text.contains("doc_task_events.prev_hash")
    )
}

/// True for a lock error that a later attempt may get past.
pub(crate) fn is_lock_error(err: &turso::Error) -> bool {
    matches!(err, turso::Error::Busy(_) | turso::Error::BusySnapshot(_))
        || err.to_string().contains("locked by another process")
}

pub(crate) fn is_doc_table(name: &str) -> bool {
    DOC_TABLES.contains(&name)
}

/// A non-empty lowercase identifier: a letter, then letters, digits or underscores.
pub(crate) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc864_lock_error_classification() {
        assert!(is_lock_error(&turso::Error::Busy(
            "File is locked by another process".into()
        )));
        assert!(is_lock_error(&turso::Error::BusySnapshot(
            "snapshot".into()
        )));
        assert!(!is_lock_error(&turso::Error::Constraint(
            "UNIQUE constraint failed: doc_task_meta.task_no".into()
        )));
    }

    #[test]
    fn tc865_chain_conflict_classification() {
        let conflict = anyhow::Error::from(turso::Error::Constraint(
            "UNIQUE constraint failed: doc_task_events.prev_hash".into(),
        ));
        assert!(is_chain_conflict(&conflict));
        let other_index = anyhow::Error::from(turso::Error::Constraint(
            "UNIQUE constraint failed: doc_task_events.id".into(),
        ));
        assert!(!is_chain_conflict(&other_index));
        let busy = anyhow::Error::from(turso::Error::Busy(
            "UNIQUE constraint failed: doc_task_events.prev_hash".into(),
        ));
        assert!(!is_chain_conflict(&busy));
        assert!(!is_chain_conflict(&anyhow::anyhow!(
            "doc_task_events.prev_hash"
        )));
    }

    #[test]
    fn tc866_identifier_and_table_checks() {
        assert!(is_identifier("doc_tasks"));
        assert!(!is_identifier("a;b"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("Name"));
        assert!(!is_identifier("9a"));
        assert!(!is_doc_table("doc_tasks"));
        assert!(is_doc_table("doc_counters"));
    }

    #[test]
    fn ut_validate_write_allows_only_title_and_description_on_doc_tasks() {
        let edit = RowWrite::Update {
            table: "doc_tasks",
            key: "id",
            key_value: Value::from("t"),
            values: vec![("title", Value::from("x")), ("description", Value::from("y"))],
        };
        assert!(validate_write(&edit).is_ok());
        let status = RowWrite::Update {
            table: "doc_tasks",
            key: "id",
            key_value: Value::from("t"),
            values: vec![("status", Value::from("done"))],
        };
        assert!(validate_write(&status).is_err());
        let delete = RowWrite::Delete {
            table: "doc_tasks",
            filters: vec![("id", Value::from("t"))],
        };
        assert!(validate_write(&delete).is_err());
    }

    #[test]
    fn tc867_anchor_key_parsing() {
        assert_eq!(parse_anchor_key("H1:A#1"), (1, "A".to_string()));
        assert_eq!(parse_anchor_key("H2:Scope#1"), (2, "Scope".to_string()));
        assert_eq!(parse_anchor_key("H1:C#1#2"), (1, "C#1".to_string()));
        assert_eq!(parse_anchor_key("ROOT"), (0, ROOT_TITLE.to_string()));
        assert_eq!(parse_anchor_key("bogus"), (0, String::new()));
        assert_eq!(parse_anchor_key("H:X#1"), (0, String::new()));
    }

    #[test]
    fn tc868_legacy_payload_rule() {
        assert_eq!(
            legacy_payload("task.state_changed", "working"),
            (
                r#"{"legacy":true,"payload":"working"}"#.to_string(),
                Some("working".to_string())
            )
        );
        let (text, state) = legacy_payload("task.state_changed", "not-a-state");
        assert_eq!(state, None);
        let body: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(body["sha256"], json!(sha256_hex(b"not-a-state")));
        assert_eq!(body["bytes"], json!(11));
        let (empty, empty_state) = legacy_payload("task.state_changed", "");
        assert_eq!(empty_state, None);
        assert!(empty.contains("sha256"));
        let (title, title_state) = legacy_payload("task.created", "Secret Title Alpha");
        assert_eq!(title_state, None);
        let body: Json = serde_json::from_str(&title).unwrap();
        assert_eq!(
            body["sha256"],
            json!(sha256_hex("Secret Title Alpha".as_bytes()))
        );
        assert_eq!(body["bytes"], json!("Secret Title Alpha".len()));
        let (note, _) = legacy_payload("annotation.created", "Secret Note Beta");
        assert!(!note.contains("Secret Note Beta"));
    }

    #[tokio::test]
    async fn tc864_retry_policy_bounds() {
        let attempts = std::cell::Cell::new(0u32);

        let exhausted: Result<()> = with_retry(|| {
            attempts.set(attempts.get() + 1);
            async { Err(anyhow::Error::from(turso::Error::Busy("locked".into()))) }
        })
        .await;
        let busy = exhausted.unwrap_err();
        assert!(busy.downcast_ref::<StoreBusy>().is_some());
        assert_eq!(attempts.get(), LOCK_ATTEMPTS);

        attempts.set(0);
        let recovered: Result<u32> = with_retry(|| {
            let n = attempts.get() + 1;
            attempts.set(n);
            async move {
                if n < 3 {
                    Err(anyhow::Error::from(turso::Error::Busy("locked".into())))
                } else {
                    Ok(n)
                }
            }
        })
        .await;
        assert_eq!(recovered.unwrap(), 3);

        attempts.set(0);
        let conflict: Result<()> = with_retry(|| {
            attempts.set(attempts.get() + 1);
            async {
                Err(anyhow::Error::from(turso::Error::Constraint(
                    "UNIQUE constraint failed: doc_task_events.prev_hash".into(),
                )))
            }
        })
        .await;
        assert!(is_chain_conflict(&conflict.unwrap_err()));
        assert_eq!(attempts.get(), CHAIN_RETRIES + 1);

        attempts.set(0);
        let other: Result<()> = with_retry(|| {
            attempts.set(attempts.get() + 1);
            async { Err(anyhow::anyhow!("plain failure")) }
        })
        .await;
        assert!(other.is_err());
        assert_eq!(attempts.get(), 1);
    }
}
