//! Pure model of the document-task module: states, principals, operations,
//! transitions T1 to T11 with guard inputs, stable error codes, record types,
//! handles and section selectors, hash encodings, field limits, text
//! validation, the display sanitiser, the governance chain check and the
//! dependency scheduler. This module performs no I/O and does not depend on
//! the store or the engine.

use crate::nema::{topo_batches, Dependency};
use serde_json::to_vec;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Previous-hash value of the first governance event: 64 `0` characters.
pub const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 4000;
pub const MAX_TASK_RATIONALE_CHARS: usize = 1000;
pub const MAX_PLAN_SUMMARY_CHARS: usize = 500;
pub const MAX_PLAN_RATIONALE_CHARS: usize = 4000;
pub const MAX_PLAN_IMPACT_CHARS: usize = 2000;
pub const MAX_PLAN_OPTIONS: usize = 5;
pub const MAX_PLAN_OPTION_CHARS: usize = 500;
pub const MAX_REASON_CHARS: usize = 500;
pub const MAX_VERIFY_SUMMARY_CHARS: usize = 4000;
pub const MAX_ANNOTATION_CHARS: usize = 4000;
pub const MAX_QUOTE_CHARS: usize = 500;
pub const MAX_LINK_LABEL_CHARS: usize = 120;
pub const MAX_READ_REFS: usize = 10;
pub const MAX_AGENT_DEPENDENCIES: usize = 10;
pub const MAX_NEW_TEXT_BYTES: usize = 262_144;
pub const MAX_TARGET_FILE_BYTES: usize = 1_048_576;
pub const MIN_HEX_PREFIX_CHARS: usize = 8;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Every variant in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The stored string of the variant.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text),+
                }
            }

            /// The variant whose stored string equals `text` exactly.
            pub fn parse(text: &str) -> Option<$name> {
                match text {
                    $($text => Some($name::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

string_enum!(
    /// Task state (design section 4.1). The seven strings are the legacy values.
    TaskState {
        Open = "open",
        Analysis = "analysis",
        Approval = "approval",
        Working = "working",
        Verification = "verification",
        Done = "done",
        Cancelled = "cancelled",
    }
);
string_enum!(
    /// Principal that performs an operation; fixed by the entry point.
    Principal {
        Human = "human",
        Agent = "agent",
        System = "system",
    }
);
string_enum!(
    /// Actor kind stored with a record; `unknown` marks imported legacy data.
    ActorKind {
        Human = "human",
        Agent = "agent",
        System = "system",
        Unknown = "unknown",
    }
);
string_enum!(
    /// Whether a task created by an agent has been adopted by the owner.
    ProposalState {
        Accepted = "accepted",
        Proposed = "proposed",
        Dismissed = "dismissed",
    }
);
string_enum!(
    /// Entry point that created a record.
    SourceKind {
        Cli = "cli",
        Console = "console",
        Hermes = "hermes",
        Legacy = "legacy",
    }
);
string_enum!(
    /// Anchor state of a task or annotation section (design section 3.3).
    AnchorState {
        Valid = "valid",
        NeedsReview = "needs_review",
        Relocated = "relocated",
        Ambiguous = "ambiguous",
        Orphaned = "orphaned",
        Unknown = "unknown",
        FileMissing = "file_missing",
        Unchecked = "unchecked",
    }
);
string_enum!(
    /// Visibility of an annotation or task towards agents.
    Visibility {
        Shared = "shared",
        LocalOnly = "local_only",
    }
);
string_enum!(
    /// Plan status (design section 3.3).
    PlanStatus {
        Submitted = "submitted",
        Approved = "approved",
        Applying = "applying",
        Applied = "applied",
        Conflicted = "conflicted",
        Rejected = "rejected",
        Superseded = "superseded",
    }
);
string_enum!(
    /// Edit kind of a plan; the empty string is an analysis-only plan.
    EditKind {
        AnalysisOnly = "",
        ReplaceSection = "replace_section",
        InsertAfterSection = "insert_after_section",
    }
);
string_enum!(
    /// Plan risk.
    Risk {
        Low = "low",
        Medium = "medium",
        High = "high",
    }
);
string_enum!(
    /// Approval record kind.
    ApprovalKind {
        Plan = "plan",
        Result = "result",
    }
);
string_enum!(
    /// Approval record decision.
    ApprovalDecision {
        Approved = "approved",
        Rejected = "rejected",
    }
);
string_enum!(
    /// Evidence kind.
    EvidenceKind {
        TestRun = "test_run",
        Report = "report",
        Note = "note",
        Version = "version",
        Commit = "commit",
        AgentChange = "agent_change",
        Waiver = "waiver",
    }
);
string_enum!(
    /// Evidence result.
    EvidenceResult {
        Pass = "pass",
        Fail = "fail",
        Info = "info",
    }
);
string_enum!(
    /// Who reported the evidence: the ledger (verified) or the agent (not verified).
    EvidenceSource {
        Ledger = "ledger",
        Reported = "reported",
    }
);
string_enum!(
    /// Kind of a link from a task to a design, code, test or other target.
    LinkKind {
        Design = "design",
        Detail = "detail",
        Code = "code",
        Ut = "ut",
        It = "it",
        St = "st",
        Doc = "doc",
        Target = "target",
        Other = "other",
    }
);
string_enum!(
    /// Kind of a link target.
    TargetKind {
        File = "file",
        FileSection = "file_section",
        GitCommit = "git_commit",
        Url = "url",
        Task = "task",
    }
);
string_enum!(
    /// Lineage proposal state for a file move.
    LineageState {
        Proposed = "proposed",
        Confirmed = "confirmed",
        Rejected = "rejected",
    }
);

impl From<Principal> for ActorKind {
    fn from(principal: Principal) -> ActorKind {
        match principal {
            Principal::Human => ActorKind::Human,
            Principal::Agent => ActorKind::Agent,
            Principal::System => ActorKind::System,
        }
    }
}

impl AnchorState {
    /// Guard T3-3 and T6-3: the anchor is bound to a baseline and is `valid`,
    /// or is `needs_review`, `relocated` or `unknown` and has been reviewed.
    pub fn is_settled(self, baseline_bound: bool, reviewed: bool) -> bool {
        if !baseline_bound {
            return false;
        }
        match self {
            AnchorState::Valid => true,
            AnchorState::NeedsReview | AnchorState::Relocated | AnchorState::Unknown => reviewed,
            _ => false,
        }
    }
}

/// Transition identifiers of design section 4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    T1,
    T2,
    T3,
    T4,
    T5,
    T6,
    T7,
    T8,
    T9,
    T10,
    T11,
}

impl Transition {
    /// The identifier as written in the design, for example `T3`.
    pub fn code(self) -> &'static str {
        match self {
            Transition::T1 => "T1",
            Transition::T2 => "T2",
            Transition::T3 => "T3",
            Transition::T4 => "T4",
            Transition::T5 => "T5",
            Transition::T6 => "T6",
            Transition::T7 => "T7",
            Transition::T8 => "T8",
            Transition::T9 => "T9",
            Transition::T10 => "T10",
            Transition::T11 => "T11",
        }
    }
}

/// One row of the transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionSpec {
    pub id: Transition,
    pub from: TaskState,
    pub to: TaskState,
    pub actors: &'static [Principal],
    pub event_kind: &'static str,
}

const ACTORS_HUMAN: &[Principal] = &[Principal::Human];
const ACTORS_HUMAN_AGENT: &[Principal] = &[Principal::Human, Principal::Agent];
const ACTORS_SYSTEM: &[Principal] = &[Principal::System];
const EVENT_TRANSITIONED: &str = "dt.task.transitioned";
const EVENT_REOPENED: &str = "dt.task.reopened";

/// Transition table of design section 4.2 (T2 and T3 to T6 carry their
/// additional writes in the service layer; this table fixes who may execute
/// each edge and which event the state change records).
pub const TRANSITIONS: &[TransitionSpec] = &[
    TransitionSpec {
        id: Transition::T1,
        from: TaskState::Open,
        to: TaskState::Analysis,
        actors: ACTORS_HUMAN_AGENT,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T2,
        from: TaskState::Analysis,
        to: TaskState::Approval,
        actors: ACTORS_HUMAN_AGENT,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T3,
        from: TaskState::Approval,
        to: TaskState::Working,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T4,
        from: TaskState::Approval,
        to: TaskState::Analysis,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T5,
        from: TaskState::Working,
        to: TaskState::Verification,
        actors: ACTORS_HUMAN_AGENT,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T6,
        from: TaskState::Verification,
        to: TaskState::Done,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T7,
        from: TaskState::Verification,
        to: TaskState::Working,
        actors: ACTORS_HUMAN_AGENT,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T8,
        from: TaskState::Working,
        to: TaskState::Analysis,
        actors: ACTORS_HUMAN_AGENT,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T9,
        from: TaskState::Open,
        to: TaskState::Cancelled,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T9,
        from: TaskState::Analysis,
        to: TaskState::Cancelled,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T9,
        from: TaskState::Approval,
        to: TaskState::Cancelled,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T9,
        from: TaskState::Working,
        to: TaskState::Cancelled,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T9,
        from: TaskState::Verification,
        to: TaskState::Cancelled,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_TRANSITIONED,
    },
    TransitionSpec {
        id: Transition::T10,
        from: TaskState::Done,
        to: TaskState::Open,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_REOPENED,
    },
    TransitionSpec {
        id: Transition::T10,
        from: TaskState::Cancelled,
        to: TaskState::Open,
        actors: ACTORS_HUMAN,
        event_kind: EVENT_REOPENED,
    },
    TransitionSpec {
        id: Transition::T11,
        from: TaskState::Approval,
        to: TaskState::Analysis,
        actors: ACTORS_SYSTEM,
        event_kind: EVENT_TRANSITIONED,
    },
];

fn is_human_only(actors: &[Principal]) -> bool {
    actors.len() == 1 && actors[0] == Principal::Human
}

/// Resolves the transition for a move from `from` to `to` by `principal`.
///
/// No row for the pair gives `E_TRANSITION_DENIED`. A row that lists the
/// principal gives its transition. When the pair has a human-only row and the
/// principal is not human, the result is `E_HUMAN_ONLY`. Otherwise the result
/// is `E_TRANSITION_DENIED`, with the transitions the principal may take from
/// `from` in the detail.
pub fn lookup_transition(
    from: TaskState,
    to: TaskState,
    principal: Principal,
) -> Result<Transition, DocError> {
    let rows: Vec<&TransitionSpec> = TRANSITIONS
        .iter()
        .filter(|r| r.from == from && r.to == to)
        .collect();
    if rows.is_empty() {
        return Err(transition_denied(from, principal));
    }
    if let Some(row) = rows.iter().find(|r| r.actors.contains(&principal)) {
        return Ok(row.id);
    }
    if rows.iter().any(|r| is_human_only(r.actors)) && principal != Principal::Human {
        return Err(human_only(&format!(
            "transition {} -> {} by {}",
            from.as_str(),
            to.as_str(),
            principal.as_str()
        )));
    }
    Err(transition_denied(from, principal))
}

/// The transitions `principal` may take from `from`, with their targets.
pub fn allowed_from(from: TaskState, principal: Principal) -> Vec<(TaskState, Transition)> {
    TRANSITIONS
        .iter()
        .filter(|r| r.from == from && r.actors.contains(&principal))
        .map(|r| (r.to, r.id))
        .collect()
}

fn transition_denied(from: TaskState, principal: Principal) -> DocError {
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
}

/// Operations of design section 4.2 and section 10.5, grouped by the
/// principals allowed to perform them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Approve,
    Reject,
    Close,
    Cancel,
    Reopen,
    AdoptProposal,
    DismissProposal,
    ApplyPlan,
    RecoverPlan,
    BaselineRegister,
    BaselineRevoke,
    EditTaskAttributes,
    ShareAnnotation,
    AnchorAck,
    AnchorRelocate,
    AnchorRebase,
    LineageConfirm,
    LineageReject,
    RemoveDependency,
    RemoveLink,
    ExportFile,
    Verify,
    RestoreVersion,
    ControlHermes,
    Read,
    CreateProposal,
    AddAnnotation,
    SubmitPlan,
    AddLink,
    AddDependency,
    Reconcile,
    Backfill,
    Migrate,
    ExpirePlan,
}

