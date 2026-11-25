//! Pure export layer of the document-task module: the audit report in Markdown and JSON and the
//! .NEMA traceability graph of tasks, clauses, files and dependencies. The module performs no
//! I/O, reads no clock and no environment variable: the caller passes the records, the export
//! options and the generation time. Writing the text to a file belongs to the command layer.

use crate::doctask_model::{
    first_chain_break, sanitize_display, ActorKind, AnchorState, ApprovalRecord, DocError,
    ErrorCode, EventRecord, EvidenceRecord, LinkKind, PlanRecord, SourceKind, TargetKind,
    TaskRecord, Visibility, GENESIS_PREV_HASH,
};
use crate::nema::{parse_graph_edges, topo_batches, Dependency};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Schema name written to the JSON report.
pub const REPORT_SCHEMA: &str = "ladon.doctask.report.v1";
/// Kind of the event that records an export; it never enters the report content.
const EXPORT_EVENT_KIND: &str = "dt.export.generated";
/// Longest node label written to the .NEMA text, in characters.
const NEMA_LABEL_CHARS: usize = 80;
/// Shortest node token: the first characters of the hexadecimal id.
const NODE_TOKEN_MIN: usize = 8;
/// Lengthening step of a node token whose prefix collides with another one.
const NODE_TOKEN_STEP: usize = 4;
/// Command that restores the file to a version, as printed in the report.
const RESTORE_PREFIX: &str = "ladon restore ";
const TASK_HINT: &str =
    "确认任务编号；私人任务需要 --include-local-only，门禁排除的文件需要 --include-sensitive";
const FILE_HINT: &str = "确认文件路径；门禁排除的文件需要 --include-sensitive";

/// Which part of the workspace an export covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every task, annotation, clause, link, version and commit.
    All,
    /// One task (by id), its clause, annotations and related records.
    Task(String),
    /// The tasks of one file, restricted to one section when given.
    File {
        file_id: String,
        section: Option<String>,
    },
}

/// Scope and visibility switches of one export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub scope: Scope,
    /// Includes `local_only` annotations and tasks.
    pub include_local_only: bool,
    /// Includes the content of files that the external-transfer gate refuses.
    pub include_sensitive: bool,
}

/// One tracked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFile {
    pub id: String,
    pub path: String,
    /// True when the external-transfer gate refuses the file.
    pub sensitive: bool,
}

/// One clause of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportClause {
    pub file_id: String,
    pub section: String,
    /// Anchor id; the node token of the clause in the .NEMA text is derived from it.
    pub anchor_id: String,
    pub ordinal: i64,
    pub number: String,
    pub title: String,
}

/// One annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportAnnotation {
    pub id: String,
    pub no: i64,
    pub file_id: String,
    pub section: String,
    pub text: String,
    pub visibility: Visibility,
    pub author_kind: ActorKind,
    pub anchor_state: AnchorState,
}

/// One link from a task to a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLink {
    pub task_id: String,
    pub kind: LinkKind,
    pub target_kind: TargetKind,
    pub target: String,
    /// File of a file target; empty for URLs, commits and tasks.
    pub target_file_id: String,
    pub label: String,
}

/// A dependency: `prerequisite` must finish before `dependent` (task ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportDependency {
    pub prerequisite: String,
    pub dependent: String,
}

/// One ledger version of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportVersion {
    pub id: String,
    pub file_id: String,
    pub version_no: i64,
    pub content_hash: String,
    pub ledger_event_id: String,
    pub created_at: String,
}

/// One applied plan: the version it produced and the version before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportCommit {
    pub task_id: String,
    pub plan_id: String,
    pub file_id: String,
    pub version_id: String,
    /// Empty when the file had no earlier version.
    pub prev_version_id: String,
    pub ledger_event_id: String,
}

/// Line counts of the difference of one plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPlanStat {
    pub plan_id: String,
    pub added: u64,
    pub removed: u64,
}

/// Every record the export may use. The order of the vectors does not matter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportInput {
    /// Text of `--generated-at`; the only time that appears in the output.
    pub generated_at: String,
    pub files: Vec<ReportFile>,
    pub clauses: Vec<ReportClause>,
    pub annotations: Vec<ReportAnnotation>,
    pub tasks: Vec<TaskRecord>,
    pub plans: Vec<PlanRecord>,
    pub plan_stats: Vec<ReportPlanStat>,
    pub approvals: Vec<ApprovalRecord>,
    pub evidence: Vec<EvidenceRecord>,
    pub links: Vec<ReportLink>,
    pub dependencies: Vec<ReportDependency>,
    pub versions: Vec<ReportVersion>,
    pub commits: Vec<ReportCommit>,
    pub events: Vec<EventRecord>,
}

/// The report as data. Every string is sanitised; the JSON form is `serde_json` of this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub schema: &'static str,
    pub generated_at: String,
    pub cutoff_seq: i64,
    pub scope: String,
    pub chain: ChainRow,
    pub omitted: OmittedRow,
    pub tasks: Vec<TaskRow>,
    pub clauses: Vec<ClauseRow>,
    pub plans: Vec<PlanRow>,
    pub approvals: Vec<ApprovalRow>,
    pub commits: Vec<CommitRow>,
    pub evidence: Vec<EvidenceRow>,
    pub links: Vec<LinkRow>,
    pub dependencies: Vec<DependencyRow>,
    pub batches: Vec<Vec<String>>,
    pub rollback: Vec<RollbackRow>,
    pub anomalies: Vec<NoteRow>,
    pub legacy: Vec<NoteRow>,
    pub timeline: Vec<EventRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChainRow {
    pub ok: bool,
    pub first_break_seq: Option<i64>,
    /// Events up to the cutoff, export events included.
    pub events: usize,
    pub head_hash: String,
}

