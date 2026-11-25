//! Anchor reconciliation, the owner's decisions on moved anchors, and file-move lineage of the
//! document-task module (design 57, sections 5.4 to 5.7). Reconciliation compares each anchor
//! with the outline of the latest version of its file and records every change of state, key or
//! version as one governance event. Acknowledgement, relocation and rebase are owner decisions
//! that carry the subject the owner was shown. Lineage proposes the new file of a deleted file
//! and confirms it only for the owner. Every write is one `DocStore::commit_step`.

use crate::crypto::digest;
use crate::doctask_model::{
    authorize, parse_section_selector, parse_task_handle, ActorKind, AnchorState, DocError,
    ErrorCode, Handle, LineageState, Operation, MAX_TARGET_FILE_BYTES,
};
use crate::doctask_outline::{
    self, classify_anchor, AnchorRef, Clause, ClauseKind, LatestVersion, Method, Outline,
};
use crate::doctask_service::Caller;
use crate::doctask_store::{
    AnchorFingerprint, AnchorRecord, DocStore, LineageRow, NewEvent, RowWrite, StepEvent, TaskRow,
};
use crate::engine::Engine;
use crate::model::{FileRecord, VersionRecord};
use anyhow::Result;
use serde_json::{json, Value as Json};
use std::collections::HashSet;
use std::path::Path;
use turso::Value;
use uuid::Uuid;

/// Longest chain of confirmed moves followed from one file id.
pub const MAX_LINEAGE_HOPS: usize = 8;
/// Lowest section-overlap score, in per mille, that proposes a lineage candidate.
const LINEAGE_MIN_PERMILLE: i64 = 800;
/// Score of a candidate with the same content hash.
const SAME_HASH_PERMILLE: i64 = 1000;
/// Actor name of the system: reconciliation and lineage proposals.
const SYSTEM_NAME: &str = "ladon";

/// One state, key or version change written by reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorChange {
    pub anchor_id: String,
    pub from_state: String,
    pub to_state: String,
    pub from_key: String,
    pub to_key: String,
    pub method: String,
    pub event_seq: i64,
}

/// The changes of one reconciliation run, in anchor order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub examined: usize,
    pub changes: Vec<AnchorChange>,
}

/// The result of an owner decision on an anchor. `event_seq` is `None` when nothing changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorStep {
    pub anchor_id: String,
    pub state: String,
    pub event_seq: Option<i64>,
}

/// One row of the migration report: the stored anchor and what reconciliation would make of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveLine {
    pub anchor_id: String,
    pub file_id: String,
    pub section_key: String,
    pub state: String,
    pub key: String,
    pub would_change: bool,
    pub verdict_state: String,
    pub verdict_key: String,
    pub method: String,
}

/// The reconciliation of one anchor, computed without writes.
struct Assessment {
    target_id: String,
    state: String,
    key: String,
    method: String,
    version_id: String,
    current_hash: String,
    fill: Option<AnchorFingerprint>,
    changed: bool,
}

/// The document-task reconciler for one caller.
pub struct Reconciler<'a> {
    engine: &'a Engine,
    ds: DocStore,
    caller: Caller,
}