const PRINCIPALS_HUMAN: &[Principal] = &[Principal::Human];
const PRINCIPALS_AGENT: &[Principal] = &[Principal::Human, Principal::Agent];
const PRINCIPALS_SYSTEM: &[Principal] = &[Principal::Human, Principal::System];

impl Operation {
    /// Principals allowed to perform the operation.
    pub fn allowed_principals(self) -> &'static [Principal] {
        match self {
            Operation::Read
            | Operation::CreateProposal
            | Operation::AddAnnotation
            | Operation::SubmitPlan
            | Operation::AddLink
            | Operation::AddDependency => PRINCIPALS_AGENT,
            Operation::Reconcile
            | Operation::Backfill
            | Operation::Migrate
            | Operation::ExpirePlan => PRINCIPALS_SYSTEM,
            _ => PRINCIPALS_HUMAN,
        }
    }
}

/// Checks that `principal` may perform `op`. A principal outside the allowed
/// list receives `E_HUMAN_ONLY`, whatever the operation.
pub fn authorize(principal: Principal, op: Operation) -> Result<(), DocError> {
    if op.allowed_principals().contains(&principal) {
        return Ok(());
    }
    Err(human_only(&format!(
        "operation {:?} by {}: allowed {:?}",
        op,
        principal.as_str(),
        op.allowed_principals()
    )))
}

fn human_only(context: &str) -> DocError {
    DocError::new(
        ErrorCode::HumanOnly,
        "该操作只允许人工主体",
        "由所有者在控制台或一次性命令中执行",
    )
    .with_detail(context.to_string())
}

/// Stable error codes shared by the command line, the console and the MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidArg,
    NotFound,
    Ambiguous,
    SectionNotFound,
    NoBaseline,
    FileMissing,
    TransitionDenied,
    GuardFailed,
    HumanOnly,
    NotInteractive,
    PolicyDenied,
    StalePlan,
    StaleSubject,
    Conflict,
    FileChanged,
    DepCycle,
    DepDangling,
    AnchorBroken,
    UnsupportedFile,
    PlanTooLarge,
    Quota,
    RateLimit,
    StoreBusy,
    SchemaOutdated,
    MigrationBackup,
    Integrity,
    Internal,
}

impl ErrorCode {
    /// Every code in the order of design section 14.1.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::InvalidArg,
        ErrorCode::NotFound,
        ErrorCode::Ambiguous,
        ErrorCode::SectionNotFound,
        ErrorCode::NoBaseline,
        ErrorCode::FileMissing,
        ErrorCode::TransitionDenied,
        ErrorCode::GuardFailed,
        ErrorCode::HumanOnly,
        ErrorCode::NotInteractive,
        ErrorCode::PolicyDenied,
        ErrorCode::StalePlan,
        ErrorCode::StaleSubject,
        ErrorCode::Conflict,
        ErrorCode::FileChanged,
        ErrorCode::DepCycle,
        ErrorCode::DepDangling,
        ErrorCode::AnchorBroken,
        ErrorCode::UnsupportedFile,
        ErrorCode::PlanTooLarge,
        ErrorCode::Quota,
        ErrorCode::RateLimit,
        ErrorCode::StoreBusy,
        ErrorCode::SchemaOutdated,
        ErrorCode::MigrationBackup,
        ErrorCode::Integrity,
        ErrorCode::Internal,
    ];

    /// The code as written in messages and JSON output, for example `E_STALE_PLAN`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArg => "E_INVALID_ARG",
            ErrorCode::NotFound => "E_NOT_FOUND",
            ErrorCode::Ambiguous => "E_AMBIGUOUS",
            ErrorCode::SectionNotFound => "E_SECTION_NOT_FOUND",
            ErrorCode::NoBaseline => "E_NO_BASELINE",
            ErrorCode::FileMissing => "E_FILE_MISSING",
            ErrorCode::TransitionDenied => "E_TRANSITION_DENIED",
            ErrorCode::GuardFailed => "E_GUARD_FAILED",
            ErrorCode::HumanOnly => "E_HUMAN_ONLY",
            ErrorCode::NotInteractive => "E_NOT_INTERACTIVE",
            ErrorCode::PolicyDenied => "E_POLICY_DENIED",
            ErrorCode::StalePlan => "E_STALE_PLAN",
            ErrorCode::StaleSubject => "E_STALE_SUBJECT",
            ErrorCode::Conflict => "E_CONFLICT",
            ErrorCode::FileChanged => "E_FILE_CHANGED",
            ErrorCode::DepCycle => "E_DEP_CYCLE",
            ErrorCode::DepDangling => "E_DEP_DANGLING",
            ErrorCode::AnchorBroken => "E_ANCHOR_BROKEN",
            ErrorCode::UnsupportedFile => "E_UNSUPPORTED_FILE",
            ErrorCode::PlanTooLarge => "E_PLAN_TOO_LARGE",
            ErrorCode::Quota => "E_QUOTA",
            ErrorCode::RateLimit => "E_RATE_LIMIT",
            ErrorCode::StoreBusy => "E_STORE_BUSY",
            ErrorCode::SchemaOutdated => "E_SCHEMA_OUTDATED",
            ErrorCode::MigrationBackup => "E_MIGRATION_BACKUP",
            ErrorCode::Integrity => "E_INTEGRITY",
            ErrorCode::Internal => "E_INTERNAL",
        }
    }

    /// The process exit code of design section 9.3 and section 14.1.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorCode::NotFound | ErrorCode::Ambiguous | ErrorCode::SectionNotFound => 3,
            ErrorCode::HumanOnly | ErrorCode::NotInteractive => 5,
            ErrorCode::StalePlan
            | ErrorCode::StaleSubject
            | ErrorCode::Conflict
            | ErrorCode::FileChanged => 6,
            ErrorCode::StoreBusy => 7,
            ErrorCode::PolicyDenied => 8,
            ErrorCode::Integrity => 9,
            ErrorCode::Internal => 1,
            _ => 4,
        }
    }
}

/// An error with a stable code and Chinese text. The render module supplies the
/// English text from the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocError {
    pub code: ErrorCode,
    pub message_zh: String,
    pub hint_zh: String,
    pub detail: String,
}

impl DocError {
    /// Builds an error with an empty detail.
    pub fn new(
        code: ErrorCode,
        message_zh: impl Into<String>,
        hint_zh: impl Into<String>,
    ) -> DocError {
        DocError {
            code,
            message_zh: message_zh.into(),
            hint_zh: hint_zh.into(),
            detail: String::new(),
        }
    }

    /// Replaces the detail. The detail never carries user text.
    pub fn with_detail(mut self, detail: impl Into<String>) -> DocError {
        self.detail = detail.into();
        self
    }
}

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message_zh)
    }
}

impl std::error::Error for DocError {}

/// Facts that the guards of a transition read. `GuardInput::new` gives the
/// state in which every guard holds; a test or caller changes one fact at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardInput<'a> {
    pub principal: Principal,
    pub proposal_accepted: bool,
    pub anchor: AnchorState,
    pub baseline_bound: bool,
    pub anchor_reviewed: bool,
    pub summary: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub plan_applied_or_no_edit: bool,
    pub latest_evidence_failed: bool,
    pub has_applying_plan: bool,
    pub latest_plan_submitted: bool,
    pub subject_hash_matches: bool,
    pub dependencies_done: bool,
    pub plan_stale: bool,
    pub evidence_accepted: bool,
}

