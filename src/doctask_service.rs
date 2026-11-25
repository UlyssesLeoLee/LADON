//! Service core of the document-task module. The one-shot command, the console and the MCP
//! server call it with a `Caller` that fixes the principal. Each operation checks the principal
//! against the authorization table, resolves the file or task reference, applies the transfer
//! gate for the agent principal, validates the text, checks the subject shown to the human and
//! then writes through `DocStore`. The service holds no SQL.

use crate::crypto::digest;
use crate::doctask_model::{
    authorize, parse_annotation_handle, parse_section_selector, parse_task_handle, required_text,
    sha256_hex, validate_priority, validate_text, ActorKind, AnchorState, DocError, ErrorCode,
    Handle, Operation, Principal, ProposalState, SectionSelector, SourceKind, TaskState,
    Visibility, MAX_ANNOTATION_CHARS, MAX_DESCRIPTION_CHARS, MAX_QUOTE_CHARS, MAX_REASON_CHARS,
    MAX_TARGET_FILE_BYTES, MAX_TASK_RATIONALE_CHARS, MAX_TITLE_CHARS, MIN_HEX_PREFIX_CHARS,
};
use crate::doctask_outline::{self, Clause, Outline};
use crate::doctask_store::{
    AnchorFingerprint, AnnotationRow, BaselineRow, DocStore, NewAnnotation, NewEvent, NewTask,
    RowWrite, TaskRow,
};
use crate::engine::Engine;
use crate::model::{FileRecord, VersionRecord};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Map, Value as Json};
use std::collections::HashMap;
use std::path::Path;
use turso::Value;
use uuid::Uuid;

/// Longest owner name, in characters.
const MAX_OWNER_CHARS: usize = 200;

/// The principal and the name, session and source kind it writes with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub principal: Principal,
    pub name: String,
    pub session: String,
    pub source: SourceKind,
}

impl Caller {
    /// A human principal (owner) with its entry point.
    pub fn human(name: &str, source: SourceKind) -> Caller {
        Caller {
            principal: Principal::Human,
            name: name.to_string(),
            session: String::new(),
            source,
        }
    }

    /// The agent principal of one Hermes launch.
    pub fn agent(launch_id: &str) -> Caller {
        Caller {
            principal: Principal::Agent,
            name: "hermes".to_string(),
            session: format!("hermes:{launch_id}"),
            source: SourceKind::Hermes,
        }
    }

    fn actor_kind(&self) -> ActorKind {
        ActorKind::from(self.principal)
    }
}

/// The subject that the human saw before a confirmation: a version id and a content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub version_id: String,
    pub hash: String,
}

/// A list of records and the number of records omitted for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing<T> {
    pub items: Vec<T>,
    pub hidden: u64,
}

/// The external-transfer gate of design 13.1. Every refusal, including an error of the rule set,
/// is `false`.
#[async_trait]
pub trait TransferGate: Send + Sync {
    /// True when the rule set in force is usable.
    async fn policy_usable(&self) -> bool;
    /// True when the bytes of the file may leave the machine.
    async fn file_allowed(&self, file: &FileRecord) -> bool;
    /// True when the bytes of this version may leave the machine.
    async fn version_allowed(&self, version: &VersionRecord) -> bool;
}

/// The gate of the engine: `Engine::remote_read_allowed` and `Engine::remote_version_allowed`.
pub struct EngineGate<'a> {
    engine: &'a Engine,
}

impl<'a> EngineGate<'a> {
    pub fn new(engine: &'a Engine) -> EngineGate<'a> {
        EngineGate { engine }
    }
}

#[async_trait]
impl<'a> TransferGate for EngineGate<'a> {
    async fn policy_usable(&self) -> bool {
        self.engine.refresh_policy_if_stale().is_ok()
    }

    async fn file_allowed(&self, file: &FileRecord) -> bool {
        self.engine.remote_read_allowed(file).await.unwrap_or(false)
    }

    async fn version_allowed(&self, version: &VersionRecord) -> bool {
        self.engine
            .remote_version_allowed(version)
            .await
            .unwrap_or(false)
    }
}

/// A task as the caller may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    pub id: String,
    pub handle: String,
    pub task_no: i64,
    pub file_ref: String,
    pub path: String,
    pub anchor_id: String,
    pub section: String,
    pub clause_number: String,
    pub anchor_state: String,
    pub unbound: bool,
    pub base_version_id: String,
    pub base_version_no: i64,
    pub base_stale: bool,
    pub file_state: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub priority: i64,
    pub proposal_state: String,
    pub visibility: String,
    pub source_kind: String,
    pub created_by: String,
    pub created_by_kind: String,
    pub created_at: String,
    pub updated_at: String,
    pub blocked_by: Vec<String>,
}

/// One event of the chain. The payload is absent for the agent principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventView {
    pub seq: i64,
    pub kind: String,
    pub from_status: String,
    pub to_status: String,
    pub actor: String,
    pub actor_kind: String,
    pub timestamp: String,
    pub payload: Option<String>,
}

/// The card of one task: the task and its events in `seq` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCard {
    pub task: TaskView,
    pub events: Vec<EventView>,
}

/// An annotation as the caller may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationView {
    pub id: String,
    pub handle: String,
    pub file_ref: String,
    pub path: String,
    pub section: String,
    pub base_version_id: String,
    pub text: String,
    pub author: String,
    pub author_kind: String,
    pub visibility: String,
    pub rationale: String,
    pub quote: String,
    pub created_at: String,
}

/// The result of adding an annotation. `existing` is true when an identical annotation was returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationRef {
    pub id: String,
    pub handle: String,
    pub existing: bool,
}

/// The result of creating a task. `existing` is true when an open task with the same title was returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRef {
    pub id: String,
    pub handle: String,
    pub existing: bool,
}

/// The fields of a new task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    pub title: String,
    pub description: String,
    pub owner: String,
    pub priority: i64,
    pub private: bool,
}

