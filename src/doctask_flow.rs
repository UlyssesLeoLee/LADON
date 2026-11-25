//! Workflow layer of the document-task module. It moves tasks through the state machine of design
//! section 4.2, records approvals and rejections, adopts and dismisses agent proposals, manages
//! dependencies, links, evidence, quotas, the owner inbox, the parallel schedule and the
//! verification of the governance chain. Every state change is one `DocStore::commit_step`; the
//! module holds no SQL.

use crate::doctask_model::{
    allowed_from, authorize, check_guards, evidence_hash, lookup_transition, normalize_new_text,
    parse_section_selector, parse_task_handle, plan_hash, required_text, schedule_batches,
    sha256_hex, validate_plan_text, validate_text, ActorKind, AnchorState, ApprovalDecision,
    ApprovalKind, ApprovalRecord, DocError, EditKind, ErrorCode, EventRecord, EvidenceKind,
    EvidenceRecord, EvidenceResult, EvidenceSource, GuardInput, Handle, LinkKind, Operation,
    PlanFields, PlanRecord, PlanStatus, Principal, ProposalState, Risk, SchedTask, Schedule,
    TargetKind, TaskState, Transition, Visibility, MAX_AGENT_DEPENDENCIES, MAX_LINK_LABEL_CHARS,
    MAX_REASON_CHARS, MAX_VERIFY_SUMMARY_CHARS,
};
use crate::doctask_outline;
use crate::doctask_service::{Caller, EngineGate, TransferGate};
use crate::doctask_store::{
    AnchorReview, DocStore, LinkRecord, NewEvent, RowWrite, StatusChange, StepEvent, TaskRow,
};
use crate::engine::Engine;
use crate::model::FileRecord;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use turso::Value;
use uuid::Uuid;

/// Largest target file whose digest the stale check of an edit plan reads.
const STALE_CHECK_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Length of the agent rate window, in minutes.
const RATE_WINDOW_MINUTES: i64 = 10;
/// Events of one agent session within the rate window that refuse the next change.
const RATE_LIMIT_EVENTS: usize = 40;
/// Undecided proposals one agent session may hold.
const PROPOSAL_LIMIT: usize = 10;

/// Events whose kind only the human principal may write (design 4.8 and 10.5). Transitions are
/// checked by their pair, see `replay_status`.
const HUMAN_ONLY_KINDS: [&str; 15] = [
    "dt.approval.recorded",
    "dt.proposal.accepted",
    "dt.proposal.dismissed",
    "dt.task.edited",
    "dt.dep.removed",
    "dt.link.removed",
    "dt.baseline.registered",
    "dt.baseline.revoked",
    "dt.commit.applied",
    "dt.anchor.acknowledged",
    "dt.anchor.relocated",
    "dt.anchor.rebased",
    "dt.lineage.confirmed",
    "dt.lineage.rejected",
    "dt.annotation.visibility_changed",
];

/// Event kinds that reset the status replay of a task (design 4.9).
const RESET_KINDS: [&str; 3] = [
    "dt.task.created",
    "dt.task.imported",
    "legacy.task.state_changed",
];

/// The subject fields that an edit component computes for an edit plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSubject {
    pub expected_version_id: String,
    pub expected_hash: String,
    pub result_hash: String,
}

/// The edit component of design 8.1. WP14 implements it; the flow refuses edit plans without it.
#[async_trait]
pub trait EditPlanSupport: Send + Sync {
    /// Validates a replacement of `section` in `target` and returns the subject the plan records.
    async fn prepare(
        &self,
        target: &FileRecord,
        section: &str,
        kind: EditKind,
        new_text: &str,
    ) -> Result<EditSubject>;
}

/// The fields of a plan submission. An edit is optional; without it the plan is analysis-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanSpec {
    pub summary: String,
    pub rationale: String,
    pub impact: String,
    /// `low`, `medium` or `high`; empty means `medium`.
    pub risk: String,
    pub options: Vec<String>,
    pub edit: Option<EditSpec>,
}

/// The target of an edit plan: a file reference, a section selector, the kind and the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSpec {
    pub file_ref: String,
    pub section: String,
    pub kind: String,
    pub new_text: String,
}

/// The result of a state change or of a row change: the handle, the move and the event sequence
/// numbers in the order they were written. `from` and `to` are empty when the status is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub task_id: String,
    pub handle: String,
    pub from: String,
    pub to: String,
    pub events: Vec<i64>,
}

/// The result of a plan submission. `step` is absent when an identical plan already exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRef {
    pub id: String,
    pub plan_no: i64,
    pub plan_hash: String,
    pub existing: bool,
    pub step: Option<Step>,
}

/// A task in an inbox list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBrief {
    pub handle: String,
    pub title: String,
    pub status: String,
    /// Hash of the latest plan, empty when the task has none.
    pub plan_hash: String,
}

/// The owner's inbox: tasks awaiting a decision, a verification, or an adoption.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inbox {
    pub approvals: Vec<TaskBrief>,
    pub verifications: Vec<TaskBrief>,
    pub proposals: Vec<TaskBrief>,
}

/// The schedule of design section 7, with the number of tasks omitted for the caller.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScheduleView {
    pub schedule: Schedule,
    pub hidden: u64,
}

/// One finding of `verify`. Task handles are `T-n`; `seq` is the chain position when relevant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    pub level: &'static str,
    pub task: Option<String>,
    pub seq: Option<i64>,
    pub detail: String,
}

/// The result of `verify`. `ok` is false when any finding is an error.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerifyReport {
    pub ok: bool,
    pub findings: Vec<Finding>,
}

/// Everything the operations read, loaded once per call.
struct Snapshot {
    tasks: Vec<TaskRow>,
    plans: Vec<PlanRecord>,
    approvals: Vec<ApprovalRecord>,
    evidence: Vec<EvidenceRecord>,
    links: Vec<LinkRecord>,
    deps: Vec<(String, String)>,
    anchors: HashMap<String, AnchorReview>,
}

/// The workflow service for one caller.
pub struct DocFlow<'a> {
    engine: &'a Engine,
    ds: DocStore,
    caller: Caller,
    gate: Box<dyn TransferGate + 'a>,
    edit: Option<Box<dyn EditPlanSupport + 'a>>,
}

/// The fields of a governance event that are not common to a call.
#[derive(Clone, Copy)]
struct Fact<'a> {
    subject_kind: &'a str,
    subject_id: &'a str,
    kind: &'a str,
    from: &'a str,
    to: &'a str,
}

/// The stored columns of an edit plan. Analysis-only plans keep every field empty.
struct PlanBody {
    edit_kind: EditKind,
    target_file_id: String,
    target_section: String,
    new_text: String,
    expected_version_id: String,
    expected_hash: String,
    result_hash: String,
}

impl PlanBody {
    fn analysis_only() -> PlanBody {
        PlanBody {
            edit_kind: EditKind::AnalysisOnly,
            target_file_id: String::new(),
            target_section: String::new(),
            new_text: String::new(),
            expected_version_id: String::new(),
            expected_hash: String::new(),
            result_hash: String::new(),
        }
    }
}

fn fail(code: ErrorCode, message: &str, hint: &str) -> anyhow::Error {
    DocError::new(code, message, hint).into()
}

fn not_found() -> anyhow::Error {
    fail(ErrorCode::NotFound, "未找到任务或文件", "确认引用后重试")
}

fn invalid(message: &str, hint: &str) -> anyhow::Error {
    fail(ErrorCode::InvalidArg, message, hint)
}

fn human_only() -> anyhow::Error {
    fail(
        ErrorCode::HumanOnly,
        "该操作只允许人工主体",
        "由所有者在控制台或一次性命令中执行",
    )
}

fn integrity() -> anyhow::Error {
    fail(
        ErrorCode::Integrity,
        "存储的取值不合法",
        "运行 doc verify 检查数据库",
    )
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

fn handle_of(task_no: i64) -> String {
    format!("T-{task_no}")
}

fn task_state(task: &TaskRow) -> Result<TaskState> {
    TaskState::parse(&task.status).ok_or_else(integrity)
}

/// The transition error of a move from `from`, listing the moves the principal may take.
fn denied(from: TaskState, principal: Principal) -> anyhow::Error {
    let options: Vec<String> = allowed_from(from, principal)
        .iter()
        .map(|(to, id)| format!("{} -> {}", id.code(), to.as_str()))
        .collect();
    let listing = if options.is_empty() {
        "none".to_string()
    } else {
        options.join(", ")
    };
    DocError::new(
        ErrorCode::TransitionDenied,
        "该迁移不被允许",
        "查看当前状态允许的迁移后重试",
    )
    .with_detail(format!(
        "from {} by {}; allowed: {}",
        from.as_str(),
        principal.as_str(),
        listing
    ))
    .into()
}

/// The current state of a task, when it is one of `allowed`; otherwise the transition error.
fn expect_from(task: &TaskRow, allowed: &[TaskState], principal: Principal) -> Result<TaskState> {
    let from = task_state(task)?;
    if allowed.contains(&from) {
        return Ok(from);
    }
    Err(denied(from, principal))
}

/// A proposal must be adopted before any transition or plan submission.
fn require_adopted(task: &TaskRow) -> Result<()> {
    if task.proposal_state == ProposalState::Accepted.as_str() {
        return Ok(());
    }
    Err(fail(
        ErrorCode::TransitionDenied,
        "提案尚未被采纳，不能迁移",
        "请所有者采纳该提案（owner must adopt）",
    ))
}

fn anchor_state(task: &TaskRow) -> AnchorState {
    task.anchor_state
        .as_deref()
        .and_then(AnchorState::parse)
        .unwrap_or(AnchorState::Orphaned)
}

/// The latest plan of a task (highest plan number).
fn latest_plan<'p>(plans: &'p [PlanRecord], task_id: &str) -> Option<&'p PlanRecord> {
    plans
        .iter()
        .filter(|plan| plan.task_id == task_id)
        .max_by_key(|plan| plan.plan_no)
}