impl<'a> GuardInput<'a> {
    /// The input in which every guard holds, except that T7 by an agent needs
    /// `latest_evidence_failed` and T11 needs `plan_stale`.
    pub fn new(principal: Principal) -> GuardInput<'a> {
        GuardInput {
            principal,
            proposal_accepted: true,
            anchor: AnchorState::Valid,
            baseline_bound: true,
            anchor_reviewed: false,
            summary: Some("done"),
            reason: Some("reason"),
            plan_applied_or_no_edit: true,
            latest_evidence_failed: false,
            has_applying_plan: false,
            latest_plan_submitted: true,
            subject_hash_matches: true,
            dependencies_done: true,
            plan_stale: false,
            evidence_accepted: true,
        }
    }
}

fn guard_failed(message: &str) -> DocError {
    DocError::new(ErrorCode::GuardFailed, message, "检查守卫条件后重试")
}

fn anchor_broken() -> DocError {
    DocError::new(
        ErrorCode::AnchorBroken,
        "锚点状态不允许此操作",
        "先执行 ack、relocate 或 rebase",
    )
}

/// Evaluates the pure guards of transition `t` in the order of design section 4.
/// Principal checks that the table already encodes are repeated where the
/// guard reads the principal (T3 and T7).
pub fn check_guards(t: Transition, g: &GuardInput<'_>) -> Result<(), DocError> {
    match t {
        Transition::T1 => {
            if !g.proposal_accepted {
                return Err(DocError::new(
                    ErrorCode::TransitionDenied,
                    "提案尚未被采纳，不能开始分析",
                    "请所有者先采纳该提案（owner must adopt）",
                ));
            }
            if matches!(
                g.anchor,
                AnchorState::Orphaned | AnchorState::FileMissing | AnchorState::Ambiguous
            ) {
                return Err(anchor_broken());
            }
            Ok(())
        }
        Transition::T2 => Ok(()),
        Transition::T3 => {
            if g.principal != Principal::Human {
                return Err(human_only("guard T3 principal"));
            }
            if !g.latest_plan_submitted {
                return Err(guard_failed("没有待批准的最新方案"));
            }
            if !g.subject_hash_matches {
                return Err(DocError::new(
                    ErrorCode::StaleSubject,
                    "展示后方案已变化，请重新阅读",
                    "重新查看方案后再批准",
                ));
            }
            if !g.dependencies_done {
                return Err(guard_failed("存在未完成的先行任务"));
            }
            if !g.anchor.is_settled(g.baseline_bound, g.anchor_reviewed) {
                return Err(anchor_broken());
            }
            if g.plan_stale {
                return Err(DocError::new(
                    ErrorCode::StalePlan,
                    "方案的目标版本或哈希已过期，任务已回到分析阶段",
                    "重新提交方案",
                ));
            }
            Ok(())
        }
        Transition::T4 | Transition::T10 => required_text("reason", g.reason, MAX_REASON_CHARS),
        Transition::T5 => {
            required_text("summary", g.summary, MAX_VERIFY_SUMMARY_CHARS)?;
            if !g.plan_applied_or_no_edit {
                return Err(guard_failed("方案修改了文件但尚未应用"));
            }
            Ok(())
        }
        Transition::T6 => {
            if !g.evidence_accepted {
                return Err(guard_failed("没有满足验收条件的人工证据"));
            }
            if !g.dependencies_done {
                return Err(guard_failed("存在未完成的先行任务"));
            }
            if !g.anchor.is_settled(g.baseline_bound, g.anchor_reviewed) {
                return Err(anchor_broken());
            }
            if !g.subject_hash_matches {
                return Err(DocError::new(
                    ErrorCode::StaleSubject,
                    "展示后证据已变化，请重新阅读",
                    "重新查看证据后再验收",
                ));
            }
            Ok(())
        }
        Transition::T7 => {
            if g.principal == Principal::Agent && !g.latest_evidence_failed {
                return Err(guard_failed("智能体只能在最新证据失败时退回"));
            }
            required_text("reason", g.reason, MAX_REASON_CHARS)
        }
        Transition::T8 | Transition::T9 => {
            required_text("reason", g.reason, MAX_REASON_CHARS)?;
            if g.has_applying_plan {
                return Err(guard_failed("存在正在应用的方案，请先恢复"));
            }
            Ok(())
        }
        Transition::T11 => {
            if !g.plan_stale {
                return Err(guard_failed("方案未过期，系统不能退回"));
            }
            Ok(())
        }
    }
}

/// A handle of a task or an annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handle {
    Number(i64),
    Uuid(String),
    Prefix(String),
}

/// Parses a task handle (`T-12`, or for humans `12`, a full UUID or a hexadecimal prefix).
pub fn parse_task_handle(input: &str, principal: Principal) -> Result<Handle, DocError> {
    parse_handle(input, principal, "T-")
}

/// Parses an annotation handle (`N-5`, or for humans `5`, a full UUID or a hexadecimal prefix).
pub fn parse_annotation_handle(input: &str, principal: Principal) -> Result<Handle, DocError> {
    parse_handle(input, principal, "N-")
}

fn invalid_handle() -> DocError {
    DocError::new(
        ErrorCode::InvalidArg,
        "无法识别的句柄",
        "使用 T-12 形式的编号，或完整 UUID，或至少 8 位的十六进制前缀",
    )
}

fn parse_handle(input: &str, principal: Principal, prefix: &str) -> Result<Handle, DocError> {
    let text = input.trim();
    if let Some(rest) = text.strip_prefix(prefix) {
        return parse_positive(rest)
            .map(Handle::Number)
            .ok_or_else(invalid_handle);
    }
    if principal != Principal::Human {
        return Err(invalid_handle());
    }
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return parse_positive(text)
            .map(Handle::Number)
            .ok_or_else(invalid_handle);
    }
    if is_uuid(text) {
        return Ok(Handle::Uuid(text.to_ascii_lowercase()));
    }
    let hex = text.len() >= MIN_HEX_PREFIX_CHARS
        && text.bytes().all(|b| b.is_ascii_hexdigit())
        && text
            .bytes()
            .any(|b| matches!(b.to_ascii_lowercase(), b'a'..=b'f'));
    if hex {
        return Ok(Handle::Prefix(text.to_ascii_lowercase()));
    }
    Err(invalid_handle())
}

fn parse_positive(digits: &str) -> Option<i64> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i64 = digits.parse().ok()?;
    if value < 1 {
        return None;
    }
    Some(value)
}

fn is_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// A section selector of design section 5.2. Resolution against an outline is
/// done by the outline module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionSelector {
    Root,
    Ordinal(usize),
    Clause(String),
    Key(String),
    Text(String),
}

/// Parses a section selector. An absent or blank input is `Root`. Any control
/// character, including newline and tab, is rejected.
pub fn parse_section_selector(input: Option<&str>) -> Result<SectionSelector, DocError> {
    let raw = match input {
        None => return Ok(SectionSelector::Root),
        Some(text) => text,
    };
    if raw.chars().any(|c| c.is_control()) {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            "章节选择器含控制字符",
            "删除控制字符后重试",
        ));
    }
    let text = raw.trim();
    if text.is_empty() {
        return Ok(SectionSelector::Root);
    }
    if let Some(rest) = text.strip_prefix('#') {
        return match parse_positive(rest) {
            Some(n) => usize::try_from(n)
                .map(SectionSelector::Ordinal)
                .map_err(|_| {
                    DocError::new(ErrorCode::InvalidArg, "章节序号超出范围", "使用较小的序号")
                }),
            None => Err(DocError::new(
                ErrorCode::InvalidArg,
                "章节序号须为正整数",
                "使用 #1 形式",
            )),
        };
    }
    if let Some(key) = text.strip_prefix("key:") {
        if key.is_empty() {
            return Err(DocError::new(
                ErrorCode::InvalidArg,
                "章节键不能为空",
                "使用 key:<精确键> 形式",
            ));
        }
        return Ok(SectionSelector::Key(key.to_string()));
    }
    if is_clause_number(text) {
        return Ok(SectionSelector::Clause(text.to_ascii_uppercase()));
    }
    Ok(SectionSelector::Text(text.to_string()))
}

fn is_clause_number(text: &str) -> bool {
    let Some(dash) = text.find('-') else {
        return false;
    };
    let (head, tail) = (&text[..dash], &text[dash + 1..]);
    head.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && head.bytes().all(|b| b.is_ascii_alphanumeric())
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
}

/// True for the characters that a write refuses: control characters other
/// than newline and tab, and the bidirectional embedding, override and isolate controls.
pub fn is_forbidden_text_char(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Validates a text field: its length in Unicode scalar values must lie in
/// `min_chars..=max_chars`, and it must contain no forbidden character.
/// Messages do not repeat the user text.
pub fn validate_text(
    field: &str,
    value: &str,
    min_chars: usize,
    max_chars: usize,
) -> Result<(), DocError> {
    let count = value.chars().count();
    if count < min_chars || count > max_chars {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            format!("字段 {field} 长度须在 {min_chars} 到 {max_chars} 字符之间"),
            "调整字段长度后重试",
        )
        .with_detail(format!("{field}: {count} characters")));
    }
    if value.chars().any(is_forbidden_text_char) {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            format!("字段 {field} 含有控制字符或双向控制符"),
            "删除这些字符后重试",
        ));
    }
    Ok(())
}

/// Requires a non-blank value of at most `max_chars` characters.
pub fn required_text(field: &str, value: Option<&str>, max_chars: usize) -> Result<(), DocError> {
    match value {
        Some(text) if !text.trim().is_empty() => validate_text(field, text, 1, max_chars),
        _ => Err(DocError::new(
            ErrorCode::InvalidArg,
            format!("缺少必填字段 {field}"),
            "补充该字段后重试",
        )),
    }
}