impl TaskSpec {
    /// A task with the given title, priority 2, no description or owner, and shared visibility.
    pub fn new(title: &str) -> TaskSpec {
        TaskSpec {
            title: title.to_string(),
            description: String::new(),
            owner: String::new(),
            priority: 2,
            private: false,
        }
    }
}

/// The fields of a task edit. An absent field is unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskEdit {
    pub title: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub priority: Option<i64>,
    pub visibility: Option<Visibility>,
}

/// Per-call state of the agent projection: gate results per file and per version, and the
/// number of records omitted.
#[derive(Default)]
struct Projection {
    files: HashMap<String, bool>,
    versions: HashMap<String, bool>,
    hidden: u64,
}

/// The service of the document-task module for one caller.
pub struct DocTaskService<'a> {
    engine: &'a Engine,
    ds: DocStore,
    gate: Box<dyn TransferGate + 'a>,
    caller: Caller,
}

impl<'a> DocTaskService<'a> {
    /// Applies migration 4 when needed, imports legacy rows, and returns the service. May write.
    pub async fn open(engine: &'a Engine, caller: Caller) -> Result<DocTaskService<'a>> {
        engine.store.ensure_schema(false).await?;
        let ds = DocStore::new(engine.store.clone());
        ds.backfill().await?;
        Ok(Self::build(engine, ds, caller))
    }

    /// Returns the service when migration 4 is recorded. Writes nothing.
    pub async fn attach(engine: &'a Engine, caller: Caller) -> Result<DocTaskService<'a>> {
        let ds = DocStore::new(engine.store.clone());
        if !ds.schema_current().await? {
            return Err(err(
                ErrorCode::SchemaOutdated,
                "数据库结构版本低于 4",
                "由所有者运行 doc migrate 后重试",
            ));
        }
        Ok(Self::build(engine, ds, caller))
    }

    /// Replaces the transfer gate. Used by callers that hold a different gate.
    pub fn with_gate(mut self, gate: Box<dyn TransferGate + 'a>) -> DocTaskService<'a> {
        self.gate = gate;
        self
    }

    fn build(engine: &'a Engine, ds: DocStore, caller: Caller) -> DocTaskService<'a> {
        DocTaskService {
            engine,
            ds,
            gate: Box::new(EngineGate::new(engine)),
            caller,
        }
    }

    fn is_agent(&self) -> bool {
        self.caller.principal == Principal::Agent
    }

    /// For the agent principal, fails when the rule set is not usable.
    async fn ensure_policy(&self) -> Result<()> {
        if self.is_agent() && !self.gate.policy_usable().await {
            return Err(err(
                ErrorCode::PolicyDenied,
                "外传规则不可用",
                "修复 .LADONignore 后重试",
            ));
        }
        Ok(())
    }

    /// Resolves a file reference. The human may give a path or an id; the agent gives an id
    /// only, and a refused file is the same not-found as a missing one.
    pub async fn resolve_file(&self, file_ref: &str) -> Result<FileRecord> {
        self.ensure_policy().await?;
        let reference = file_ref.trim();
        if self.is_agent() {
            let file = self
                .engine
                .store
                .file_by_id(reference)
                .await?
                .ok_or_else(not_found)?;
            if !self.gate.file_allowed(&file).await {
                return Err(not_found());
            }
            return Ok(file);
        }
        let relative = reference.replace('\\', "/");
        if let Some(file) = self.engine.store.find_path(&relative).await? {
            return Ok(file);
        }
        self.engine
            .store
            .file_by_id(reference)
            .await?
            .ok_or_else(not_found)
    }

    /// Every visible task with the number of omitted ones.
    pub async fn list_tasks(&self) -> Result<Listing<TaskView>> {
        self.ensure_policy().await?;
        let rows = self.ds.tasks().await?;
        let deps = self.dependencies().await?;
        let mut proj = Projection::default();
        let mut items = Vec::new();
        for row in &rows {
            let Some(file) = self.task_file(&mut proj, row).await? else {
                continue;
            };
            let blocked = self.blocked_by(&mut proj, row, &deps, &rows).await?;
            items.push(self.task_view(row, &file, blocked).await?);
        }
        Ok(Listing {
            items,
            hidden: proj.hidden,
        })
    }

    /// The card of one task. A refused, `local_only` or missing task is the same `E_NOT_FOUND`.
    pub async fn show_task(&self, handle: &str) -> Result<TaskCard> {
        self.ensure_policy().await?;
        let rows = self.ds.tasks().await?;
        let row = find_task(&rows, handle, self.caller.principal)?;
        let deps = self.dependencies().await?;
        let mut proj = Projection::default();
        let file = self
            .task_file(&mut proj, &row)
            .await?
            .ok_or_else(not_found)?;
        let blocked = self.blocked_by(&mut proj, &row, &deps, &rows).await?;
        let task = self.task_view(&row, &file, blocked).await?;
        let events = self.task_events(&row.id).await?;
        Ok(TaskCard { task, events })
    }

    /// Registers the baseline of one version of a file. The subject shown must equal the version
    /// id and content hash; the live file must equal the ledger. Human only.
    pub async fn register_baseline(
        &self,
        file_ref: &str,
        version_no: Option<i64>,
        note: &str,
        shown: &Shown,
    ) -> Result<BaselineRow> {
        authorize(self.caller.principal, Operation::BaselineRegister)?;
        validate_text("note", note, 0, MAX_REASON_CHARS)?;
        let file = self.resolve_file(file_ref).await?;
        let found = match version_no {
            Some(number) => self
                .engine
                .store
                .versions(&file.id)
                .await?
                .into_iter()
                .find(|v| v.number == number),
            None => self.engine.store.latest_version(&file.id).await?,
        };
        let version = found.ok_or_else(not_found)?;
        if version.id != shown.version_id || version.content_hash != shown.hash {
            return Err(stale_subject());
        }
        self.check_live_file(&file)?;
        if let Some(existing) = self.active_baseline_of(&file.id, &version.id).await? {
            return Ok(existing);
        }
        let replaces = self
            .ds
            .baselines(Some(file.id.as_str()))
            .await?
            .into_iter()
            .filter(|b| b.version_id == version.id && b.revoked_at.is_some())
            .last()
            .map(|b| b.id);
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let mut payload = json!({
            "version_no": version.number,
            "content_hash": version.content_hash,
            "note": note,
        });
        if let Some(previous) = replaces {
            payload["replaces_revoked"] = json!(previous);
        }
        let event = NewEvent {
            task_id: String::new(),
            file_id: file.id.clone(),
            subject_kind: "baseline".to_string(),
            subject_id: id.clone(),
            kind: "dt.baseline.registered".to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: self.caller.name.clone(),
            actor_kind: self.caller.actor_kind(),
            session: self.caller.session.clone(),
            claimed_session: String::new(),
            payload_json: payload.to_string(),
        };
        let writes = vec![RowWrite::Insert {
            table: "doc_baselines",
            values: vec![
                ("id", Value::from(id.as_str())),
                ("file_id", Value::from(file.id.as_str())),
                ("version_id", Value::from(version.id.as_str())),
                ("version_no", Value::from(version.number)),
                ("content_hash", Value::from(version.content_hash.as_str())),
                ("approver", Value::from(self.caller.name.as_str())),
                (
                    "approver_kind",
                    Value::from(self.caller.actor_kind().as_str()),
                ),
                ("note", Value::from(note)),
                ("created_at", Value::from(now.as_str())),
            ],
        }];
        self.ds.write_with_event(&writes, &event).await?;
        self.baseline_row(&id).await
    }

    /// Revokes an active baseline. The reason is required; the subject shown must equal the
    /// baseline's version id and content hash. Human only.
    pub async fn revoke_baseline(
        &self,
        baseline_id: &str,
        reason: &str,
        shown: &Shown,
    ) -> Result<BaselineRow> {
        authorize(self.caller.principal, Operation::BaselineRevoke)?;
        required_text("reason", Some(reason), MAX_REASON_CHARS)?;
        let baseline = self
            .ds
            .baselines(None)
            .await?
            .into_iter()
            .find(|b| b.id == baseline_id)
            .ok_or_else(not_found)?;
        if baseline.revoked_at.is_some() {
            return Err(err(
                ErrorCode::InvalidArg,
                "基线已撤销",
                "对同一版本重新登记后再撤销",
            ));
        }
        if baseline.version_id != shown.version_id || baseline.content_hash != shown.hash {
            return Err(stale_subject());
        }
        let now = now_rfc3339();
        let payload = json!({
            "version_no": baseline.version_no,
            "content_hash": baseline.content_hash,
            "reason": reason,
        });
        let event = NewEvent {
            task_id: String::new(),
            file_id: baseline.file_id.clone(),
            subject_kind: "baseline".to_string(),
            subject_id: baseline.id.clone(),
            kind: "dt.baseline.revoked".to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: self.caller.name.clone(),
            actor_kind: self.caller.actor_kind(),
            session: self.caller.session.clone(),
            claimed_session: String::new(),
            payload_json: payload.to_string(),
        };
        let writes = vec![RowWrite::Update {
            table: "doc_baselines",
            key: "id",
            key_value: Value::from(baseline.id.as_str()),
            values: vec![
                ("revoked_at", Value::from(now.as_str())),
                ("revoked_by", Value::from(self.caller.name.as_str())),
            ],
        }];
        self.ds.write_with_event(&writes, &event).await?;
        self.baseline_row(baseline_id).await
    }

    /// Baselines of one file, or of every visible file. Read only.
    pub async fn list_baselines(&self, file_ref: Option<&str>) -> Result<Listing<BaselineRow>> {
        authorize(self.caller.principal, Operation::Read)?;
        self.ensure_policy().await?;
        let filter = match file_ref {
            Some(reference) => Some(self.resolve_file(reference).await?.id),
            None => None,
        };
        let rows = self.ds.baselines(filter.as_deref()).await?;
        let mut proj = Projection::default();
        let mut items = Vec::new();
        for row in rows {
            let allowed = match self.engine.store.file_by_id(&row.file_id).await? {
                Some(file) => self.file_ok(&mut proj, &file).await,
                None => false,
            };
            if allowed {
                items.push(row);
            } else {
                proj.hidden += 1;
            }
        }
        Ok(Listing {
            items,
            hidden: proj.hidden,
        })
    }

    /// Adds an annotation to a section of the file, on the baseline version. The agent's
    /// annotation is `shared`; the human's is `local_only` when `private`.
    pub async fn add_annotation(
        &self,
        file_ref: &str,
        section: &str,
        text: &str,
        rationale: &str,
        quote: &str,
        private: bool,
    ) -> Result<AnnotationRef> {
        authorize(self.caller.principal, Operation::AddAnnotation)?;
        validate_text("text", text, 1, MAX_ANNOTATION_CHARS)?;
        validate_text("rationale", rationale, 0, MAX_TASK_RATIONALE_CHARS)?;
        validate_text("quote", quote, 0, MAX_QUOTE_CHARS)?;
        let file = self.resolve_file(file_ref).await?;
        let (baseline, version) = self.writable_base(&file).await?;
        let (anchor_id, clause) = self
            .strict_anchor(&file, &baseline, &version, section)
            .await?;
        let agent = self.is_agent();
        let existing = self.ds.annotations().await?.into_iter().find(|a| {
            a.anchor_id == anchor_id
                && a.text == text
                && (!agent || a.visibility == Visibility::Shared.as_str())
        });
        if let Some(row) = existing {
            return Ok(AnnotationRef {
                id: row.id,
                handle: format!("N-{}", row.annotation_no),
                existing: true,
            });
        }
        let visibility = if !agent && private {
            Visibility::LocalOnly
        } else {
            Visibility::Shared
        };
        let created = self
            .ds
            .add_annotation(&NewAnnotation {
                file_id: file.id.clone(),
                anchor_id,
                text: text.to_string(),
                author: self.caller.name.clone(),
                author_kind: self.caller.actor_kind(),
                rationale: rationale.to_string(),
                visibility,
                quote: quote.to_string(),
                line_start: clause.first_line as i64,
                line_end: clause.last_line as i64,
                session: self.caller.session.clone(),
            })
            .await?;
        Ok(AnnotationRef {
            id: created.annotation_id,
            handle: format!("N-{}", created.annotation_no),
            existing: false,
        })
    }

    /// Annotations of one file, or of every visible file. Read only.
    pub async fn list_annotations(
        &self,
        file_ref: Option<&str>,
    ) -> Result<Listing<AnnotationView>> {
        authorize(self.caller.principal, Operation::Read)?;
        self.ensure_policy().await?;
        let filter = match file_ref {
            Some(reference) => Some(self.resolve_file(reference).await?.id),
            None => None,
        };
        let rows = self.ds.annotations().await?;
        let mut proj = Projection::default();
        let mut items = Vec::new();
        for row in rows {
            if let Some(file_id) = &filter {
                if &row.file_id != file_id {
                    continue;
                }
            }
            if self.is_agent() && row.visibility != Visibility::Shared.as_str() {
                proj.hidden += 1;
                continue;
            }
            let Some(file) = self.engine.store.file_by_id(&row.file_id).await? else {
                continue;
            };
            if !self.file_ok(&mut proj, &file).await {
                proj.hidden += 1;
                continue;
            }
            if self.is_agent() {
                let version_ok = match self.engine.store.version(&row.base_version_id).await? {
                    Some(version) => self.version_ok(&mut proj, &version).await,
                    None => false,
                };
                if !version_ok {
                    proj.hidden += 1;
                    continue;
                }
            }
            items.push(annotation_view(&row, &file));
        }
        Ok(Listing {
            items,
            hidden: proj.hidden,
        })
    }

    /// Changes the visibility of an annotation. The SHA-256 of its text must equal
    /// `expected_text_hash`. Human only.
    pub async fn set_annotation_visibility(
        &self,
        handle: &str,
        visibility: Visibility,
        expected_text_hash: &str,
    ) -> Result<AnnotationView> {
        authorize(self.caller.principal, Operation::ShareAnnotation)?;
        let rows = self.ds.annotations().await?;
        let row = find_annotation(&rows, handle, self.caller.principal)?;
        let file = self
            .engine
            .store
            .file_by_id(&row.file_id)
            .await?
            .ok_or_else(not_found)?;
        if sha256_hex(row.text.as_bytes()) != expected_text_hash {
            return Err(stale_subject());
        }
        if row.visibility != visibility.as_str() {
            let payload = json!({
                "from": row.visibility,
                "to": visibility.as_str(),
                "text_sha256": sha256_hex(row.text.as_bytes()),
                "text_len": row.text.chars().count(),
            });
            let event = NewEvent {
                task_id: String::new(),
                file_id: row.file_id.clone(),
                subject_kind: "annotation".to_string(),
                subject_id: row.id.clone(),
                kind: "dt.annotation.visibility_changed".to_string(),
                from_status: String::new(),
                to_status: String::new(),
                actor: self.caller.name.clone(),
                actor_kind: self.caller.actor_kind(),
                session: self.caller.session.clone(),
                claimed_session: String::new(),
                payload_json: payload.to_string(),
            };
            let writes = vec![RowWrite::Update {
                table: "doc_annotation_meta",
                key: "annotation_id",
                key_value: Value::from(row.id.as_str()),
                values: vec![("visibility", Value::from(visibility.as_str()))],
            }];
            self.ds.write_with_event(&writes, &event).await?;
        }
        let updated = self
            .ds
            .annotations()
            .await?
            .into_iter()
            .find(|a| a.id == row.id)
            .ok_or_else(not_found)?;
        Ok(annotation_view(&updated, &file))
    }

    /// Creates a task on a section of the file, pinned to the baseline version. The agent's task
    /// is a proposal, `shared` and sourced from Hermes; the human's is accepted.
    pub async fn create_task(
        &self,
        file_ref: &str,
        section: &str,
        spec: &TaskSpec,
    ) -> Result<TaskRef> {
        authorize(self.caller.principal, Operation::CreateProposal)?;
        required_text("title", Some(spec.title.as_str()), MAX_TITLE_CHARS)?;
        validate_text("description", &spec.description, 0, MAX_DESCRIPTION_CHARS)?;
        validate_text("owner", &spec.owner, 0, MAX_OWNER_CHARS)?;
        let priority = validate_priority(spec.priority)?;
        let file = self.resolve_file(file_ref).await?;
        let (baseline, version) = self.writable_base(&file).await?;
        let (anchor_id, _clause) = self
            .strict_anchor(&file, &baseline, &version, section)
            .await?;
        let agent = self.is_agent();
        let existing = self.ds.tasks().await?.into_iter().find(|t| {
            t.anchor_id == anchor_id
                && t.title == spec.title
                && t.status != TaskState::Done.as_str()
                && t.status != TaskState::Cancelled.as_str()
                && (!agent || t.visibility == Visibility::Shared.as_str())
        });
        if let Some(row) = existing {
            return Ok(TaskRef {
                id: row.id,
                handle: format!("T-{}", row.task_no),
                existing: true,
            });
        }
        let (proposal_state, visibility, source_kind) = if agent {
            (
                ProposalState::Proposed,
                Visibility::Shared,
                SourceKind::Hermes,
            )
        } else {
            let visibility = if spec.private {
                Visibility::LocalOnly
            } else {
                Visibility::Shared
            };
            (ProposalState::Accepted, visibility, self.caller.source)
        };
        let created = self
            .ds
            .create_task(&NewTask {
                file_id: file.id.clone(),
                anchor_id,
                title: spec.title.clone(),
                description: spec.description.clone(),
                owner: spec.owner.clone(),
                priority,
                proposal_state,
                visibility,
                source_kind,
                created_by: self.caller.name.clone(),
                created_by_kind: self.caller.actor_kind(),
                session: self.caller.session.clone(),
            })
            .await?;
        Ok(TaskRef {
            id: created.task_id,
            handle: format!("T-{}", created.task_no),
            existing: false,
        })
    }

    /// Edits the attributes of a task. The task content hash must equal `expected_hash`. The
    /// event records the SHA-256 and length of each changed text field. Human only.
    pub async fn edit_task(
        &self,
        handle: &str,
        edit: &TaskEdit,
        expected_hash: &str,
    ) -> Result<TaskView> {
        authorize(self.caller.principal, Operation::EditTaskAttributes)?;
        if let Some(title) = &edit.title {
            required_text("title", Some(title.as_str()), MAX_TITLE_CHARS)?;
        }
        if let Some(description) = &edit.description {
            validate_text("description", description, 0, MAX_DESCRIPTION_CHARS)?;
        }
        if let Some(owner) = &edit.owner {
            validate_text("owner", owner, 0, MAX_OWNER_CHARS)?;
        }
        let priority = match edit.priority {
            Some(value) => Some(validate_priority(value)?),
            None => None,
        };
        let rows = self.ds.tasks().await?;
        let row = find_task(&rows, handle, self.caller.principal)?;
        let deps = self.dependencies().await?;
        let prerequisites = deps.get(&row.id).cloned().unwrap_or_default();
        let current = task_content_hash(
            &row.title,
            &row.description,
            &row.anchor_id,
            row.priority,
            &prerequisites,
        );
        if current != expected_hash {
            return Err(stale_subject());
        }

        let mut changes = Map::new();
        let mut task_values: Vec<(&'static str, Value)> = Vec::new();
        let mut meta_values: Vec<(&'static str, Value)> = Vec::new();
        if let Some(title) = &edit.title {
            if *title != row.title {
                changes.insert("title".into(), text_change(&row.title, title));
                task_values.push(("title", Value::from(title.as_str())));
            }
        }
        if let Some(description) = &edit.description {
            if *description != row.description {
                changes.insert(
                    "description".into(),
                    text_change(&row.description, description),
                );
                task_values.push(("description", Value::from(description.as_str())));
            }
        }
        if let Some(owner) = &edit.owner {
            if *owner != row.owner {
                changes.insert("owner".into(), text_change(&row.owner, owner));
                meta_values.push(("owner", Value::from(owner.as_str())));
            }
        }
        if let Some(value) = priority {
            if i64::from(value) != row.priority {
                changes.insert(
                    "priority".into(),
                    json!({"old": row.priority, "new": value}),
                );
                meta_values.push(("priority", Value::from(i64::from(value))));
            }
        }
        if let Some(visibility) = edit.visibility {
            if visibility.as_str() != row.visibility {
                changes.insert(
                    "visibility".into(),
                    json!({"old": row.visibility, "new": visibility.as_str()}),
                );
                meta_values.push(("visibility", Value::from(visibility.as_str())));
            }
        }
        if changes.is_empty() {
            return self.view_of(&rows, &row).await;
        }

        let now = now_rfc3339();
        meta_values.push(("updated_at", Value::from(now.as_str())));
        let mut writes = Vec::new();
        if !task_values.is_empty() {
            writes.push(RowWrite::Update {
                table: "doc_tasks",
                key: "id",
                key_value: Value::from(row.id.as_str()),
                values: task_values,
            });
        }
        writes.push(RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: Value::from(row.id.as_str()),
            values: meta_values,
        });
        let event = NewEvent {
            task_id: row.id.clone(),
            file_id: row.file_id.clone(),
            subject_kind: "task".to_string(),
            subject_id: row.id.clone(),
            kind: "dt.task.edited".to_string(),
            from_status: String::new(),
            to_status: String::new(),
            actor: self.caller.name.clone(),
            actor_kind: self.caller.actor_kind(),
            session: self.caller.session.clone(),
            claimed_session: String::new(),
            payload_json: json!({ "changed": changes }).to_string(),
        };
        self.ds.write_with_event(&writes, &event).await?;
        let fresh_rows = self.ds.tasks().await?;
        let fresh = fresh_rows
            .iter()
            .find(|t| t.id == row.id)
            .cloned()
            .ok_or_else(not_found)?;
        self.view_of(&fresh_rows, &fresh).await
    }

    /// The legacy `annotate` command: no baseline is required. Human only.
    pub async fn legacy_annotate(
        &self,
        file_ref: &str,
        section: &str,
        text: &str,
    ) -> Result<AnnotationRef> {
        self.legacy_human()?;
        let file = self.resolve_file(file_ref).await?;
        let (baseline_id, version) = self.legacy_base(&file).await?;
        let anchor_id = self
            .legacy_anchor(&file, &baseline_id, &version, section)
            .await?;
        let created = self
            .ds
            .add_annotation(&NewAnnotation {
                file_id: file.id.clone(),
                anchor_id,
                text: text.to_string(),
                author: self.caller.name.clone(),
                author_kind: self.caller.actor_kind(),
                rationale: String::new(),
                visibility: Visibility::Shared,
                quote: String::new(),
                line_start: 0,
                line_end: 0,
                session: self.caller.session.clone(),
            })
            .await?;
        Ok(AnnotationRef {
            id: created.annotation_id,
            handle: format!("N-{}", created.annotation_no),
            existing: false,
        })
    }

    /// The legacy `task` command: no baseline is required; the task is accepted, shared and
    /// sourced as legacy. Human only.
    pub async fn legacy_task(
        &self,
        file_ref: &str,
        section: &str,
        title: &str,
        description: &str,
    ) -> Result<TaskRef> {
        self.legacy_human()?;
        let file = self.resolve_file(file_ref).await?;
        let (baseline_id, version) = self.legacy_base(&file).await?;
        let anchor_id = self
            .legacy_anchor(&file, &baseline_id, &version, section)
            .await?;
        let created = self
            .ds
            .create_task(&NewTask {
                file_id: file.id.clone(),
                anchor_id,
                title: title.to_string(),
                description: description.to_string(),
                owner: String::new(),
                priority: 2,
                proposal_state: ProposalState::Accepted,
                visibility: Visibility::Shared,
                source_kind: SourceKind::Legacy,
                created_by: self.caller.name.clone(),
                created_by_kind: self.caller.actor_kind(),
                session: self.caller.session.clone(),
            })
            .await?;
        Ok(TaskRef {
            id: created.task_id,
            handle: format!("T-{}", created.task_no),
            existing: false,
        })
    }

    fn legacy_human(&self) -> Result<()> {
        if self.caller.principal != Principal::Human {
            return Err(err(
                ErrorCode::HumanOnly,
                "该操作只允许人工主体",
                "由所有者在控制台或一次性命令中执行",
            ));
        }
        Ok(())
    }

    /// Base of a legacy write: the active baseline when there is one, otherwise the latest version
    /// with an empty baseline id.
    async fn legacy_base(&self, file: &FileRecord) -> Result<(String, VersionRecord)> {
        self.check_not_deleted(file)?;
        if let Some(baseline) = self.active_baseline(&file.id).await? {
            let version = self
                .engine
                .store
                .version(&baseline.version_id)
                .await?
                .ok_or_else(not_found)?;
            return Ok((baseline.id, version));
        }
        let version = self
            .engine
            .store
            .latest_version(&file.id)
            .await?
            .ok_or_else(|| err(ErrorCode::NotFound, "文件没有版本", "先捕获文件"))?;
        Ok((String::new(), version))
    }

    /// Anchor of a legacy write. A section that resolves gets a fingerprinted `valid` anchor; any
    /// other section, or cold bytes, gets an `unchecked` anchor keyed by the raw text.
    async fn legacy_anchor(
        &self,
        file: &FileRecord,
        baseline_id: &str,
        version: &VersionRecord,
        section: &str,
    ) -> Result<String> {
        let clause = match self.version_outline(file, version) {
            Ok(outline) => legacy_clause(&outline, section),
            Err(_) => None,
        };
        match clause {
            Some(clause) => {
                self.ds
                    .ensure_anchor_clause(&file.id, &version.id, &fingerprint(baseline_id, &clause))
                    .await
            }
            None => {
                self.ds
                    .ensure_anchor(&file.id, &version.id, section, AnchorState::Unchecked)
                    .await
            }
        }
    }

    /// The checks shared by the writes that need a baseline: the file is tracked and live, the
    /// baseline exists, and for the agent the baseline version is not refused.
    async fn writable_base(&self, file: &FileRecord) -> Result<(BaselineRow, VersionRecord)> {
        self.check_not_deleted(file)?;
        let baseline = self.active_baseline(&file.id).await?.ok_or_else(|| {
            err(
                ErrorCode::NoBaseline,
                "文件没有未撤销的基线",
                "先执行 doc baseline add 登记基线",
            )
        })?;
        let version = self
            .engine
            .store
            .version(&baseline.version_id)
            .await?
            .ok_or_else(not_found)?;
        if self.is_agent() && !self.gate.version_allowed(&version).await {
            return Err(not_found());
        }
        Ok((baseline, version))
    }

    /// Anchor of a strict write: the section must resolve on the baseline version bytes.
    async fn strict_anchor(
        &self,
        file: &FileRecord,
        baseline: &BaselineRow,
        version: &VersionRecord,
        section: &str,
    ) -> Result<(String, Clause)> {
        let outline = self.version_outline(file, version)?;
        let clause = strict_clause(&outline, section)?;
        let anchor_id = self
            .ds
            .ensure_anchor_clause(&file.id, &version.id, &fingerprint(&baseline.id, &clause))
            .await?;
        Ok((anchor_id, clause))
    }

    /// Outline of a version's bytes. Cold bytes are `E_ANCHOR_BROKEN`.
    fn version_outline(&self, file: &FileRecord, version: &VersionRecord) -> Result<Outline> {
        if !self.engine.cas.has(&version.content_hash) {
            return Err(err(
                ErrorCode::AnchorBroken,
                "基线版本字节不在本地",
                "重新登记基线",
            ));
        }
        let bytes = self
            .engine
            .cas
            .load_bytes(&version.content_hash, MAX_TARGET_FILE_BYTES)
            .map_err(|_| {
                err(
                    ErrorCode::AnchorBroken,
                    "基线版本字节不可读",
                    "重新登记基线",
                )
            })?;
        let extension = Path::new(&file.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        Ok(doctask_outline::outline(&bytes, &extension))
    }

    fn check_not_deleted(&self, file: &FileRecord) -> Result<()> {
        if file.deleted {
            return Err(err(
                ErrorCode::FileMissing,
                "文件已删除",
                "恢复文件或停止跟踪后重试",
            ));
        }
        Ok(())
    }

    /// Digest of the live file must equal the ledger's current hash.
    fn check_live_file(&self, file: &FileRecord) -> Result<()> {
        self.check_not_deleted(file)?;
        let path = self.engine.root.join(&file.relative_path);
        let bytes = std::fs::read(&path).map_err(|_| {
            err(
                ErrorCode::FileMissing,
                "现存文件不可读",
                "确认文件仍在工作区内",
            )
        })?;
        if digest(&bytes) != file.current_hash {
            return Err(err(
                ErrorCode::FileChanged,
                "现存文件与账本不一致",
                "先运行 capture 记录当前版本",
            ));
        }
        Ok(())
    }

    async fn active_baseline(&self, file_id: &str) -> Result<Option<BaselineRow>> {
        Ok(self
            .ds
            .baselines(Some(file_id))
            .await?
            .into_iter()
            .filter(|b| b.revoked_at.is_none())
            .last())
    }

    async fn active_baseline_of(
        &self,
        file_id: &str,
        version_id: &str,
    ) -> Result<Option<BaselineRow>> {
        Ok(self
            .ds
            .baselines(Some(file_id))
            .await?
            .into_iter()
            .find(|b| b.version_id == version_id && b.revoked_at.is_none()))
    }

    async fn baseline_row(&self, id: &str) -> Result<BaselineRow> {
        self.ds
            .baselines(None)
            .await?
            .into_iter()
            .find(|b| b.id == id)
            .ok_or_else(not_found)
    }

    /// Gate result of a file for the caller, cached per call.
    async fn file_ok(&self, proj: &mut Projection, file: &FileRecord) -> bool {
        if !self.is_agent() {
            return true;
        }
        if let Some(known) = proj.files.get(&file.id) {
            return *known;
        }
        let allowed = self.gate.file_allowed(file).await;
        proj.files.insert(file.id.clone(), allowed);
        allowed
    }

    /// Gate result of a version for the caller, cached per call.
    async fn version_ok(&self, proj: &mut Projection, version: &VersionRecord) -> bool {
        if !self.is_agent() {
            return true;
        }
        if let Some(known) = proj.versions.get(&version.id) {
            return *known;
        }
        let allowed = self.gate.version_allowed(version).await;
        proj.versions.insert(version.id.clone(), allowed);
        allowed
    }

    /// The file of a task when the caller may see the task; `None` when it is omitted and counted.
    async fn task_file(&self, proj: &mut Projection, row: &TaskRow) -> Result<Option<FileRecord>> {
        if self.is_agent() && row.visibility == Visibility::LocalOnly.as_str() {
            proj.hidden += 1;
            return Ok(None);
        }
        let Some(file) = crate::doctask_reconcile::follow_file(self.engine, &self.ds, &row.file_id).await? else {
            proj.hidden += 1;
            return Ok(None);
        };
        if !self.file_ok(proj, &file).await {
            proj.hidden += 1;
            return Ok(None);
        }
        if self.is_agent() {
            let allowed = match self.engine.store.version(&row.base_version_id).await? {
                Some(version) => self.version_ok(proj, &version).await,
                None => false,
            };
            if !allowed {
                proj.hidden += 1;
                return Ok(None);
            }
        }
        Ok(Some(file))
    }

    /// Visible unfinished prerequisites of a task as handles. Omitted prerequisites are counted.
    async fn blocked_by(
        &self,
        proj: &mut Projection,
        row: &TaskRow,
        deps: &HashMap<String, Vec<String>>,
        rows: &[TaskRow],
    ) -> Result<Vec<String>> {
        let prerequisites = deps.get(&row.id).cloned().unwrap_or_default();
        let mut handles = Vec::new();
        for prerequisite_id in prerequisites {
            let Some(prerequisite) = rows.iter().find(|t| t.id == prerequisite_id) else {
                continue;
            };
            if prerequisite.status == TaskState::Done.as_str()
                || prerequisite.status == TaskState::Cancelled.as_str()
            {
                continue;
            }
            if self.task_file(proj, prerequisite).await?.is_some() {
                handles.push(format!("T-{}", prerequisite.task_no));
            }
        }
        Ok(handles)
    }

    async fn task_view(
        &self,
        row: &TaskRow,
        file: &FileRecord,
        blocked_by: Vec<String>,
    ) -> Result<TaskView> {
        let base = self.engine.store.version(&row.base_version_id).await?;
        let base_version_no = base.map(|v| v.number).unwrap_or(0);
        let latest = self.engine.store.latest_version(&file.id).await?;
        let base_stale = latest.map(|v| v.id != row.base_version_id).unwrap_or(false);
        Ok(TaskView {
            id: row.id.clone(),
            handle: format!("T-{}", row.task_no),
            task_no: row.task_no,
            file_ref: row.file_id.clone(),
            path: file.relative_path.clone(),
            anchor_id: row.anchor_id.clone(),
            section: row.anchor_section.clone(),
            clause_number: row.clause_number.clone().unwrap_or_default(),
            anchor_state: row.anchor_state.clone().unwrap_or_default(),
            unbound: row.baseline_id.as_deref().unwrap_or("").is_empty(),
            base_version_id: row.base_version_id.clone(),
            base_version_no,
            base_stale,
            file_state: if file.deleted { "missing" } else { "live" }.to_string(),
            status: row.status.clone(),
            title: row.title.clone(),
            description: row.description.clone(),
            owner: row.owner.clone(),
            priority: row.priority,
            proposal_state: row.proposal_state.clone(),
            visibility: row.visibility.clone(),
            source_kind: row.source_kind.clone(),
            created_by: row.created_by.clone(),
            created_by_kind: row.created_by_kind.clone(),
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
            blocked_by,
        })
    }

    /// View of one task for the human, after its file is known.
    async fn view_of(&self, rows: &[TaskRow], row: &TaskRow) -> Result<TaskView> {
        let deps = self.dependencies().await?;
        let mut proj = Projection::default();
        let file = crate::doctask_reconcile::follow_file(self.engine, &self.ds, &row.file_id)
            .await?
            .ok_or_else(not_found)?;
        let blocked = self.blocked_by(&mut proj, row, &deps, rows).await?;
        self.task_view(row, &file, blocked).await
    }

    async fn task_events(&self, task_id: &str) -> Result<Vec<EventView>> {
        let with_payload = !self.is_agent();
        let mut out = Vec::new();
        for event in self.ds.events().await? {
            if event.task_id != task_id {
                continue;
            }
            out.push(EventView {
                seq: event.seq,
                kind: event.kind,
                from_status: event.from_status,
                to_status: event.to_status,
                actor: event.actor,
                actor_kind: event.actor_kind,
                timestamp: event.timestamp,
                payload: if with_payload {
                    Some(event.payload_json)
                } else {
                    None
                },
            });
        }
        Ok(out)
    }

    async fn dependencies(&self) -> Result<HashMap<String, Vec<String>>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (task_id, depends_on) in self.ds.dependency_edges().await? {
            map.entry(task_id).or_default().push(depends_on);
        }
        Ok(map)
    }
}

fn err(code: ErrorCode, message: &str, hint: &str) -> anyhow::Error {
    DocError::new(code, message, hint).into()
}

/// The one not-found error of the module. Refused, `local_only` and missing references all use it.
fn not_found() -> anyhow::Error {
    err(
        ErrorCode::NotFound,
        "未找到记录",
        "确认句柄或文件引用后重试",
    )
}

fn stale_subject() -> anyhow::Error {
    err(
        ErrorCode::StaleSubject,
        "展示后主体已变化",
        "重新查看后再确认",
    )
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn text_change(old: &str, new: &str) -> Json {
    json!({
        "old_sha256": sha256_hex(old.as_bytes()),
        "old_len": old.chars().count(),
        "new_sha256": sha256_hex(new.as_bytes()),
        "new_len": new.chars().count(),
    })
}

fn fingerprint(baseline_id: &str, clause: &Clause) -> AnchorFingerprint {
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

/// The clause of a selector (design 5.2). Failure to resolve is an error.
fn strict_clause(outline: &Outline, section: &str) -> Result<Clause> {
    let selector = parse_section_selector(Some(section))?;
    Ok(doctask_outline::resolve(outline, &selector)?.clone())
}

/// The clause named by a legacy section string: first as an exact key, then as a selector.
fn legacy_clause(outline: &Outline, section: &str) -> Option<Clause> {
    if let Ok(clause) =
        doctask_outline::resolve(outline, &SectionSelector::Key(section.to_string()))
    {
        return Some(clause.clone());
    }
    strict_clause(outline, section).ok()
}

fn find_task(rows: &[TaskRow], handle: &str, principal: Principal) -> Result<TaskRow> {
    let matches: Vec<&TaskRow> = match parse_task_handle(handle, principal)? {
        Handle::Number(number) => rows.iter().filter(|r| r.task_no == number).collect(),
        Handle::Uuid(id) => rows.iter().filter(|r| r.id == id).collect(),
        Handle::Prefix(prefix) => rows.iter().filter(|r| r.id.starts_with(&prefix)).collect(),
    };
    match matches.as_slice() {
        [] => Err(not_found()),
        [only] => Ok((*only).clone()),
        _ => Err(err(
            ErrorCode::Ambiguous,
            "句柄匹配多个任务",
            "使用 T-n 编号指定任务",
        )),
    }
}

fn find_annotation(
    rows: &[AnnotationRow],
    handle: &str,
    principal: Principal,
) -> Result<AnnotationRow> {
    let matches: Vec<&AnnotationRow> = match parse_annotation_handle(handle, principal)? {
        Handle::Number(number) => rows.iter().filter(|r| r.annotation_no == number).collect(),
        Handle::Uuid(id) => rows.iter().filter(|r| r.id == id).collect(),
        Handle::Prefix(prefix) => rows.iter().filter(|r| r.id.starts_with(&prefix)).collect(),
    };
    match matches.as_slice() {
        [] => Err(not_found()),
        [only] => Ok((*only).clone()),
        _ => Err(err(
            ErrorCode::Ambiguous,
            "句柄匹配多个批注",
            "使用 N-n 编号指定批注",
        )),
    }
}

fn annotation_view(row: &AnnotationRow, file: &FileRecord) -> AnnotationView {
    AnnotationView {
        id: row.id.clone(),
        handle: format!("N-{}", row.annotation_no),
        file_ref: row.file_id.clone(),
        path: file.relative_path.clone(),
        section: row.section_key.clone(),
        base_version_id: row.base_version_id.clone(),
        text: row.text.clone(),
        author: row.author.clone(),
        author_kind: row.author_kind.clone(),
        visibility: row.visibility.clone(),
        rationale: row.rationale.clone(),
        quote: row.quote.clone(),
        created_at: row.created_at.clone(),
    }
}

/// Checks the `--yes` rule of design 9.5: `--yes` needs `--expect-hash`, and the hash prefix must
/// be at least eight hexadecimal characters.
pub fn confirm_yes(yes: bool, expect_hash: Option<&str>) -> Result<(), DocError> {
    match expect_hash {
        None => {
            if yes {
                return Err(DocError::new(
                    ErrorCode::InvalidArg,
                    "--yes 必须同时给出 --expect-hash",
                    "取自展示的主体哈希前缀后重试",
                ));
            }
            Ok(())
        }
        Some(prefix) => {
            if prefix.len() < MIN_HEX_PREFIX_CHARS || !prefix.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(DocError::new(
                    ErrorCode::InvalidArg,
                    "哈希前缀至少 8 位十六进制字符",
                    "使用展示的主体哈希前缀",
                ));
            }
            Ok(())
        }
    }
}

/// True when `prefix` is at least eight hexadecimal characters and a case-insensitive prefix of `full`.
pub fn hash_prefix_matches(prefix: &str, full: &str) -> bool {
    prefix.len() >= MIN_HEX_PREFIX_CHARS
        && prefix.bytes().all(|b| b.is_ascii_hexdigit())
        && full
            .to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
}

/// SHA-256 of the task fields that an edit shows: title, description, anchor, priority and the
/// sorted prerequisites.
pub fn task_content_hash(
    title: &str,
    description: &str,
    anchor_id: &str,
    priority: i64,
    depends_on: &[String],
) -> String {
    let mut deps: Vec<&str> = depends_on.iter().map(|d| d.as_str()).collect();
    deps.sort_unstable();
    let body = json!([title, description, anchor_id, priority, deps]);
    sha256_hex(body.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc878_confirm_rule_and_prefix_match() {
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
        assert!(confirm_yes(false, Some("abc12")).is_err());
        assert!(hash_prefix_matches("ABCDEF12", "abcdef1234"));
        assert!(!hash_prefix_matches("abcdef1", "abcdef1234"));
        let deps = vec!["b".to_string(), "a".to_string()];
        let sorted = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            task_content_hash("t", "d", "anchor", 2, &deps),
            task_content_hash("t", "d", "anchor", 2, &sorted)
        );
    }
}