/// Counts of the items that the visibility rules kept out of the export.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OmittedRow {
    pub local_only_tasks: usize,
    pub local_only_annotations: usize,
    pub gated_tasks: usize,
    pub gated_annotations: usize,
    pub dependencies: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskRow {
    pub handle: String,
    pub title: String,
    pub status: String,
    pub file: String,
    pub section: String,
    pub owner: String,
    pub visibility: String,
    pub anchor_state: String,
    pub source_kind: String,
    pub created_by_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClauseRow {
    pub file: String,
    pub number: String,
    pub title: String,
    pub annotations: Vec<AnnotationRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnnotationRow {
    pub handle: String,
    pub visibility: String,
    pub anchor_state: String,
    pub author_kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanRow {
    pub task: String,
    pub plan_no: i64,
    pub status: String,
    /// Empty for an analysis-only plan.
    pub edit_kind: String,
    pub risk: String,
    pub summary: String,
    pub target_file: String,
    pub target_section: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
    pub expected_hash: String,
    pub result_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRow {
    pub task: String,
    pub plan_no: i64,
    pub kind: String,
    pub decision: String,
    pub approver_kind: String,
    pub subject_hash: String,
    pub comment: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitRow {
    pub task: String,
    pub plan_no: i64,
    pub file: String,
    pub version_id: String,
    pub ledger_event_id: String,
    pub prev_version_id: String,
    /// Restore command to the version before the change; absent when there is none.
    pub restore_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceRow {
    pub task: String,
    pub kind: String,
    pub result: String,
    pub source: String,
    pub actor_kind: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRow {
    pub task: String,
    pub kind: String,
    pub target_kind: String,
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependencyRow {
    pub prerequisite: String,
    pub dependent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RollbackRow {
    pub file: String,
    pub versions: Vec<VersionRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionRow {
    pub version_no: i64,
    pub version_id: String,
    pub ledger_event_id: String,
    pub content_hash: String,
    pub created_at: String,
}

/// An anomaly or a record of unknown origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteRow {
    pub subject: String,
    pub detail: String,
}

/// One event of the timeline. Payloads are never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventRow {
    pub seq: i64,
    pub kind: String,
    /// Task handle, or empty for an event without a task.
    pub task: String,
    pub from_status: String,
    pub to_status: String,
    pub actor: String,
    pub actor_kind: String,
    pub timestamp: String,
}

/// Output of the selection, shared by the three renderers.
struct Built {
    report: Report,
    tasks: Vec<TaskRecord>,
    clauses: Vec<ReportClause>,
    links: Vec<ReportLink>,
    /// Exported `Blocks` edges as (prerequisite id, dependent id), in emission order.
    blocking: Vec<(String, String)>,
    paths: BTreeMap<String, String>,
    task_tokens: BTreeMap<String, String>,
}

/// Renders the audit report as Markdown. The text has no trailing newline.
pub fn render_markdown(input: &ReportInput, options: &ExportOptions) -> Result<String, DocError> {
    let built = build(input, options)?;
    let r = &built.report;
    let mut md = String::new();
    line(&mut md, "# 文档任务审计报告");
    line(&mut md, "");
    line(
        &mut md,
        &format!("- 生成时间: {}", md_inline(&r.generated_at)),
    );
    line(
        &mut md,
        &format!("- 数据截止点 cutoff_seq: {}", r.cutoff_seq),
    );
    line(&mut md, &format!("- 范围: {}", md_inline(&r.scope)));
    line(&mut md, &format!("- 链校验: {}", chain_text(&r.chain)));
    line(
        &mut md,
        &format!("- 链头哈希: {}", md_inline(&r.chain.head_hash)),
    );
    line(
        &mut md,
        &format!(
            "- 已省略: 本地专属任务 {}，门禁排除任务 {}，本地专属批注 {}，门禁排除批注 {}，省略依赖 {}",
            r.omitted.local_only_tasks,
            r.omitted.gated_tasks,
            r.omitted.local_only_annotations,
            r.omitted.gated_annotations,
            r.omitted.dependencies
        ),
    );
    line(&mut md, "");

    line(&mut md, "## 1. 摘要");
    let annotations: usize = r.clauses.iter().map(|c| c.annotations.len()).sum();
    line(
        &mut md,
        &format!(
            "- 任务 {}，条款 {}，批注 {}，方案 {}，批准 {}，提交 {}，证据 {}，关联 {}，依赖 {}，事件 {}",
            r.tasks.len(),
            r.clauses.len(),
            annotations,
            r.plans.len(),
            r.approvals.len(),
            r.commits.len(),
            r.evidence.len(),
            r.links.len(),
            r.dependencies.len(),
            r.timeline.len()
        ),
    );
    line(&mut md, "");

    line(&mut md, "## 2. 条款与批注");
    if r.clauses.is_empty() {
        line(&mut md, "（无）");
    }
    for clause in &r.clauses {
        let heading = if clause.number.is_empty() {
            format!(
                "### {} {}",
                md_inline(&clause.file),
                md_inline(&clause.title)
            )
        } else {
            format!(
                "### {} {} {}",
                md_inline(&clause.file),
                md_inline(&clause.number),
                md_inline(&clause.title)
            )
        };
        line(&mut md, &heading);
        line(&mut md, "");
        if clause.annotations.is_empty() {
            line(&mut md, "（无批注）");
            line(&mut md, "");
        }
        for note in &clause.annotations {
            line(
                &mut md,
                &format!(
                    "**{}** [{}] 锚点 {}，作者类型 {}",
                    md_inline(&note.handle),
                    md_inline(&note.visibility),
                    md_inline(&note.anchor_state),
                    md_inline(&note.author_kind)
                ),
            );
            line(&mut md, "");
            quote(&mut md, &note.text);
            line(&mut md, "");
        }
    }

    line(&mut md, "## 3. 任务与状态时间线");
    if r.tasks.is_empty() {
        line(&mut md, "（无任务）");
    }
    for task in &r.tasks {
        line(
            &mut md,
            &format!("### {} {}", md_inline(&task.handle), md_inline(&task.title)),
        );
        line(
            &mut md,
            &format!(
                "- 状态: {}，文件: {}，章节: {}",
                md_inline(&task.status),
                md_inline(&task.file),
                md_inline(&task.section)
            ),
        );
        line(
            &mut md,
            &format!(
                "- 负责人: {}，可见性: {}，锚点: {}，来源: {}，创建者类型: {}",
                md_inline(&task.owner),
                md_inline(&task.visibility),
                md_inline(&task.anchor_state),
                md_inline(&task.source_kind),
                md_inline(&task.created_by_kind)
            ),
        );
        line(&mut md, "- 时间线:");
        let mut any = false;
        for event in r.timeline.iter().filter(|e| e.task == task.handle) {
            any = true;
            line(&mut md, &format!("  - {}", event_text(event)));
        }
        if !any {
            line(&mut md, "  - （无）");
        }
        line(&mut md, "");
    }
    line(&mut md, "### 文件与其他事件");
    let mut any = false;
    for event in r.timeline.iter().filter(|e| e.task.is_empty()) {
        any = true;
        line(&mut md, &format!("- {}", event_text(event)));
    }
    if !any {
        line(&mut md, "（无）");
    }
    line(&mut md, "");

    line(&mut md, "## 4. 批准");
    list_or_none(
        &mut md,
        r.approvals.iter().map(|a| {
            let mut text = format!(
                "{} 方案 {} [{}] {}，批准人类型 {}，绑定哈希 {}，时间 {}",
                md_inline(&a.task),
                a.plan_no,
                md_inline(&a.kind),
                md_inline(&a.decision),
                md_inline(&a.approver_kind),
                md_inline(&a.subject_hash),
                md_inline(&a.created_at)
            );
            if !a.comment.is_empty() {
                text.push_str(&format!("，说明: {}", md_inline(&a.comment)));
            }
            text
        }),
    );

    line(&mut md, "## 5. 方案与差异统计");
    list_or_none(
        &mut md,
        r.plans.iter().map(|p| {
            let edit = if p.edit_kind.is_empty() {
                "只分析".to_string()
            } else {
                md_inline(&p.edit_kind)
            };
            let diff = match (p.added, p.removed) {
                (Some(a), Some(d)) => format!("+{} -{}", a, d),
                _ => "未计算".to_string(),
            };
            format!(
            "{} 方案 {} [{}] {}，风险 {}，目标 {} {}，差异 {}，期望哈希 {}，结果哈希 {}，摘要: {}",
            md_inline(&p.task),
            p.plan_no,
            md_inline(&p.status),
            edit,
            md_inline(&p.risk),
            md_inline(&p.target_file),
            md_inline(&p.target_section),
            diff,
            md_inline(&p.expected_hash),
            md_inline(&p.result_hash),
            md_inline(&p.summary)
        )
        }),
    );

    line(&mut md, "## 6. 提交与账本事件");
    list_or_none(
        &mut md,
        r.commits.iter().map(|c| {
            format!(
                "{} 方案 {}: 文件 {}，版本 {}，账本事件 {}",
                md_inline(&c.task),
                c.plan_no,
                md_inline(&c.file),
                md_inline(&c.version_id),
                md_inline(&c.ledger_event_id)
            )
        }),
    );

    line(&mut md, "## 7. 测试证据");
    list_or_none(
        &mut md,
        r.evidence.iter().map(|e| {
            let source = if e.source == "ledger" {
                "账本".to_string()
            } else {
                "hermes 报告，未核验".to_string()
            };
            format!(
                "{} [{}] {}，来源 {}，类型 {}，时间 {}",
                md_inline(&e.task),
                md_inline(&e.kind),
                md_inline(&e.result),
                source,
                md_inline(&e.actor_kind),
                md_inline(&e.created_at)
            )
        }),
    );

    line(&mut md, "## 8. 关联");
    list_or_none(
        &mut md,
        r.links.iter().map(|l| {
            let mut text = format!(
                "{} [{}] {} {}",
                md_inline(&l.task),
                md_inline(&l.kind),
                md_inline(&l.target_kind),
                md_inline(&l.target)
            );
            if !l.label.is_empty() {
                text.push_str(&format!(" ({})", md_inline(&l.label)));
            }
            text
        }),
    );

    line(&mut md, "## 9. 依赖与批次");
    list_or_none(
        &mut md,
        r.dependencies.iter().map(|d| {
            format!(
                "{} -> {}",
                md_inline(&d.prerequisite),
                md_inline(&d.dependent)
            )
        }),
    );
    line(&mut md, "");
    line(&mut md, "批次:");
    if r.batches.is_empty() {
        line(&mut md, "（无）");
    }
    for (index, batch) in r.batches.iter().enumerate() {
        let names: Vec<String> = batch.iter().map(|n| md_inline(n)).collect();
        line(
            &mut md,
            &format!("- 批次 {}: {}", index + 1, names.join(", ")),
        );
    }
    line(&mut md, "");

    line(&mut md, "## 10. 回滚链");
    if r.rollback.is_empty() {
        line(&mut md, "（无）");
    }
    for file in &r.rollback {
        line(&mut md, &format!("### {}", md_inline(&file.file)));
        for version in &file.versions {
            line(
                &mut md,
                &format!(
                    "- 版本 {}: {}，账本事件 {}，哈希 {}，时间 {}",
                    version.version_no,
                    md_inline(&version.version_id),
                    md_inline(&version.ledger_event_id),
                    md_inline(&version.content_hash),
                    md_inline(&version.created_at)
                ),
            );
        }
        for commit in r.commits.iter().filter(|c| c.file == file.file) {
            match &commit.restore_command {
                Some(command) => line(
                    &mut md,
                    &format!(
                        "- {} 方案 {} 应用于版本 {}，回到应用前版本: `{}`",
                        md_inline(&commit.task),
                        commit.plan_no,
                        md_inline(&commit.version_id),
                        command
                    ),
                ),
                None => line(
                    &mut md,
                    &format!(
                        "- {} 方案 {} 应用于版本 {}（无前序版本）",
                        md_inline(&commit.task),
                        commit.plan_no,
                        md_inline(&commit.version_id)
                    ),
                ),
            }
        }
        line(&mut md, "");
    }

    line(&mut md, "## 11. 异常");
    list_or_none(&mut md, r.anomalies.iter().map(note_text));
    line(&mut md, "");

    line(&mut md, "## 12. 来源未知的存量数据（未经核验）");
    list_or_none(&mut md, r.legacy.iter().map(note_text));

    Ok(md.trim_end_matches('\n').to_string())
}

/// Renders the audit report as JSON with two-space indentation. The text has no trailing newline.
pub fn render_json(input: &ReportInput, options: &ExportOptions) -> Result<String, DocError> {
    let built = build(input, options)?;
    serde_json::to_string_pretty(&built.report).map_err(|e| internal(&format!("json: {}", e)))
}

/// Renders the .NEMA traceability graph and its task dependency domain. The text ends with one
/// newline. Nothing is returned unless the text reads back to the emitted edges and lays out.
pub fn render_nema(input: &ReportInput, options: &ExportOptions) -> Result<String, DocError> {
    let built = build(input, options)?;
    nema_text(&built)
}

fn build(input: &ReportInput, options: &ExportOptions) -> Result<Built, DocError> {
    let paths: BTreeMap<String, String> = input
        .files
        .iter()
        .map(|f| (f.id.clone(), f.path.clone()))
        .collect();
    let path_of = |id: &str| -> String { paths.get(id).cloned().unwrap_or_else(|| id.to_string()) };
    let gated_files: BTreeSet<String> = input
        .files
        .iter()
        .filter(|f| f.sensitive && !options.include_sensitive)
        .map(|f| f.id.clone())
        .collect();
    let is_gated = |id: &str| gated_files.contains(id);
    let local_only =
        |t: &TaskRecord| t.visibility == Visibility::LocalOnly && !options.include_local_only;

    let scope_text = match &options.scope {
        Scope::All => "全部".to_string(),
        Scope::Task(id) => match input.tasks.iter().find(|t| &t.id == id) {
            Some(t) if !is_gated(t.file_id.as_str()) && !local_only(t) => {
                format!("任务 T-{}", t.task_no)
            }
            _ => return Err(not_found(TASK_HINT)),
        },
        Scope::File { file_id, section } => match input.files.iter().find(|f| &f.id == file_id) {
            Some(f) if !is_gated(f.id.as_str()) => match section {
                Some(s) => format!("文件 {} 章节 {}", one_line(&f.path), one_line(s)),
                None => format!("文件 {}", one_line(&f.path)),
            },
            _ => return Err(not_found(FILE_HINT)),
        },
    };

    let task_sections: BTreeSet<(String, String)> = match &options.scope {
        Scope::Task(id) => input
            .tasks
            .iter()
            .filter(|t| &t.id == id)
            .map(|t| (t.file_id.clone(), t.section.clone()))
            .collect(),
        _ => BTreeSet::new(),
    };
    let section_in = |file_id: &str, section: &str| -> bool {
        match &options.scope {
            Scope::All => true,
            Scope::Task(_) => task_sections.contains(&(file_id.to_string(), section.to_string())),
            Scope::File {
                file_id: wanted,
                section: wanted_section,
            } => {
                file_id == wanted.as_str()
                    && wanted_section.as_deref().map_or(true, |s| s == section)
            }
        }
    };
    let task_in_scope = |t: &TaskRecord| -> bool {
        section_in(t.file_id.as_str(), t.section.as_str())
            && match &options.scope {
                Scope::Task(id) => &t.id == id,
                _ => true,
            }
    };

    let mut omitted = OmittedRow::default();
    let mut hidden_ids: BTreeSet<String> = BTreeSet::new();
    let mut visible: BTreeMap<String, &TaskRecord> = BTreeMap::new();
    let mut exported: Vec<TaskRecord> = Vec::new();
    for t in &input.tasks {
        let gated = is_gated(t.file_id.as_str());
        let local = !gated && local_only(t);
        if gated || local {
            hidden_ids.insert(t.id.clone());
        } else {
            visible.insert(t.id.clone(), t);
        }
        if !task_in_scope(t) {
            continue;
        }
        if gated {
            omitted.gated_tasks += 1;
        } else if local {
            omitted.local_only_tasks += 1;
        } else {
            exported.push(t.clone());
        }
    }
    exported.sort_by_key(|t| t.task_no);
    let exported_ids: BTreeSet<String> = exported.iter().map(|t| t.id.clone()).collect();
    let no_of = |id: &str| -> i64 { visible.get(id).map(|t| t.task_no).unwrap_or(0) };

    let mut annotations: Vec<&ReportAnnotation> = Vec::new();
    for a in &input.annotations {
        let gated = is_gated(a.file_id.as_str());
        let local = !gated && a.visibility == Visibility::LocalOnly && !options.include_local_only;
        if gated || local {
            hidden_ids.insert(a.id.clone());
        }
        if !section_in(a.file_id.as_str(), a.section.as_str()) {
            continue;
        }
        if gated {
            omitted.gated_annotations += 1;
        } else if local {
            omitted.local_only_annotations += 1;
        } else {
            annotations.push(a);
        }
    }
    annotations.sort_by_key(|a| a.no);

    let mut clauses: Vec<&ReportClause> = input
        .clauses
        .iter()
        .filter(|c| {
            !is_gated(c.file_id.as_str()) && section_in(c.file_id.as_str(), c.section.as_str())
        })
        .collect();
    clauses.sort_by_key(|c| (path_of(&c.file_id), c.ordinal, c.section.clone()));

    let mut groups: BTreeMap<(String, String), Vec<&ReportAnnotation>> = BTreeMap::new();
    for a in &annotations {
        groups
            .entry((a.file_id.clone(), a.section.clone()))
            .or_default()
            .push(*a);
    }
    let mut clause_rows: Vec<ClauseRow> = Vec::new();
    let mut used: BTreeSet<(String, String)> = BTreeSet::new();
    for c in &clauses {
        let key = (c.file_id.clone(), c.section.clone());
        if !used.insert(key.clone()) {
            continue;
        }
        clause_rows.push(ClauseRow {
            file: path_of(&c.file_id),
            number: one_line(&c.number),
            title: one_line(&c.title),
            annotations: annotation_rows(groups.get(&key)),
        });
    }
    for (key, group) in &groups {
        if used.contains(key) {
            continue;
        }
        clause_rows.push(ClauseRow {
            file: path_of(&key.0),
            number: String::new(),
            title: one_line(&key.1),
            annotations: annotation_rows(Some(group)),
        });
    }

    let mut plans: Vec<&PlanRecord> = input
        .plans
        .iter()
        .filter(|p| exported_ids.contains(&p.task_id))
        .collect();
    plans.sort_by_key(|p| (no_of(&p.task_id), p.plan_no));
    let plan_no_of: BTreeMap<&str, i64> = input
        .plans
        .iter()
        .map(|p| (p.id.as_str(), p.plan_no))
        .collect();
    let stats: BTreeMap<&str, (u64, u64)> = input
        .plan_stats
        .iter()
        .map(|s| (s.plan_id.as_str(), (s.added, s.removed)))
        .collect();
    let plan_rows: Vec<PlanRow> = plans
        .iter()
        .map(|p| {
            let stat = stats.get(p.id.as_str());
            PlanRow {
                task: format!("T-{}", no_of(&p.task_id)),
                plan_no: p.plan_no,
                status: p.status.as_str().to_string(),
                edit_kind: p.edit_kind.as_str().to_string(),
                risk: p.risk.as_str().to_string(),
                summary: one_line(&p.summary),
                target_file: if p.target_file_id.is_empty() {
                    String::new()
                } else {
                    path_of(&p.target_file_id)
                },
                target_section: one_line(&p.target_section),
                added: stat.map(|s| s.0),
                removed: stat.map(|s| s.1),
                expected_hash: one_line(&p.expected_hash),
                result_hash: one_line(&p.result_hash),
            }
        })
        .collect();

    let mut approvals: Vec<&ApprovalRecord> = input
        .approvals
        .iter()
        .filter(|a| exported_ids.contains(&a.task_id))
        .collect();
    approvals.sort_by_key(|a| (no_of(&a.task_id), a.created_at.clone(), a.id.clone()));
    let approval_rows: Vec<ApprovalRow> = approvals
        .iter()
        .map(|a| ApprovalRow {
            task: format!("T-{}", no_of(&a.task_id)),
            plan_no: plan_no_of.get(a.plan_id.as_str()).copied().unwrap_or(0),
            kind: a.kind.as_str().to_string(),
            decision: a.decision.as_str().to_string(),
            approver_kind: a.approver_kind.as_str().to_string(),
            subject_hash: one_line(&a.subject_hash),
            comment: sanitized(&a.comment),
            created_at: one_line(&a.created_at),
        })
        .collect();

    let mut evidence: Vec<&EvidenceRecord> = input
        .evidence
        .iter()
        .filter(|e| exported_ids.contains(&e.task_id))
        .collect();
    evidence.sort_by_key(|e| (no_of(&e.task_id), e.created_at.clone(), e.id.clone()));
    let evidence_rows: Vec<EvidenceRow> = evidence
        .iter()
        .map(|e| EvidenceRow {
            task: format!("T-{}", no_of(&e.task_id)),
            kind: e.kind.as_str().to_string(),
            result: e.result.as_str().to_string(),
            source: e.source.as_str().to_string(),
            actor_kind: e.actor_kind.as_str().to_string(),
            created_at: one_line(&e.created_at),
        })
        .collect();

    let mut links: Vec<&ReportLink> = input
        .links
        .iter()
        .filter(|l| exported_ids.contains(&l.task_id) && !is_gated(l.target_file_id.as_str()))
        .collect();
    links.sort_by_key(|l| (no_of(&l.task_id), l.kind.as_str(), l.target.clone()));
    let link_rows: Vec<LinkRow> = links
        .iter()
        .map(|l| LinkRow {
            task: format!("T-{}", no_of(&l.task_id)),
            kind: l.kind.as_str().to_string(),
            target_kind: l.target_kind.as_str().to_string(),
            target: one_line(&l.target),
            label: one_line(&l.label),
        })
        .collect();
    let links_owned: Vec<ReportLink> = links.iter().map(|l| (*l).clone()).collect();

    let mut dependency_pairs: Vec<(String, String)> = Vec::new();
    let mut blocking: Vec<(String, String)> = Vec::new();
    for d in &input.dependencies {
        let prerequisite_exported = exported_ids.contains(&d.prerequisite);
        let dependent_exported = exported_ids.contains(&d.dependent);
        if !prerequisite_exported && !dependent_exported {
            continue;
        }
        if !(visible.contains_key(&d.prerequisite) && visible.contains_key(&d.dependent)) {
            omitted.dependencies += 1;
            continue;
        }
        dependency_pairs.push((d.prerequisite.clone(), d.dependent.clone()));
        if prerequisite_exported && dependent_exported {
            blocking.push((d.prerequisite.clone(), d.dependent.clone()));
        }
    }
    let pair_key = |pair: &(String, String)| {
        (
            no_of(&pair.0),
            no_of(&pair.1),
            pair.0.clone(),
            pair.1.clone(),
        )
    };
    dependency_pairs.sort_by_key(pair_key);
    blocking.sort_by_key(pair_key);
    let dependency_rows: Vec<DependencyRow> = dependency_pairs
        .iter()
        .map(|(p, n)| DependencyRow {
            prerequisite: format!("T-{}", no_of(p)),
            dependent: format!("T-{}", no_of(n)),
        })
        .collect();

    let task_tokens: BTreeMap<String, String> = exported
        .iter()
        .map(|t| (t.id.clone(), format!("T-{}", t.task_no)))
        .collect();
    let batch_nodes: Vec<String> = exported
        .iter()
        .map(|t| format!("T-{}", t.task_no))
        .collect();
    let batch_edges: Vec<Dependency> = blocking
        .iter()
        .map(|(p, n)| Dependency {
            from: format!("T-{}", no_of(p)),
            edge_id: String::new(),
            to: format!("T-{}", no_of(n)),
            label: "Blocks".to_string(),
        })
        .collect();
    let batches = topo_batches(&batch_nodes, &batch_edges).map_err(|_| dep_cycle())?;

    let scope_files: Option<BTreeSet<String>> = match &options.scope {
        Scope::All => None,
        Scope::Task(_) => Some(exported.iter().map(|t| t.file_id.clone()).collect()),
        Scope::File { file_id, .. } => Some(std::iter::once(file_id.clone()).collect()),
    };
    let mut versions: Vec<&ReportVersion> = input
        .versions
        .iter()
        .filter(|v| {
            !is_gated(v.file_id.as_str())
                && scope_files
                    .as_ref()
                    .map_or(true, |s| s.contains(&v.file_id))
        })
        .collect();
    versions.sort_by_key(|v| (path_of(&v.file_id), v.version_no, v.id.clone()));
    let mut rollback: Vec<RollbackRow> = Vec::new();
    for v in &versions {
        let file = path_of(&v.file_id);
        let row = VersionRow {
            version_no: v.version_no,
            version_id: one_line(&v.id),
            ledger_event_id: one_line(&v.ledger_event_id),
            content_hash: one_line(&v.content_hash),
            created_at: one_line(&v.created_at),
        };
        let starts_new_file = rollback.last().map_or(true, |current| current.file != file);
        if starts_new_file {
            rollback.push(RollbackRow {
                file,
                versions: vec![row],
            });
        } else if let Some(current) = rollback.last_mut() {
            current.versions.push(row);
        }
    }

    let mut commits: Vec<&ReportCommit> = input
        .commits
        .iter()
        .filter(|c| exported_ids.contains(&c.task_id))
        .collect();
    commits.sort_by_key(|c| {
        (
            no_of(&c.task_id),
            plan_no_of.get(c.plan_id.as_str()).copied().unwrap_or(0),
            c.version_id.clone(),
        )
    });
    let commit_rows: Vec<CommitRow> = commits
        .iter()
        .map(|c| CommitRow {
            task: format!("T-{}", no_of(&c.task_id)),
            plan_no: plan_no_of.get(c.plan_id.as_str()).copied().unwrap_or(0),
            file: path_of(&c.file_id),
            version_id: one_line(&c.version_id),
            ledger_event_id: one_line(&c.ledger_event_id),
            prev_version_id: one_line(&c.prev_version_id),
            restore_command: if c.prev_version_id.is_empty() {
                None
            } else {
                Some(format!(
                    "{}{} --approve",
                    RESTORE_PREFIX,
                    id_chars(&c.prev_version_id)
                ))
            },
        })
        .collect();

    let mut sorted: Vec<&EventRecord> = input.events.iter().collect();
    sorted.sort_by_key(|e| e.seq);
    let cutoff_seq = sorted
        .iter()
        .filter(|e| e.kind != EXPORT_EVENT_KIND)
        .map(|e| e.seq)
        .max()
        .unwrap_or(0);
    let prefix: Vec<EventRecord> = sorted
        .iter()
        .filter(|e| e.seq <= cutoff_seq)
        .map(|e| (*e).clone())
        .collect();
    let first_break = first_chain_break(&prefix);
    let head_hash = prefix
        .last()
        .map(|e| e.hash.clone())
        .unwrap_or_else(|| GENESIS_PREV_HASH.to_string());
    let chain = ChainRow {
        ok: first_break.is_none(),
        first_break_seq: first_break,
        events: prefix.len(),
        head_hash: one_line(&head_hash),
    };

    let mut timeline: Vec<EventRow> = Vec::new();
    for e in &prefix {
        if e.kind == EXPORT_EVENT_KIND {
            continue;
        }
        let hidden_task = !e.task_id.is_empty() && hidden_ids.contains(&e.task_id);
        let hidden_subject = !e.subject_id.is_empty() && hidden_ids.contains(&e.subject_id);
        let hidden_file = !e.file_id.is_empty() && is_gated(e.file_id.as_str());
        if hidden_task || hidden_subject || hidden_file {
            continue;
        }
        let included = match &options.scope {
            Scope::All => true,
            Scope::Task(id) => e.task_id == *id || e.subject_id == *id,
            Scope::File { file_id, .. } => {
                e.file_id == *file_id && (e.task_id.is_empty() || exported_ids.contains(&e.task_id))
            }
        };
        if !included {
            continue;
        }
        timeline.push(EventRow {
            seq: e.seq,
            kind: one_line(&e.kind),
            task: visible
                .get(e.task_id.as_str())
                .map(|t| format!("T-{}", t.task_no))
                .unwrap_or_default(),
            from_status: one_line(&e.from_status),
            to_status: one_line(&e.to_status),
            actor: one_line(&e.actor),
            actor_kind: one_line(&e.actor_kind),
            timestamp: one_line(&e.timestamp),
        });
    }

    let mut anomalies: Vec<NoteRow> = Vec::new();
    for t in &exported {
        if matches!(
            t.anchor_state,
            AnchorState::NeedsReview
                | AnchorState::Relocated
                | AnchorState::Ambiguous
                | AnchorState::Orphaned
                | AnchorState::Unknown
                | AnchorState::FileMissing
        ) {
            anomalies.push(NoteRow {
                subject: format!("T-{}", t.task_no),
                detail: format!("对账状态 {}", t.anchor_state.as_str()),
            });
        }
    }
    if matches!(options.scope, Scope::All) {
        let mut open: BTreeSet<String> = BTreeSet::new();
        for e in &prefix {
            if e.session.is_empty() {
                continue;
            }
            if e.kind == "dt.session.started" {
                open.insert(e.session.clone());
            } else if e.kind == "dt.session.ended" {
                open.remove(&e.session);
            }
        }
        for session in open {
            anomalies.push(NoteRow {
                subject: one_line(&session),
                detail: "会话未结束".to_string(),
            });
        }
    }

    let mut legacy: Vec<NoteRow> = Vec::new();
    for t in &exported {
        if t.source_kind == SourceKind::Legacy || t.created_by_kind == ActorKind::Unknown {
            legacy.push(NoteRow {
                subject: format!("T-{}", t.task_no),
                detail: format!(
                    "来源 {}，创建者类型 {}",
                    t.source_kind.as_str(),
                    t.created_by_kind.as_str()
                ),
            });
        }
    }
    for row in &timeline {
        if row.kind.starts_with("legacy.") {
            legacy.push(NoteRow {
                subject: format!("#{}", row.seq),
                detail: format!("未核验的存量事件 {}", row.kind),
            });
        }
    }

    let task_rows: Vec<TaskRow> = exported
        .iter()
        .map(|t| TaskRow {
            handle: format!("T-{}", t.task_no),
            title: one_line(&t.title),
            status: t.status.as_str().to_string(),
            file: path_of(&t.file_id),
            section: one_line(&t.section),
            owner: one_line(&t.owner),
            visibility: t.visibility.as_str().to_string(),
            anchor_state: t.anchor_state.as_str().to_string(),
            source_kind: t.source_kind.as_str().to_string(),
            created_by_kind: t.created_by_kind.as_str().to_string(),
        })
        .collect();

    let report = Report {
        schema: REPORT_SCHEMA,
        generated_at: one_line(&input.generated_at),
        cutoff_seq,
        scope: scope_text,
        chain,
        omitted,
        tasks: task_rows,
        clauses: clause_rows,
        plans: plan_rows,
        approvals: approval_rows,
        commits: commit_rows,
        evidence: evidence_rows,
        links: link_rows,
        dependencies: dependency_rows,
        batches: batches
            .into_iter()
            .map(|batch| batch.into_iter().collect())
            .collect(),
        rollback,
        anomalies,
        legacy,
        timeline,
    };

    Ok(Built {
        report,
        tasks: exported,
        clauses: clauses.iter().map(|c| (*c).clone()).collect(),
        links: links_owned,
        blocking,
        paths,
        task_tokens,
    })
}

fn nema_text(built: &Built) -> Result<String, DocError> {
    let clause_of = |task: &TaskRecord| {
        built
            .clauses
            .iter()
            .find(|c| c.file_id == task.file_id && c.section == task.section)
    };

    let mut used_anchors: BTreeSet<String> = BTreeSet::new();
    for task in &built.tasks {
        if let Some(clause) = clause_of(task) {
            used_anchors.insert(clause.anchor_id.clone());
        }
    }
    let clause_tokens = assign_tokens("C", &used_anchors)?;
    let mut file_ids: BTreeSet<String> = BTreeSet::new();
    for link in &built.links {
        if !link.target_file_id.is_empty() {
            file_ids.insert(link.target_file_id.clone());
        }
    }
    let file_tokens = assign_tokens("F", &file_ids)?;

    let mut nodes: Vec<String> = Vec::new();
    let mut node_rows: Vec<String> = Vec::new();
    let mut seen_anchor: BTreeSet<String> = BTreeSet::new();
    for clause in &built.clauses {
        if used_anchors.contains(&clause.anchor_id) && seen_anchor.insert(clause.anchor_id.clone())
        {
            let token = clause_tokens[&clause.anchor_id].clone();
            let label = nema_label(&format!("{} {}", clause.number, clause.title));
            node_rows.push(format!("{} {}", token, label));
            nodes.push(token);
        }
    }
    for task in &built.tasks {
        let token = format!("T-{}", task.task_no);
        node_rows.push(format!("{} {}", token, nema_label(&task.title)));
        nodes.push(token);
    }
    let mut file_order: Vec<String> = file_ids.iter().cloned().collect();
    file_order.sort_by_key(|id| (built.paths.get(id).cloned().unwrap_or_default(), id.clone()));
    for id in &file_order {
        let token = file_tokens[id].clone();
        let path = built.paths.get(id).map(String::as_str).unwrap_or("-");
        node_rows.push(format!("{} {}", token, nema_label(path)));
        nodes.push(token);
    }

    let mut writer = EdgeWriter::default();
    for task in &built.tasks {
        let task_token = format!("T-{}", task.task_no);
        if let Some(clause) = clause_of(task) {
            writer.push(&clause_tokens[&clause.anchor_id], &task_token, "Traces");
        }
        for link in built
            .links
            .iter()
            .filter(|l| l.task_id == task.id && !l.target_file_id.is_empty())
        {
            writer.push(
                &task_token,
                &file_tokens[&link.target_file_id],
                link_label(link.kind),
            );
        }
    }
    let domain_one_edges = writer.lines.len();
    for (prerequisite, dependent) in &built.blocking {
        let from = built
            .task_tokens
            .get(prerequisite)
            .cloned()
            .unwrap_or_default();
        let to = built
            .task_tokens
            .get(dependent)
            .cloned()
            .unwrap_or_default();
        writer.push(&from, &to, "Blocks");
    }

    let mut text = String::from(" \\>1 1\n");
    for row in &node_rows {
        text.push_str(row);
        text.push('\n');
    }
    for edge_line in &writer.lines[..domain_one_edges] {
        text.push_str(edge_line);
        text.push('\n');
    }
    text.push_str(" \\>2 1\n");
    for edge_line in &writer.lines[domain_one_edges..] {
        text.push_str(edge_line);
        text.push('\n');
    }

    let parsed = parse_graph_edges(&text).map_err(|e| internal(&format!("nema parse: {}", e)))?;
    if parsed != writer.emitted {
        return Err(internal("nema read-back differs from the emitted edges"));
    }
    topo_batches(&nodes, &parsed).map_err(|_| dep_cycle())?;
    Ok(text)
}

/// Collects the edge rows of the .NEMA text and the matching dependency values.
#[derive(Default)]
struct EdgeWriter {
    lines: Vec<String>,
    emitted: Vec<Dependency>,
}

impl EdgeWriter {
    fn push(&mut self, from: &str, to: &str, label: &str) {
        let edge_id = format!("E{}", self.emitted.len() + 1);
        self.lines
            .push(format!("{}>{}>{} {}", from, edge_id, to, label));
        self.emitted.push(Dependency {
            from: from.to_string(),
            edge_id,
            to: to.to_string(),
            label: label.to_string(),
        });
    }
}

/// Node tokens of one kind. Each id keeps its hexadecimal digits; the first eight are used, and
/// ids whose tokens collide are lengthened by four until every token is distinct.
fn assign_tokens(
    prefix: &str,
    ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>, DocError> {
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for id in ids {
        let key: String = id
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if key.is_empty() {
            return Err(internal("node id without hexadecimal digits"));
        }
        keys.insert(id.clone(), key);
    }
    let mut lengths: BTreeMap<String, usize> = keys
        .iter()
        .map(|(id, key)| (id.clone(), key.len().min(NODE_TOKEN_MIN)))
        .collect();
    loop {
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (id, key) in &keys {
            let len = lengths[id];
            groups
                .entry(key[..len].to_string())
                .or_default()
                .push(id.clone());
        }
        let mut grew = false;
        for members in groups.values() {
            if members.len() < 2 {
                continue;
            }
            for id in members {
                let full = keys[id].len();
                let len = lengths[id];
                if len < full {
                    lengths.insert(id.clone(), (len + NODE_TOKEN_STEP).min(full));
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    let mut tokens: BTreeMap<String, String> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (id, key) in &keys {
        let token = format!("{}-{}", prefix, &key[..lengths[id]]);
        if !seen.insert(token.clone()) {
            return Err(internal("node tokens collide"));
        }
        tokens.insert(id.clone(), token);
    }
    Ok(tokens)
}

/// Label of a .NEMA node: line breaks, tabs and U+3000 become spaces, the text is sanitised,
/// trimmed and cut to 80 characters; an empty label is `-`.
fn nema_label(text: &str) -> String {
    let flat = one_line(text);
    let cut: String = flat.trim().chars().take(NEMA_LABEL_CHARS).collect();
    let cut = cut.trim_end();
    if cut.is_empty() {
        "-".to_string()
    } else {
        cut.to_string()
    }
}

fn link_label(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Design | LinkKind::Detail => "Refines",
        LinkKind::Code => "Implements",
        LinkKind::Ut | LinkKind::It | LinkKind::St => "VerifiedBy",
        LinkKind::Doc | LinkKind::Target | LinkKind::Other => "Targets",
    }
}

fn annotation_rows(group: Option<&Vec<&ReportAnnotation>>) -> Vec<AnnotationRow> {
    group
        .map(|list| {
            list.iter()
                .map(|a| AnnotationRow {
                    handle: format!("N-{}", a.no),
                    visibility: a.visibility.as_str().to_string(),
                    anchor_state: a.anchor_state.as_str().to_string(),
                    author_kind: a.author_kind.as_str().to_string(),
                    text: sanitized(&a.text),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn event_text(event: &EventRow) -> String {
    let transition = match (event.from_status.is_empty(), event.to_status.is_empty()) {
        (true, true) => String::new(),
        _ => format!(
            "，{} -> {}",
            if event.from_status.is_empty() {
                "（无）"
            } else {
                event.from_status.as_str()
            },
            if event.to_status.is_empty() {
                "（无）"
            } else {
                event.to_status.as_str()
            }
        ),
    };
    format!(
        "#{} {}{}，执行者 {} ({})，{}",
        event.seq,
        md_inline(&event.kind),
        md_inline(&transition),
        md_inline(&event.actor),
        md_inline(&event.actor_kind),
        md_inline(&event.timestamp)
    )
}

fn note_text(note: &NoteRow) -> String {
    format!("{}: {}", md_inline(&note.subject), md_inline(&note.detail))
}

fn chain_text(chain: &ChainRow) -> String {
    if chain.ok {
        format!("通过（截止点之前 {} 条事件）", chain.events)
    } else {
        format!(
            "失败：第 {} 条事件的哈希或前序哈希不一致",
            chain.first_break_seq.unwrap_or(0)
        )
    }
}

fn list_or_none(md: &mut String, items: impl Iterator<Item = String>) {
    let mut any = false;
    for item in items {
        any = true;
        line(md, &format!("- {}", item));
    }
    if !any {
        line(md, "（无）");
    }
    line(md, "");
}

fn quote(md: &mut String, text: &str) {
    for row in sanitized(text).split('\n') {
        line(md, &format!("> {}", escape_md(row)));
    }
}

fn line(out: &mut String, text: &str) {
    out.push_str(text);
    out.push('\n');
}

/// Escapes the Markdown characters that could start structure, after single-line flattening.
fn md_inline(text: &str) -> String {
    escape_md(&one_line(text))
}

fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' => out.push(' '),
            '\\' | '`' | '*' | '[' | ']' | '<' | '>' | '|' | '#' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Text on one line: line breaks, tabs, U+3000 and the Unicode line separators become spaces,
/// then the display hazards are replaced by `<U+XXXX>`.
fn one_line(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| match c {
            '\t' | '\r' | '\n' | '\u{3000}' | '\u{2028}' | '\u{2029}' => ' ',
            c => c,
        })
        .collect();
    sanitize_display(&mapped)
}

/// Multi-line text: line endings are normalised to `\n` and the display hazards are replaced.
fn sanitized(text: &str) -> String {
    sanitize_display(&text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Characters of an identifier that may appear in a command line.
fn id_chars(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn internal(detail: &str) -> DocError {
    DocError::new(
        ErrorCode::Internal,
        "导出结果未通过内部校验，未写出任何内容",
        "保留数据库并报告此错误",
    )
    .with_detail(detail)
}

fn dep_cycle() -> DocError {
    DocError::new(
        ErrorCode::DepCycle,
        "依赖成环，无法导出",
        "删除成环的依赖后重试",
    )
}

fn not_found(hint: &str) -> DocError {
    DocError::new(ErrorCode::NotFound, "导出范围中没有可见的对象", hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctask_model::{
        ApprovalDecision, ApprovalKind, EditKind, EvidenceKind, EvidenceResult, EvidenceSource,
        PlanStatus, ProposalState, Risk, TaskState,
    };

    const F1: &str = "0f1e2d3c-4b5a-4968-8778-6a5b4c3d2e1f";
    const F2: &str = "aa000000-0000-4000-8000-0000000000f2";
    const ANCHOR: &str = "a1b2c3d4e5f60718";

    fn all_options() -> ExportOptions {
        ExportOptions {
            scope: Scope::All,
            include_local_only: false,
            include_sensitive: false,
        }
    }

    fn task(
        id: &str,
        no: i64,
        file: &str,
        section: &str,
        title: &str,
        vis: Visibility,
    ) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            task_no: no,
            file_id: file.into(),
            section: section.into(),
            title: title.into(),
            description: String::new(),
            status: TaskState::Working,
            priority: 3,
            proposal_state: ProposalState::Accepted,
            source_kind: SourceKind::Cli,
            created_by_kind: ActorKind::Human,
            visibility: vis,
            anchor_state: AnchorState::Valid,
            owner: "owner".into(),
            cancel_reason: String::new(),
            closed_at: None,
        }
    }

    fn plan(id: &str, task_id: &str, plan_no: i64, summary: &str) -> PlanRecord {
        PlanRecord {
            id: id.into(),
            task_id: task_id.into(),
            plan_no,
            summary: summary.into(),
            rationale: String::new(),
            impact: String::new(),
            risk: Risk::High,
            options: Vec::new(),
            chosen_option: String::new(),
            edit_kind: EditKind::ReplaceSection,
            target_file_id: F1.into(),
            target_section: "H1:需求#1".into(),
            new_text: String::new(),
            expected_version_id: String::new(),
            expected_hash: "e".repeat(64),
            result_hash: "f".repeat(64),
            plan_hash: "1".repeat(64),
            status: PlanStatus::Applied,
            submitted_by_kind: ActorKind::Agent,
            created_at: "2026-11-24T09:00:00Z".into(),
        }
    }

    fn base_input() -> ReportInput {
        ReportInput {
            generated_at: "2026-11-25T10:00:00Z".into(),
            files: vec![
                ReportFile {
                    id: F1.into(),
                    path: "docs/spec.md".into(),
                    sensitive: false,
                },
                ReportFile {
                    id: F2.into(),
                    path: "tests/x.rs".into(),
                    sensitive: false,
                },
            ],
            clauses: vec![ReportClause {
                file_id: F1.into(),
                section: "H1:需求#1".into(),
                anchor_id: ANCHOR.into(),
                ordinal: 1,
                number: "SPEC-1".into(),
                title: "需求".into(),
            }],
            tasks: vec![
                task("t1", 1, F1, "H1:需求#1", "同步流程", Visibility::Shared),
                task("t2", 2, F1, "H2:其他#9", "复测流程", Visibility::Shared),
            ],
            links: vec![
                ReportLink {
                    task_id: "t1".into(),
                    kind: LinkKind::Design,
                    target_kind: TargetKind::File,
                    target: "docs/spec.md".into(),
                    target_file_id: F1.into(),
                    label: "设计".into(),
                },
                ReportLink {
                    task_id: "t1".into(),
                    kind: LinkKind::Ut,
                    target_kind: TargetKind::File,
                    target: "tests/x.rs".into(),
                    target_file_id: F2.into(),
                    label: "用例".into(),
                },
            ],
            dependencies: vec![ReportDependency {
                prerequisite: "t1".into(),
                dependent: "t2".into(),
            }],
            ..ReportInput::default()
        }
    }

    /// Builds a chain of events from (kind, task id, file id, subject id) tuples, numbered from 1.
    fn chain(specs: &[(&str, &str, &str, &str)]) -> Vec<EventRecord> {
        let mut events: Vec<EventRecord> = Vec::new();
        let mut prev = GENESIS_PREV_HASH.to_string();
        for (index, (kind, task_id, file_id, subject)) in specs.iter().enumerate() {
            let seq = index as i64 + 1;
            let mut event = EventRecord {
                seq,
                id: format!("ev-{}", seq),
                task_id: task_id.to_string(),
                file_id: file_id.to_string(),
                subject_kind: "task".into(),
                subject_id: subject.to_string(),
                kind: kind.to_string(),
                from_status: String::new(),
                to_status: String::new(),
                actor: "owner".into(),
                actor_kind: "human".into(),
                session: String::new(),
                claimed_session: String::new(),
                payload_json: "{}".into(),
                timestamp: format!("2026-11-25T10:00:{:02}Z", seq),
                prev_hash: prev.clone(),
                hash: String::new(),
            };
            event.hash = event.computed_hash();
            prev = event.hash.clone();
            events.push(event);
        }
        events
    }

    fn json_value(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("the JSON report parses")
    }

    fn nema_edges(text: &str) -> Vec<Dependency> {
        parse_graph_edges(text).expect("the NEMA text parses")
    }

    /// A node row starts with its token, which holds no `>`; header and edge rows do not qualify.
    fn is_node_row(line: &str) -> bool {
        !line.starts_with(' ')
            && !line.is_empty()
            && !line.split(' ').next().unwrap_or("").contains('>')
    }

    fn node_tokens(text: &str) -> Vec<String> {
        text.lines()
            .filter(|l| is_node_row(l))
            .map(|l| l.split(' ').next().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn ut_153_nema_roundtrip() {
        tc920_nema_roundtrip_and_batches();
        tc921_nema_label_sanitising();
        tc922_node_token_rules();
        tc923_dependency_errors();
    }

    #[test]
    fn ut_154_report_determinism() {
        tc924_repeat_export_identical();
        tc925_cutoff_ignores_export_event();
        tc926_local_only_omitted();
        tc927_gated_file_omitted();
        tc928_rollback_chain_and_restore_command();
        tc929_chain_check_and_empty_chain();
    }

    fn tc920_nema_roundtrip_and_batches() {
        let text = render_nema(&base_input(), &all_options()).unwrap();
        assert!(text.starts_with(" \\>1 1\n"));
        assert!(text.contains("\n \\>2 1\n"));
        let edges = nema_edges(&text);
        let rows: Vec<(String, String, String, String)> = edges
            .iter()
            .map(|e| {
                (
                    e.from.clone(),
                    e.edge_id.clone(),
                    e.to.clone(),
                    e.label.clone(),
                )
            })
            .collect();
        let expected = vec![
            ("C-a1b2c3d4", "E1", "T-1", "Traces"),
            ("T-1", "E2", "F-0f1e2d3c", "Refines"),
            ("T-1", "E3", "F-aa000000", "VerifiedBy"),
            ("T-1", "E4", "T-2", "Blocks"),
        ];
        let expected: Vec<(String, String, String, String)> = expected
            .into_iter()
            .map(|(a, b, c, d)| (a.to_string(), b.to_string(), c.to_string(), d.to_string()))
            .collect();
        assert_eq!(rows, expected);

        let all = node_tokens(&text);
        assert_eq!(all.len(), 5);
        topo_batches(&all, &edges).unwrap();
        let tasks = vec!["T-1".to_string(), "T-2".to_string()];
        let blocks: Vec<Dependency> = edges
            .iter()
            .filter(|e| e.label == "Blocks")
            .cloned()
            .collect();
        assert_eq!(
            topo_batches(&tasks, &blocks).unwrap(),
            vec![vec!["T-1".to_string()], vec!["T-2".to_string()]]
        );
    }

    fn tc921_nema_label_sanitising() {
        let mut input = base_input();
        input.tasks[0].title = "甲\t乙\u{3000}丙\n丁\r戊\u{1b}\u{202e}".into();
        input.tasks.push(task(
            "t3",
            3,
            F1,
            "H2:其他#9",
            &"字".repeat(120),
            Visibility::Shared,
        ));
        input
            .tasks
            .push(task("t4", 4, F1, "H2:其他#9", "", Visibility::Shared));
        let text = render_nema(&input, &all_options()).unwrap();
        nema_edges(&text);
        let rows: Vec<&str> = text.lines().filter(|l| is_node_row(l)).collect();
        for row in &rows {
            let (token, label) = row.split_once(' ').unwrap();
            assert!(
                token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{}",
                token
            );
            assert!(
                !label.contains('\t') && !label.contains('\u{3000}'),
                "{}",
                row
            );
            assert!(label.chars().count() <= NEMA_LABEL_CHARS, "{}", row);
        }
        let t1 = rows.iter().find(|r| r.starts_with("T-1 ")).unwrap();
        assert!(t1.contains("甲 乙 丙 丁 戊"), "{}", t1);
        assert!(t1.contains("<U+001B>") && t1.contains("<U+202E>"), "{}", t1);
        let t3 = rows.iter().find(|r| r.starts_with("T-3 ")).unwrap();
        assert_eq!(
            t3.split_once(' ').unwrap().1.chars().count(),
            NEMA_LABEL_CHARS
        );
        assert!(rows.iter().any(|r| *r == "T-4 -"));
        assert_eq!(one_line("a\u{3000}b\tc"), "a b c");
        assert_eq!(escape_md("a|b#c<d>"), "a\\|b\\#c\\<d\\>");
    }

    fn tc922_node_token_rules() {
        let ids: BTreeSet<String> = [
            "abcdef0123456789aaaa",
            "abcdef0123459999aaaa",
            "1f-2e_3d:4c",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let tokens = assign_tokens("C", &ids).unwrap();
        assert_eq!(tokens["abcdef0123456789aaaa"], "C-abcdef0123456789");
        assert_eq!(tokens["abcdef0123459999aaaa"], "C-abcdef0123459999");
        assert_eq!(tokens["1f-2e_3d:4c"], "C-1f2e3d4c");

        let file: BTreeSet<String> = ["F-1".to_string()].into_iter().collect();
        assert_eq!(assign_tokens("F", &file).unwrap()["F-1"], "F-f1");

        let no_digits: BTreeSet<String> = ["zz".to_string()].into_iter().collect();
        assert_eq!(
            assign_tokens("F", &no_digits).unwrap_err().code,
            ErrorCode::Internal
        );

        let same: BTreeSet<String> = ["ab-cd".to_string(), "abcd".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            assign_tokens("C", &same).unwrap_err().code,
            ErrorCode::Internal
        );
    }

    fn tc923_dependency_errors() {
        let mut cycle = base_input();
        cycle.dependencies.push(ReportDependency {
            prerequisite: "t2".into(),
            dependent: "t1".into(),
        });
        assert_eq!(
            render_nema(&cycle, &all_options()).unwrap_err().code,
            ErrorCode::DepCycle
        );
        assert_eq!(
            render_markdown(&cycle, &all_options()).unwrap_err().code,
            ErrorCode::DepCycle
        );

        let mut dangling = base_input();
        dangling.dependencies = vec![ReportDependency {
            prerequisite: "ghost".into(),
            dependent: "t2".into(),
        }];
        let json = json_value(&render_json(&dangling, &all_options()).unwrap());
        assert_eq!(json["omitted"]["dependencies"], 1);
        let text = render_nema(&dangling, &all_options()).unwrap();
        assert!(nema_edges(&text).iter().all(|e| e.label != "Blocks"));

        let mut hidden = base_input();
        hidden.tasks[0].visibility = Visibility::LocalOnly;
        let json = json_value(&render_json(&hidden, &all_options()).unwrap());
        assert_eq!(json["omitted"]["dependencies"], 1);

        let mut outside = base_input();
        outside.tasks[1].id = "t2".into();
        let scope = ExportOptions {
            scope: Scope::Task("t2".into()),
            include_local_only: false,
            include_sensitive: false,
        };
        let json = json_value(&render_json(&outside, &scope).unwrap());
        assert_eq!(json["dependencies"][0]["prerequisite"], "T-1");
        assert_eq!(json["dependencies"][0]["dependent"], "T-2");
        assert_eq!(json["omitted"]["dependencies"], 0);
        let text = render_nema(&outside, &scope).unwrap();
        assert!(nema_edges(&text).iter().all(|e| e.label != "Blocks"));
    }

    fn full_input() -> ReportInput {
        let mut input = base_input();
        input.annotations = vec![ReportAnnotation {
            id: "a5".into(),
            no: 5,
            file_id: F1.into(),
            section: "H1:需求#1".into(),
            text: "批注\r\n第二行 <b>".into(),
            visibility: Visibility::Shared,
            author_kind: ActorKind::Human,
            anchor_state: AnchorState::Valid,
        }];
        input.plans = vec![plan("p1", "t1", 1, "替换章节")];
        input.plan_stats = vec![ReportPlanStat {
            plan_id: "p1".into(),
            added: 3,
            removed: 1,
        }];
        input.approvals = vec![ApprovalRecord {
            id: "ap1".into(),
            task_id: "t1".into(),
            kind: ApprovalKind::Plan,
            decision: ApprovalDecision::Approved,
            approver_kind: ActorKind::Human,
            plan_id: "p1".into(),
            subject_hash: "2".repeat(64),
            base_version_id: "ver1".into(),
            comment: "通过".into(),
            event_id: "ev-1".into(),
            created_at: "2026-11-24T10:00:00Z".into(),
        }];
        input.evidence = vec![EvidenceRecord {
            id: "e1".into(),
            task_id: "t1".into(),
            kind: EvidenceKind::TestRun,
            result: EvidenceResult::Pass,
            source: EvidenceSource::Reported,
            actor_kind: ActorKind::Agent,
            created_at: "2026-11-24T11:00:00Z".into(),
        }];
        input.files[0].path = "docs/spec.md".into();
        input.versions = vec![
            ReportVersion {
                id: "ver1".into(),
                file_id: F1.into(),
                version_no: 1,
                content_hash: "3".repeat(64),
                ledger_event_id: "e1".into(),
                created_at: "2026-11-20T10:00:00Z".into(),
            },
            ReportVersion {
                id: "ver2".into(),
                file_id: F1.into(),
                version_no: 2,
                content_hash: "4".repeat(64),
                ledger_event_id: "e2".into(),
                created_at: "2026-11-21T10:00:00Z".into(),
            },
        ];
        input.commits = vec![ReportCommit {
            task_id: "t1".into(),
            plan_id: "p1".into(),
            file_id: F1.into(),
            version_id: "ver2".into(),
            prev_version_id: "ver1".into(),
            ledger_event_id: "e2".into(),
        }];
        input.events = chain(&[
            ("dt.task.created", "t1", "", "t1"),
            ("dt.task.transitioned", "t1", "", "t1"),
            ("dt.annotation.created", "", F1, "a5"),
            ("dt.task.transitioned", "t2", "", "t2"),
            ("dt.plan.submitted", "t1", "", "p1"),
        ]);
        input
    }

    fn tc924_repeat_export_identical() {
        let input = full_input();
        let options = all_options();
        let md = render_markdown(&input, &options).unwrap();
        let js = render_json(&input, &options).unwrap();
        let nema = render_nema(&input, &options).unwrap();
        assert_eq!(render_markdown(&input, &options).unwrap(), md);
        assert_eq!(render_json(&input, &options).unwrap(), js);
        assert_eq!(render_nema(&input, &options).unwrap(), nema);

        let mut reversed = input.clone();
        reversed.tasks.reverse();
        reversed.annotations.reverse();
        reversed.clauses.reverse();
        reversed.links.reverse();
        reversed.dependencies.reverse();
        reversed.versions.reverse();
        reversed.events.reverse();
        reversed.files.reverse();
        assert_eq!(render_markdown(&reversed, &options).unwrap(), md);
        assert_eq!(render_json(&reversed, &options).unwrap(), js);
        assert_eq!(render_nema(&reversed, &options).unwrap(), nema);

        assert!(md.contains("生成时间: 2026-11-25T10:00:00Z"));
        assert!(md.contains("> 第二行 \\<b\\>"));
        assert!(md.contains("+3 -1"));
        let source = include_str!("doctask_report.rs");
        for forbidden in [
            concat!("crate::", "store"),
            concat!("crate::", "engine"),
            concat!("crate::", "doctask_store"),
            concat!("crate::", "doctask_service"),
            concat!("crate::", "doctask_render"),
        ] {
            assert!(!source.contains(forbidden), "{}", forbidden);
        }
    }

    fn tc925_cutoff_ignores_export_event() {
        let mut events = full_input().events;
        events.push(chain_event(6, &events[4].hash, "dt.export.generated", ""));
        let seventh = chain_event(7, &events[5].hash, "dt.export.generated", "");
        events.push(seventh);

        let mut before = full_input();
        before.events.truncate(5);
        let mut after = full_input();
        after.events = events.clone();
        let mut reversed = after.clone();
        reversed.events.reverse();

        let options = all_options();
        let first = render_markdown(&before, &options).unwrap();
        assert_eq!(render_markdown(&after, &options).unwrap(), first);
        assert_eq!(render_markdown(&reversed, &options).unwrap(), first);
        assert!(first.contains("数据截止点 cutoff_seq: 5"));

        for input in [&before, &after, &reversed] {
            let json = json_value(&render_json(input, &options).unwrap());
            assert_eq!(json["cutoff_seq"], 5);
            assert_eq!(json["chain"]["head_hash"], events[4].hash.as_str());
            let timeline = json["timeline"].as_array().unwrap();
            assert!(timeline.iter().all(|e| e["kind"] != "dt.export.generated"));
        }
        assert_eq!(
            render_json(&after, &options).unwrap(),
            render_json(&before, &options).unwrap()
        );
    }

    fn chain_event(seq: i64, prev: &str, kind: &str, task: &str) -> EventRecord {
        let mut event = EventRecord {
            seq,
            id: format!("ev-{}", seq),
            task_id: task.into(),
            file_id: String::new(),
            subject_kind: "export".into(),
            subject_id: String::new(),
            kind: kind.into(),
            from_status: String::new(),
            to_status: String::new(),
            actor: "owner".into(),
            actor_kind: "human".into(),
            session: String::new(),
            claimed_session: String::new(),
            payload_json: "{}".into(),
            timestamp: format!("2026-11-25T11:00:{:02}Z", seq),
            prev_hash: prev.into(),
            hash: String::new(),
        };
        event.hash = event.computed_hash();
        event
    }

    fn tc926_local_only_omitted() {
        let mut input = base_input();
        input.tasks.push(task(
            "t5",
            5,
            F1,
            "H1:需求#1",
            "私密标题",
            Visibility::LocalOnly,
        ));
        input.tasks.push(task(
            "t6",
            6,
            F1,
            "H1:需求#1",
            "公开标题",
            Visibility::Shared,
        ));
        input.plans.push(plan("p5", "t5", 1, "私密方案"));
        input.annotations = vec![ReportAnnotation {
            id: "a9".into(),
            no: 9,
            file_id: F1.into(),
            section: "H1:需求#1".into(),
            text: "私密批注".into(),
            visibility: Visibility::LocalOnly,
            author_kind: ActorKind::Human,
            anchor_state: AnchorState::Valid,
        }];

        let options = all_options();
        let md = render_markdown(&input, &options).unwrap();
        let js = render_json(&input, &options).unwrap();
        let nema = render_nema(&input, &options).unwrap();
        for text in [&md, &js, &nema] {
            assert!(
                !text.contains("私密标题")
                    && !text.contains("私密方案")
                    && !text.contains("私密批注")
            );
        }
        assert!(!md.contains("T-5") && !js.contains("\"T-5\"") && !nema.contains("T-5"));
        assert!(md.contains("公开标题") && nema.contains("公开标题"));
        let json = json_value(&js);
        assert_eq!(json["omitted"]["local_only_tasks"], 1);
        assert_eq!(json["omitted"]["local_only_annotations"], 1);

        let scope = ExportOptions {
            scope: Scope::Task("t5".into()),
            include_local_only: false,
            include_sensitive: false,
        };
        let err = render_markdown(&input, &scope).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(err.hint_zh.contains("--include-local-only"));

        let open = ExportOptions {
            scope: Scope::All,
            include_local_only: true,
            include_sensitive: false,
        };
        let md = render_markdown(&input, &open).unwrap();
        let js = render_nema(&input, &open).unwrap();
        assert!(md.contains("私密标题") && md.contains("私密方案") && md.contains("私密批注"));
        assert!(js.contains("私密标题"));
        let scoped = ExportOptions {
            scope: Scope::Task("t5".into()),
            include_local_only: true,
            include_sensitive: false,
        };
        assert!(render_markdown(&input, &scoped).is_ok());
    }

    fn tc927_gated_file_omitted() {
        let mut input = base_input();
        input.files = vec![
            ReportFile {
                id: F1.into(),
                path: "secret/keys.md".into(),
                sensitive: true,
            },
            ReportFile {
                id: F2.into(),
                path: "docs/open.md".into(),
                sensitive: false,
            },
        ];
        input.clauses = vec![ReportClause {
            file_id: F1.into(),
            section: "H1:机密#1".into(),
            anchor_id: ANCHOR.into(),
            ordinal: 1,
            number: "SEC-1".into(),
            title: "机密条款".into(),
        }];
        input.tasks = vec![
            task("t7", 7, F1, "H1:机密#1", "机密任务标题", Visibility::Shared),
            task("t8", 8, F2, "H1:公开#1", "公开任务", Visibility::Shared),
        ];
        input.annotations = vec![ReportAnnotation {
            id: "a1".into(),
            no: 1,
            file_id: F1.into(),
            section: "H1:机密#1".into(),
            text: "机密批注".into(),
            visibility: Visibility::Shared,
            author_kind: ActorKind::Human,
            anchor_state: AnchorState::Valid,
        }];
        input.links = vec![ReportLink {
            task_id: "t7".into(),
            kind: LinkKind::Doc,
            target_kind: TargetKind::File,
            target: "secret/keys.md".into(),
            target_file_id: F1.into(),
            label: String::new(),
        }];
        input.dependencies = Vec::new();

        let options = all_options();
        let md = render_markdown(&input, &options).unwrap();
        let js = render_json(&input, &options).unwrap();
        let nema = render_nema(&input, &options).unwrap();
        for text in [&md, &js, &nema] {
            for hidden in [
                "secret/keys.md",
                "机密条款",
                "机密批注",
                "机密任务标题",
                "T-7",
            ] {
                assert!(!text.contains(hidden), "{}", hidden);
            }
            assert!(text.contains("公开任务"));
        }
        let json = json_value(&js);
        assert_eq!(json["omitted"]["gated_tasks"], 1);
        assert_eq!(json["omitted"]["gated_annotations"], 1);

        let file_scope = ExportOptions {
            scope: Scope::File {
                file_id: F1.into(),
                section: None,
            },
            include_local_only: false,
            include_sensitive: false,
        };
        let err = render_markdown(&input, &file_scope).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(err.hint_zh.contains("--include-sensitive"));
        let task_scope = ExportOptions {
            scope: Scope::Task("t7".into()),
            include_local_only: false,
            include_sensitive: false,
        };
        assert_eq!(
            render_markdown(&input, &task_scope).unwrap_err().code,
            ErrorCode::NotFound
        );

        let open = ExportOptions {
            scope: Scope::All,
            include_local_only: false,
            include_sensitive: true,
        };
        let md = render_markdown(&input, &open).unwrap();
        let nema = render_nema(&input, &open).unwrap();
        assert!(
            md.contains("secret/keys.md") && md.contains("机密条款") && md.contains("机密批注")
        );
        assert!(nema.contains("机密任务标题"));
    }

    fn tc928_rollback_chain_and_restore_command() {
        let mut input = base_input();
        input.tasks = vec![
            task("t9", 9, F1, "H1:需求#1", "回滚任务", Visibility::Shared),
            task("t10", 10, F1, "H1:需求#1", "第二任务", Visibility::Shared),
        ];
        input.dependencies = Vec::new();
        input.plans = vec![
            plan("p9", "t9", 1, "第一方案"),
            plan("p10", "t10", 1, "第二方案"),
        ];
        input.versions = vec![
            ReportVersion {
                id: "ver1".into(),
                file_id: F1.into(),
                version_no: 1,
                content_hash: "5".repeat(64),
                ledger_event_id: "e1".into(),
                created_at: "2026-11-20T10:00:00Z".into(),
            },
            ReportVersion {
                id: "ver2".into(),
                file_id: F1.into(),
                version_no: 2,
                content_hash: "6".repeat(64),
                ledger_event_id: "e2".into(),
                created_at: "2026-11-21T10:00:00Z".into(),
            },
            ReportVersion {
                id: "ver3".into(),
                file_id: F1.into(),
                version_no: 3,
                content_hash: "7".repeat(64),
                ledger_event_id: "e3".into(),
                created_at: "2026-11-22T10:00:00Z".into(),
            },
        ];
        input.commits = vec![
            ReportCommit {
                task_id: "t9".into(),
                plan_id: "p9".into(),
                file_id: F1.into(),
                version_id: "ver3".into(),
                prev_version_id: "ver2".into(),
                ledger_event_id: "e3".into(),
            },
            ReportCommit {
                task_id: "t10".into(),
                plan_id: "p10".into(),
                file_id: F1.into(),
                version_id: "ver1".into(),
                prev_version_id: String::new(),
                ledger_event_id: "e1".into(),
            },
        ];

        let options = all_options();
        let md = render_markdown(&input, &options).unwrap();
        assert!(md.contains("- 版本 1: ver1，账本事件 e1"));
        assert!(md.contains("- 版本 3: ver3，账本事件 e3"));
        assert!(md.contains("回到应用前版本: `ladon restore ver2 --approve`"));
        assert!(md.contains("应用于版本 ver1（无前序版本）"));
        assert!(md.contains("账本事件 e3"));

        let json = json_value(&render_json(&input, &options).unwrap());
        let versions = json["rollback"][0]["versions"].as_array().unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[2]["ledger_event_id"], "e3");
        assert_eq!(versions[0]["version_no"], 1);
        let commits = json["commits"].as_array().unwrap();
        assert_eq!(
            commits[0]["restore_command"],
            "ladon restore ver2 --approve"
        );
        assert!(commits[1]["restore_command"].is_null());
    }

    fn tc929_chain_check_and_empty_chain() {
        let options = all_options();
        let mut intact = base_input();
        intact.events = chain(&[
            ("dt.task.created", "t1", "", "t1"),
            ("dt.task.transitioned", "t1", "", "t1"),
            ("dt.plan.submitted", "t1", "", "p1"),
            ("dt.task.transitioned", "t1", "", "t1"),
        ]);
        let json = json_value(&render_json(&intact, &options).unwrap());
        assert_eq!(json["chain"]["ok"], true);
        assert!(json["chain"]["first_break_seq"].is_null());
        assert_eq!(json["chain"]["events"], 4);
        assert_eq!(json["chain"]["head_hash"], intact.events[3].hash.as_str());

        let mut broken = intact.clone();
        broken.events[2].payload_json = "{\"tampered\":true}".into();
        let json = json_value(&render_json(&broken, &options).unwrap());
        assert_eq!(json["chain"]["ok"], false);
        assert_eq!(json["chain"]["first_break_seq"], 3);

        let mut empty = base_input();
        empty.events = Vec::new();
        let json = json_value(&render_json(&empty, &options).unwrap());
        assert_eq!(json["chain"]["ok"], true);
        assert_eq!(json["cutoff_seq"], 0);
        assert_eq!(json["chain"]["events"], 0);
        assert_eq!(json["chain"]["head_hash"], "0".repeat(64).as_str());

        let mut only_export = base_input();
        only_export.events = vec![chain_event(1, GENESIS_PREV_HASH, "dt.export.generated", "")];
        let json = json_value(&render_json(&only_export, &options).unwrap());
        assert_eq!(json["cutoff_seq"], 0);
        assert_eq!(json["chain"]["events"], 0);
        assert_eq!(json["chain"]["ok"], true);
    }
}