/// Validates the text fields of a plan: summary 1 to 500, rationale at most
/// 4000, impact at most 2000, at most five options of 1 to 500 characters.
pub fn validate_plan_text(
    summary: &str,
    rationale: &str,
    impact: &str,
    options: &[&str],
) -> Result<(), DocError> {
    validate_text("summary", summary, 1, MAX_PLAN_SUMMARY_CHARS)?;
    validate_text("rationale", rationale, 0, MAX_PLAN_RATIONALE_CHARS)?;
    validate_text("impact", impact, 0, MAX_PLAN_IMPACT_CHARS)?;
    if options.len() > MAX_PLAN_OPTIONS {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            "备选方案最多 5 项",
            "减少备选方案后重试",
        ));
    }
    for option in options {
        validate_text("option", option, 1, MAX_PLAN_OPTION_CHARS)?;
    }
    Ok(())
}

/// Validates a priority: 1, 2 or 3.
pub fn validate_priority(value: i64) -> Result<u8, DocError> {
    if (1..=3).contains(&value) {
        Ok(value as u8)
    } else {
        Err(DocError::new(
            ErrorCode::InvalidArg,
            "优先级须为 1 到 3",
            "使用 1、2 或 3",
        ))
    }
}

/// Normalises the new text of a plan. Its raw size must not exceed 262144
/// bytes. Each `\r\n` becomes `\n`; any remaining carriage return or other
/// control character, except newline and tab, is rejected.
pub fn normalize_new_text(text: &str) -> Result<String, DocError> {
    if text.len() > MAX_NEW_TEXT_BYTES {
        return Err(DocError::new(
            ErrorCode::PlanTooLarge,
            "新文本超过 262144 字节",
            "缩短新文本后重试",
        ));
    }
    let normalized = text.replace("\r\n", "\n");
    if normalized
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            "新文本含有控制字符",
            "删除控制字符后重试",
        ));
    }
    Ok(normalized)
}

/// Checks that a target file can take a plan edit: the extension is `md`,
/// `markdown`, `nema` or `nemat` (any case); the size is at most 1 MiB; the
/// bytes contain no NUL, do not start with a UTF-16 byte order mark and are
/// valid UTF-8. A UTF-8 byte order mark is accepted.
pub fn check_plan_target(extension: &str, bytes: &[u8]) -> Result<(), DocError> {
    let extension = extension.to_ascii_lowercase();
    if !matches!(extension.as_str(), "md" | "markdown" | "nema" | "nemat") {
        return Err(DocError::new(
            ErrorCode::UnsupportedFile,
            "目标文件类型不支持方案编辑",
            "选择 md、markdown、nema 或 nemat 文件",
        ));
    }
    if bytes.len() > MAX_TARGET_FILE_BYTES {
        return Err(DocError::new(
            ErrorCode::PlanTooLarge,
            "目标文件超过 1 MiB",
            "拆分为更小的文件后重试",
        ));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) || bytes.contains(&0) {
        return Err(DocError::new(
            ErrorCode::UnsupportedFile,
            "目标文件是 UTF-16 或含 NUL 字节",
            "转换为 UTF-8 后重试",
        ));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(DocError::new(
            ErrorCode::UnsupportedFile,
            "目标文件不是有效的 UTF-8",
            "转换为 UTF-8 后重试",
        ));
    }
    Ok(())
}

/// True for the characters that the display sanitiser replaces: control
/// characters other than newline and tab, zero-width characters, the
/// left-to-right and right-to-left marks, the Arabic letter mark and the
/// bidirectional embedding, override and isolate controls.
pub fn is_display_hazard(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{061C}' | '\u{FEFF}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
}

/// Replaces each display hazard with `<U+XXXX>`, a visible marker of the code point.
pub fn sanitize_display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_display_hazard(c) {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// SHA-256 of `bytes` as 64 lowercase hexadecimal digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// The fields of a governance event, in the order of the hashed tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFields<'a> {
    pub prev_hash: &'a str,
    pub id: &'a str,
    pub task_id: &'a str,
    pub file_id: &'a str,
    pub subject_kind: &'a str,
    pub subject_id: &'a str,
    pub kind: &'a str,
    pub from_status: &'a str,
    pub to_status: &'a str,
    pub actor: &'a str,
    pub actor_kind: &'a str,
    pub session: &'a str,
    pub claimed_session: &'a str,
    pub payload_json: &'a str,
    pub timestamp: &'a str,
}

/// Event hash: SHA-256 of the JSON array of the fifteen fields in the order of
/// design section 4.7. The payload text is used exactly as stored.
pub fn event_hash(fields: &EventFields<'_>) -> String {
    let tuple = (
        fields.prev_hash,
        fields.id,
        fields.task_id,
        fields.file_id,
        fields.subject_kind,
        fields.subject_id,
        fields.kind,
        fields.from_status,
        fields.to_status,
        fields.actor,
        fields.actor_kind,
        fields.session,
        fields.claimed_session,
        fields.payload_json,
        fields.timestamp,
    );
    sha256_hex(&to_vec(&tuple).expect("a tuple of strings serializes"))
}

/// The fields of a plan that the plan hash covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanFields<'a> {
    pub task_id: &'a str,
    pub plan_no: i64,
    pub summary: &'a str,
    pub edit_kind: &'a str,
    pub target_file_id: &'a str,
    pub target_section: &'a str,
    pub new_text: &'a str,
    pub expected_hash: &'a str,
    pub result_hash: &'a str,
}

/// Plan hash: SHA-256 of the JSON array of the nine fields of design section 8.1.
pub fn plan_hash(plan: &PlanFields<'_>) -> String {
    let tuple = (
        plan.task_id,
        plan.plan_no,
        plan.summary,
        plan.edit_kind,
        plan.target_file_id,
        plan.target_section,
        plan.new_text,
        plan.expected_hash,
        plan.result_hash,
    );
    sha256_hex(&to_vec(&tuple).expect("a tuple of strings and an integer serializes"))
}

/// Evidence hash: SHA-256 of the JSON array `[plan_hash, ids]`, where `ids` are
/// the evidence ids sorted in ascending byte order.
pub fn evidence_hash(plan_hash: &str, evidence_ids: &[&str]) -> String {
    let mut ids: Vec<&str> = evidence_ids.to_vec();
    ids.sort_unstable();
    sha256_hex(&to_vec(&(plan_hash, ids)).expect("a string and a list of strings serialize"))
}

/// One row of `doc_task_events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub seq: i64,
    pub id: String,
    pub task_id: String,
    pub file_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub kind: String,
    pub from_status: String,
    pub to_status: String,
    pub actor: String,
    pub actor_kind: String,
    pub session: String,
    pub claimed_session: String,
    pub payload_json: String,
    pub timestamp: String,
    pub prev_hash: String,
    pub hash: String,
}

impl EventRecord {
    /// The hash that the stored fields produce.
    pub fn computed_hash(&self) -> String {
        event_hash(&EventFields {
            prev_hash: &self.prev_hash,
            id: &self.id,
            task_id: &self.task_id,
            file_id: &self.file_id,
            subject_kind: &self.subject_kind,
            subject_id: &self.subject_id,
            kind: &self.kind,
            from_status: &self.from_status,
            to_status: &self.to_status,
            actor: &self.actor,
            actor_kind: &self.actor_kind,
            session: &self.session,
            claimed_session: &self.claimed_session,
            payload_json: &self.payload_json,
            timestamp: &self.timestamp,
        })
    }
}

/// Walks the chain in `seq` order from the genesis value and returns the `seq`
/// of the first record whose previous hash or stored hash is inconsistent.
/// Returns `None` for an intact or empty chain.
pub fn first_chain_break(events: &[EventRecord]) -> Option<i64> {
    let mut expected_prev: &str = GENESIS_PREV_HASH;
    for event in events {
        if event.prev_hash != expected_prev || event.computed_hash() != event.hash {
            return Some(event.seq);
        }
        expected_prev = event.hash.as_str();
    }
    None
}

/// The task row fields that the pure logic reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: String,
    pub task_no: i64,
    pub file_id: String,
    pub section: String,
    pub title: String,
    pub description: String,
    pub status: TaskState,
    pub priority: u8,
    pub proposal_state: ProposalState,
    pub source_kind: SourceKind,
    pub created_by_kind: ActorKind,
    pub visibility: Visibility,
    pub anchor_state: AnchorState,
    pub owner: String,
    pub cancel_reason: String,
    pub closed_at: Option<String>,
}

/// The plan row fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRecord {
    pub id: String,
    pub task_id: String,
    pub plan_no: i64,
    pub summary: String,
    pub rationale: String,
    pub impact: String,
    pub risk: Risk,
    pub options: Vec<String>,
    pub chosen_option: String,
    pub edit_kind: EditKind,
    pub target_file_id: String,
    pub target_section: String,
    pub new_text: String,
    pub expected_version_id: String,
    pub expected_hash: String,
    pub result_hash: String,
    pub plan_hash: String,
    pub status: PlanStatus,
    pub submitted_by_kind: ActorKind,
    pub created_at: String,
}

/// The approval row fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: String,
    pub task_id: String,
    pub kind: ApprovalKind,
    pub decision: ApprovalDecision,
    pub approver_kind: ActorKind,
    pub plan_id: String,
    pub subject_hash: String,
    pub base_version_id: String,
    pub comment: String,
    pub event_id: String,
    pub created_at: String,
}

/// The evidence row fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub id: String,
    pub task_id: String,
    pub kind: EvidenceKind,
    pub result: EvidenceResult,
    pub source: EvidenceSource,
    pub actor_kind: ActorKind,
    pub created_at: String,
}