impl<'a> Reconciler<'a> {
    /// Applies migration 4 when needed, imports legacy rows, and returns the reconciler. May write.
    pub async fn open(engine: &'a Engine, caller: Caller) -> Result<Reconciler<'a>> {
        engine.store.ensure_schema(false).await?;
        let ds = DocStore::new(engine.store.clone());
        ds.backfill().await?;
        Ok(Reconciler { engine, ds, caller })
    }

    /// Returns the reconciler when migration 4 is recorded. Writes nothing.
    pub async fn attach(engine: &'a Engine, caller: Caller) -> Result<Reconciler<'a>> {
        let ds = DocStore::new(engine.store.clone());
        if !ds.schema_current().await? {
            return Err(fail(
                ErrorCode::SchemaOutdated,
                "数据库结构版本低于 4",
                "由所有者运行 doc migrate 后重试",
            ));
        }
        Ok(Reconciler { engine, ds, caller })
    }

    /// Reconciles every anchor against the latest version of its file.
    pub async fn reconcile_all(&self) -> Result<ReconcileReport> {
        authorize(self.caller.principal, Operation::Reconcile)?;
        let anchors = self.ds.anchors().await?;
        reconcile_anchors(self.engine, &self.ds, &anchors).await
    }

    /// Reconciles the anchors whose file, or whose followed file, is the named file.
    pub async fn reconcile_file(&self, file_ref: &str) -> Result<ReconcileReport> {
        authorize(self.caller.principal, Operation::Reconcile)?;
        let file = self.resolve_file(file_ref).await?;
        let lineage = self.ds.lineage().await?;
        let anchors: Vec<AnchorRecord> = self
            .ds
            .anchors()
            .await?
            .into_iter()
            .filter(|anchor| successor_chain(&lineage, &anchor.file_id) == file.id)
            .collect();
        reconcile_anchors(self.engine, &self.ds, &anchors).await
    }

    /// The migration report of design 5.6: every anchor with its stored and computed state.
    /// Writes nothing.
    pub async fn report(&self, file_ref: Option<&str>) -> Result<Vec<MoveLine>> {
        authorize(self.caller.principal, Operation::Reconcile)?;
        let file_id = match file_ref {
            Some(reference) => Some(self.resolve_file(reference).await?.id),
            None => None,
        };
        let lineage = self.ds.lineage().await?;
        let mut lines = Vec::new();
        for anchor in self.ds.anchors().await? {
            if let Some(id) = &file_id {
                if successor_chain(&lineage, &anchor.file_id) != *id {
                    continue;
                }
            }
            let assessment = assess(self.engine, &lineage, &anchor).await?;
            lines.push(MoveLine {
                anchor_id: anchor.id.clone(),
                file_id: anchor.file_id.clone(),
                section_key: anchor.section_key.clone(),
                state: anchor.state.clone(),
                key: anchor.current_key.clone(),
                would_change: assessment.changed,
                verdict_state: assessment.state,
                verdict_key: assessment.key,
                method: assessment.method,
            });
        }
        Ok(lines)
    }

    /// Owner acknowledgement of a `needs_review`, `relocated` or `unknown` anchor.
    pub async fn ack(
        &self,
        handle: &str,
        expected_version_id: &str,
        expected_hash: &str,
    ) -> Result<AnchorStep> {
        authorize(self.caller.principal, Operation::AnchorAck)?;
        let task = self.task_by_handle(handle).await?;
        reconcile_decision(self.engine, &self.ds, &task.anchor_id).await?;
        let anchor = self.anchor_of(&task).await?;
        if !matches!(
            AnchorState::parse(&anchor.state),
            Some(AnchorState::NeedsReview | AnchorState::Relocated | AnchorState::Unknown)
        ) {
            return Err(guard(
                "锚点状态不接受确认",
                "只有需复核、已重定位或未知的锚点可以确认",
            ));
        }
        if anchor.current_version_id != expected_version_id || anchor.current_hash != expected_hash
        {
            return Err(stale());
        }
        let target = self.target_of(&anchor).await?;
        check_live(self.engine, &target)?;
        let event = self.human_event(
            &task.id,
            &anchor.file_id,
            &anchor.id,
            "dt.anchor.acknowledged",
            json!({
                "anchor_id": anchor.id,
                "version_id": anchor.current_version_id,
                "hash": anchor.current_hash,
            }),
        );
        let writes = [RowWrite::Update {
            table: "doc_anchors",
            key: "id",
            key_value: text(&anchor.id),
            values: vec![
                ("reviewed_version_id", text(&anchor.current_version_id)),
                ("updated_at", text(&now())),
            ],
        }];
        let records = self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(AnchorStep {
            anchor_id: anchor.id,
            state: anchor.state,
            event_seq: records.first().map(|record| record.seq),
        })
    }

    /// Owner relocation of an `orphaned` or `ambiguous` anchor to a clause of the latest version.
    pub async fn relocate(
        &self,
        handle: &str,
        selector: &str,
        expected_version_id: &str,
        expected_hash: &str,
    ) -> Result<AnchorStep> {
        authorize(self.caller.principal, Operation::AnchorRelocate)?;
        let task = self.task_by_handle(handle).await?;
        reconcile_decision(self.engine, &self.ds, &task.anchor_id).await?;
        let anchor = self.anchor_of(&task).await?;
        if !matches!(
            AnchorState::parse(&anchor.state),
            Some(AnchorState::Orphaned | AnchorState::Ambiguous)
        ) {
            return Err(guard(
                "锚点状态不接受重定位",
                "只有孤立或有歧义的锚点可以重定位",
            ));
        }
        let parsed = parse_section_selector(Some(selector))?;
        let target = self.target_of(&anchor).await?;
        let latest = self.latest_of(&target).await?;
        let bytes = read_bytes(self.engine, &latest).ok_or_else(anchor_broken)?;
        let outline = doctask_outline::outline(&bytes, &extension_of(&target.relative_path));
        let clause = doctask_outline::resolve(&outline, &parsed)?;
        if clause.kind == ClauseKind::Row {
            return Err(guard("虚拟条款不能作为重定位目标", "选择一个标题条款"));
        }
        if latest.id != expected_version_id || clause.full_hash != expected_hash {
            return Err(stale());
        }
        check_live(self.engine, &target)?;
        let fingerprint = fingerprint_of(&anchor.baseline_id, clause);
        let event = self.human_event(
            &task.id,
            &anchor.file_id,
            &anchor.id,
            "dt.anchor.relocated",
            json!({
                "anchor_id": anchor.id,
                "from_key": anchor.current_key,
                "to_key": clause.key,
                "version_id": latest.id,
                "hash": clause.full_hash,
                "method": "manual",
            }),
        );
        let writes = [RowWrite::Update {
            table: "doc_anchors",
            key: "id",
            key_value: text(&anchor.id),
            values: fingerprint_values(&fingerprint, "relocated", &latest.id, &latest.id),
        }];
        let records = self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(AnchorStep {
            anchor_id: anchor.id,
            state: AnchorState::Relocated.as_str().to_string(),
            event_seq: records.first().map(|record| record.seq),
        })
    }

    /// Owner re-pin of a task to the active baseline version. The task's key must exist there.
    pub async fn rebase(
        &self,
        handle: &str,
        expected_version_id: &str,
        expected_hash: &str,
    ) -> Result<AnchorStep> {
        authorize(self.caller.principal, Operation::AnchorRebase)?;
        let task = self.task_by_handle(handle).await?;
        let anchor = self.anchor_of(&task).await?;
        let baseline = self
            .ds
            .baselines(Some(&anchor.file_id))
            .await?
            .into_iter()
            .filter(|row| row.revoked_at.is_none())
            .last()
            .ok_or_else(|| {
                fail(
                    ErrorCode::NoBaseline,
                    "文件没有未撤销的基线",
                    "先执行 doc baseline add 登记基线",
                )
            })?;
        if baseline.version_id != expected_version_id || baseline.content_hash != expected_hash {
            return Err(stale());
        }
        let target = self.target_of(&anchor).await?;
        check_live(self.engine, &target)?;
        let base_version = self
            .engine
            .store
            .version(&baseline.version_id)
            .await?
            .ok_or_else(not_found)?;
        let bytes = read_bytes(self.engine, &base_version).ok_or_else(anchor_broken)?;
        let file = self
            .engine
            .store
            .file_by_id(&anchor.file_id)
            .await?
            .ok_or_else(not_found)?;
        let outline = doctask_outline::outline(&bytes, &extension_of(&file.relative_path));
        let clause = outline.find_key(&anchor.current_key).ok_or_else(|| {
            fail(
                ErrorCode::SectionNotFound,
                "基线版本中没有该条款",
                "重定位任务或重新登记基线",
            )
        })?;
        let fingerprint = fingerprint_of(&baseline.id, clause);
        let new_anchor = self
            .ds
            .ensure_anchor_clause(&anchor.file_id, &baseline.version_id, &fingerprint)
            .await?;
        if new_anchor == anchor.id {
            return Ok(AnchorStep {
                anchor_id: anchor.id,
                state: anchor.state,
                event_seq: None,
            });
        }
        let state = self
            .ds
            .anchors()
            .await?
            .into_iter()
            .find(|row| row.id == new_anchor)
            .map(|row| row.state)
            .unwrap_or_default();
        let event = self.human_event(
            &task.id,
            &anchor.file_id,
            &new_anchor,
            "dt.anchor.rebased",
            json!({
                "from_anchor": anchor.id,
                "to_anchor": new_anchor,
                "from_version_id": anchor.base_version_id,
                "to_version_id": baseline.version_id,
                "section_key": clause.key,
            }),
        );
        let writes = [RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: text(&task.id),
            values: vec![("anchor_id", text(&new_anchor))],
        }];
        let records = self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(AnchorStep {
            anchor_id: new_anchor,
            state,
            event_seq: records.first().map(|record| record.seq),
        })
    }

    /// Proposes lineage candidates for every deleted file that has an anchor. Returns the rows
    /// proposed by this call.
    pub async fn scan_lineage(&self) -> Result<Vec<LineageRow>> {
        authorize(self.caller.principal, Operation::Reconcile)?;
        let files = self.engine.store.list_files().await?;
        let anchored: HashSet<String> = self
            .ds
            .anchors()
            .await?
            .into_iter()
            .map(|anchor| anchor.file_id)
            .collect();
        let known: HashSet<(String, String)> = self
            .ds
            .lineage()
            .await?
            .into_iter()
            .map(|row| (row.old_file_id, row.new_file_id))
            .collect();
        let mut proposed = Vec::new();
        for old in files
            .iter()
            .filter(|file| file.deleted && anchored.contains(&file.id))
        {
            let Some(old_last) = self.engine.store.latest_version(&old.id).await? else {
                continue;
            };
            let old_outline = read_bytes(self.engine, &old_last)
                .map(|bytes| doctask_outline::outline(&bytes, &extension_of(&old.relative_path)));
            for new in files
                .iter()
                .filter(|file| !file.deleted && file.id != old.id)
            {
                if known.contains(&(old.id.clone(), new.id.clone())) {
                    continue;
                }
                let (evidence, score) = if new.current_hash == old_last.content_hash {
                    ("same_hash", SAME_HASH_PERMILLE)
                } else {
                    let Some(old_outline) = &old_outline else {
                        continue;
                    };
                    let Some(new_last) = self.engine.store.latest_version(&new.id).await? else {
                        continue;
                    };
                    let Some(new_bytes) = read_bytes(self.engine, &new_last) else {
                        continue;
                    };
                    let new_outline =
                        doctask_outline::outline(&new_bytes, &extension_of(&new.relative_path));
                    let score = overlap_permille(
                        &section_hashes(old_outline),
                        &section_hashes(&new_outline),
                    );
                    if score < LINEAGE_MIN_PERMILLE {
                        continue;
                    }
                    ("section_overlap", score)
                };
                proposed.push(self.propose(old, new, evidence, score).await?);
            }
        }
        Ok(proposed)
    }

    /// Every lineage row, in creation order.
    pub async fn lineage(&self) -> Result<Vec<LineageRow>> {
        authorize(self.caller.principal, Operation::Reconcile)?;
        self.ds.lineage().await
    }

    /// Owner confirmation of a proposed move. The shown file ids and the new file's content hash
    /// must still hold.
    pub async fn confirm_lineage(
        &self,
        id: &str,
        expected_old_file_id: &str,
        expected_new_file_id: &str,
        expected_new_hash: &str,
    ) -> Result<LineageRow> {
        authorize(self.caller.principal, Operation::LineageConfirm)?;
        let rows = self.ds.lineage().await?;
        let row = rows
            .iter()
            .find(|row| row.id == id)
            .cloned()
            .ok_or_else(not_found)?;
        if row.state != LineageState::Proposed.as_str() {
            return Err(guard("谱系候选不是待确认状态", "只能确认待确认的候选"));
        }
        if row.old_file_id != expected_old_file_id || row.new_file_id != expected_new_file_id {
            return Err(stale());
        }
        let old = self
            .engine
            .store
            .file_by_id(&row.old_file_id)
            .await?
            .ok_or_else(not_found)?;
        let new = self
            .engine
            .store
            .file_by_id(&row.new_file_id)
            .await?
            .ok_or_else(not_found)?;
        if !old.deleted {
            return Err(guard(
                "旧文件仍然存在，不构成移动",
                "只能确认已删除旧文件的候选",
            ));
        }
        if new.deleted {
            return Err(guard("新文件已删除", "选择存在的新文件"));
        }
        if new.current_hash != expected_new_hash {
            return Err(stale());
        }
        if rows.iter().any(|other| {
            other.state == LineageState::Confirmed.as_str() && other.old_file_id == row.old_file_id
        }) {
            return Err(guard("旧文件已有确认的谱系", "旧文件只能对应一个新文件"));
        }
        let mut edges: Vec<(String, String)> = rows
            .iter()
            .filter(|other| other.state == LineageState::Confirmed.as_str())
            .map(|other| (other.old_file_id.clone(), other.new_file_id.clone()))
            .collect();
        edges.push((row.old_file_id.clone(), row.new_file_id.clone()));
        let from_new = walk(&edges, &row.new_file_id);
        if from_new.contains(&row.old_file_id) {
            return Err(guard("谱系会形成环", "拒绝该候选"));
        }
        if walk(&edges, &row.old_file_id).len() - 1 > MAX_LINEAGE_HOPS {
            return Err(guard("谱系深度超过上限", "拒绝该候选"));
        }
        let decided = now();
        let event = self.human_event(
            "",
            &row.old_file_id,
            &row.id,
            "dt.lineage.confirmed",
            json!({"old_file_id": row.old_file_id, "new_file_id": row.new_file_id, "new_hash": expected_new_hash, "score_permille": row.score_permille}),
        );
        let writes = [RowWrite::Update {
            table: "doc_file_lineage",
            key: "id",
            key_value: text(&row.id),
            values: vec![
                ("state", text(LineageState::Confirmed.as_str())),
                ("decided_by", text(&self.caller.name)),
                ("decided_at", text(&decided)),
            ],
        }];
        self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(LineageRow {
            state: LineageState::Confirmed.as_str().to_string(),
            decided_by: self.caller.name.clone(),
            decided_at: Some(decided),
            ..row
        })
    }

    /// Owner rejection of a proposed move. A rejected pair is not proposed again.
    pub async fn reject_lineage(&self, id: &str) -> Result<LineageRow> {
        authorize(self.caller.principal, Operation::LineageReject)?;
        let rows = self.ds.lineage().await?;
        let row = rows
            .iter()
            .find(|row| row.id == id)
            .cloned()
            .ok_or_else(not_found)?;
        if row.state != LineageState::Proposed.as_str() {
            return Err(guard("谱系候选不是待确认状态", "只能拒绝待确认的候选"));
        }
        let decided = now();
        let event = self.human_event(
            "",
            &row.old_file_id,
            &row.id,
            "dt.lineage.rejected",
            json!({"old_file_id": row.old_file_id, "new_file_id": row.new_file_id}),
        );
        let writes = [RowWrite::Update {
            table: "doc_file_lineage",
            key: "id",
            key_value: text(&row.id),
            values: vec![
                ("state", text(LineageState::Rejected.as_str())),
                ("decided_by", text(&self.caller.name)),
                ("decided_at", text(&decided)),
            ],
        }];
        self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(LineageRow {
            state: LineageState::Rejected.as_str().to_string(),
            decided_by: self.caller.name.clone(),
            decided_at: Some(decided),
            ..row
        })
    }

    async fn resolve_file(&self, reference: &str) -> Result<FileRecord> {
        let reference = reference.trim();
        if let Some(file) = self
            .engine
            .store
            .find_path(&reference.replace('\\', "/"))
            .await?
        {
            return Ok(file);
        }
        self.engine
            .store
            .file_by_id(reference)
            .await?
            .ok_or_else(not_found)
    }

    async fn task_by_handle(&self, handle: &str) -> Result<TaskRow> {
        let parsed = parse_task_handle(handle, self.caller.principal)?;
        let tasks = self.ds.tasks().await?;
        let found = match parsed {
            Handle::Number(number) => tasks.into_iter().find(|task| task.task_no == number),
            Handle::Uuid(uuid) => tasks.into_iter().find(|task| task.id == uuid),
            Handle::Prefix(prefix) => {
                let mut matches = tasks
                    .into_iter()
                    .filter(|task| task.id.starts_with(&prefix));
                let first = matches.next();
                if matches.next().is_some() {
                    return Err(fail(
                        ErrorCode::Ambiguous,
                        "任务前缀不唯一",
                        "使用更长的前缀或任务号",
                    ));
                }
                first
            }
        };
        found.ok_or_else(not_found)
    }

    async fn anchor_of(&self, task: &TaskRow) -> Result<AnchorRecord> {
        self.ds
            .anchors()
            .await?
            .into_iter()
            .find(|anchor| anchor.id == task.anchor_id)
            .ok_or_else(not_found)
    }

    /// The file that a decision on `anchor` acts on: the anchor's file or its confirmed successor.
    async fn target_of(&self, anchor: &AnchorRecord) -> Result<FileRecord> {
        let lineage = self.ds.lineage().await?;
        let target = successor_chain(&lineage, &anchor.file_id);
        self.engine
            .store
            .file_by_id(&target)
            .await?
            .ok_or_else(not_found)
    }

    async fn latest_of(&self, target: &FileRecord) -> Result<VersionRecord> {
        if target.deleted {
            return Err(file_missing());
        }
        self.engine
            .store
            .latest_version(&target.id)
            .await?
            .ok_or_else(anchor_broken)
    }

    fn human_event(
        &self,
        task_id: &str,
        file_id: &str,
        subject_id: &str,
        kind: &str,
        payload: Json,
    ) -> NewEvent {
        NewEvent {
            task_id: task_id.to_string(),
            file_id: file_id.to_string(),
            subject_kind: "anchor".to_string(),
            subject_id: subject_id.to_string(),
            kind: kind.to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: self.caller.name.clone(),
            actor_kind: ActorKind::from(self.caller.principal),
            session: self.caller.session.clone(),
            claimed_session: String::new(),
            payload_json: payload.to_string(),
        }
    }

    async fn propose(
        &self,
        old: &FileRecord,
        new: &FileRecord,
        evidence: &str,
        score: i64,
    ) -> Result<LineageRow> {
        let id = new_id();
        let created = now();
        let row = LineageRow {
            id: id.clone(),
            old_file_id: old.id.clone(),
            new_file_id: new.id.clone(),
            evidence: evidence.to_string(),
            score_permille: score,
            state: LineageState::Proposed.as_str().to_string(),
            decided_by: String::new(),
            decided_at: None,
            created_at: created.clone(),
        };
        let writes = [RowWrite::Insert {
            table: "doc_file_lineage",
            values: vec![
                ("id", text(&row.id)),
                ("old_file_id", text(&row.old_file_id)),
                ("new_file_id", text(&row.new_file_id)),
                ("evidence", text(&row.evidence)),
                ("score_permille", Value::Integer(score)),
                ("state", text(&row.state)),
                ("decided_by", text("")),
                ("decided_at", Value::Null),
                ("created_at", text(&created)),
            ],
        }];
        let event = NewEvent {
            task_id: String::new(),
            file_id: old.id.clone(),
            subject_kind: "lineage".to_string(),
            subject_id: id,
            kind: "dt.lineage.proposed".to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: SYSTEM_NAME.to_string(),
            actor_kind: ActorKind::System,
            session: String::new(),
            claimed_session: String::new(),
            payload_json: json!({
                "old_file_id": old.id,
                "new_file_id": new.id,
                "evidence": evidence,
                "score_permille": score,
            })
            .to_string(),
        };
        self.ds.commit_step(None, &writes, &[step(event)]).await?;
        Ok(row)
    }
}

/// Reconciles the given anchors in order, one commit per changed anchor.
async fn reconcile_anchors(
    engine: &Engine,
    ds: &DocStore,
    anchors: &[AnchorRecord],
) -> Result<ReconcileReport> {
    let lineage = ds.lineage().await?;
    let mut report = ReconcileReport {
        examined: anchors.len(),
        changes: Vec::new(),
    };
    for anchor in anchors {
        let assessment = assess(engine, &lineage, anchor).await?;
        if let Some(change) = write_assessment(ds, anchor, &assessment).await? {
            report.changes.push(change);
        }
    }
    Ok(report)
}

/// Reconciles one anchor as the system, before an owner decision reads its state.
pub(crate) async fn reconcile_decision(
    engine: &Engine,
    ds: &DocStore,
    anchor_id: &str,
) -> Result<()> {
    let Some(anchor) = ds
        .anchors()
        .await?
        .into_iter()
        .find(|row| row.id == anchor_id)
    else {
        return Ok(());
    };
    let lineage = ds.lineage().await?;
    let assessment = assess(engine, &lineage, &anchor).await?;
    write_assessment(ds, &anchor, &assessment).await?;
    Ok(())
}

/// The file reached from `file_id` over confirmed lineage edges, at most `MAX_LINEAGE_HOPS`
/// edges. `None` when the file id is unknown to the ledger.
pub(crate) async fn follow_file(
    engine: &Engine,
    ds: &DocStore,
    file_id: &str,
) -> Result<Option<FileRecord>> {
    let lineage = ds.lineage().await?;
    let target = successor_chain(&lineage, file_id);
    engine.store.file_by_id(&target).await
}

/// The file id at the end of the confirmed chain from `file_id`.
fn successor_chain(lineage: &[LineageRow], file_id: &str) -> String {
    let edges = confirmed_edges(lineage);
    walk(&edges, file_id)
        .pop()
        .unwrap_or_else(|| file_id.to_string())
}

fn confirmed_edges(lineage: &[LineageRow]) -> Vec<(String, String)> {
    lineage
        .iter()
        .filter(|row| row.state == LineageState::Confirmed.as_str())
        .map(|row| (row.old_file_id.clone(), row.new_file_id.clone()))
        .collect()
}

/// The files visited from `start` over the first edge found at each step, stopping at a repeat
/// or after `MAX_LINEAGE_HOPS` edges. The first element is `start`.
fn walk(edges: &[(String, String)], start: &str) -> Vec<String> {
    let mut visited = vec![start.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(start.to_string());
    let mut current = start.to_string();
    for _ in 0..MAX_LINEAGE_HOPS {
        let Some((_, next)) = edges.iter().find(|(old, _)| *old == current) else {
            break;
        };
        if !seen.insert(next.clone()) {
            break;
        }
        visited.push(next.clone());
        current = next.clone();
    }
    visited
}

/// Section-overlap score in per mille: the old sections whose full hash occurs among the new
/// sections, times 1000, divided by the number of old sections, rounded down. Zero for no old
/// sections.
pub fn overlap_permille(old: &[String], new: &[String]) -> i64 {
    if old.is_empty() {
        return 0;
    }
    let present = old.iter().filter(|hash| new.contains(hash)).count() as i64;
    present * 1000 / old.len() as i64
}

fn section_hashes(outline: &Outline) -> Vec<String> {
    outline
        .sections
        .iter()
        .map(|clause| clause.full_hash.clone())
        .collect()
}

/// The reconciliation of one anchor (design 5.4). Reads the ledger and the bytes of the latest
/// version; writes nothing.
async fn assess(
    engine: &Engine,
    lineage: &[LineageRow],
    anchor: &AnchorRecord,
) -> Result<Assessment> {
    let target_id = successor_chain(lineage, &anchor.file_id);
    let mut out = Assessment {
        target_id: target_id.clone(),
        state: anchor.state.clone(),
        key: anchor.current_key.clone(),
        method: Method::None.as_str().to_string(),
        version_id: anchor.current_version_id.clone(),
        current_hash: anchor.current_hash.clone(),
        fill: None,
        changed: false,
    };
    let Some(target) = engine.store.file_by_id(&target_id).await? else {
        out.state = AnchorState::FileMissing.as_str().to_string();
        return Ok(finish(out, anchor));
    };
    if target.deleted {
        out.state = AnchorState::FileMissing.as_str().to_string();
        return Ok(finish(out, anchor));
    }
    let Some(latest) = engine.store.latest_version(&target.id).await? else {
        out.state = AnchorState::Unknown.as_str().to_string();
        return Ok(finish(out, anchor));
    };
    if latest.id == anchor.current_version_id && is_settled_state(&anchor.state) {
        return Ok(finish(out, anchor));
    }
    let Some(bytes) = read_bytes(engine, &latest) else {
        out.state = AnchorState::Unknown.as_str().to_string();
        out.version_id = latest.id.clone();
        return Ok(finish(out, anchor));
    };
    let extension = extension_of(&target.relative_path);
    let outline = doctask_outline::outline(&bytes, &extension);
    let (full_hash, body_hash) = if anchor.full_hash.is_empty() {
        match fill_fingerprint(engine, anchor, &extension).await? {
            Some(fingerprint) => {
                let pair = (fingerprint.full_hash.clone(), fingerprint.body_hash.clone());
                out.fill = Some(fingerprint);
                pair
            }
            None => return Ok(finish(out, anchor)),
        }
    } else {
        (anchor.full_hash.clone(), anchor.body_hash.clone())
    };
    let verdict = classify_anchor(
        &AnchorRef {
            key: &anchor.current_key,
            full_hash: &full_hash,
            body_hash: &body_hash,
        },
        LatestVersion::Present(&outline),
    );
    out.state = verdict.state.as_str().to_string();
    out.key = verdict.key.clone();
    out.method = verdict.method.as_str().to_string();
    out.version_id = latest.id.clone();
    out.current_hash = outline
        .find_key(&verdict.key)
        .map(|clause| clause.full_hash.clone())
        .unwrap_or_default();
    Ok(finish(out, anchor))
}

/// True for the states whose verdict is final for the version the anchor is on.
fn is_settled_state(state: &str) -> bool {
    matches!(
        AnchorState::parse(state),
        Some(
            AnchorState::Valid
                | AnchorState::NeedsReview
                | AnchorState::Relocated
                | AnchorState::Ambiguous
                | AnchorState::Orphaned
        )
    )
}

/// Sets `changed` by comparing the assessment with the stored anchor.
fn finish(mut out: Assessment, anchor: &AnchorRecord) -> Assessment {
    out.changed = out.state != anchor.state
        || out.key != anchor.current_key
        || out.version_id != anchor.current_version_id
        || out.current_hash != anchor.current_hash
        || out.fill.is_some();
    out
}

/// The fingerprint of the anchor's section on its base version, when that version is local.
async fn fill_fingerprint(
    engine: &Engine,
    anchor: &AnchorRecord,
    extension: &str,
) -> Result<Option<AnchorFingerprint>> {
    let Some(base) = engine.store.version(&anchor.base_version_id).await? else {
        return Ok(None);
    };
    let Some(bytes) = read_bytes(engine, &base) else {
        return Ok(None);
    };
    let outline = doctask_outline::outline(&bytes, extension);
    Ok(outline
        .find_key(&anchor.section_key)
        .map(|clause| fingerprint_of(&anchor.baseline_id, clause)))
}

/// Writes the assessment of an anchor as one commit with its event, when it changed.
async fn write_assessment(
    ds: &DocStore,
    anchor: &AnchorRecord,
    assessment: &Assessment,
) -> Result<Option<AnchorChange>> {
    if !assessment.changed {
        return Ok(None);
    }
    let mut values = vec![
        ("state", text(&assessment.state)),
        ("current_version_id", text(&assessment.version_id)),
        ("current_key", text(&assessment.key)),
        ("current_hash", text(&assessment.current_hash)),
        ("updated_at", text(&now())),
    ];
    if let Some(fingerprint) = &assessment.fill {
        values.extend(fill_values(fingerprint));
    }
    let payload = json!({
        "anchor_id": anchor.id,
        "from_state": anchor.state,
        "to_state": assessment.state,
        "from_key": anchor.current_key,
        "to_key": assessment.key,
        "method": assessment.method,
        "from_version_id": anchor.current_version_id,
        "to_version_id": assessment.version_id,
        "target_file_id": assessment.target_id,
    });
    let event = NewEvent {
        task_id: String::new(),
        file_id: anchor.file_id.clone(),
        subject_kind: "anchor".to_string(),
        subject_id: anchor.id.clone(),
        kind: "dt.anchor.reconciled".to_string(),
        from_status: String::new(),
        to_status: String::new(),
        actor: SYSTEM_NAME.to_string(),
        actor_kind: ActorKind::System,
        session: String::new(),
        claimed_session: String::new(),
        payload_json: payload.to_string(),
    };
    let writes = [RowWrite::Update {
        table: "doc_anchors",
        key: "id",
        key_value: text(&anchor.id),
        values,
    }];
    let records = ds.commit_step(None, &writes, &[step(event)]).await?;
    let seq = records.first().map(|record| record.seq).unwrap_or_default();
    Ok(Some(AnchorChange {
        anchor_id: anchor.id.clone(),
        from_state: anchor.state.clone(),
        to_state: assessment.state.clone(),
        from_key: anchor.current_key.clone(),
        to_key: assessment.key.clone(),
        method: assessment.method.clone(),
        event_seq: seq,
    }))
}

/// Fingerprint columns of an anchor, with its state and version columns, for a decision.
fn fingerprint_values(
    fingerprint: &AnchorFingerprint,
    state: &str,
    current_version_id: &str,
    reviewed_version_id: &str,
) -> Vec<(&'static str, Value)> {
    let mut values = fill_values(fingerprint);
    values.extend([
        ("state", text(state)),
        ("current_key", text(&fingerprint.section_key)),
        ("current_hash", text(&fingerprint.full_hash)),
        ("current_version_id", text(current_version_id)),
        ("reviewed_version_id", text(reviewed_version_id)),
        ("updated_at", text(&now())),
    ]);
    values
}

fn fill_values(fingerprint: &AnchorFingerprint) -> Vec<(&'static str, Value)> {
    vec![
        ("title", text(&fingerprint.title)),
        ("level", Value::Integer(fingerprint.level)),
        ("line_start", Value::Integer(fingerprint.line_start)),
        ("line_end", Value::Integer(fingerprint.line_end)),
        ("clause_id", text(&fingerprint.clause_number)),
        ("full_hash", text(&fingerprint.full_hash)),
        ("body_hash", text(&fingerprint.body_hash)),
    ]
}

/// The fingerprint of a clause, as stored by an anchor created on `baseline_id`.
fn fingerprint_of(baseline_id: &str, clause: &Clause) -> AnchorFingerprint {
    AnchorFingerprint {
        baseline_id: baseline_id.to_string(),
        section_key: clause.key.clone(),
        clause_number: clause.number.clone().unwrap_or_default(),
        title: clause.title.clone(),
        level: i64::from(clause.level),
        line_start: clause.first_line as i64,
        line_end: clause.last_line as i64,
        full_hash: clause.full_hash.clone(),
        body_hash: clause.body_hash.clone(),
    }
}

/// The bytes of a version when they are local and within the size limit.
fn read_bytes(engine: &Engine, version: &VersionRecord) -> Option<Vec<u8>> {
    if !engine.cas.has(&version.content_hash) {
        return None;
    }
    engine
        .cas
        .load_bytes(&version.content_hash, MAX_TARGET_FILE_BYTES)
        .ok()
}

/// The live file must exist, and its digest must equal the ledger's current hash.
fn check_live(engine: &Engine, file: &FileRecord) -> Result<()> {
    if file.deleted {
        return Err(file_missing());
    }
    let bytes = std::fs::read(engine.root.join(&file.relative_path)).map_err(|_| file_missing())?;
    if digest(&bytes) != file.current_hash {
        return Err(fail(
            ErrorCode::FileChanged,
            "现存文件与账本不一致",
            "先运行 capture 记录当前版本",
        ));
    }
    Ok(())
}

fn extension_of(relative_path: &str) -> String {
    Path::new(relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn step(event: NewEvent) -> StepEvent {
    StepEvent {
        id: new_id(),
        event,
    }
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn fail(code: ErrorCode, message: &str, hint: &str) -> anyhow::Error {
    DocError::new(code, message, hint).into()
}

fn not_found() -> anyhow::Error {
    fail(
        ErrorCode::NotFound,
        "未找到任务、文件或谱系记录",
        "确认引用后重试",
    )
}

fn stale() -> anyhow::Error {
    fail(
        ErrorCode::StaleSubject,
        "展示后主体已变化",
        "重新展示后再确认",
    )
}

fn guard(message: &str, hint: &str) -> anyhow::Error {
    fail(ErrorCode::GuardFailed, message, hint)
}

fn file_missing() -> anyhow::Error {
    fail(
        ErrorCode::FileMissing,
        "文件已删除或不可读",
        "恢复文件或确认文件移动的谱系",
    )
}

fn anchor_broken() -> anyhow::Error {
    fail(
        ErrorCode::AnchorBroken,
        "版本字节不在本地或不可读",
        "重新登记基线或恢复字节",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashes(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    /// Per-mille overlap rounds down and is zero without old sections (design 5.7).
    #[test]
    fn tc909_overlap_permille_rounds_down_and_is_pure() {
        let old = hashes(&["a", "b", "c", "d", "e"]);
        assert_eq!(
            overlap_permille(&old, &hashes(&["a", "b", "c", "d", "x"])),
            800
        );
        assert_eq!(
            overlap_permille(&old, &hashes(&["a", "b", "c", "x", "y"])),
            600
        );
        assert_eq!(
            overlap_permille(&old, &hashes(&["a", "b", "c", "d", "e"])),
            1000
        );
        assert_eq!(overlap_permille(&old, &hashes(&["x"])), 0);
        assert_eq!(overlap_permille(&[], &hashes(&["a"])), 0);
        let seven = hashes(&["a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(
            overlap_permille(&seven, &hashes(&["a", "b", "c", "d", "x", "y", "z"])),
            571
        );
    }

    /// The successor chain follows confirmed edges only, stops at a repeat and at the hop limit.
    #[test]
    fn tc908_walk_stops_at_repeats_and_hop_limit() {
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ];
        assert_eq!(walk(&edges, "a"), vec!["a".to_string(), "b".to_string()]);
        let chain: Vec<(String, String)> = (0..12)
            .map(|index| (format!("f{index}"), format!("f{}", index + 1)))
            .collect();
        assert_eq!(walk(&chain, "f0").len(), MAX_LINEAGE_HOPS + 1);
        assert_eq!(walk(&chain, "unknown"), vec!["unknown".to_string()]);
    }
}