/// The latest plan of a task that is neither superseded nor rejected.
fn latest_live_plan<'p>(plans: &'p [PlanRecord], task_id: &str) -> Option<&'p PlanRecord> {
    plans
        .iter()
        .filter(|plan| {
            plan.task_id == task_id
                && plan.status != PlanStatus::Superseded
                && plan.status != PlanStatus::Rejected
        })
        .max_by_key(|plan| plan.plan_no)
}

fn is_acceptable_kind(kind: EvidenceKind) -> bool {
    matches!(
        kind,
        EvidenceKind::TestRun | EvidenceKind::Report | EvidenceKind::Note
    )
}

/// True when some row satisfies the acceptance rule of design 4.5: a test run, report or note
/// with result `pass`, written by a human or listed in `accepted_agent`, with no `fail` row after
/// it in chain order. `rows` are the evidence rows of one task in chain order.
fn evidence_accepts(rows: &[EvidenceRecord], accepted_agent: &[String]) -> bool {
    let last_fail = rows
        .iter()
        .rposition(|row| row.result == EvidenceResult::Fail);
    rows.iter().enumerate().any(|(index, row)| {
        is_acceptable_kind(row.kind)
            && row.result == EvidenceResult::Pass
            && (row.actor_kind == ActorKind::Human || accepted_agent.contains(&row.id))
            && last_fail.map_or(true, |fail| index > fail)
    })
}

/// Builds the step with its move and the sequence numbers of its events.
fn step_of(task: &TaskRow, moved: (&str, &str), records: &[EventRecord]) -> Step {
    Step {
        task_id: task.id.clone(),
        handle: handle_of(task.task_no),
        from: moved.0.to_string(),
        to: moved.1.to_string(),
        events: records.iter().map(|record| record.seq).collect(),
    }
}

/// The schedule input of every task, with the given dependency edges `(task, depends_on)`.
fn sched_tasks(snap: &Snapshot, edges: &[(String, String)]) -> Vec<SchedTask> {
    snap.tasks
        .iter()
        .map(|task| {
            let depends_on: Vec<String> = edges
                .iter()
                .filter(|(from, _)| *from == task.id)
                .map(|(_, to)| to.clone())
                .collect();
            let plan_target = latest_live_plan(&snap.plans, &task.id)
                .filter(|plan| !plan.target_file_id.is_empty())
                .map(|plan| plan.target_file_id.clone());
            let link_target = snap
                .links
                .iter()
                .find(|link| {
                    link.task_id == task.id
                        && link.link_kind == LinkKind::Target.as_str()
                        && link.target_kind == TargetKind::File.as_str()
                        && link.removed_at.is_none()
                        && !link.target_file_id.is_empty()
                })
                .map(|link| link.target_file_id.clone());
            SchedTask {
                id: task.id.clone(),
                task_no: task.task_no,
                status: TaskState::parse(&task.status).unwrap_or(TaskState::Cancelled),
                proposal_accepted: task.proposal_state == ProposalState::Accepted.as_str(),
                depends_on,
                plan_target,
                link_target,
            }
        })
        .collect()
}

/// True when the anchor of a task has a review of its current version.
fn anchor_reviewed(snap: &Snapshot, anchor_id: &str) -> bool {
    snap.anchors.get(anchor_id).is_some_and(|anchor| {
        !anchor.reviewed_version_id.is_empty()
            && anchor.reviewed_version_id == anchor.current_version_id
    })
}

/// Replays the status events of the chain per task and reports the illegal moves and the human-only
/// transitions that an actor other than a human performed. Returns `(code, task id, seq, detail)`.
fn replay_status(events: &[EventRecord]) -> Vec<(&'static str, String, i64, String)> {
    let mut found = Vec::new();
    let mut state: HashMap<String, TaskState> = HashMap::new();
    for event in events {
        if event.task_id.is_empty() {
            continue;
        }
        if RESET_KINDS.contains(&event.kind.as_str()) {
            if let Some(to) = TaskState::parse(&event.to_status) {
                state.insert(event.task_id.clone(), to);
            }
            continue;
        }
        if event.kind != "dt.task.transitioned" && event.kind != "dt.task.reopened" {
            continue;
        }
        let current = state
            .get(&event.task_id)
            .copied()
            .unwrap_or(TaskState::Open);
        let (Some(from), Some(to)) = (
            TaskState::parse(&event.from_status),
            TaskState::parse(&event.to_status),
        ) else {
            found.push((
                "ILLEGAL_TRANSITION",
                event.task_id.clone(),
                event.seq,
                "unknown state in a transition event".to_string(),
            ));
            continue;
        };
        if from != current {
            found.push((
                "ILLEGAL_TRANSITION",
                event.task_id.clone(),
                event.seq,
                format!(
                    "from {} but the replayed state is {}",
                    from.as_str(),
                    current.as_str()
                ),
            ));
        } else if event.actor_kind != ActorKind::Unknown.as_str() {
            match Principal::parse(&event.actor_kind) {
                Some(principal) => match lookup_transition(from, to, principal) {
                    Ok(transition) => {
                        let reopen = transition == Transition::T10;
                        if reopen != (event.kind == "dt.task.reopened") {
                            found.push((
                                "ILLEGAL_TRANSITION",
                                event.task_id.clone(),
                                event.seq,
                                "event kind does not match the transition".to_string(),
                            ));
                        }
                    }
                    Err(err) if err.code == ErrorCode::HumanOnly => found.push((
                        "HUMAN_ONLY_FORGED",
                        event.task_id.clone(),
                        event.seq,
                        format!(
                            "{} -> {} by {}",
                            from.as_str(),
                            to.as_str(),
                            principal.as_str()
                        ),
                    )),
                    Err(_) => found.push((
                        "ILLEGAL_TRANSITION",
                        event.task_id.clone(),
                        event.seq,
                        format!(
                            "{} -> {} by {}",
                            from.as_str(),
                            to.as_str(),
                            principal.as_str()
                        ),
                    )),
                },
                None => found.push((
                    "ILLEGAL_TRANSITION",
                    event.task_id.clone(),
                    event.seq,
                    "unknown actor kind".to_string(),
                )),
            }
        }
        state.insert(event.task_id.clone(), to);
    }
    found
}