/// One task as seen by the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedTask {
    pub id: String,
    pub task_no: i64,
    pub status: TaskState,
    pub proposal_accepted: bool,
    pub depends_on: Vec<String>,
    /// File of the latest plan that is neither superseded nor rejected.
    pub plan_target: Option<String>,
    /// File of the task's target link.
    pub link_target: Option<String>,
}

/// Tasks that write the same file within one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub file: String,
    pub tasks: Vec<String>,
}

/// A task that cannot be scheduled, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub task: String,
    pub reason: String,
}

/// Output of `schedule_batches`. Every task is written as `T-n`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    pub batches: Vec<Vec<String>>,
    pub conflicts: Vec<Conflict>,
    pub blocked: Vec<Blocked>,
    pub unknown_targets: Vec<String>,
}

fn handle_of(task_no: i64) -> String {
    format!("T-{}", task_no)
}

fn dependency_edge(from: &str, to: &str) -> Dependency {
    Dependency {
        from: from.to_string(),
        edge_id: format!("{from}>{to}"),
        to: to.to_string(),
        label: "depends_on".to_string(),
    }
}

fn is_schedulable(task: &SchedTask) -> bool {
    matches!(
        task.status,
        TaskState::Open | TaskState::Analysis | TaskState::Approval | TaskState::Working
    ) && task.proposal_accepted
}

fn cycle_error() -> DocError {
    DocError::new(ErrorCode::DepCycle, "依赖成环", "删除循环中的依赖后重试")
}