impl<'a> DocFlow<'a> {
    /// Applies migration 4 when needed, imports legacy rows, and returns the flow. May write.
    pub async fn open(engine: &'a Engine, caller: Caller) -> Result<DocFlow<'a>> {
        engine.store.ensure_schema(false).await?;
        let ds = DocStore::new(engine.store.clone());
        ds.backfill().await?;
        Ok(Self::build(engine, ds, caller))
    }

    /// Returns the flow when migration 4 is recorded. Writes nothing.
    pub async fn attach(engine: &'a Engine, caller: Caller) -> Result<DocFlow<'a>> {
        let ds = DocStore::new(engine.store.clone());
        if !ds.schema_current().await? {
            return Err(fail(
                ErrorCode::SchemaOutdated,
                "数据库结构版本低于 4",
                "由所有者运行 doc migrate 后重试",
            ));
        }
        Ok(Self::build(engine, ds, caller))
    }

    fn build(engine: &'a Engine, ds: DocStore, caller: Caller) -> DocFlow<'a> {
        DocFlow {
            engine,
            ds,
            caller,
            gate: Box::new(EngineGate::new(engine)),
            edit: None,
        }
    }

    /// Replaces the transfer gate.
    pub fn with_gate(mut self, gate: Box<dyn TransferGate + 'a>) -> DocFlow<'a> {
        self.gate = gate;
        self
    }

    /// Registers the edit component. Without it only analysis-only plans are accepted.
    pub fn with_edit_support(mut self, edit: Box<dyn EditPlanSupport + 'a>) -> DocFlow<'a> {
        self.edit = Some(edit);
        self
    }

    fn is_agent(&self) -> bool {
        self.caller.principal == Principal::Agent
    }

    fn actor_kind(&self) -> ActorKind {
        ActorKind::from(self.caller.principal)
    }

    fn require_human(&self) -> Result<()> {
        if self.caller.principal == Principal::Human {
            return Ok(());
        }
        Err(human_only())
    }

    fn require_person(&self) -> Result<()> {
        if self.caller.principal == Principal::System {
            return Err(human_only());
        }
        Ok(())
    }

    /// Builds one event of the caller.
    fn event(&self, task: &TaskRow, fact: Fact<'_>, payload: Json) -> StepEvent {
        make_event(
            task,
            fact,
            payload,
            &self.caller.name,
            self.actor_kind(),
            &self.caller.session,
        )
    }

    /// Refuses the agent's change call when its session has reached the event limit of the window.
    async fn check_agent_rate(&self) -> Result<()> {
        if !self.is_agent() {
            return Ok(());
        }
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(RATE_WINDOW_MINUTES);
        let count = self
            .ds
            .events()
            .await?
            .iter()
            .filter(|event| {
                event.session == self.caller.session
                    && chrono::DateTime::parse_from_rfc3339(&event.timestamp)
                        .map_or(false, |time| time.with_timezone(&chrono::Utc) >= cutoff)
            })
            .count();
        if count >= RATE_LIMIT_EVENTS {
            return Err(fail(ErrorCode::RateLimit, "变更调用过于频繁", "稍后再试"));
        }
        Ok(())
    }

    /// For the agent, checks that a new proposal fits the session quota. Calls `check_agent_rate`
    /// first. The creation path calls this before it creates an agent task.
    pub async fn admit_proposal(&self) -> Result<()> {
        if !self.is_agent() {
            return Ok(());
        }
        self.check_agent_rate().await?;
        let pending = self
            .ds
            .tasks()
            .await?
            .iter()
            .filter(|task| {
                task.session == self.caller.session
                    && task.proposal_state == ProposalState::Proposed.as_str()
            })
            .count();
        if pending >= PROPOSAL_LIMIT {
            return Err(fail(
                ErrorCode::Quota,
                "未决提案数量已达上限",
                "等待所有者处理提案后再提出",
            ));
        }
        Ok(())
    }

    async fn snapshot(&self) -> Result<Snapshot> {
        if self.is_agent() && !self.gate.policy_usable().await {
            return Err(fail(
                ErrorCode::PolicyDenied,
                "外传规则不可用",
                "修复 .LADONignore 后重试",
            ));
        }
        let anchors: HashMap<String, AnchorReview> = self
            .ds
            .anchor_reviews()
            .await?
            .into_iter()
            .map(|anchor| (anchor.id.clone(), anchor))
            .collect();
        Ok(Snapshot {
            tasks: self.ds.tasks().await?,
            plans: self.ds.plans().await?,
            approvals: self.ds.approvals().await?,
            evidence: self.ds.evidence().await?,
            links: self.ds.links().await?,
            deps: self.ds.dependency_edges().await?,
            anchors,
        })
    }

    /// True when the caller may see the task: shared, on a file and a base version the gate allows.
    async fn agent_sees(&self, task: &TaskRow) -> Result<bool> {
        if task.visibility != Visibility::Shared.as_str() {
            return Ok(false);
        }
        let Some(file) = self.engine.store.file_by_id(&task.file_id).await? else {
            return Ok(false);
        };
        if !self.gate.file_allowed(&file).await {
            return Ok(false);
        }
        let Some(version) = self.engine.store.version(&task.base_version_id).await? else {
            return Ok(false);
        };
        Ok(self.gate.version_allowed(&version).await)
    }

    /// The tasks the caller may see.
    async fn visible_tasks(&self, snap: &Snapshot) -> Result<Vec<TaskRow>> {
        if !self.is_agent() {
            return Ok(snap.tasks.clone());
        }
        let mut visible = Vec::new();
        for task in &snap.tasks {
            if self.agent_sees(task).await? {
                visible.push(task.clone());
            }
        }
        Ok(visible)
    }

    /// Resolves a task handle among the visible tasks. `None` when no task matches.
    async fn locate(&self, snap: &Snapshot, handle: &str) -> Result<Option<TaskRow>> {
        let parsed = parse_task_handle(handle, self.caller.principal)?;
        let visible = self.visible_tasks(snap).await?;
        let matches: Vec<&TaskRow> = match &parsed {
            Handle::Number(number) => visible
                .iter()
                .filter(|task| task.task_no == *number)
                .collect(),
            Handle::Uuid(id) => visible.iter().filter(|task| task.id == *id).collect(),
            Handle::Prefix(prefix) => visible
                .iter()
                .filter(|task| task.id.starts_with(prefix.as_str()))
                .collect(),
        };
        match matches.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some((*only).clone())),
            _ => Err(fail(
                ErrorCode::Ambiguous,
                "句柄匹配多个任务",
                "使用 T-n 编号指定任务",
            )),
        }
    }

    async fn find_task(&self, snap: &Snapshot, handle: &str) -> Result<TaskRow> {
        self.locate(snap, handle).await?.ok_or_else(not_found)
    }

    /// Resolves a file reference: the human gives a path or an id, the agent an id that the gate
    /// allows. A refused file is the same not-found as a missing one.
    async fn resolve_file(&self, file_ref: &str) -> Result<FileRecord> {
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

    /// The guard inputs that every transition shares, filled from the snapshot.
    fn base_guard<'g>(&self, snap: &Snapshot, task: &TaskRow) -> GuardInput<'g> {
        let mut guard = GuardInput::new(self.caller.principal);
        let plans: Vec<&PlanRecord> = snap
            .plans
            .iter()
            .filter(|plan| plan.task_id == task.id)
            .collect();
        let latest = latest_plan(&snap.plans, &task.id);
        guard.proposal_accepted = task.proposal_state == ProposalState::Accepted.as_str();
        guard.anchor = anchor_state(task);
        guard.baseline_bound = task
            .baseline_id
            .as_deref()
            .is_some_and(|baseline| !baseline.is_empty());
        guard.anchor_reviewed = anchor_reviewed(snap, &task.anchor_id);
        guard.dependencies_done =
            snap.deps
                .iter()
                .filter(|(from, _)| *from == task.id)
                .all(|(_, depends_on)| {
                    snap.tasks
                        .iter()
                        .find(|other| other.id == *depends_on)
                        .is_some_and(|other| other.status == TaskState::Done.as_str())
                });
        guard.latest_plan_submitted =
            latest.is_some_and(|plan| plan.status == PlanStatus::Submitted);
        guard.plan_applied_or_no_edit = latest.map_or(true, |plan| {
            plan.edit_kind == EditKind::AnalysisOnly || plan.status == PlanStatus::Applied
        });
        guard.has_applying_plan = plans.iter().any(|plan| plan.status == PlanStatus::Applying);
        guard.latest_evidence_failed = snap
            .evidence
            .iter()
            .filter(|row| row.task_id == task.id)
            .last()
            .is_some_and(|row| row.result == EvidenceResult::Fail);
        guard
    }

    /// True when the edit plan no longer matches its target (design 4.4, guard 4).
    async fn plan_is_stale(&self, plan: Option<&PlanRecord>) -> Result<bool> {
        let Some(plan) = plan else {
            return Ok(false);
        };
        if plan.edit_kind == EditKind::AnalysisOnly {
            return Ok(false);
        }
        let Some(latest) = self
            .engine
            .store
            .latest_version(&plan.target_file_id)
            .await?
        else {
            return Ok(true);
        };
        if latest.id != plan.expected_version_id {
            return Ok(true);
        }
        let Some(file) = self.engine.store.file_by_id(&plan.target_file_id).await? else {
            return Ok(true);
        };
        let path = self.engine.root.join(&file.relative_path);
        let Ok(metadata) = std::fs::metadata(&path) else {
            return Ok(true);
        };
        if metadata.len() > STALE_CHECK_MAX_BYTES {
            return Ok(true);
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return Ok(true);
        };
        Ok(sha256_hex(&bytes) != plan.expected_hash)
    }

    /// The version number of the task's baseline.
    async fn baseline_number(&self, task: &TaskRow) -> Result<i64> {
        let baseline_id = task
            .baseline_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| fail(ErrorCode::NoBaseline, "任务没有基线", "先登记基线"))?;
        let baseline = self
            .ds
            .baselines(Some(task.file_id.as_str()))
            .await?
            .into_iter()
            .find(|row| row.id == baseline_id)
            .ok_or_else(not_found)?;
        Ok(baseline.version_no)
    }

    /// Writes one step. The status change, when given, is compare-and-set in the same transaction.
    async fn apply(
        &self,
        task: &TaskRow,
        change: Option<StatusChange>,
        writes: Vec<RowWrite>,
        events: Vec<StepEvent>,
        moved: (&str, &str),
    ) -> Result<Step> {
        let records = self
            .ds
            .commit_step(change.as_ref(), &writes, &events)
            .await?;
        Ok(step_of(task, moved, &records))
    }

    /// Starts the analysis of an open task (T1).
    pub async fn start_analysis(&self, handle: &str) -> Result<Step> {
        self.require_person()?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Open], self.caller.principal)?;
        let to = TaskState::Analysis;
        let transition = lookup_transition(from, to, self.caller.principal)?;
        let guard = self.base_guard(&snap, &task);
        check_guards(transition, &guard)?;
        let moved = (from.as_str(), to.as_str());
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: moved.0,
                to: moved.1,
            },
            json!({}),
        );
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(&task, Some(change), Vec::new(), vec![event], moved)
            .await
    }

    /// Submits a plan for the task (T2). A repeat of the latest submitted plan returns it.
    pub async fn submit_plan(&self, handle: &str, spec: &PlanSpec) -> Result<PlanRef> {
        self.require_person()?;
        authorize(self.caller.principal, Operation::SubmitPlan)?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let options: Vec<&str> = spec.options.iter().map(String::as_str).collect();
        validate_plan_text(&spec.summary, &spec.rationale, &spec.impact, &options)?;
        let risk = if spec.risk.is_empty() {
            Risk::Medium
        } else {
            Risk::parse(&spec.risk)
                .ok_or_else(|| invalid("风险取值须为 low、medium 或 high", "使用合法的风险取值"))?
        };
        let body = match &spec.edit {
            None => PlanBody::analysis_only(),
            Some(edit) => self.prepare_edit(edit).await?,
        };
        let latest = latest_plan(&snap.plans, &task.id).cloned();
        if let Some(previous) = latest
            .as_ref()
            .filter(|plan| plan.status == PlanStatus::Submitted)
        {
            let candidate = plan_hash(&PlanFields {
                task_id: &task.id,
                plan_no: previous.plan_no,
                summary: &spec.summary,
                edit_kind: body.edit_kind.as_str(),
                target_file_id: &body.target_file_id,
                target_section: &body.target_section,
                new_text: &body.new_text,
                expected_hash: &body.expected_hash,
                result_hash: &body.result_hash,
            });
            if candidate == previous.plan_hash {
                return Ok(PlanRef {
                    id: previous.id.clone(),
                    plan_no: previous.plan_no,
                    plan_hash: previous.plan_hash.clone(),
                    existing: true,
                    step: None,
                });
            }
        }
        let from = expect_from(&task, &[TaskState::Analysis], self.caller.principal)?;
        let to = TaskState::Approval;
        lookup_transition(from, to, self.caller.principal)?;
        let plan_no = latest.as_ref().map_or(1, |plan| plan.plan_no + 1);
        let id = new_id();
        let hash = plan_hash(&PlanFields {
            task_id: &task.id,
            plan_no,
            summary: &spec.summary,
            edit_kind: body.edit_kind.as_str(),
            target_file_id: &body.target_file_id,
            target_section: &body.target_section,
            new_text: &body.new_text,
            expected_hash: &body.expected_hash,
            result_hash: &body.result_hash,
        });
        let created = now();
        let options_json =
            serde_json::to_string(&spec.options).unwrap_or_else(|_| "[]".to_string());
        let writes = vec![
            RowWrite::UpdateWhere {
                table: "doc_task_plans",
                filters: vec![
                    ("task_id", text(&task.id)),
                    ("status", text(PlanStatus::Submitted.as_str())),
                ],
                values: vec![("status", text(PlanStatus::Superseded.as_str()))],
            },
            RowWrite::Insert {
                table: "doc_task_plans",
                values: vec![
                    ("id", text(&id)),
                    ("task_id", text(&task.id)),
                    ("plan_no", Value::Integer(plan_no)),
                    ("summary", text(&spec.summary)),
                    ("rationale", text(&spec.rationale)),
                    ("impact", text(&spec.impact)),
                    ("risk", text(risk.as_str())),
                    ("options_json", text(&options_json)),
                    ("edit_kind", text(body.edit_kind.as_str())),
                    ("target_file_id", text(&body.target_file_id)),
                    ("target_section", text(&body.target_section)),
                    ("new_text", text(&body.new_text)),
                    ("expected_version_id", text(&body.expected_version_id)),
                    ("expected_hash", text(&body.expected_hash)),
                    ("result_hash", text(&body.result_hash)),
                    ("plan_hash", text(&hash)),
                    ("status", text(PlanStatus::Submitted.as_str())),
                    ("submitted_by", text(&self.caller.name)),
                    ("submitted_by_kind", text(self.actor_kind().as_str())),
                    ("session", text(&self.caller.session)),
                    ("created_at", text(&created)),
                ],
            },
        ];
        let events = vec![
            self.event(
                &task,
                Fact {
                    subject_kind: "plan",
                    subject_id: &id,
                    kind: "dt.plan.submitted",
                    from: "",
                    to: "",
                },
                json!({"plan_no": plan_no, "plan_hash": hash, "edit_kind": body.edit_kind.as_str()}),
            ),
            self.event(
                &task,
                Fact {
                    subject_kind: "task",
                    subject_id: &task.id,
                    kind: "dt.task.transitioned",
                    from: from.as_str(),
                    to: to.as_str(),
                },
                json!({"plan_no": plan_no}),
            ),
        ];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        let step = self
            .apply(
                &task,
                Some(change),
                writes,
                events,
                (from.as_str(), to.as_str()),
            )
            .await?;
        Ok(PlanRef {
            id,
            plan_no,
            plan_hash: hash,
            existing: false,
            step: Some(step),
        })
    }

    /// Validates an edit plan through the registered component. Without a component the edit is
    /// refused with `E_INVALID_ARG`.
    async fn prepare_edit(&self, edit: &EditSpec) -> Result<PlanBody> {
        let support = self.edit.as_deref().ok_or_else(|| {
            invalid(
                "编辑方案组件未注册",
                "只能提交只分析方案，或注册编辑组件后重试",
            )
        })?;
        let kind = EditKind::parse(&edit.kind)
            .filter(|kind| *kind != EditKind::AnalysisOnly)
            .ok_or_else(|| {
                invalid(
                    "编辑种类不合法",
                    "使用 replace_section 或 insert_after_section",
                )
            })?;
        let new_text = normalize_new_text(&edit.new_text)?;
        let file = self.resolve_file(&edit.file_ref).await?;
        let subject = support
            .prepare(&file, &edit.section, kind, &new_text)
            .await?;
        if subject.expected_hash == subject.result_hash {
            return Err(invalid("编辑方案不改变文件", "修改新文本后重试"));
        }
        Ok(PlanBody {
            edit_kind: kind,
            target_file_id: file.id.clone(),
            target_section: edit.section.clone(),
            new_text,
            expected_version_id: subject.expected_version_id,
            expected_hash: subject.expected_hash,
            result_hash: subject.result_hash,
        })
    }

    /// Approves the latest plan (T3). The subject hash is the one shown to the owner. When the
    /// plan is stale, the plan is superseded and the task returns to analysis in one committed
    /// transaction (T11), and `E_STALE_PLAN` is returned.
    pub async fn approve(
        &self,
        handle: &str,
        expected_subject_hash: &str,
        option: Option<usize>,
        comment: &str,
    ) -> Result<Step> {
        authorize(self.caller.principal, Operation::Approve)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Approval], Principal::Human)?;
        let to = TaskState::Working;
        let transition = lookup_transition(from, to, Principal::Human)?;
        validate_text("comment", comment, 0, MAX_REASON_CHARS)?;
        let latest = latest_plan(&snap.plans, &task.id).cloned();
        let stale = self.plan_is_stale(latest.as_ref()).await?;
        let mut guard = self.base_guard(&snap, &task);
        guard.subject_hash_matches = latest
            .as_ref()
            .is_some_and(|plan| plan.plan_hash == expected_subject_hash);
        guard.plan_stale = stale;
        if let Err(err) = check_guards(transition, &guard) {
            if err.code == ErrorCode::StalePlan {
                if let Some(plan) = latest.as_ref() {
                    self.supersede_stale(&task, plan).await?;
                }
            }
            return Err(err.into());
        }
        let Some(plan) = latest else {
            return Err(not_found());
        };
        let chosen = match option {
            None => None,
            Some(number) => {
                let index = number
                    .checked_sub(1)
                    .filter(|index| *index < plan.options.len())
                    .ok_or_else(|| invalid("备选方案序号不存在", "使用方案中的序号"))?;
                Some(plan.options[index].clone())
            }
        };
        let approval_id = new_id();
        let created = now();
        let approval_event = self.event(
            &task,
            Fact {
                subject_kind: "approval",
                subject_id: &approval_id,
                kind: "dt.approval.recorded",
                from: "",
                to: "",
            },
            json!({
                "approval_id": approval_id,
                "kind": ApprovalKind::Plan.as_str(),
                "decision": ApprovalDecision::Approved.as_str(),
                "subject_hash": expected_subject_hash,
                "comment": comment,
                "chosen_option": chosen,
            }),
        );
        let transition_event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"approval_id": approval_id}),
        );
        let mut plan_values = vec![
            ("status", text(PlanStatus::Approved.as_str())),
            ("decided_at", text(&created)),
        ];
        if let Some(option_text) = &chosen {
            plan_values.push(("chosen_option", text(option_text)));
        }
        let writes = vec![
            RowWrite::Update {
                table: "doc_task_plans",
                key: "id",
                key_value: text(&plan.id),
                values: plan_values,
            },
            RowWrite::Insert {
                table: "doc_task_approvals",
                values: vec![
                    ("id", text(&approval_id)),
                    ("task_id", text(&task.id)),
                    ("kind", text(ApprovalKind::Plan.as_str())),
                    ("decision", text(ApprovalDecision::Approved.as_str())),
                    ("approver", text(&self.caller.name)),
                    ("approver_kind", text(ActorKind::Human.as_str())),
                    ("plan_id", text(&plan.id)),
                    ("subject_hash", text(expected_subject_hash)),
                    ("base_version_id", text(&task.base_version_id)),
                    ("comment", text(comment)),
                    ("event_id", text(&approval_event.id)),
                    ("created_at", text(&created)),
                ],
            },
            RowWrite::Update {
                table: "doc_task_meta",
                key: "task_id",
                key_value: text(&task.id),
                values: vec![("last_approval_id", text(&approval_id))],
            },
        ];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![approval_event, transition_event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Commits the stale path of T3 guard 4: the plan is superseded and the task returns to
    /// analysis as the system principal, in one transaction.
    async fn supersede_stale(&self, task: &TaskRow, plan: &PlanRecord) -> Result<()> {
        let from = TaskState::Approval;
        let to = TaskState::Analysis;
        lookup_transition(from, to, Principal::System)?;
        let events = vec![
            make_event(
                task,
                Fact {
                    subject_kind: "plan",
                    subject_id: &plan.id,
                    kind: "dt.plan.superseded",
                    from: "",
                    to: "",
                },
                json!({"plan_no": plan.plan_no, "reason": "stale_plan"}),
                "system",
                ActorKind::System,
                "",
            ),
            make_event(
                task,
                Fact {
                    subject_kind: "task",
                    subject_id: &task.id,
                    kind: "dt.task.transitioned",
                    from: from.as_str(),
                    to: to.as_str(),
                },
                json!({"reason": "stale_plan"}),
                "system",
                ActorKind::System,
                "",
            ),
        ];
        let writes = vec![RowWrite::Update {
            table: "doc_task_plans",
            key: "id",
            key_value: text(&plan.id),
            values: vec![("status", text(PlanStatus::Superseded.as_str()))],
        }];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.ds
            .commit_step(Some(&change), &writes, &events)
            .await
            .map(|_| ())
    }

    /// Rejects the latest plan (T4). The reason is required.
    pub async fn reject(&self, handle: &str, reason: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::Reject)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Approval], Principal::Human)?;
        let to = TaskState::Analysis;
        let transition = lookup_transition(from, to, Principal::Human)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let plan = latest_plan(&snap.plans, &task.id)
            .cloned()
            .ok_or_else(not_found)?;
        let approval_id = new_id();
        let created = now();
        let approval_event = self.event(
            &task,
            Fact {
                subject_kind: "approval",
                subject_id: &approval_id,
                kind: "dt.approval.recorded",
                from: "",
                to: "",
            },
            json!({
                "approval_id": approval_id,
                "kind": ApprovalKind::Plan.as_str(),
                "decision": ApprovalDecision::Rejected.as_str(),
                "subject_hash": plan.plan_hash,
                "comment": reason,
            }),
        );
        let transition_event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let writes = vec![
            RowWrite::Update {
                table: "doc_task_plans",
                key: "id",
                key_value: text(&plan.id),
                values: vec![
                    ("status", text(PlanStatus::Rejected.as_str())),
                    ("decided_at", text(&created)),
                ],
            },
            RowWrite::Insert {
                table: "doc_task_approvals",
                values: vec![
                    ("id", text(&approval_id)),
                    ("task_id", text(&task.id)),
                    ("kind", text(ApprovalKind::Plan.as_str())),
                    ("decision", text(ApprovalDecision::Rejected.as_str())),
                    ("approver", text(&self.caller.name)),
                    ("approver_kind", text(ActorKind::Human.as_str())),
                    ("plan_id", text(&plan.id)),
                    ("subject_hash", text(&plan.plan_hash)),
                    ("base_version_id", text(&task.base_version_id)),
                    ("comment", text(reason)),
                    ("event_id", text(&approval_event.id)),
                    ("created_at", text(&created)),
                ],
            },
            RowWrite::Update {
                table: "doc_task_meta",
                key: "task_id",
                key_value: text(&task.id),
                values: vec![("last_approval_id", text(&approval_id))],
            },
        ];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![approval_event, transition_event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Reports the work done (T5). The summary is recorded as a note of the agent or the owner;
    /// the note's event is the transition itself.
    pub async fn report_work(&self, handle: &str, summary: &str) -> Result<Step> {
        self.require_person()?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Working], self.caller.principal)?;
        let to = TaskState::Verification;
        let transition = lookup_transition(from, to, self.caller.principal)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.summary = Some(summary);
        check_guards(transition, &guard)?;
        let row_id = new_id();
        let created = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({}),
        );
        let writes = vec![RowWrite::Insert {
            table: "doc_task_evidence",
            values: vec![
                ("id", text(&row_id)),
                ("task_id", text(&task.id)),
                ("kind", text(EvidenceKind::Note.as_str())),
                ("result", text(EvidenceResult::Info.as_str())),
                ("source", text(EvidenceSource::Reported.as_str())),
                ("summary", text(summary)),
                ("actor", text(&self.caller.name)),
                ("actor_kind", text(self.actor_kind().as_str())),
                ("session", text(&self.caller.session)),
                ("event_id", text(&event.id)),
                ("created_at", text(&created)),
            ],
        }];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Accepts the work (T6). The evidence rule of design 4.5 must hold, or a written waiver must
    /// be given. `expected_subject_hash` is the evidence hash shown to the owner.
    pub async fn close(
        &self,
        handle: &str,
        expected_subject_hash: &str,
        accept_agent: &[String],
        waive: Option<&str>,
    ) -> Result<Step> {
        authorize(self.caller.principal, Operation::Close)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Verification], Principal::Human)?;
        let to = TaskState::Done;
        let transition = lookup_transition(from, to, Principal::Human)?;
        let rows: Vec<EvidenceRecord> = snap
            .evidence
            .iter()
            .filter(|row| row.task_id == task.id)
            .cloned()
            .collect();
        for id in accept_agent {
            let accepted = rows.iter().any(|row| {
                row.id == *id
                    && is_acceptable_kind(row.kind)
                    && row.result == EvidenceResult::Pass
                    && row.actor_kind == ActorKind::Agent
            });
            if !accepted {
                return Err(invalid(
                    "接受的证据不是本任务的 Agent 通过证据",
                    "使用 doc evidence list 中的证据序号",
                ));
            }
        }
        if let Some(reason) = waive {
            required_text("waiver", Some(reason), MAX_REASON_CHARS)?;
        }
        let latest = latest_plan(&snap.plans, &task.id).cloned();
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        let subject = evidence_hash(
            latest.as_ref().map_or("", |plan| plan.plan_hash.as_str()),
            &ids,
        );
        let mut guard = self.base_guard(&snap, &task);
        guard.evidence_accepted = evidence_accepts(&rows, accept_agent) || waive.is_some();
        guard.subject_hash_matches = subject == expected_subject_hash;
        check_guards(transition, &guard)?;
        let approval_id = new_id();
        let created = now();
        let approval_event = self.event(
            &task,
            Fact {
                subject_kind: "approval",
                subject_id: &approval_id,
                kind: "dt.approval.recorded",
                from: "",
                to: "",
            },
            json!({
                "approval_id": approval_id,
                "kind": ApprovalKind::Result.as_str(),
                "decision": ApprovalDecision::Approved.as_str(),
                "subject_hash": expected_subject_hash,
                "accepted_agent_evidence": accept_agent,
                "evidence_waived": waive.is_some(),
            }),
        );
        let transition_event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"evidence_waived": waive.is_some()}),
        );
        let plan_id = latest
            .as_ref()
            .map_or(String::new(), |plan| plan.id.clone());
        let mut writes = vec![RowWrite::Insert {
            table: "doc_task_approvals",
            values: vec![
                ("id", text(&approval_id)),
                ("task_id", text(&task.id)),
                ("kind", text(ApprovalKind::Result.as_str())),
                ("decision", text(ApprovalDecision::Approved.as_str())),
                ("approver", text(&self.caller.name)),
                ("approver_kind", text(ActorKind::Human.as_str())),
                ("plan_id", text(&plan_id)),
                ("subject_hash", text(expected_subject_hash)),
                ("base_version_id", text(&task.base_version_id)),
                ("comment", text(waive.unwrap_or(""))),
                ("event_id", text(&approval_event.id)),
                ("created_at", text(&created)),
            ],
        }];
        if let Some(reason) = waive {
            writes.push(RowWrite::Insert {
                table: "doc_task_evidence",
                values: vec![
                    ("id", text(&new_id())),
                    ("task_id", text(&task.id)),
                    ("kind", text(EvidenceKind::Waiver.as_str())),
                    ("result", text(EvidenceResult::Info.as_str())),
                    ("source", text(EvidenceSource::Reported.as_str())),
                    ("summary", text(reason)),
                    ("actor", text(&self.caller.name)),
                    ("actor_kind", text(ActorKind::Human.as_str())),
                    ("session", text("")),
                    ("event_id", text(&approval_event.id)),
                    ("created_at", text(&created)),
                ],
            });
        }
        writes.push(RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: text(&task.id),
            values: vec![
                ("closed_at", text(&created)),
                ("last_approval_id", text(&approval_id)),
            ],
        });
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![approval_event, transition_event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Returns the work to the implementation (T7). The owner always may; the agent only when the
    /// latest evidence failed. The owner's return writes an approval row.
    pub async fn return_to_work(&self, handle: &str, reason: &str) -> Result<Step> {
        self.require_person()?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Verification], self.caller.principal)?;
        let to = TaskState::Working;
        let transition = lookup_transition(from, to, self.caller.principal)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let mut writes = Vec::new();
        if self.caller.principal == Principal::Human {
            let latest = latest_plan(&snap.plans, &task.id).cloned();
            writes.push(RowWrite::Insert {
                table: "doc_task_approvals",
                values: vec![
                    ("id", text(&new_id())),
                    ("task_id", text(&task.id)),
                    ("kind", text(ApprovalKind::Result.as_str())),
                    ("decision", text(ApprovalDecision::Rejected.as_str())),
                    ("approver", text(&self.caller.name)),
                    ("approver_kind", text(ActorKind::Human.as_str())),
                    (
                        "plan_id",
                        text(latest.as_ref().map_or("", |plan| plan.id.as_str())),
                    ),
                    (
                        "subject_hash",
                        text(latest.as_ref().map_or("", |plan| plan.plan_hash.as_str())),
                    ),
                    ("base_version_id", text(&task.base_version_id)),
                    ("comment", text(reason)),
                    ("event_id", text(&event.id)),
                    ("created_at", text(&now())),
                ],
            });
        }
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Redoes the plan of a working task (T8). Approved plans that were not applied are superseded.
    pub async fn rework(&self, handle: &str, reason: &str) -> Result<Step> {
        self.require_person()?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(&task, &[TaskState::Working], self.caller.principal)?;
        let to = TaskState::Analysis;
        let transition = lookup_transition(from, to, self.caller.principal)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let writes = vec![RowWrite::UpdateWhere {
            table: "doc_task_plans",
            filters: vec![
                ("task_id", text(&task.id)),
                ("status", text(PlanStatus::Approved.as_str())),
            ],
            values: vec![("status", text(PlanStatus::Superseded.as_str()))],
        }];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Cancels a task that is not done (T9). Submitted and approved plans are superseded.
    pub async fn cancel(&self, handle: &str, reason: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::Cancel)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(
            &task,
            &[
                TaskState::Open,
                TaskState::Analysis,
                TaskState::Approval,
                TaskState::Working,
                TaskState::Verification,
            ],
            Principal::Human,
        )?;
        let to = TaskState::Cancelled;
        let transition = lookup_transition(from, to, Principal::Human)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let stamp = now();
        let writes = vec![
            RowWrite::UpdateWhere {
                table: "doc_task_plans",
                filters: vec![
                    ("task_id", text(&task.id)),
                    ("status", text(PlanStatus::Submitted.as_str())),
                ],
                values: vec![("status", text(PlanStatus::Superseded.as_str()))],
            },
            RowWrite::UpdateWhere {
                table: "doc_task_plans",
                filters: vec![
                    ("task_id", text(&task.id)),
                    ("status", text(PlanStatus::Approved.as_str())),
                ],
                values: vec![("status", text(PlanStatus::Superseded.as_str()))],
            },
            RowWrite::Update {
                table: "doc_task_meta",
                key: "task_id",
                key_value: text(&task.id),
                values: vec![("cancel_reason", text(reason)), ("closed_at", text(&stamp))],
            },
        ];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Reopens a done or cancelled task (T10).
    pub async fn reopen(&self, handle: &str, reason: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::Reopen)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let from = expect_from(
            &task,
            &[TaskState::Done, TaskState::Cancelled],
            Principal::Human,
        )?;
        let to = TaskState::Open;
        let transition = lookup_transition(from, to, Principal::Human)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.reopened",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let writes = vec![RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: text(&task.id),
            values: vec![("closed_at", Value::Null)],
        }];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![event],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Adopts an agent proposal (P1). Human only.
    pub async fn adopt(&self, handle: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::AdoptProposal)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        if task.proposal_state != ProposalState::Proposed.as_str() {
            return Err(fail(
                ErrorCode::TransitionDenied,
                "该任务不是待采纳的提案",
                "只能采纳 proposed 状态的任务",
            ));
        }
        let stamp = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.proposal.accepted",
                from: "",
                to: "",
            },
            json!({"decided_by": self.caller.name}),
        );
        let writes = vec![RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: text(&task.id),
            values: vec![
                ("proposal_state", text(ProposalState::Accepted.as_str())),
                ("proposal_decided_by", text(&self.caller.name)),
                ("proposal_decided_at", text(&stamp)),
            ],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// Dismisses an agent proposal (P2): the task is cancelled with the reason as feedback.
    pub async fn dismiss(&self, handle: &str, reason: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::DismissProposal)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        if task.proposal_state != ProposalState::Proposed.as_str() {
            return Err(fail(
                ErrorCode::TransitionDenied,
                "该任务不是待忽略的提案",
                "只能忽略 proposed 状态的任务",
            ));
        }
        let from = expect_from(&task, &[TaskState::Open], Principal::Human)?;
        let to = TaskState::Cancelled;
        let transition = lookup_transition(from, to, Principal::Human)?;
        let mut guard = self.base_guard(&snap, &task);
        guard.reason = Some(reason);
        check_guards(transition, &guard)?;
        let stamp = now();
        let dismissed = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.proposal.dismissed",
                from: "",
                to: "",
            },
            json!({"reason": reason}),
        );
        let moved = self.event(
            &task,
            Fact {
                subject_kind: "task",
                subject_id: &task.id,
                kind: "dt.task.transitioned",
                from: from.as_str(),
                to: to.as_str(),
            },
            json!({"reason": reason}),
        );
        let writes = vec![RowWrite::Update {
            table: "doc_task_meta",
            key: "task_id",
            key_value: text(&task.id),
            values: vec![
                ("proposal_state", text(ProposalState::Dismissed.as_str())),
                ("proposal_decided_by", text(&self.caller.name)),
                ("proposal_decided_at", text(&stamp)),
                ("cancel_reason", text(reason)),
                ("closed_at", text(&stamp)),
            ],
        }];
        let change = StatusChange {
            task_id: task.id.clone(),
            from,
            to,
        };
        self.apply(
            &task,
            Some(change),
            writes,
            vec![dismissed, moved],
            (from.as_str(), to.as_str()),
        )
        .await
    }

    /// Adds a dependency: the task waits for `depends_on`. The agent may do so only for a task
    /// its own session created, up to `MAX_AGENT_DEPENDENCIES` per task.
    pub async fn add_dependency(&self, handle: &str, depends_on: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::AddDependency)?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        let target = self.locate(&snap, depends_on).await?.ok_or_else(|| {
            fail(
                ErrorCode::DepDangling,
                "先行任务不存在或不可见",
                "确认任务句柄",
            )
        })?;
        if target.id == task.id {
            return Err(invalid("任务不能依赖自身", "选择另一个先行任务"));
        }
        if snap
            .deps
            .iter()
            .any(|(from, to)| *from == task.id && *to == target.id)
        {
            return Err(invalid("依赖已存在", "无需重复添加"));
        }
        if self.is_agent() {
            let own = task.created_by_kind == ActorKind::Agent.as_str()
                && task.session == self.caller.session;
            if !own {
                return Err(human_only());
            }
            let declared = snap
                .deps
                .iter()
                .filter(|(from, _)| *from == task.id)
                .count();
            if declared >= MAX_AGENT_DEPENDENCIES {
                return Err(fail(
                    ErrorCode::Quota,
                    "依赖数量已达上限",
                    "减少先行任务后重试",
                ));
            }
        }
        let mut edges = snap.deps.clone();
        edges.push((task.id.clone(), target.id.clone()));
        schedule_batches(&sched_tasks(&snap, &edges))?;
        let stamp = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "dep",
                subject_id: &target.id,
                kind: "dt.dep.added",
                from: "",
                to: "",
            },
            json!({"depends_on": handle_of(target.task_no)}),
        );
        let writes = vec![RowWrite::Insert {
            table: "doc_task_deps",
            values: vec![
                ("task_id", text(&task.id)),
                ("depends_on", text(&target.id)),
                ("created_by", text(&self.caller.name)),
                ("created_at", text(&stamp)),
            ],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// Removes a dependency. Human only.
    pub async fn remove_dependency(&self, handle: &str, depends_on: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::RemoveDependency)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        let target = self.find_task(&snap, depends_on).await?;
        if !snap
            .deps
            .iter()
            .any(|(from, to)| *from == task.id && *to == target.id)
        {
            return Err(not_found());
        }
        let event = self.event(
            &task,
            Fact {
                subject_kind: "dep",
                subject_id: &target.id,
                kind: "dt.dep.removed",
                from: "",
                to: "",
            },
            json!({"depends_on": handle_of(target.task_no)}),
        );
        let writes = vec![RowWrite::Delete {
            table: "doc_task_deps",
            filters: vec![
                ("task_id", text(&task.id)),
                ("depends_on", text(&target.id)),
            ],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// Adds a link from the task to a design, code, test, document, write target or other object.
    pub async fn add_link(
        &self,
        handle: &str,
        link_kind: &str,
        target_kind: &str,
        target: &str,
        section: Option<&str>,
        label: &str,
    ) -> Result<Step> {
        authorize(self.caller.principal, Operation::AddLink)?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        let kind = LinkKind::parse(link_kind)
            .ok_or_else(|| invalid("未知的关联种类", "使用合法的 link_kind"))?;
        let target_type = TargetKind::parse(target_kind)
            .ok_or_else(|| invalid("未知的目标种类", "使用合法的 target_kind"))?;
        validate_text("label", label, 0, MAX_LINK_LABEL_CHARS)?;
        let (target_file_id, target_ref, target_section) = match target_type {
            TargetKind::File => {
                let file = self.resolve_file(target).await?;
                (file.id.clone(), file.relative_path.clone(), String::new())
            }
            TargetKind::FileSection => {
                let file = self.resolve_file(target).await?;
                let selector_text = section
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| invalid("章节关联需要章节选择器", "给出章节选择器"))?;
                let selector = parse_section_selector(Some(selector_text))?;
                let version = self
                    .engine
                    .store
                    .latest_version(&file.id)
                    .await?
                    .ok_or_else(|| {
                        fail(ErrorCode::NotFound, "未找到文件版本", "确认文件已被记录")
                    })?;
                let bytes = self.engine.load_version(&version.id, None).await?;
                let extension = Path::new(&file.relative_path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_string();
                let outline = doctask_outline::outline(&bytes, &extension);
                doctask_outline::resolve(&outline, &selector)?;
                (
                    file.id.clone(),
                    file.relative_path.clone(),
                    selector_text.to_string(),
                )
            }
            TargetKind::GitCommit => {
                let hex = target.len() >= 7
                    && target.len() <= 64
                    && target.chars().all(|c| c.is_ascii_hexdigit());
                if !hex {
                    return Err(invalid("提交哈希须为 7 到 64 位十六进制", "检查提交哈希"));
                }
                (String::new(), target.to_string(), String::new())
            }
            TargetKind::Url => {
                let scheme = target.starts_with("http://") || target.starts_with("https://");
                if !scheme || target.chars().count() > 300 {
                    return Err(invalid(
                        "地址须为 http 或 https，且至多 300 字符",
                        "检查地址",
                    ));
                }
                (String::new(), target.to_string(), String::new())
            }
            TargetKind::Task => {
                let other = self.locate(&snap, target).await?.ok_or_else(not_found)?;
                (String::new(), other.id.clone(), String::new())
            }
        };
        let link_id = new_id();
        let stamp = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "link",
                subject_id: &link_id,
                kind: "dt.link.added",
                from: "",
                to: "",
            },
            json!({
                "link_kind": kind.as_str(),
                "target_kind": target_type.as_str(),
                "target_ref": target_ref,
                "target_section": target_section,
            }),
        );
        let writes = vec![RowWrite::Insert {
            table: "doc_task_links",
            values: vec![
                ("id", text(&link_id)),
                ("task_id", text(&task.id)),
                ("link_kind", text(kind.as_str())),
                ("target_kind", text(target_type.as_str())),
                ("target_file_id", text(&target_file_id)),
                ("target_section", text(&target_section)),
                ("target_ref", text(&target_ref)),
                ("label", text(label)),
                ("created_by", text(&self.caller.name)),
                ("created_by_kind", text(self.actor_kind().as_str())),
                ("session", text(&self.caller.session)),
                ("created_at", text(&stamp)),
                ("removed_at", Value::Null),
            ],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// Removes a link. Human only; the row is kept with `removed_at` set.
    pub async fn remove_link(&self, handle: &str, link_id: &str) -> Result<Step> {
        authorize(self.caller.principal, Operation::RemoveLink)?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        let live = snap
            .links
            .iter()
            .any(|link| link.id == link_id && link.task_id == task.id && link.removed_at.is_none());
        if !live {
            return Err(not_found());
        }
        let stamp = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "link",
                subject_id: link_id,
                kind: "dt.link.removed",
                from: "",
                to: "",
            },
            json!({}),
        );
        let writes = vec![RowWrite::Update {
            table: "doc_task_links",
            key: "id",
            key_value: text(link_id),
            values: vec![("removed_at", text(&stamp))],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// Adds a piece of evidence to a working or verifying task. A version row takes its hash from
    /// the ledger and must lie above the baseline.
    pub async fn add_evidence(
        &self,
        handle: &str,
        kind: &str,
        result: &str,
        summary: &str,
        version_id: Option<&str>,
    ) -> Result<Step> {
        self.require_person()?;
        self.check_agent_rate().await?;
        let snap = self.snapshot().await?;
        let task = self.find_task(&snap, handle).await?;
        require_adopted(&task)?;
        let evidence_kind = EvidenceKind::parse(kind)
            .filter(|value| {
                matches!(
                    value,
                    EvidenceKind::TestRun
                        | EvidenceKind::Report
                        | EvidenceKind::Note
                        | EvidenceKind::Version
                )
            })
            .ok_or_else(|| invalid("证据种类不合法", "使用 test_run、report、note 或 version"))?;
        let evidence_result = EvidenceResult::parse(result)
            .ok_or_else(|| invalid("证据结果不合法", "使用 pass、fail 或 info"))?;
        validate_text("summary", summary, 1, MAX_VERIFY_SUMMARY_CHARS)?;
        let state = task_state(&task)?;
        if state != TaskState::Working && state != TaskState::Verification {
            return Err(fail(
                ErrorCode::TransitionDenied,
                "证据只能在 working 或 verification 时添加",
                "先推进任务状态后重试",
            ));
        }
        let (source, version_ref, content_hash) = match evidence_kind {
            EvidenceKind::Version => {
                let id =
                    version_id.ok_or_else(|| invalid("version 证据需要版本 id", "给出版本 id"))?;
                if evidence_result != EvidenceResult::Info {
                    return Err(invalid("version 证据的结果须为 info", "使用 info"));
                }
                let version = self
                    .engine
                    .store
                    .version(id)
                    .await?
                    .filter(|row| row.file_id == task.file_id)
                    .ok_or_else(not_found)?;
                if self.is_agent() && !self.gate.version_allowed(&version).await {
                    return Err(not_found());
                }
                let baseline = self.baseline_number(&task).await?;
                if version.number <= baseline {
                    return Err(invalid("版本号须大于基线版本", "选择基线之后的版本"));
                }
                (EvidenceSource::Ledger, id.to_string(), version.content_hash)
            }
            _ => (EvidenceSource::Reported, String::new(), String::new()),
        };
        let row_id = new_id();
        let stamp = now();
        let event = self.event(
            &task,
            Fact {
                subject_kind: "evidence",
                subject_id: &row_id,
                kind: "dt.evidence.added",
                from: "",
                to: "",
            },
            json!({
                "kind": evidence_kind.as_str(),
                "result": evidence_result.as_str(),
                "source": source.as_str(),
                "version_id": version_ref,
            }),
        );
        let writes = vec![RowWrite::Insert {
            table: "doc_task_evidence",
            values: vec![
                ("id", text(&row_id)),
                ("task_id", text(&task.id)),
                ("kind", text(evidence_kind.as_str())),
                ("result", text(evidence_result.as_str())),
                ("source", text(source.as_str())),
                ("ref", text("")),
                ("version_id", text(&version_ref)),
                ("content_hash", text(&content_hash)),
                ("summary", text(summary)),
                ("actor", text(&self.caller.name)),
                ("actor_kind", text(self.actor_kind().as_str())),
                ("session", text(&self.caller.session)),
                ("event_id", text(&event.id)),
                ("created_at", text(&stamp)),
            ],
        }];
        self.apply(&task, None, writes, vec![event], ("", "")).await
    }

    /// The owner's inbox. Human only.
    pub async fn inbox(&self) -> Result<Inbox> {
        self.require_human()?;
        let snap = self.snapshot().await?;
        let mut inbox = Inbox::default();
        for task in &snap.tasks {
            let brief = TaskBrief {
                handle: handle_of(task.task_no),
                title: task.title.clone(),
                status: task.status.clone(),
                plan_hash: latest_plan(&snap.plans, &task.id)
                    .map_or(String::new(), |plan| plan.plan_hash.clone()),
            };
            if task.status == TaskState::Approval.as_str() {
                inbox.approvals.push(brief.clone());
            }
            if task.status == TaskState::Verification.as_str() {
                inbox.verifications.push(brief.clone());
            }
            if task.proposal_state == ProposalState::Proposed.as_str() {
                inbox.proposals.push(brief);
            }
        }
        Ok(inbox)
    }

    /// The parallel schedule. For the agent the output omits what the agent may not see and counts
    /// the omitted tasks.
    pub async fn schedule(&self) -> Result<ScheduleView> {
        let snap = self.snapshot().await?;
        let full = schedule_batches(&sched_tasks(&snap, &snap.deps))?;
        if !self.is_agent() {
            return Ok(ScheduleView {
                schedule: full,
                hidden: 0,
            });
        }
        let visible: HashSet<String> = self
            .visible_tasks(&snap)
            .await?
            .iter()
            .map(|task| handle_of(task.task_no))
            .collect();
        let hidden_handles: HashSet<String> = snap
            .tasks
            .iter()
            .map(|task| handle_of(task.task_no))
            .filter(|handle| !visible.contains(handle))
            .collect();
        let mentions_hidden = |reason: &str| {
            reason
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .any(|token| hidden_handles.contains(token))
        };
        let mut appearing: HashSet<String> = HashSet::new();
        let batches: Vec<Vec<String>> = full
            .batches
            .iter()
            .map(|batch| {
                batch
                    .iter()
                    .inspect(|handle| {
                        appearing.insert((*handle).clone());
                    })
                    .filter(|handle| visible.contains(handle.as_str()))
                    .cloned()
                    .collect()
            })
            .collect();
        let mut conflicts = Vec::new();
        for conflict in &full.conflicts {
            appearing.extend(conflict.tasks.iter().cloned());
            if conflict
                .tasks
                .iter()
                .any(|handle| hidden_handles.contains(handle))
            {
                continue;
            }
            let allowed = match self.engine.store.file_by_id(&conflict.file).await? {
                Some(file) => self.gate.file_allowed(&file).await,
                None => false,
            };
            if allowed {
                conflicts.push(conflict.clone());
            }
        }
        let mut blocked = Vec::new();
        for entry in &full.blocked {
            appearing.insert(entry.task.clone());
            if hidden_handles.contains(&entry.task) || mentions_hidden(entry.reason.as_str()) {
                continue;
            }
            blocked.push(entry.clone());
        }
        let mut unknown_targets = Vec::new();
        for handle in &full.unknown_targets {
            appearing.insert(handle.clone());
            if !hidden_handles.contains(handle) {
                unknown_targets.push(handle.clone());
            }
        }
        let hidden = appearing
            .iter()
            .filter(|handle| hidden_handles.contains(handle.as_str()))
            .count() as u64;
        Ok(ScheduleView {
            schedule: Schedule {
                batches,
                conflicts,
                blocked,
                unknown_targets,
            },
            hidden,
        })
    }

    /// Checks the governance chain and the task rows. Human only. With a handle, the findings of
    /// that task are returned with the chain-level findings.
    pub async fn verify(&self, handle: Option<&str>) -> Result<VerifyReport> {
        authorize(self.caller.principal, Operation::Verify)?;
        let snap = self.snapshot().await?;
        let events = self.ds.events().await?;
        let names: HashMap<&str, String> = snap
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), handle_of(task.task_no)))
            .collect();
        let mut findings: Vec<Finding> = Vec::new();
        if let Some(seq) = self.ds.verify_chain().await? {
            findings.push(finding(
                "CHAIN_BROKEN",
                true,
                None,
                Some(seq),
                "chain link broken",
            ));
        }
        let mut previous: Option<i64> = None;
        for event in &events {
            if let Some(last) = previous {
                if event.seq != last + 1 {
                    findings.push(finding(
                        "SEQ_GAP",
                        false,
                        None,
                        Some(event.seq),
                        "gap in seq",
                    ));
                }
            }
            previous = Some(event.seq);
        }
        for drift in self.ds.status_drift().await? {
            findings.push(finding(
                "STATUS_DRIFT",
                true,
                names.get(drift.task_id.as_str()).cloned(),
                None,
                &format!(
                    "stored status {} differs from replayed {}",
                    drift.status,
                    drift.replayed.as_deref().unwrap_or("none")
                ),
            ));
        }
        for (code, task_id, seq, detail) in replay_status(&events) {
            findings.push(finding(
                code,
                true,
                names.get(task_id.as_str()).cloned(),
                Some(seq),
                &detail,
            ));
        }
        for event in &events {
            if HUMAN_ONLY_KINDS.contains(&event.kind.as_str())
                && event.actor_kind != ActorKind::Human.as_str()
                && event.actor_kind != ActorKind::Unknown.as_str()
            {
                findings.push(finding(
                    "HUMAN_ONLY_FORGED",
                    true,
                    names.get(event.task_id.as_str()).cloned(),
                    Some(event.seq),
                    &event.kind,
                ));
            }
        }
        let chain_ids: HashSet<&str> = events.iter().map(|event| event.id.as_str()).collect();
        let referenced: HashSet<&str> = snap
            .approvals
            .iter()
            .map(|approval| approval.event_id.as_str())
            .collect();
        for approval in &snap.approvals {
            if !chain_ids.contains(approval.event_id.as_str()) {
                findings.push(finding(
                    "APPROVAL_ORPHAN",
                    true,
                    names.get(approval.task_id.as_str()).cloned(),
                    None,
                    "approval row without its event",
                ));
            }
        }
        for event in events
            .iter()
            .filter(|event| event.kind == "dt.approval.recorded")
        {
            if !referenced.contains(event.id.as_str()) {
                findings.push(finding(
                    "APPROVAL_ORPHAN",
                    true,
                    names.get(event.task_id.as_str()).cloned(),
                    Some(event.seq),
                    "approval event without its row",
                ));
            }
        }
        let started: HashSet<&str> = events
            .iter()
            .filter(|event| event.kind == "dt.commit.started")
            .map(|event| event.subject_id.as_str())
            .collect();
        for plan in snap
            .plans
            .iter()
            .filter(|plan| plan.status == PlanStatus::Applying)
        {
            if !started.contains(plan.id.as_str()) {
                findings.push(finding(
                    "PLAN_INCONSISTENT",
                    true,
                    names.get(plan.task_id.as_str()).cloned(),
                    None,
                    "applying plan without a started commit",
                ));
            }
        }
        let cyclic = schedule_batches(&sched_tasks(&snap, &snap.deps))
            .err()
            .is_some_and(|err| err.code == ErrorCode::DepCycle);
        if cyclic {
            findings.push(finding("DEP_CYCLE", true, None, None, "dependency cycle"));
        }
        for (file_id, version_id, count) in self.ds.duplicate_baselines().await? {
            findings.push(finding(
                "BASELINE_DUPLICATE",
                true,
                None,
                None,
                &format!("file {file_id} version {version_id} has {count} active baselines"),
            ));
        }
        for task in &snap.tasks {
            let legacy_unapproved = task.source_kind == "legacy"
                && [
                    TaskState::Approval,
                    TaskState::Working,
                    TaskState::Verification,
                    TaskState::Done,
                ]
                .iter()
                .any(|state| state.as_str() == task.status)
                && !snap
                    .approvals
                    .iter()
                    .any(|approval| approval.task_id == task.id);
            if legacy_unapproved {
                findings.push(finding(
                    "LEGACY_UNAPPROVED",
                    false,
                    Some(handle_of(task.task_no)),
                    None,
                    "legacy task without an approval row",
                ));
            }
            let unsettled = matches!(
                anchor_state(task),
                AnchorState::NeedsReview
                    | AnchorState::Relocated
                    | AnchorState::Ambiguous
                    | AnchorState::Orphaned
            );
            if unsettled && !anchor_reviewed(&snap, &task.anchor_id) {
                findings.push(finding(
                    "ANCHOR_UNREVIEWED",
                    false,
                    Some(handle_of(task.task_no)),
                    None,
                    "anchor state not reviewed",
                ));
            }
        }
        if let Some(handle) = handle {
            let wanted = self.find_task(&snap, handle).await?.task_no;
            let wanted = handle_of(wanted);
            findings.retain(|item| {
                item.task.is_none() || item.task.as_deref() == Some(wanted.as_str())
            });
        }
        let ok = !findings.iter().any(|item| item.level == "error");
        Ok(VerifyReport { ok, findings })
    }
}

/// Builds one governance event with the given actor.
fn make_event(
    task: &TaskRow,
    fact: Fact<'_>,
    payload: Json,
    actor: &str,
    actor_kind: ActorKind,
    session: &str,
) -> StepEvent {
    StepEvent {
        id: new_id(),
        event: NewEvent {
            task_id: task.id.clone(),
            file_id: task.file_id.clone(),
            subject_kind: fact.subject_kind.to_string(),
            subject_id: fact.subject_id.to_string(),
            kind: fact.kind.to_string(),
            from_status: fact.from.to_string(),
            to_status: fact.to.to_string(),
            actor: actor.to_string(),
            actor_kind,
            session: session.to_string(),
            claimed_session: String::new(),
            payload_json: payload.to_string(),
        },
    }
}

/// Builds one finding; the level is `error` or `warning`.
fn finding(
    code: &'static str,
    error: bool,
    task: Option<String>,
    seq: Option<i64>,
    detail: &str,
) -> Finding {
    Finding {
        code,
        level: if error { "error" } else { "warning" },
        task,
        seq,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        kind: EvidenceKind,
        result: EvidenceResult,
        actor: ActorKind,
        id: &str,
    ) -> EvidenceRecord {
        EvidenceRecord {
            id: id.to_string(),
            task_id: "t".to_string(),
            kind,
            result,
            source: EvidenceSource::Reported,
            actor_kind: actor,
            created_at: String::new(),
        }
    }

    #[test]
    fn tc899_human_pass_qualifies_and_agent_pass_needs_acceptance() {
        let human = vec![row(
            EvidenceKind::Note,
            EvidenceResult::Pass,
            ActorKind::Human,
            "h",
        )];
        assert!(evidence_accepts(&human, &[]));
        let agent = vec![row(
            EvidenceKind::TestRun,
            EvidenceResult::Pass,
            ActorKind::Agent,
            "a",
        )];
        assert!(!evidence_accepts(&agent, &[]));
        assert!(evidence_accepts(&agent, &["a".to_string()]));
    }

    #[test]
    fn tc899_fail_after_pass_blocks_and_waiver_kinds_do_not_qualify() {
        let pass_then_fail = vec![
            row(
                EvidenceKind::Note,
                EvidenceResult::Pass,
                ActorKind::Human,
                "p",
            ),
            row(
                EvidenceKind::Report,
                EvidenceResult::Fail,
                ActorKind::Human,
                "f",
            ),
        ];
        assert!(!evidence_accepts(&pass_then_fail, &[]));
        let fail_then_pass = vec![
            row(
                EvidenceKind::Report,
                EvidenceResult::Fail,
                ActorKind::Human,
                "f",
            ),
            row(
                EvidenceKind::Note,
                EvidenceResult::Pass,
                ActorKind::Human,
                "p",
            ),
        ];
        assert!(evidence_accepts(&fail_then_pass, &[]));
        let waiver = vec![row(
            EvidenceKind::Waiver,
            EvidenceResult::Pass,
            ActorKind::Human,
            "w",
        )];
        assert!(!evidence_accepts(&waiver, &[]));
    }

    #[test]
    fn tc899_no_plan_and_analysis_only_plans_are_never_stale() {
        assert!(latest_plan(&[], "t").is_none());
        assert!(is_acceptable_kind(EvidenceKind::Report));
        assert!(!is_acceptable_kind(EvidenceKind::Version));
    }
}