/// Layers the dependency graph and groups the tasks that write one file.
///
/// The graph of all input tasks must be acyclic (a self-dependency is a cycle).
/// Tasks outside the schedulable set are not reported; a schedulable task whose
/// dependency is cancelled, unaccepted, unknown or outside the set is blocked,
/// and so is every task that depends on a blocked task. A dependency that is
/// `done` is dropped. Each batch is sorted by task number; a batch's tasks that
/// share a known target form one conflict group. A task without a known target
/// is listed in `unknown_targets` and still scheduled.
pub fn schedule_batches(tasks: &[SchedTask]) -> Result<Schedule, DocError> {
    let mut by_id: BTreeMap<&str, &SchedTask> = BTreeMap::new();
    for task in tasks {
        if by_id.insert(task.id.as_str(), task).is_some() {
            return Err(DocError::new(
                ErrorCode::InvalidArg,
                "任务 id 重复",
                "检查任务列表后重试",
            ));
        }
    }

    let mut all_edges = Vec::new();
    for task in tasks {
        for dep in &task.depends_on {
            if by_id.contains_key(dep.as_str()) {
                all_edges.push(dependency_edge(dep, &task.id));
            }
        }
    }
    let all_nodes: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    topo_batches(&all_nodes, &all_edges).map_err(|_| cycle_error())?;

    let schedulable: BTreeMap<&str, &SchedTask> = tasks
        .iter()
        .filter(|t| is_schedulable(t))
        .map(|t| (t.id.as_str(), t))
        .collect();

    let mut blocked: BTreeMap<&str, String> = BTreeMap::new();
    for task in schedulable.values() {
        for dep_id in &task.depends_on {
            let reason = match by_id.get(dep_id.as_str()) {
                None => Some("dependency not found".to_string()),
                Some(dep) if dep.status == TaskState::Done => None,
                Some(_) if schedulable.contains_key(dep_id.as_str()) => None,
                Some(dep) if dep.status == TaskState::Cancelled => {
                    Some(format!("dependency cancelled: {}", handle_of(dep.task_no)))
                }
                Some(dep) if !dep.proposal_accepted => {
                    Some(format!("dependency proposed: {}", handle_of(dep.task_no)))
                }
                Some(dep) => Some(format!(
                    "dependency not schedulable: {}",
                    handle_of(dep.task_no)
                )),
            };
            if let Some(reason) = reason {
                blocked.entry(task.id.as_str()).or_insert(reason);
            }
        }
    }
    loop {
        let mut newly: Vec<(&str, String)> = Vec::new();
        for task in schedulable.values() {
            if blocked.contains_key(task.id.as_str()) {
                continue;
            }
            let upstream = task
                .depends_on
                .iter()
                .find(|d| blocked.contains_key(d.as_str()));
            if let Some(dep_id) = upstream {
                let dep_no = by_id
                    .get(dep_id.as_str())
                    .map(|d| d.task_no)
                    .unwrap_or_default();
                newly.push((
                    task.id.as_str(),
                    format!("upstream blocked: {}", handle_of(dep_no)),
                ));
            }
        }
        if newly.is_empty() {
            break;
        }
        for (id, reason) in newly {
            blocked.insert(id, reason);
        }
    }

    let open_ids: Vec<String> = schedulable
        .keys()
        .filter(|id| !blocked.contains_key(**id))
        .map(|id| id.to_string())
        .collect();
    let open_set: BTreeSet<&str> = open_ids.iter().map(|s| s.as_str()).collect();
    let mut edges = Vec::new();
    for id in &open_ids {
        if let Some(task) = schedulable.get(id.as_str()) {
            for dep in &task.depends_on {
                if open_set.contains(dep.as_str()) {
                    edges.push(dependency_edge(dep, id));
                }
            }
        }
    }
    let layers = topo_batches(&open_ids, &edges).map_err(|_| cycle_error())?;

    let mut batches: Vec<Vec<String>> = Vec::new();
    let mut conflicts: Vec<(i64, Conflict)> = Vec::new();
    let mut unknown: Vec<(i64, String)> = Vec::new();
    for layer in &layers {
        let mut members: Vec<&SchedTask> = layer
            .iter()
            .filter_map(|id| schedulable.get(id.as_str()).copied())
            .collect();
        members.sort_by_key(|t| t.task_no);
        batches.push(members.iter().map(|t| handle_of(t.task_no)).collect());

        let mut by_file: BTreeMap<&str, Vec<&SchedTask>> = BTreeMap::new();
        for task in &members {
            match task
                .plan_target
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(task.link_target.as_deref().filter(|s| !s.is_empty()))
            {
                Some(file) => by_file.entry(file).or_default().push(task),
                None => unknown.push((task.task_no, handle_of(task.task_no))),
            }
        }
        for (file, group) in by_file {
            if group.len() >= 2 {
                let first = group.iter().map(|t| t.task_no).min().unwrap_or_default();
                conflicts.push((
                    first,
                    Conflict {
                        file: file.to_string(),
                        tasks: group.iter().map(|t| handle_of(t.task_no)).collect(),
                    },
                ));
            }
        }
    }
    conflicts.sort_by(|a, b| a.1.file.cmp(&b.1.file).then(a.0.cmp(&b.0)));
    unknown.sort();

    let mut blocked_out: Vec<(i64, Blocked)> = blocked
        .iter()
        .map(|(id, reason)| {
            let no = by_id.get(id).map(|t| t.task_no).unwrap_or_default();
            (
                no,
                Blocked {
                    task: handle_of(no),
                    reason: reason.clone(),
                },
            )
        })
        .collect();
    blocked_out.sort_by_key(|(no, _)| *no);

    Ok(Schedule {
        batches,
        conflicts: conflicts.into_iter().map(|(_, c)| c).collect(),
        blocked: blocked_out.into_iter().map(|(_, b)| b).collect(),
        unknown_targets: unknown.into_iter().map(|(_, h)| h).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_STATES: [&str; 7] = [
        "open",
        "analysis",
        "approval",
        "working",
        "verification",
        "done",
        "cancelled",
    ];
    const HUMAN_ONLY_PAIRS: [(TaskState, TaskState); 11] = [
        (TaskState::Approval, TaskState::Working),
        (TaskState::Approval, TaskState::Analysis),
        (TaskState::Verification, TaskState::Done),
        (TaskState::Open, TaskState::Cancelled),
        (TaskState::Analysis, TaskState::Cancelled),
        (TaskState::Approval, TaskState::Cancelled),
        (TaskState::Working, TaskState::Cancelled),
        (TaskState::Verification, TaskState::Cancelled),
        (TaskState::Done, TaskState::Open),
        (TaskState::Cancelled, TaskState::Open),
        (TaskState::Approval, TaskState::Analysis),
    ];

    #[test]
    fn tc830_status_strings_match_legacy_seven() {
        let names: Vec<&str> = TaskState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, LEGACY_STATES);
        for state in TaskState::ALL {
            assert_eq!(TaskState::parse(state.as_str()), Some(*state));
        }
        assert_eq!(TaskState::parse("Open"), None);
        assert_eq!(TaskState::parse("DONE"), None);
        assert_eq!(TaskState::parse(""), None);
    }

    #[test]
    fn tc830_module_does_not_import_store_or_engine() {
        let source = include_str!("doctask_model.rs");
        let forbidden = [
            concat!("crate::", "store"),
            concat!("crate::", "engine"),
            concat!("super::", "store"),
            concat!("super::", "engine"),
            concat!("use ", "store"),
            concat!("use ", "engine"),
        ];
        for needle in forbidden {
            assert!(!source.contains(needle), "forbidden import: {needle}");
        }
    }

    #[test]
    fn tc831_transition_matrix_is_exhaustive() {
        use Principal::{Agent, Human, System};
        use TaskState::*;
        use Transition::*;
        let permitted: [(TaskState, TaskState, Principal, Transition); 21] = [
            (Open, Analysis, Human, T1),
            (Open, Analysis, Agent, T1),
            (Analysis, Approval, Human, T2),
            (Analysis, Approval, Agent, T2),
            (Approval, Working, Human, T3),
            (Approval, Analysis, Human, T4),
            (Working, Verification, Human, T5),
            (Working, Verification, Agent, T5),
            (Verification, Done, Human, T6),
            (Verification, Working, Human, T7),
            (Verification, Working, Agent, T7),
            (Working, Analysis, Human, T8),
            (Working, Analysis, Agent, T8),
            (Open, Cancelled, Human, T9),
            (Analysis, Cancelled, Human, T9),
            (Approval, Cancelled, Human, T9),
            (Working, Cancelled, Human, T9),
            (Verification, Cancelled, Human, T9),
            (Done, Open, Human, T10),
            (Cancelled, Open, Human, T10),
            (Approval, Analysis, System, T11),
        ];
        let mut successes = 0;
        for &from in TaskState::ALL {
            for &to in TaskState::ALL {
                for &principal in Principal::ALL {
                    let expected = permitted
                        .iter()
                        .find(|r| r.0 == from && r.1 == to && r.2 == principal)
                        .map(|r| r.3);
                    match (expected, lookup_transition(from, to, principal)) {
                        (Some(want), Ok(got)) => {
                            assert_eq!(got, want, "{:?} {:?} {:?}", from, to, principal);
                            successes += 1;
                        }
                        (None, Err(err)) => {
                            let want =
                                if HUMAN_ONLY_PAIRS.contains(&(from, to)) && principal != Human {
                                    ErrorCode::HumanOnly
                                } else {
                                    ErrorCode::TransitionDenied
                                };
                            assert_eq!(err.code, want, "{:?} {:?} {:?}", from, to, principal);
                        }
                        other => panic!("{:?} {:?} {:?}: {:?}", from, to, principal, other),
                    }
                }
            }
        }
        assert_eq!(successes, 21);
    }

    #[test]
    fn tc831_terminal_states_only_reopen_by_human() {
        for &principal in Principal::ALL {
            for &from in &[TaskState::Done, TaskState::Cancelled] {
                for &to in TaskState::ALL {
                    let ok = lookup_transition(from, to, principal).is_ok();
                    let expected = to == TaskState::Open && principal == Principal::Human;
                    assert_eq!(ok, expected, "{:?} -> {:?} by {:?}", from, to, principal);
                }
            }
        }
        assert_eq!(
            allowed_from(TaskState::Done, Principal::Human),
            vec![(TaskState::Open, Transition::T10)]
        );
        assert!(allowed_from(TaskState::Done, Principal::Agent).is_empty());
        assert!(allowed_from(TaskState::Cancelled, Principal::System).is_empty());
    }

    #[test]
    fn tc831_system_only_t11_and_t4_human() {
        assert_eq!(
            lookup_transition(TaskState::Approval, TaskState::Analysis, Principal::System),
            Ok(Transition::T11)
        );
        assert_eq!(
            lookup_transition(TaskState::Approval, TaskState::Analysis, Principal::Human),
            Ok(Transition::T4)
        );
        assert_eq!(
            lookup_transition(TaskState::Approval, TaskState::Analysis, Principal::Agent)
                .map_err(|e| e.code),
            Err(ErrorCode::HumanOnly)
        );
        assert_eq!(
            lookup_transition(
                TaskState::Working,
                TaskState::Verification,
                Principal::System
            )
            .map_err(|e| e.code),
            Err(ErrorCode::TransitionDenied)
        );
        assert_eq!(
            lookup_transition(
                TaskState::Verification,
                TaskState::Working,
                Principal::Agent
            ),
            Ok(Transition::T7)
        );
        assert_eq!(
            lookup_transition(TaskState::Approval, TaskState::Working, Principal::System)
                .map_err(|e| e.code),
            Err(ErrorCode::HumanOnly)
        );
    }

    #[test]
    fn tc832_human_only_operations_reject_agent_and_system() {
        use Operation::*;
        let human_only = [
            Approve,
            Reject,
            Close,
            Cancel,
            Reopen,
            AdoptProposal,
            DismissProposal,
            ApplyPlan,
            RecoverPlan,
            BaselineRegister,
            BaselineRevoke,
            EditTaskAttributes,
            ShareAnnotation,
            AnchorAck,
            AnchorRelocate,
            AnchorRebase,
            LineageConfirm,
            LineageReject,
            RemoveDependency,
            RemoveLink,
            ExportFile,
            Verify,
            RestoreVersion,
            ControlHermes,
        ];
        assert_eq!(human_only.len(), 24);
        for op in human_only {
            assert_eq!(authorize(Principal::Human, op), Ok(()), "{:?}", op);
            for principal in [Principal::Agent, Principal::System] {
                let err =
                    authorize(principal, op).expect_err("human-only operation must be refused");
                assert_eq!(
                    err.code,
                    ErrorCode::HumanOnly,
                    "{:?} by {:?}",
                    op,
                    principal
                );
                assert_eq!(err.code.exit_code(), 5);
                assert!(err.detail.contains("[Human]"), "{}", err.detail);
            }
        }
    }

    #[test]
    fn tc832_agent_and_system_operations_follow_table() {
        use Operation::*;
        for op in [
            Read,
            CreateProposal,
            AddAnnotation,
            SubmitPlan,
            AddLink,
            AddDependency,
        ] {
            assert_eq!(authorize(Principal::Agent, op), Ok(()), "{:?}", op);
            assert_eq!(authorize(Principal::Human, op), Ok(()), "{:?}", op);
            assert_eq!(
                authorize(Principal::System, op).map_err(|e| e.code),
                Err(ErrorCode::HumanOnly)
            );
        }
        for op in [Reconcile, Backfill, Migrate, ExpirePlan] {
            assert_eq!(authorize(Principal::System, op), Ok(()), "{:?}", op);
            assert_eq!(authorize(Principal::Human, op), Ok(()), "{:?}", op);
            assert_eq!(
                authorize(Principal::Agent, op).map_err(|e| e.code),
                Err(ErrorCode::HumanOnly)
            );
        }
    }

    #[test]
    fn tc833_guards_t1_proposal_and_anchor() {
        let mut g = GuardInput::new(Principal::Agent);
        assert_eq!(check_guards(Transition::T1, &g), Ok(()));
        g.proposal_accepted = false;
        let err = check_guards(Transition::T1, &g).expect_err("unaccepted proposal");
        assert_eq!(err.code, ErrorCode::TransitionDenied);
        assert!(err.hint_zh.contains("采纳"));
        g.proposal_accepted = true;
        for anchor in [
            AnchorState::Orphaned,
            AnchorState::FileMissing,
            AnchorState::Ambiguous,
        ] {
            g.anchor = anchor;
            assert_eq!(
                check_guards(Transition::T1, &g).map_err(|e| e.code),
                Err(ErrorCode::AnchorBroken)
            );
        }
        g.anchor = AnchorState::Valid;
        assert_eq!(check_guards(Transition::T1, &g), Ok(()));
    }

    #[test]
    fn tc833_guards_t3_order_and_codes() {
        let mut g = GuardInput::new(Principal::Human);
        assert_eq!(check_guards(Transition::T3, &g), Ok(()));
        g.principal = Principal::Agent;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::HumanOnly)
        );
        g.principal = Principal::Human;
        g.latest_plan_submitted = false;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        g.latest_plan_submitted = true;
        g.subject_hash_matches = false;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::StaleSubject)
        );
        g.subject_hash_matches = true;
        g.dependencies_done = false;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        g.dependencies_done = true;

        g.baseline_bound = false;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::AnchorBroken)
        );
        g.baseline_bound = true;
        g.anchor = AnchorState::NeedsReview;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::AnchorBroken)
        );
        g.anchor_reviewed = true;
        assert_eq!(check_guards(Transition::T3, &g), Ok(()));
        g.anchor = AnchorState::Relocated;
        assert_eq!(check_guards(Transition::T3, &g), Ok(()));
        g.anchor = AnchorState::Ambiguous;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::AnchorBroken)
        );
        g.anchor = AnchorState::Valid;
        g.plan_stale = true;
        assert_eq!(
            check_guards(Transition::T3, &g).map_err(|e| e.code),
            Err(ErrorCode::StalePlan)
        );
        g.plan_stale = false;
        assert_eq!(check_guards(Transition::T3, &g), Ok(()));
    }

    #[test]
    fn tc833_guards_t5_t6_summary_evidence_and_subject() {
        let mut g = GuardInput::new(Principal::Agent);
        g.summary = None;
        assert_eq!(
            check_guards(Transition::T5, &g).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        let long = "文".repeat(MAX_VERIFY_SUMMARY_CHARS + 1);
        g.summary = Some(&long);
        assert_eq!(
            check_guards(Transition::T5, &g).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        let exact = "文".repeat(MAX_VERIFY_SUMMARY_CHARS);
        g.summary = Some(&exact);
        g.plan_applied_or_no_edit = false;
        assert_eq!(
            check_guards(Transition::T5, &g).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        g.plan_applied_or_no_edit = true;
        assert_eq!(check_guards(Transition::T5, &g), Ok(()));

        let mut h = GuardInput::new(Principal::Human);
        h.evidence_accepted = false;
        assert_eq!(
            check_guards(Transition::T6, &h).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        h.evidence_accepted = true;
        h.dependencies_done = false;
        assert_eq!(
            check_guards(Transition::T6, &h).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        h.dependencies_done = true;
        h.anchor = AnchorState::Orphaned;
        assert_eq!(
            check_guards(Transition::T6, &h).map_err(|e| e.code),
            Err(ErrorCode::AnchorBroken)
        );
        h.anchor = AnchorState::Valid;
        h.subject_hash_matches = false;
        assert_eq!(
            check_guards(Transition::T6, &h).map_err(|e| e.code),
            Err(ErrorCode::StaleSubject)
        );
        h.subject_hash_matches = true;
        assert_eq!(check_guards(Transition::T6, &h), Ok(()));
    }

    #[test]
    fn tc833_guards_reason_applying_plan_and_t7_t11() {
        for t in [
            Transition::T4,
            Transition::T7,
            Transition::T8,
            Transition::T9,
            Transition::T10,
        ] {
            let mut g = GuardInput::new(Principal::Human);
            g.reason = None;
            assert_eq!(
                check_guards(t, &g).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{:?}",
                t
            );
            g.reason = Some("   ");
            assert_eq!(
                check_guards(t, &g).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{:?}",
                t
            );
            let too_long = "a".repeat(MAX_REASON_CHARS + 1);
            g.reason = Some(&too_long);
            assert_eq!(
                check_guards(t, &g).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{:?}",
                t
            );
            let exact = "a".repeat(MAX_REASON_CHARS);
            g.reason = Some(&exact);
            assert_eq!(check_guards(t, &g), Ok(()), "{:?}", t);
        }
        for t in [Transition::T8, Transition::T9] {
            let mut g = GuardInput::new(Principal::Human);
            g.has_applying_plan = true;
            assert_eq!(
                check_guards(t, &g).map_err(|e| e.code),
                Err(ErrorCode::GuardFailed),
                "{:?}",
                t
            );
        }
        let mut agent = GuardInput::new(Principal::Agent);
        assert_eq!(
            check_guards(Transition::T7, &agent).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        agent.latest_evidence_failed = true;
        assert_eq!(check_guards(Transition::T7, &agent), Ok(()));
        let human = GuardInput::new(Principal::Human);
        assert_eq!(check_guards(Transition::T7, &human), Ok(()));

        let mut sys = GuardInput::new(Principal::System);
        assert_eq!(
            check_guards(Transition::T11, &sys).map_err(|e| e.code),
            Err(ErrorCode::GuardFailed)
        );
        sys.plan_stale = true;
        assert_eq!(check_guards(Transition::T11, &sys), Ok(()));
    }

    #[test]
    fn tc834_handles_human_and_agent() {
        assert_eq!(
            parse_task_handle("T-12", Principal::Human),
            Ok(Handle::Number(12))
        );
        assert_eq!(
            parse_task_handle(" T-12 ", Principal::Human),
            Ok(Handle::Number(12))
        );
        assert_eq!(
            parse_task_handle("12", Principal::Human),
            Ok(Handle::Number(12))
        );
        assert_eq!(
            parse_task_handle("12345678", Principal::Human),
            Ok(Handle::Number(12_345_678))
        );
        assert_eq!(
            parse_task_handle("0F5C6A7B-1D2E-4F30-8A9B-0C1D2E3F4A5B", Principal::Human),
            Ok(Handle::Uuid(
                "0f5c6a7b-1d2e-4f30-8a9b-0c1d2e3f4a5b".to_string()
            ))
        );
        assert_eq!(
            parse_task_handle("0f5c6a7b", Principal::Human),
            Ok(Handle::Prefix("0f5c6a7b".to_string()))
        );
        for bad in [
            "t-12", "0", "-3", "N-5", "12abc", "abc", "", "   ", "0f5c6a7", "zz5c6a7b",
        ] {
            assert_eq!(
                parse_task_handle(bad, Principal::Human).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_task_handle("T-12", Principal::Agent),
            Ok(Handle::Number(12))
        );
        assert_eq!(
            parse_annotation_handle("N-5", Principal::Agent),
            Ok(Handle::Number(5))
        );
        assert_eq!(
            parse_task_handle("12", Principal::Agent).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            parse_task_handle("0f5c6a7b-1d2e-4f30-8a9b-0c1d2e3f4a5b", Principal::Agent)
                .map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            parse_task_handle("N-5", Principal::Agent).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            parse_annotation_handle("N-5", Principal::Human),
            Ok(Handle::Number(5))
        );
    }

    #[test]
    fn tc835_section_selectors() {
        assert_eq!(parse_section_selector(None), Ok(SectionSelector::Root));
        assert_eq!(parse_section_selector(Some("")), Ok(SectionSelector::Root));
        assert_eq!(
            parse_section_selector(Some("   ")),
            Ok(SectionSelector::Root)
        );
        assert_eq!(
            parse_section_selector(Some("#1")),
            Ok(SectionSelector::Ordinal(1))
        );
        for bad in ["#0", "#x", "#", "key:", "Sc\u{7}ope", "a\nb"] {
            assert_eq!(
                parse_section_selector(Some(bad)).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{bad:?}"
            );
        }
        assert_eq!(
            parse_section_selector(Some("key:H2:Scope#1")),
            Ok(SectionSelector::Key("H2:Scope#1".to_string()))
        );
        assert_eq!(
            parse_section_selector(Some("REQ-101")),
            Ok(SectionSelector::Clause("REQ-101".to_string()))
        );
        assert_eq!(
            parse_section_selector(Some("req-101")),
            Ok(SectionSelector::Clause("REQ-101".to_string()))
        );
        assert_eq!(
            parse_section_selector(Some("REQ-101 scope")),
            Ok(SectionSelector::Text("REQ-101 scope".to_string()))
        );
        assert_eq!(
            parse_section_selector(Some("Scope")),
            Ok(SectionSelector::Text("Scope".to_string()))
        );
    }

    #[test]
    fn tc836_limits_and_text_validation() {
        assert_eq!(
            validate_text("title", "", 1, MAX_TITLE_CHARS).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(validate_text("title", "a", 1, MAX_TITLE_CHARS), Ok(()));
        assert_eq!(
            validate_text("title", &"a".repeat(200), 1, MAX_TITLE_CHARS),
            Ok(())
        );
        assert_eq!(
            validate_text("title", &"a".repeat(201), 1, MAX_TITLE_CHARS).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_text("title", &"文".repeat(200), 1, MAX_TITLE_CHARS),
            Ok(())
        );
        for bad in ["a\u{0}b", "a\u{1b}b", "a\rb", "a\u{202E}b", "a\u{2066}b"] {
            assert_eq!(
                validate_text("title", bad, 1, 10).map_err(|e| e.code),
                Err(ErrorCode::InvalidArg),
                "{bad:?}"
            );
        }
        assert_eq!(validate_text("description", "a\tb\nc", 0, 10), Ok(()));
        assert_eq!(validate_priority(1), Ok(1));
        assert_eq!(validate_priority(3), Ok(3));
        assert_eq!(
            validate_priority(0).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_priority(4).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            required_text("summary", Some("  "), 500).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            required_text("summary", None, 500).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_plan_text("s", "", "", &["a", "b", "c", "d", "e"]),
            Ok(())
        );
        assert_eq!(
            validate_plan_text("s", "", "", &["a", "b", "c", "d", "e", "f"]).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_plan_text("", "", "", &[]).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_plan_text("s", "", "", &[""]).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            validate_plan_text(&"a".repeat(501), "", "", &[]).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
    }

    #[test]
    fn tc836_new_text_and_plan_target_checks() {
        assert_eq!(normalize_new_text("a\r\nb"), Ok("a\nb".to_string()));
        assert_eq!(normalize_new_text("a\nb"), Ok("a\nb".to_string()));
        assert_eq!(
            normalize_new_text("a\rb").map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            normalize_new_text("a\u{1}b").map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert_eq!(
            normalize_new_text("a\r\r\nb").map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );
        assert!(normalize_new_text(&"a".repeat(MAX_NEW_TEXT_BYTES)).is_ok());
        assert_eq!(
            normalize_new_text(&"a".repeat(MAX_NEW_TEXT_BYTES + 1)).map_err(|e| e.code),
            Err(ErrorCode::PlanTooLarge)
        );

        assert_eq!(check_plan_target("md", b"# T\n"), Ok(()));
        assert_eq!(check_plan_target("MD", b"# T\n"), Ok(()));
        assert_eq!(check_plan_target("nemat", b"A>B\n"), Ok(()));
        assert_eq!(
            check_plan_target("txt", b"x").map_err(|e| e.code),
            Err(ErrorCode::UnsupportedFile)
        );
        assert_eq!(
            check_plan_target("md", b"a\0b").map_err(|e| e.code),
            Err(ErrorCode::UnsupportedFile)
        );
        assert_eq!(
            check_plan_target("md", &[0xFF, 0xFE, 0x41, 0x00]).map_err(|e| e.code),
            Err(ErrorCode::UnsupportedFile)
        );
        assert_eq!(
            check_plan_target("md", &[0xFE, 0xFF, 0x00, 0x41]).map_err(|e| e.code),
            Err(ErrorCode::UnsupportedFile)
        );
        assert_eq!(
            check_plan_target("md", &[0xC3, 0x28]).map_err(|e| e.code),
            Err(ErrorCode::UnsupportedFile)
        );
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice(b"text");
        assert_eq!(check_plan_target("md", &bom), Ok(()));
        assert_eq!(
            check_plan_target("md", &vec![b'a'; MAX_TARGET_FILE_BYTES]),
            Ok(())
        );
        assert_eq!(
            check_plan_target("md", &vec![b'a'; MAX_TARGET_FILE_BYTES + 1]).map_err(|e| e.code),
            Err(ErrorCode::PlanTooLarge)
        );
    }

    #[test]
    fn tc837_display_sanitize_replaces_hazards() {
        assert_eq!(sanitize_display("a\u{1b}[31mb"), "a<U+001B>[31mb");
        assert_eq!(sanitize_display("x\u{202E}y"), "x<U+202E>y");
        assert_eq!(
            sanitize_display("a\u{2066}b\u{200B}c\u{FEFF}d\u{200E}e\u{061C}f\u{7}g"),
            "a<U+2066>b<U+200B>c<U+FEFF>d<U+200E>e<U+061C>f<U+0007>g"
        );
        assert_eq!(sanitize_display("line1\n\tline2"), "line1\n\tline2");
        assert_eq!(sanitize_display("plain 文档"), "plain 文档");
        assert!(is_display_hazard('\u{200C}'));
        assert!(is_display_hazard('\u{1b}'));
        assert!(!is_display_hazard('a'));
        assert!(!is_display_hazard('\n'));
    }

    fn sample_event_fields() -> Vec<String> {
        [
            GENESIS_PREV_HASH,
            "evt-0001",
            "task-0001",
            "file-0001",
            "task",
            "task-0001",
            "dt.task.transitioned",
            "open",
            "analysis",
            "owner",
            "human",
            "",
            "",
            "{\"reason\":\"start\"}",
            "2026-11-25T10:00:00+09:00",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn event_of(v: &[String]) -> EventFields<'_> {
        EventFields {
            prev_hash: &v[0],
            id: &v[1],
            task_id: &v[2],
            file_id: &v[3],
            subject_kind: &v[4],
            subject_id: &v[5],
            kind: &v[6],
            from_status: &v[7],
            to_status: &v[8],
            actor: &v[9],
            actor_kind: &v[10],
            session: &v[11],
            claimed_session: &v[12],
            payload_json: &v[13],
            timestamp: &v[14],
        }
    }

    const EVENT_GOLDEN: &str = "097a3e88fcbbd5be1a7b1706436e02f1197ab999d080625c7b3721cd248f6b2e";
    const EVENT_GOLDEN_UNICODE: &str =
        "0d897271ad472c434bd0f3288ab6acb4d564af702b272dc65ab5dc0782b31c43";
    const PLAN_GOLDEN: &str = "31c19c5aa751a460d23523fb842f73093a50cfd5f96fcc9ea68d6cb19a3ecb4e";
    const EVIDENCE_GOLDEN: &str =
        "5c4b79612d9bd644f1072e1d7e4b25d3009784d96928e96dbee04d122f076785";

    #[test]
    fn tc838_event_hash_golden_and_field_sensitivity() {
        let base = sample_event_fields();
        assert_eq!(event_hash(&event_of(&base)), EVENT_GOLDEN);
        let mut unicode = base.clone();
        unicode[13] = "{\"title\":\"文档\"}".to_string();
        assert_eq!(event_hash(&event_of(&unicode)), EVENT_GOLDEN_UNICODE);
        for index in 0..15 {
            let mut changed = base.clone();
            changed[index].push('x');
            assert_ne!(
                event_hash(&event_of(&changed)),
                EVENT_GOLDEN,
                "field {index} must change the hash"
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_digest(
        task: &str,
        no: i64,
        summary: &str,
        edit: &str,
        file: &str,
        section: &str,
        text: &str,
        expected: &str,
        result: &str,
    ) -> String {
        plan_hash(&PlanFields {
            task_id: task,
            plan_no: no,
            summary,
            edit_kind: edit,
            target_file_id: file,
            target_section: section,
            new_text: text,
            expected_hash: expected,
            result_hash: result,
        })
    }

    #[test]
    fn tc838_plan_and_evidence_hash_golden_and_field_sensitivity() {
        let a64 = "a".repeat(64);
        let b64 = "b".repeat(64);
        let base = plan_digest(
            "task-0001",
            1,
            "Clarify wording",
            "replace_section",
            "file-0001",
            "H2:Scope#1",
            "New line\n",
            &a64,
            &b64,
        );
        assert_eq!(base, PLAN_GOLDEN);
        let changed = [
            plan_digest(
                "task-0002",
                1,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                2,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording!",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "insert_after_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "replace_section",
                "file-0002",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#2",
                "New line\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\r\n",
                &a64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &b64,
                &b64,
            ),
            plan_digest(
                "task-0001",
                1,
                "Clarify wording",
                "replace_section",
                "file-0001",
                "H2:Scope#1",
                "New line\n",
                &a64,
                &a64,
            ),
        ];
        for (index, digest) in changed.iter().enumerate() {
            assert_ne!(digest, &base, "plan field {index} must change the hash");
        }

        assert_eq!(
            evidence_hash(&base, &["ev-2", "ev-1", "ev-10"]),
            EVIDENCE_GOLDEN
        );
        assert_eq!(
            evidence_hash(&base, &["ev-1", "ev-10", "ev-2"]),
            EVIDENCE_GOLDEN
        );
        assert_ne!(evidence_hash(&base, &["ev-1", "ev-10"]), EVIDENCE_GOLDEN);
        assert_ne!(
            evidence_hash(&changed[0], &["ev-1", "ev-10", "ev-2"]),
            EVIDENCE_GOLDEN
        );
    }

    fn chain_event(seq: i64, prev: &str, payload: &str) -> EventRecord {
        let mut record = EventRecord {
            seq,
            id: format!("evt-{seq:04}"),
            task_id: "task-0001".to_string(),
            file_id: "file-0001".to_string(),
            subject_kind: "task".to_string(),
            subject_id: "task-0001".to_string(),
            kind: "dt.task.transitioned".to_string(),
            from_status: "open".to_string(),
            to_status: "analysis".to_string(),
            actor: "owner".to_string(),
            actor_kind: "human".to_string(),
            session: String::new(),
            claimed_session: String::new(),
            payload_json: payload.to_string(),
            timestamp: "2026-11-25T10:00:00+09:00".to_string(),
            prev_hash: prev.to_string(),
            hash: String::new(),
        };
        record.hash = record.computed_hash();
        record
    }

    #[test]
    fn tc838_chain_break_is_detected_at_first_altered_seq() {
        let one = chain_event(1, GENESIS_PREV_HASH, "{}");
        let two = chain_event(2, &one.hash, "{\"n\":2}");
        let three = chain_event(3, &two.hash, "{\"n\":3}");
        let intact = vec![one.clone(), two.clone(), three.clone()];
        assert_eq!(first_chain_break(&intact), None);
        assert_eq!(first_chain_break(&[]), None);

        let mut payload_changed = intact.clone();
        payload_changed[1].payload_json = "{\"n\":9}".to_string();
        assert_eq!(first_chain_break(&payload_changed), Some(2));

        let mut link_changed = intact.clone();
        link_changed[2].prev_hash = "1".repeat(64);
        assert_eq!(first_chain_break(&link_changed), Some(3));
    }

    fn sched(no: i64, status: TaskState, deps: &[i64], target: Option<&str>) -> SchedTask {
        SchedTask {
            id: format!("id-{no}"),
            task_no: no,
            status,
            proposal_accepted: true,
            depends_on: deps.iter().map(|d| format!("id-{d}")).collect(),
            plan_target: target.map(str::to_string),
            link_target: None,
        }
    }

    fn handles(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tc839_schedule_layers_and_groups_conflicts() {
        let tasks = vec![
            sched(1, TaskState::Open, &[], Some("a.md")),
            sched(2, TaskState::Analysis, &[1], Some("a.md")),
            sched(3, TaskState::Working, &[], Some("c.md")),
            sched(4, TaskState::Open, &[], None),
        ];
        let out = schedule_batches(&tasks).expect("acyclic");
        assert_eq!(
            out.batches,
            vec![handles(&["T-1", "T-3", "T-4"]), handles(&["T-2"])]
        );
        assert!(out.conflicts.is_empty());
        assert_eq!(out.unknown_targets, handles(&["T-4"]));
        assert!(out.blocked.is_empty());
        assert_eq!(schedule_batches(&[]), Ok(Schedule::default()));
    }

    #[test]
    fn tc839_schedule_conflict_group_and_done_edge() {
        let tasks = vec![
            sched(5, TaskState::Open, &[], Some("x.md")),
            sched(6, TaskState::Open, &[], Some("x.md")),
            sched(7, TaskState::Open, &[8], Some("y.md")),
            sched(8, TaskState::Done, &[], None),
        ];
        let out = schedule_batches(&tasks).expect("acyclic");
        assert_eq!(out.batches, vec![handles(&["T-5", "T-6", "T-7"])]);
        assert_eq!(
            out.conflicts,
            vec![Conflict {
                file: "x.md".to_string(),
                tasks: handles(&["T-5", "T-6"])
            }]
        );
        assert!(out.blocked.is_empty());
    }

    #[test]
    fn tc839_schedule_rejects_cycles_and_blocks_dependents() {
        let two_cycle = vec![
            sched(9, TaskState::Open, &[10], None),
            sched(10, TaskState::Open, &[9], None),
        ];
        assert_eq!(
            schedule_batches(&two_cycle).map_err(|e| e.code),
            Err(ErrorCode::DepCycle)
        );
        let self_dep = vec![sched(11, TaskState::Open, &[11], None)];
        assert_eq!(
            schedule_batches(&self_dep).map_err(|e| e.code),
            Err(ErrorCode::DepCycle)
        );
        let duplicate = vec![
            sched(12, TaskState::Open, &[], None),
            sched(12, TaskState::Open, &[], None),
        ];
        assert_eq!(
            schedule_batches(&duplicate).map_err(|e| e.code),
            Err(ErrorCode::InvalidArg)
        );

        let mut proposal = sched(24, TaskState::Open, &[], None);
        proposal.proposal_accepted = false;
        let tasks = vec![
            sched(21, TaskState::Cancelled, &[], None),
            sched(22, TaskState::Open, &[21], None),
            sched(23, TaskState::Open, &[22], None),
            proposal,
            sched(25, TaskState::Open, &[24], None),
            sched(26, TaskState::Open, &[999], None),
            sched(27, TaskState::Verification, &[], None),
            sched(28, TaskState::Open, &[27], None),
            sched(29, TaskState::Open, &[], None),
        ];
        let out = schedule_batches(&tasks).expect("acyclic");
        assert_eq!(out.batches, vec![handles(&["T-29"])]);
        let reasons: Vec<(String, String)> = out
            .blocked
            .iter()
            .map(|b| (b.task.clone(), b.reason.clone()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                ("T-22".to_string(), "dependency cancelled: T-21".to_string()),
                ("T-23".to_string(), "upstream blocked: T-22".to_string()),
                ("T-25".to_string(), "dependency proposed: T-24".to_string()),
                ("T-26".to_string(), "dependency not found".to_string()),
                (
                    "T-28".to_string(),
                    "dependency not schedulable: T-27".to_string()
                ),
            ]
        );
    }
}
