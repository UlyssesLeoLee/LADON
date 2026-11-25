//! Pure text layer of the document-task module: Chinese and English message
//! catalogues, display width, tables, the outline, trace and approval screens,
//! and error rendering. It performs no I/O.

use crate::doctask_model::{
    sanitize_display, ActorKind, AnchorState, DocError, ErrorCode, EvidenceKind, EvidenceResult,
    EvidenceSource, LinkKind, PlanStatus, Risk, TaskState, Visibility, MAX_PLAN_OPTIONS,
};

/// Output language of the screens and error texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    /// Both languages in catalogue order.
    pub const ALL: &'static [Lang] = &[Lang::Zh, Lang::En];

    /// The language tag, `zh` or `en`.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }

    /// The language with the exact tag `text`; any other input is `None`.
    pub fn parse(text: &str) -> Option<Lang> {
        match text {
            "zh" => Some(Lang::Zh),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

/// Whether the output may carry escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plain,
    Color,
}

/// `Color` when the standard output is a console and `NO_COLOR` is not set.
pub fn detect_mode(stdout_is_console: bool, no_color: bool) -> Mode {
    if stdout_is_console && !no_color {
        Mode::Color
    } else {
        Mode::Plain
    }
}

type Pairs = &'static [(&'static str, &'static str)];

/// The key and text pairs of one language.
pub fn catalog(lang: Lang) -> Pairs {
    match lang {
        Lang::Zh => ZH,
        Lang::En => EN,
    }
}

/// The catalogue text of `key`, or the key itself when it is absent.
pub fn text<'a>(lang: Lang, key: &'a str) -> &'a str {
    catalog(lang)
        .iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
        .unwrap_or(key)
}

/// Replaces each `{name}` that has a value in `values`, in one left-to-right pass.
/// Substituted text is not scanned again, and a name without a value stays literal.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match values.iter().find(|entry| entry.0 == name) {
                    Some(entry) => out.push_str(entry.1),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Display columns of one character: 2 inside the East Asian wide ranges, 1 otherwise.
pub fn char_width(c: char) -> usize {
    let code = c as u32;
    let wide = matches!(
        code,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
    );
    if wide {
        2
    } else {
        1
    }
}

/// Display columns of a text.
pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Appends spaces until the display width reaches `width`. A longer text is returned unchanged.
pub fn pad_to(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    for _ in display_width(text)..width {
        out.push(' ');
    }
    out
}

/// A value on one table line: sanitised, with newlines and tabs replaced by spaces.
pub fn cell(text: &str) -> String {
    sanitize_display(text)
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect()
}

/// The first twelve characters of a sanitised hash, or the whole value when it is shorter.
pub fn short_hash(hash: &str) -> String {
    sanitize_display(hash).chars().take(12).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Heading,
    Error,
    Dim,
}

fn paint(mode: Mode, style: Style, text: &str) -> String {
    match mode {
        Mode::Plain => text.to_string(),
        Mode::Color => {
            let open = match style {
                Style::Heading => "\u{1b}[1m",
                Style::Error => "\u{1b}[1;31m",
                Style::Dim => "\u{1b}[2m",
            };
            format!("{open}{text}\u{1b}[0m")
        }
    }
}

/// A table with a header line, a rule of hyphens and one line per row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Renders the table. Rows are fitted to the header count, columns are
    /// padded by display width, and no line ends with a space.
    pub fn render(&self, mode: Mode) -> String {
        let columns = self.headers.len();
        if columns == 0 {
            return String::new();
        }
        let headers: Vec<String> = self.headers.iter().map(|h| cell(h)).collect();
        let rows: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|row| {
                (0..columns)
                    .map(|i| row.get(i).map(|c| cell(c)).unwrap_or_default())
                    .collect()
            })
            .collect();
        let widths: Vec<usize> = (0..columns)
            .map(|i| {
                rows.iter()
                    .map(|row| display_width(&row[i]))
                    .chain(std::iter::once(display_width(&headers[i])))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let join = |cells: &[String]| -> String {
            let parts: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if i + 1 == columns {
                        c.clone()
                    } else {
                        pad_to(c, widths[i])
                    }
                })
                .collect();
            parts.join("  ").trim_end().to_string()
        };
        let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        let mut lines = vec![
            paint(mode, Style::Heading, &join(headers.as_slice())),
            paint(mode, Style::Dim, &rule.join("  ")),
        ];
        for row in &rows {
            lines.push(join(row.as_slice()));
        }
        lines.join("\n")
    }
}

fn error_key(code: ErrorCode, part: &str) -> String {
    format!("err.{}.{}", code.as_str(), part)
}

fn error_text(error: &DocError, lang: Lang, part: &str) -> String {
    let own = match (lang, part) {
        (Lang::Zh, "message") => error.message_zh.as_str(),
        (Lang::Zh, "hint") => error.hint_zh.as_str(),
        _ => "",
    };
    if own.trim().is_empty() {
        cell(text(lang, &error_key(error.code, part)))
    } else {
        cell(own)
    }
}

/// Three lines: the error code with its message, the hint, and the detail when present.
/// In `Zh` the error's own Chinese texts are used; in `En` the catalogue texts of the code are used.
pub fn render_error(error: &DocError, lang: Lang, mode: Mode) -> String {
    let label = paint(mode, Style::Error, text(lang, "common.error_label"));
    let mut lines = vec![format!(
        "{} {}: {}",
        label,
        error.code.as_str(),
        error_text(error, lang, "message")
    )];
    lines.push(format!(
        "{}: {}",
        text(lang, "common.hint_label"),
        error_text(error, lang, "hint")
    ));
    if !error.detail.trim().is_empty() {
        lines.push(format!(
            "{}: {}",
            text(lang, "common.detail_label"),
            cell(&error.detail)
        ));
    }
    lines.join("\n")
}

fn heading(mode: Mode, lang: Lang, key: &str) -> String {
    paint(mode, Style::Heading, text(lang, key))
}

fn label(lang: Lang, family: &str, value: &str) -> String {
    text(lang, &format!("{family}.{value}")).to_string()
}

fn join_list(lang: Lang, items: &[String]) -> String {
    if items.is_empty() {
        return text(lang, "common.none").to_string();
    }
    let sep = text(lang, "common.list_sep");
    items
        .iter()
        .map(|item| cell(item))
        .collect::<Vec<String>>()
        .join(sep)
}

/// The approved baseline of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineView {
    pub version: i64,
    pub approved_at: String,
    pub approved_by: String,
}

/// One clause of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRow {
    pub ordinal: usize,
    pub clause: String,
    pub title: String,
    pub first_line: usize,
    pub last_line: usize,
    pub notes: usize,
    pub tasks: usize,
}

/// The outline screen of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineScreen {
    pub file: String,
    pub baseline: Option<BaselineView>,
    pub current_version: i64,
    pub baseline_matches: bool,
    pub rows: Vec<OutlineRow>,
    pub next_step: String,
}

/// Renders the outline screen.
pub fn render_outline(screen: &OutlineScreen, lang: Lang, mode: Mode) -> String {
    let current = format!(
        "{} v{}",
        text(lang, "outline.current"),
        screen.current_version
    );
    let header = match &screen.baseline {
        Some(baseline) => {
            let detail = fill(
                text(lang, "outline.baseline_detail"),
                &[
                    ("at", &cell(&baseline.approved_at)),
                    ("by", &cell(&baseline.approved_by)),
                ],
            );
            let state = if screen.baseline_matches {
                "outline.matches"
            } else {
                "outline.differs"
            };
            format!(
                "{}   {} v{}{}  {}  {}",
                cell(&screen.file),
                text(lang, "outline.baseline"),
                baseline.version,
                detail,
                current,
                text(lang, state)
            )
        }
        None => format!(
            "{}   {}  {}",
            cell(&screen.file),
            current,
            text(lang, "outline.no_baseline")
        ),
    };
    let body = if screen.rows.is_empty() {
        text(lang, "outline.empty").to_string()
    } else {
        let table = Table {
            headers: [
                "outline.col_no",
                "outline.col_clause",
                "outline.col_title",
                "outline.col_lines",
                "outline.col_notes",
                "outline.col_tasks",
            ]
            .iter()
            .map(|key| text(lang, key).to_string())
            .collect(),
            rows: screen
                .rows
                .iter()
                .map(|row| {
                    vec![
                        row.ordinal.to_string(),
                        row.clause.clone(),
                        row.title.clone(),
                        format!("{}-{}", row.first_line, row.last_line),
                        row.notes.to_string(),
                        row.tasks.to_string(),
                    ]
                })
                .collect(),
        };
        table.render(mode)
    };
    let next = paint(
        mode,
        Style::Dim,
        &fill(
            text(lang, "common.next"),
            &[("command", &cell(&screen.next_step))],
        ),
    );
    [paint(mode, Style::Heading, &header), body, next].join("\n")
}

/// An annotation of the traced clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationLine {
    pub handle: String,
    pub text: String,
    pub visibility: Visibility,
}

/// A task that depends on the traced clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLine {
    pub handle: String,
    pub title: String,
    pub status: TaskState,
    pub plan: Option<PlanStatus>,
    pub depends_on: Vec<String>,
}

/// A link of a task to a design, code, test or other target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLine {
    pub kind: LinkKind,
    pub label: String,
    pub target: String,
}

/// An applied plan: its version, its ledger event and its approver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeLine {
    pub plan: String,
    pub version_id: String,
    pub event_id: String,
    pub approver: String,
}

/// A piece of evidence for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceLine {
    pub kind: EvidenceKind,
    pub result: EvidenceResult,
    pub source: EvidenceSource,
    pub summary: String,
}

/// The version sequence of an applied change and the command that restores the version before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackLine {
    pub versions: Vec<String>,
    pub restore_version: String,
}

/// A recent ledger event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLine {
    pub seq: i64,
    pub kind: String,
    pub actor: String,
    pub at: String,
}

/// The trace screen of one clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceScreen {
    pub file: String,
    pub clause: String,
    pub clause_title: String,
    pub anchor: AnchorState,
    pub baseline_bound: bool,
    pub annotations: Vec<AnnotationLine>,
    pub tasks: Vec<TaskLine>,
    pub links: Vec<LinkLine>,
    pub changes: Vec<ChangeLine>,
    pub evidence: Vec<EvidenceLine>,
    pub rollback: Vec<RollbackLine>,
    pub events: Vec<EventLine>,
    pub next_step: String,
}

fn annotation_item(lang: Lang, line: &AnnotationLine) -> String {
    format!(
        "{} [{}] {}",
        cell(&line.handle),
        label(lang, "visibility", line.visibility.as_str()),
        cell(&line.text)
    )
}

fn task_item(lang: Lang, line: &TaskLine) -> String {
    let mut out = format!(
        "{} [{}] {}",
        cell(&line.handle),
        label(lang, "task", line.status.as_str()),
        cell(&line.title)
    );
    if let Some(plan) = line.plan {
        let plan_text = fill(
            text(lang, "trace.task_plan"),
            &[("plan", &label(lang, "plan", plan.as_str()))],
        );
        out.push_str("  ");
        out.push_str(&plan_text);
    }
    if !line.depends_on.is_empty() {
        let deps = fill(
            text(lang, "trace.task_depends"),
            &[("deps", &join_list(lang, &line.depends_on))],
        );
        out.push_str("  ");
        out.push_str(&deps);
    }
    out
}

fn link_item(lang: Lang, line: &LinkLine) -> String {
    format!(
        "[{}] {} -> {}",
        label(lang, "link", line.kind.as_str()),
        cell(&line.label),
        cell(&line.target)
    )
}

fn change_item(lang: Lang, line: &ChangeLine) -> String {
    fill(
        text(lang, "trace.change_line"),
        &[
            ("plan", &cell(&line.plan)),
            ("version", &cell(&line.version_id)),
            ("event", &cell(&line.event_id)),
            ("approver", &cell(&line.approver)),
        ],
    )
}

fn evidence_item(lang: Lang, line: &EvidenceLine) -> String {
    format!(
        "[{}] {} {}: {}",
        label(lang, "evidence.kind", line.kind.as_str()),
        label(lang, "evidence.result", line.result.as_str()),
        label(lang, "evidence.source", line.source.as_str()),
        cell(&line.summary)
    )
}

fn event_item(line: &EventLine) -> String {
    format!(
        "#{} {} {} {}",
        line.seq,
        cell(&line.kind),
        cell(&line.actor),
        cell(&line.at)
    )
}

/// Renders the trace screen: the title, the anchor line, and one section per kind of
/// related record, each with its items or the none text.
pub fn render_trace(screen: &TraceScreen, lang: Lang, mode: Mode) -> String {
    let baseline_key = if screen.baseline_bound {
        "trace.baseline_bound"
    } else {
        "trace.baseline_unbound"
    };
    let mut out = vec![
        paint(
            mode,
            Style::Heading,
            &fill(
                text(lang, "trace.title"),
                &[
                    ("file", &cell(&screen.file)),
                    ("clause", &cell(&screen.clause)),
                    ("title", &cell(&screen.clause_title)),
                ],
            ),
        ),
        fill(
            text(lang, "trace.anchor"),
            &[
                ("anchor", &label(lang, "anchor", screen.anchor.as_str())),
                ("baseline", text(lang, baseline_key)),
            ],
        ),
    ];
    let rollback_items: Vec<String> = screen
        .rollback
        .iter()
        .flat_map(|line| {
            let versions: Vec<String> = line.versions.iter().map(|v| cell(v)).collect();
            [
                versions.join(" -> "),
                fill(
                    text(lang, "trace.rollback_command"),
                    &[("version", &cell(&line.restore_version))],
                ),
            ]
        })
        .collect();
    let sections: Vec<(&str, Vec<String>)> = vec![
        (
            "trace.section.annotations",
            screen
                .annotations
                .iter()
                .map(|line| annotation_item(lang, line))
                .collect(),
        ),
        (
            "trace.section.tasks",
            screen
                .tasks
                .iter()
                .map(|line| task_item(lang, line))
                .collect(),
        ),
        (
            "trace.section.links",
            screen
                .links
                .iter()
                .map(|line| link_item(lang, line))
                .collect(),
        ),
        (
            "trace.section.changes",
            screen
                .changes
                .iter()
                .map(|line| change_item(lang, line))
                .collect(),
        ),
        (
            "trace.section.evidence",
            screen
                .evidence
                .iter()
                .map(|line| evidence_item(lang, line))
                .collect(),
        ),
        ("trace.section.rollback", rollback_items),
        (
            "trace.section.events",
            screen.events.iter().map(event_item).collect(),
        ),
    ];
    for (key, items) in sections {
        out.push(String::new());
        out.push(heading(mode, lang, key));
        if items.is_empty() {
            out.push(format!("  {}", text(lang, "common.none")));
        }
        for item in items {
            out.push(format!("  {item}"));
        }
    }
    out.push(String::new());
    out.push(paint(
        mode,
        Style::Dim,
        &fill(
            text(lang, "common.next"),
            &[("command", &cell(&screen.next_step))],
        ),
    ));
    out.join("\n")
}

/// How the target section of a plan compares with the baseline of its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineCheck {
    Matches,
    Differs,
    NoBaseline,
}

/// The approval screen of one task's plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalScreen {
    pub task: String,
    pub task_title: String,
    pub clause: String,
    pub target_file: String,
    pub target_section: String,
    pub submitted_by: String,
    pub submitted_kind: ActorKind,
    pub session: String,
    pub summary: String,
    pub risk: Risk,
    pub impact: String,
    pub options: Vec<String>,
    pub diff: Vec<String>,
    pub expected_hash: String,
    pub result_hash: String,
    pub baseline: BaselineCheck,
    pub other_plans: Vec<String>,
    pub dependencies: Vec<String>,
    pub anchor: AnchorState,
}

/// Renders the approval screen. The hashes are shown in short form, the baseline
/// check takes a line of its own, and the last line names the task to type.
pub fn render_approval(screen: &ApprovalScreen, lang: Lang, mode: Mode) -> String {
    let mut out = vec![
        paint(
            mode,
            Style::Heading,
            &fill(
                text(lang, "approval.title"),
                &[
                    ("task", &cell(&screen.task)),
                    ("title", &cell(&screen.task_title)),
                ],
            ),
        ),
        fill(
            text(lang, "approval.clause"),
            &[("clause", &cell(&screen.clause))],
        ),
        fill(
            text(lang, "approval.target"),
            &[
                ("file", &cell(&screen.target_file)),
                ("section", &cell(&screen.target_section)),
            ],
        ),
        fill(
            text(lang, "approval.submitted"),
            &[
                ("actor", &cell(&screen.submitted_by)),
                (
                    "kind",
                    &label(lang, "actor", screen.submitted_kind.as_str()),
                ),
            ],
        ),
        fill(
            text(lang, "approval.session"),
            &[("session", &cell(&screen.session))],
        ),
        fill(
            text(lang, "approval.summary"),
            &[("summary", &cell(&screen.summary))],
        ),
        fill(
            text(lang, "approval.risk"),
            &[("risk", &label(lang, "risk", screen.risk.as_str()))],
        ),
        fill(
            text(lang, "approval.impact"),
            &[("impact", &cell(&screen.impact))],
        ),
    ];
    if !screen.options.is_empty() {
        out.push(String::new());
        out.push(heading(mode, lang, "approval.options"));
        for (index, option) in screen.options.iter().take(MAX_PLAN_OPTIONS).enumerate() {
            out.push(format!("  {}. {}", index + 1, cell(option)));
        }
    }
    out.push(String::new());
    out.push(heading(mode, lang, "approval.diff"));
    if screen.diff.is_empty() {
        out.push(format!("  {}", text(lang, "common.none")));
    }
    for line in &screen.diff {
        out.push(cell(line));
    }
    out.push(String::new());
    out.push(fill(
        text(lang, "approval.expected_hash"),
        &[("hash", &short_hash(&screen.expected_hash))],
    ));
    out.push(fill(
        text(lang, "approval.result_hash"),
        &[("hash", &short_hash(&screen.result_hash))],
    ));
    let baseline_key = match screen.baseline {
        BaselineCheck::Matches => "approval.baseline_matches",
        BaselineCheck::Differs => "approval.baseline_differs",
        BaselineCheck::NoBaseline => "approval.baseline_none",
    };
    out.push(text(lang, baseline_key).to_string());
    out.push(fill(
        text(lang, "approval.other_plans"),
        &[("plans", &join_list(lang, &screen.other_plans))],
    ));
    out.push(fill(
        text(lang, "approval.dependencies"),
        &[("deps", &join_list(lang, &screen.dependencies))],
    ));
    out.push(fill(
        text(lang, "approval.anchor"),
        &[("anchor", &label(lang, "anchor", screen.anchor.as_str()))],
    ));
    out.push(paint(
        mode,
        Style::Dim,
        &fill(
            text(lang, "approval.prompt"),
            &[("task", &cell(&screen.task))],
        ),
    ));
    out.join("\n")
}

const ZH: Pairs = &[
    ("common.none", "（无）"),
    ("common.error_label", "错误"),
    ("common.hint_label", "提示"),
    ("common.detail_label", "详情"),
    ("common.next", "下一步: {command}"),
    ("common.list_sep", "、"),
    ("task.open", "打开"),
    ("task.analysis", "分析"),
    ("task.approval", "待批准"),
    ("task.working", "执行中"),
    ("task.verification", "待验收"),
    ("task.done", "已完成"),
    ("task.cancelled", "已取消"),
    ("plan.submitted", "已提交"),
    ("plan.approved", "已批准"),
    ("plan.applying", "应用中"),
    ("plan.applied", "已应用"),
    ("plan.conflicted", "冲突"),
    ("plan.rejected", "已驳回"),
    ("plan.superseded", "已替代"),
    ("anchor.valid", "有效"),
    ("anchor.needs_review", "需确认"),
    ("anchor.relocated", "已重定位"),
    ("anchor.ambiguous", "有歧义"),
    ("anchor.orphaned", "孤立"),
    ("anchor.unknown", "未知"),
    ("anchor.file_missing", "文件缺失"),
    ("anchor.unchecked", "未检查"),
    ("visibility.shared", "共享"),
    ("visibility.local_only", "仅本地"),
    ("link.design", "设计"),
    ("link.detail", "细化"),
    ("link.code", "代码"),
    ("link.ut", "单元测试"),
    ("link.it", "集成测试"),
    ("link.st", "系统测试"),
    ("link.doc", "文档"),
    ("link.target", "目标"),
    ("link.other", "其他"),
    ("evidence.kind.test_run", "测试运行"),
    ("evidence.kind.report", "报告"),
    ("evidence.kind.note", "备注"),
    ("evidence.kind.version", "版本"),
    ("evidence.kind.commit", "提交"),
    ("evidence.kind.agent_change", "Agent 变更"),
    ("evidence.kind.waiver", "书面豁免"),
    ("evidence.result.pass", "通过"),
    ("evidence.result.fail", "失败"),
    ("evidence.result.info", "信息"),
    ("evidence.source.ledger", "账本"),
    ("evidence.source.reported", "Agent 上报"),
    ("risk.low", "低"),
    ("risk.medium", "中"),
    ("risk.high", "高"),
    ("actor.human", "人工"),
    ("actor.agent", "Agent"),
    ("actor.system", "系统"),
    ("actor.unknown", "未知"),
    ("outline.baseline", "已批准"),
    ("outline.baseline_detail", "（{at}，{by}）"),
    ("outline.current", "当前"),
    ("outline.matches", "与基线一致"),
    ("outline.differs", "与基线不同"),
    ("outline.no_baseline", "无基线"),
    ("outline.col_no", "#"),
    ("outline.col_clause", "条款"),
    ("outline.col_title", "标题"),
    ("outline.col_lines", "行范围"),
    ("outline.col_notes", "批注"),
    ("outline.col_tasks", "任务"),
    ("outline.empty", "（无条款）"),
    ("trace.title", "追踪 {file} {clause}  {title}"),
    ("trace.anchor", "锚点 {anchor}，{baseline}"),
    ("trace.baseline_bound", "基线已绑定"),
    ("trace.baseline_unbound", "基线未绑定"),
    ("trace.section.annotations", "批注"),
    ("trace.section.tasks", "下游任务"),
    ("trace.section.links", "关联"),
    ("trace.section.changes", "Agent 变更与提交"),
    ("trace.section.evidence", "测试证据"),
    ("trace.section.rollback", "回滚链"),
    ("trace.section.events", "最近事件"),
    ("trace.task_plan", "方案: {plan}"),
    ("trace.task_depends", "依赖: {deps}"),
    (
        "trace.change_line",
        "方案 {plan}  版本 {version}  事件 {event}  批准人 {approver}",
    ),
    (
        "trace.rollback_command",
        "回滚: ladon restore {version} --approve",
    ),
    ("approval.title", "审批 {task}  {title}"),
    ("approval.clause", "条款: {clause}"),
    ("approval.target", "目标: {file} {section}"),
    ("approval.submitted", "提交者: {actor} ({kind})"),
    ("approval.session", "会话: {session}"),
    ("approval.summary", "方案: {summary}"),
    ("approval.risk", "风险: {risk}"),
    ("approval.impact", "影响: {impact}"),
    ("approval.options", "备选方案"),
    ("approval.diff", "统一差异预览"),
    ("approval.expected_hash", "期望哈希 expected_hash {hash}"),
    ("approval.result_hash", "结果哈希 result_hash {hash}"),
    ("approval.baseline_matches", "目标章节与基线：一致"),
    ("approval.baseline_differs", "目标章节与基线：不同"),
    ("approval.baseline_none", "目标文件无基线"),
    (
        "approval.other_plans",
        "同一文件上的其他已批准方案: {plans}",
    ),
    ("approval.dependencies", "依赖: {deps}"),
    ("approval.anchor", "锚点: {anchor}"),
    (
        "approval.prompt",
        "批准请输入任务号 {task} 并回车；驳回请输入 n 再说明原因；直接回车跳过",
    ),
    ("err.E_INVALID_ARG.message", "参数不合法"),
    ("err.E_INVALID_ARG.hint", "检查参数后重试"),
    ("err.E_NOT_FOUND.message", "未找到对象"),
    ("err.E_NOT_FOUND.hint", "确认任务号、文件或方案存在"),
    ("err.E_AMBIGUOUS.message", "匹配到多个对象"),
    ("err.E_AMBIGUOUS.hint", "使用更具体的编号"),
    ("err.E_SECTION_NOT_FOUND.message", "章节不存在"),
    ("err.E_SECTION_NOT_FOUND.hint", "用 大纲 命令查看可用条款"),
    ("err.E_NO_BASELINE.message", "文件没有基线"),
    ("err.E_NO_BASELINE.hint", "先登记基线"),
    ("err.E_FILE_MISSING.message", "文件已删除或不再跟踪"),
    ("err.E_FILE_MISSING.hint", "恢复文件或重新纳入跟踪"),
    ("err.E_TRANSITION_DENIED.message", "当前状态不允许该迁移"),
    ("err.E_TRANSITION_DENIED.hint", "查看可用的迁移后重试"),
    ("err.E_GUARD_FAILED.message", "前置条件未满足"),
    ("err.E_GUARD_FAILED.hint", "按提示满足前置条件"),
    ("err.E_HUMAN_ONLY.message", "该操作只允许人工执行"),
    ("err.E_HUMAN_ONLY.hint", "由所有者在控制台或命令行执行"),
    ("err.E_NOT_INTERACTIVE.message", "需要真实控制台"),
    ("err.E_NOT_INTERACTIVE.hint", "在 Windows 控制台中运行"),
    ("err.E_POLICY_DENIED.message", "外传门禁不可用"),
    ("err.E_POLICY_DENIED.hint", "检查全局规则文件与规则集"),
    ("err.E_STALE_PLAN.message", "方案已过期"),
    ("err.E_STALE_PLAN.hint", "重新提交方案"),
    ("err.E_STALE_SUBJECT.message", "展示后主体已变化"),
    ("err.E_STALE_SUBJECT.hint", "重新查看后再决定"),
    ("err.E_CONFLICT.message", "写入冲突"),
    ("err.E_CONFLICT.hint", "重新生成方案后再应用"),
    ("err.E_FILE_CHANGED.message", "文件与账本不一致"),
    ("err.E_FILE_CHANGED.hint", "先执行 capture 记录当前内容"),
    ("err.E_DEP_CYCLE.message", "依赖成环"),
    ("err.E_DEP_CYCLE.hint", "移除循环依赖"),
    ("err.E_DEP_DANGLING.message", "依赖目标不存在"),
    ("err.E_DEP_DANGLING.hint", "确认依赖的任务编号"),
    ("err.E_ANCHOR_BROKEN.message", "锚点状态不允许该操作"),
    ("err.E_ANCHOR_BROKEN.hint", "按提示确认、重定位或重定基"),
    ("err.E_UNSUPPORTED_FILE.message", "文件类型不支持该操作"),
    (
        "err.E_UNSUPPORTED_FILE.hint",
        "仅支持 md、markdown、nema、nemat 文件",
    ),
    ("err.E_PLAN_TOO_LARGE.message", "方案或文件超过大小限制"),
    ("err.E_PLAN_TOO_LARGE.hint", "缩小改动范围"),
    ("err.E_QUOTA.message", "超出提案配额"),
    ("err.E_QUOTA.hint", "等待配额释放或请所有者处理"),
    ("err.E_RATE_LIMIT.message", "调用过于频繁"),
    ("err.E_RATE_LIMIT.hint", "稍后重试"),
    ("err.E_STORE_BUSY.message", "数据库正被占用"),
    (
        "err.E_STORE_BUSY.hint",
        "稍后重试，并确认没有其他进程长时间占用",
    ),
    ("err.E_SCHEMA_OUTDATED.message", "数据库结构版本过旧"),
    ("err.E_SCHEMA_OUTDATED.hint", "由所有者执行迁移命令"),
    ("err.E_MIGRATION_BACKUP.message", "迁移前备份无法验证"),
    ("err.E_MIGRATION_BACKUP.hint", "确认备份后由所有者继续"),
    ("err.E_INTEGRITY.message", "校验发现错误"),
    ("err.E_INTEGRITY.hint", "查看校验报告并修复"),
    ("err.E_INTERNAL.message", "内部错误"),
    ("err.E_INTERNAL.hint", "保留现场并报告问题"),
];

const EN: Pairs = &[
    ("common.none", "(none)"),
    ("common.error_label", "Error"),
    ("common.hint_label", "Hint"),
    ("common.detail_label", "Detail"),
    ("common.next", "Next: {command}"),
    ("common.list_sep", ", "),
    ("task.open", "open"),
    ("task.analysis", "analysis"),
    ("task.approval", "approval"),
    ("task.working", "working"),
    ("task.verification", "verification"),
    ("task.done", "done"),
    ("task.cancelled", "cancelled"),
    ("plan.submitted", "submitted"),
    ("plan.approved", "approved"),
    ("plan.applying", "applying"),
    ("plan.applied", "applied"),
    ("plan.conflicted", "conflicted"),
    ("plan.rejected", "rejected"),
    ("plan.superseded", "superseded"),
    ("anchor.valid", "valid"),
    ("anchor.needs_review", "needs review"),
    ("anchor.relocated", "relocated"),
    ("anchor.ambiguous", "ambiguous"),
    ("anchor.orphaned", "orphaned"),
    ("anchor.unknown", "unknown"),
    ("anchor.file_missing", "file missing"),
    ("anchor.unchecked", "unchecked"),
    ("visibility.shared", "shared"),
    ("visibility.local_only", "local only"),
    ("link.design", "design"),
    ("link.detail", "detail"),
    ("link.code", "code"),
    ("link.ut", "unit test"),
    ("link.it", "integration test"),
    ("link.st", "system test"),
    ("link.doc", "document"),
    ("link.target", "target"),
    ("link.other", "other"),
    ("evidence.kind.test_run", "test run"),
    ("evidence.kind.report", "report"),
    ("evidence.kind.note", "note"),
    ("evidence.kind.version", "version"),
    ("evidence.kind.commit", "commit"),
    ("evidence.kind.agent_change", "agent change"),
    ("evidence.kind.waiver", "waiver"),
    ("evidence.result.pass", "pass"),
    ("evidence.result.fail", "fail"),
    ("evidence.result.info", "info"),
    ("evidence.source.ledger", "ledger"),
    ("evidence.source.reported", "reported"),
    ("risk.low", "low"),
    ("risk.medium", "medium"),
    ("risk.high", "high"),
    ("actor.human", "human"),
    ("actor.agent", "agent"),
    ("actor.system", "system"),
    ("actor.unknown", "unknown"),
    ("outline.baseline", "Approved"),
    ("outline.baseline_detail", " ({at}, {by})"),
    ("outline.current", "Current"),
    ("outline.matches", "matches baseline"),
    ("outline.differs", "differs from baseline"),
    ("outline.no_baseline", "no baseline"),
    ("outline.col_no", "#"),
    ("outline.col_clause", "Clause"),
    ("outline.col_title", "Title"),
    ("outline.col_lines", "Lines"),
    ("outline.col_notes", "Notes"),
    ("outline.col_tasks", "Tasks"),
    ("outline.empty", "(no clauses)"),
    ("trace.title", "Trace {file} {clause}  {title}"),
    ("trace.anchor", "anchor {anchor}, {baseline}"),
    ("trace.baseline_bound", "baseline bound"),
    ("trace.baseline_unbound", "baseline not bound"),
    ("trace.section.annotations", "Annotations"),
    ("trace.section.tasks", "Downstream tasks"),
    ("trace.section.links", "Links"),
    ("trace.section.changes", "Agent changes and commits"),
    ("trace.section.evidence", "Test evidence"),
    ("trace.section.rollback", "Rollback chain"),
    ("trace.section.events", "Recent events"),
    ("trace.task_plan", "plan: {plan}"),
    ("trace.task_depends", "depends on: {deps}"),
    (
        "trace.change_line",
        "plan {plan}  version {version}  event {event}  approver {approver}",
    ),
    ("trace.rollback_command", "rollback: ladon restore {version} --approve"),
    ("approval.title", "Approval {task}  {title}"),
    ("approval.clause", "Clause: {clause}"),
    ("approval.target", "Target: {file} {section}"),
    ("approval.submitted", "Submitted by: {actor} ({kind})"),
    ("approval.session", "Session: {session}"),
    ("approval.summary", "Plan: {summary}"),
    ("approval.risk", "Risk: {risk}"),
    ("approval.impact", "Impact: {impact}"),
    ("approval.options", "Options"),
    ("approval.diff", "Unified diff preview"),
    ("approval.expected_hash", "expected_hash {hash}"),
    ("approval.result_hash", "result_hash {hash}"),
    ("approval.baseline_matches", "Target section and baseline match"),
    ("approval.baseline_differs", "Target section and baseline differ"),
    ("approval.baseline_none", "Target file has no baseline"),
    ("approval.other_plans", "Other approved plans on this file: {plans}"),
    ("approval.dependencies", "Dependencies: {deps}"),
    ("approval.anchor", "Anchor: {anchor}"),
    (
        "approval.prompt",
        "To approve, type {task} and press Enter; to reject, type n and then the reason; press Enter alone to skip",
    ),
    ("err.E_INVALID_ARG.message", "Invalid argument"),
    ("err.E_INVALID_ARG.hint", "Check the argument and try again"),
    ("err.E_NOT_FOUND.message", "Not found"),
    ("err.E_NOT_FOUND.hint", "Check that the task, file or plan exists"),
    ("err.E_AMBIGUOUS.message", "Ambiguous reference"),
    ("err.E_AMBIGUOUS.hint", "Use a more specific number"),
    ("err.E_SECTION_NOT_FOUND.message", "Section not found"),
    ("err.E_SECTION_NOT_FOUND.hint", "List the clauses with the outline command"),
    ("err.E_NO_BASELINE.message", "No baseline"),
    ("err.E_NO_BASELINE.hint", "Register a baseline first"),
    ("err.E_FILE_MISSING.message", "File is missing or untracked"),
    ("err.E_FILE_MISSING.hint", "Restore the file or track it again"),
    ("err.E_TRANSITION_DENIED.message", "Transition not allowed"),
    ("err.E_TRANSITION_DENIED.hint", "Check the allowed transitions and retry"),
    ("err.E_GUARD_FAILED.message", "Guard condition not met"),
    ("err.E_GUARD_FAILED.hint", "Meet the listed precondition and retry"),
    ("err.E_HUMAN_ONLY.message", "Human-only operation"),
    (
        "err.E_HUMAN_ONLY.hint",
        "The owner must run this operation in the console or the command line",
    ),
    ("err.E_NOT_INTERACTIVE.message", "Interactive console required"),
    ("err.E_NOT_INTERACTIVE.hint", "Run it in a real Windows console"),
    ("err.E_POLICY_DENIED.message", "Disclosure gate unavailable"),
    ("err.E_POLICY_DENIED.hint", "Check the global rule file and the rule set"),
    ("err.E_STALE_PLAN.message", "Plan is stale"),
    ("err.E_STALE_PLAN.hint", "Resubmit the plan"),
    ("err.E_STALE_SUBJECT.message", "Subject changed after display"),
    ("err.E_STALE_SUBJECT.hint", "Review the subject again before deciding"),
    ("err.E_CONFLICT.message", "Write conflict"),
    ("err.E_CONFLICT.hint", "Regenerate the plan and apply again"),
    ("err.E_FILE_CHANGED.message", "File differs from the ledger"),
    ("err.E_FILE_CHANGED.hint", "Run capture to record the current content first"),
    ("err.E_DEP_CYCLE.message", "Dependency cycle"),
    ("err.E_DEP_CYCLE.hint", "Remove the cyclic dependency"),
    ("err.E_DEP_DANGLING.message", "Dangling dependency"),
    ("err.E_DEP_DANGLING.hint", "Check the dependency task number"),
    ("err.E_ANCHOR_BROKEN.message", "Anchor state does not allow this"),
    (
        "err.E_ANCHOR_BROKEN.hint",
        "Acknowledge, relocate or rebase the anchor as suggested",
    ),
    ("err.E_UNSUPPORTED_FILE.message", "Unsupported file type"),
    (
        "err.E_UNSUPPORTED_FILE.hint",
        "Only md, markdown, nema and nemat files are supported",
    ),
    ("err.E_PLAN_TOO_LARGE.message", "Plan or file too large"),
    ("err.E_PLAN_TOO_LARGE.hint", "Reduce the size of the change"),
    ("err.E_QUOTA.message", "Proposal quota exceeded"),
    ("err.E_QUOTA.hint", "Wait for the quota to free up or ask the owner"),
    ("err.E_RATE_LIMIT.message", "Rate limit reached"),
    ("err.E_RATE_LIMIT.hint", "Retry later"),
    ("err.E_STORE_BUSY.message", "Store is busy"),
    (
        "err.E_STORE_BUSY.hint",
        "Retry later and check that no other process holds it",
    ),
    ("err.E_SCHEMA_OUTDATED.message", "Schema is outdated"),
    ("err.E_SCHEMA_OUTDATED.hint", "The owner must run the migration command"),
    ("err.E_MIGRATION_BACKUP.message", "Migration backup cannot be verified"),
    ("err.E_MIGRATION_BACKUP.hint", "Confirm the backup before the owner continues"),
    ("err.E_INTEGRITY.message", "Integrity check failed"),
    ("err.E_INTEGRITY.hint", "Review the verification report and repair the findings"),
    ("err.E_INTERNAL.message", "Internal error"),
    ("err.E_INTERNAL.hint", "Keep the state and report the problem"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctask_model::{
        ActorKind, AnchorState, DocError, ErrorCode, EvidenceKind, EvidenceResult, EvidenceSource,
        LinkKind, PlanStatus, Risk, TaskState, Visibility,
    };

    fn keys(pairs: Pairs) -> Vec<&'static str> {
        pairs.iter().map(|entry| entry.0).collect()
    }

    fn present(key: &str) -> bool {
        Lang::ALL.iter().all(|lang| {
            catalog(*lang)
                .iter()
                .any(|entry| entry.0 == key && !entry.1.is_empty())
        })
    }

    fn named_slots(template: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            match after.find('}') {
                Some(end) => {
                    names.push(after[..end].to_string());
                    rest = &after[end + 1..];
                }
                None => break,
            }
        }
        names.sort();
        names.dedup();
        names
    }

    fn assert_sgr_only(output: &str) {
        let mut chars = output.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                continue;
            }
            assert_eq!(
                chars.next(),
                Some('['),
                "escape without bracket in {output}"
            );
            loop {
                match chars.next() {
                    Some('m') => break,
                    Some(d) if d.is_ascii_digit() || d == ';' => {}
                    other => panic!("non-SGR escape {other:?} in {output}"),
                }
            }
        }
    }

    fn hostile() -> String {
        "x\u{1b}[31m\u{202E}\u{200B}".to_string()
    }

    fn sample_outline(lang_baseline: bool) -> OutlineScreen {
        OutlineScreen {
            file: "要件定义.md".into(),
            baseline: if lang_baseline {
                Some(BaselineView {
                    version: 12,
                    approved_at: "2026-11-20 10:21".into(),
                    approved_by: "owner".into(),
                })
            } else {
                None
            },
            current_version: 12,
            baseline_matches: true,
            rows: vec![
                OutlineRow {
                    ordinal: 1,
                    clause: "SPEC-11".into(),
                    title: "同步流程".into(),
                    first_line: 3,
                    last_line: 14,
                    notes: 0,
                    tasks: 0,
                },
                OutlineRow {
                    ordinal: 2,
                    clause: "SPEC-12".into(),
                    title: "复测流程".into(),
                    first_line: 15,
                    last_line: 30,
                    notes: 2,
                    tasks: 3,
                },
            ],
            next_step: "批注 要件定义.md #2 \"文字\"".into(),
        }
    }

    fn sample_trace(filled: bool) -> TraceScreen {
        if !filled {
            return TraceScreen {
                file: "要件定义.md".into(),
                clause: "SPEC-12".into(),
                clause_title: "复测流程".into(),
                anchor: AnchorState::Valid,
                baseline_bound: true,
                annotations: vec![],
                tasks: vec![],
                links: vec![],
                changes: vec![],
                evidence: vec![],
                rollback: vec![],
                events: vec![],
                next_step: "追踪 要件定义.md SPEC-12".into(),
            };
        }
        TraceScreen {
            file: "要件定义.md".into(),
            clause: "SPEC-12".into(),
            clause_title: "复测流程".into(),
            anchor: AnchorState::Valid,
            baseline_bound: true,
            annotations: vec![
                AnnotationLine {
                    handle: "N-5".into(),
                    text: "文字".into(),
                    visibility: Visibility::Shared,
                },
                AnnotationLine {
                    handle: "N-6".into(),
                    text: "local note".into(),
                    visibility: Visibility::LocalOnly,
                },
            ],
            tasks: vec![TaskLine {
                handle: "T-12".into(),
                title: "复测流程".into(),
                status: TaskState::Working,
                plan: Some(PlanStatus::Approved),
                depends_on: vec!["T-9".into()],
            }],
            links: vec![LinkLine {
                kind: LinkKind::Ut,
                label: "复测用例".into(),
                target: "tests/x.rs".into(),
            }],
            changes: vec![ChangeLine {
                plan: "P1".into(),
                version_id: "v12".into(),
                event_id: "evt-0042".into(),
                approver: "owner".into(),
            }],
            evidence: vec![EvidenceLine {
                kind: EvidenceKind::TestRun,
                result: EvidenceResult::Pass,
                source: EvidenceSource::Ledger,
                summary: "cargo test".into(),
            }],
            rollback: vec![RollbackLine {
                versions: vec!["v11".into(), "v12".into()],
                restore_version: "v11".into(),
            }],
            events: vec![EventLine {
                seq: 42,
                kind: "dt.commit.applied".into(),
                actor: "owner".into(),
                at: "2026-11-25T10:00:00+09:00".into(),
            }],
            next_step: "追踪 要件定义.md SPEC-12 --events".into(),
        }
    }

    fn sample_approval(baseline: BaselineCheck) -> ApprovalScreen {
        ApprovalScreen {
            task: "T-12".into(),
            task_title: "复测流程".into(),
            clause: "SPEC-12".into(),
            target_file: "要件定义.md".into(),
            target_section: "SPEC-12".into(),
            submitted_by: "hermes".into(),
            submitted_kind: ActorKind::Agent,
            session: "hermes:launch-1".into(),
            summary: "澄清复测步骤".into(),
            risk: Risk::High,
            impact: "复测流程的验收条件".into(),
            options: vec!["保留原文".into(), "拆分为两条".into()],
            diff: vec!["-old".into(), "+new".into()],
            expected_hash: "a".repeat(64),
            result_hash: "b".repeat(64),
            baseline,
            other_plans: vec!["P0".into()],
            dependencies: vec!["T-9".into()],
            anchor: AnchorState::Valid,
        }
    }

    fn sample_error(detail: &str) -> DocError {
        DocError::new(ErrorCode::StalePlan, "方案已过期", "重新提交方案").with_detail(detail)
    }

    fn tc910_display_width_and_padding() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("文档"), 4);
        assert_eq!(display_width("A文B"), 4);
        assert_eq!(display_width("\u{FF0C}\u{3002}"), 4);
        assert_eq!(display_width("\u{FF21}\u{FF22}"), 4);
        assert_eq!(display_width("\u{D55C}"), 2);
        assert_eq!(display_width("\u{E9}"), 1);
        assert_eq!(display_width("\u{1F600}"), 1);
        assert_eq!(display_width(""), 0);
        assert_eq!(char_width('\u{1100}'), 2);
        assert_eq!(char_width('\u{115F}'), 2);
        assert_eq!(char_width('\u{1160}'), 1);
        assert_eq!(char_width('\u{A4CF}'), 2);
        assert_eq!(char_width('\u{A4D0}'), 1);
        assert_eq!(char_width('\u{FFE6}'), 2);
        assert_eq!(char_width('\u{FFE7}'), 1);
        assert_eq!(pad_to("文档", 6), "文档  ");
        assert_eq!(pad_to("abc", 2), "abc");
        assert_eq!(pad_to("文", 3), "文 ");
    }

    fn tc911_table_alignment_and_shape() {
        let table = Table {
            headers: vec!["名称".into(), "Name".into(), "数量".into()],
            rows: vec![
                vec!["文档任务".into(), "a".into(), "3".into()],
                vec!["x".into(), "中英 mix".into(), "12".into()],
                vec!["tab\there".into(), "new\nline".into()],
            ],
        };
        let lines: Vec<String> = table
            .render(Mode::Plain)
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(
            lines,
            vec![
                "名称      Name      数量".to_string(),
                "--------  --------  ----".to_string(),
                "文档任务  a         3".to_string(),
                "x         中英 mix  12".to_string(),
                "tab here  new line".to_string(),
            ]
        );
        for line in &lines {
            assert!(!line.ends_with(' '), "trailing space in {line:?}");
        }
        let short = Table {
            headers: vec!["A".into(), "B".into(), "C".into()],
            rows: vec![vec!["1".into(), "2".into(), "3".into(), "4".into()]],
        };
        assert_eq!(short.render(Mode::Plain), "A  B  C\n-  -  -\n1  2  3");
        let empty = Table {
            headers: vec!["A".into()],
            rows: vec![],
        };
        assert_eq!(empty.render(Mode::Plain), "A\n-");
        let none = Table {
            headers: vec![],
            rows: vec![vec!["x".into()]],
        };
        assert_eq!(none.render(Mode::Plain), "");
    }

    fn tc912_catalogue_key_sets_identical_and_complete() {
        let mut zh = keys(catalog(Lang::Zh));
        let mut en = keys(catalog(Lang::En));
        zh.sort();
        en.sort();
        assert_eq!(zh, en);
        let mut unique = zh.clone();
        unique.dedup();
        assert_eq!(unique.len(), zh.len());
        for lang in Lang::ALL {
            for (key, value) in catalog(*lang) {
                assert!(!value.trim().is_empty(), "empty text for {key}");
            }
        }
        for (key, zh_text) in catalog(Lang::Zh) {
            assert_eq!(
                named_slots(zh_text),
                named_slots(text(Lang::En, key)),
                "named slots differ for {key}"
            );
        }
        for state in TaskState::ALL {
            assert!(present(&format!("task.{}", state.as_str())));
        }
        for state in PlanStatus::ALL {
            assert!(present(&format!("plan.{}", state.as_str())));
        }
        for state in AnchorState::ALL {
            assert!(present(&format!("anchor.{}", state.as_str())));
        }
        for value in Visibility::ALL {
            assert!(present(&format!("visibility.{}", value.as_str())));
        }
        for value in LinkKind::ALL {
            assert!(present(&format!("link.{}", value.as_str())));
        }
        for value in EvidenceKind::ALL {
            assert!(present(&format!("evidence.kind.{}", value.as_str())));
        }
        for value in EvidenceResult::ALL {
            assert!(present(&format!("evidence.result.{}", value.as_str())));
        }
        for value in EvidenceSource::ALL {
            assert!(present(&format!("evidence.source.{}", value.as_str())));
        }
        for value in Risk::ALL {
            assert!(present(&format!("risk.{}", value.as_str())));
        }
        for value in ActorKind::ALL {
            assert!(present(&format!("actor.{}", value.as_str())));
        }
    }

    #[test]
    fn tc913_every_error_code_has_both_languages() {
        let names: Vec<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        for code in ErrorCode::ALL {
            assert!(present(&format!("err.{}.message", code.as_str())));
            assert!(present(&format!("err.{}.hint", code.as_str())));
        }
        for lang in Lang::ALL {
            let error_keys: Vec<&str> = keys(catalog(*lang))
                .into_iter()
                .filter(|key| key.starts_with("err."))
                .collect();
            assert_eq!(error_keys.len(), 54, "error keys in {lang:?}");
            for key in error_keys {
                let body = key.trim_start_matches("err.");
                let code = body
                    .strip_suffix(".message")
                    .or_else(|| body.strip_suffix(".hint"))
                    .expect("error key suffix");
                assert!(names.contains(&code), "orphan key {key}");
            }
        }
    }

    fn tc914_error_rendering_and_sanitisation() {
        let error = sample_error("a\u{1b}[31m\u{202E}x");
        assert_eq!(
            render_error(&error, Lang::Zh, Mode::Plain),
            "错误 E_STALE_PLAN: 方案已过期\n提示: 重新提交方案\n详情: a<U+001B>[31m<U+202E>x"
        );
        let bare = DocError::new(ErrorCode::StalePlan, "", "");
        assert_eq!(
            render_error(&bare, Lang::En, Mode::Plain),
            "Error E_STALE_PLAN: Plan is stale\nHint: Resubmit the plan"
        );
        let zh_fallback = DocError::new(ErrorCode::StalePlan, "方案已过期", "");
        assert_eq!(
            render_error(&zh_fallback, Lang::Zh, Mode::Plain),
            "错误 E_STALE_PLAN: 方案已过期\n提示: 重新提交方案"
        );
        let colour = render_error(&error, Lang::Zh, Mode::Color);
        assert!(colour.starts_with("\u{1b}[1;31m错误\u{1b}[0m E_STALE_PLAN: 方案已过期"));
        assert_eq!(colour.matches('\u{1b}').count(), 2);
        assert!(colour.contains("<U+001B>"));
    }

    fn tc915_outline_screen_golden() {
        let zh = render_outline(&sample_outline(true), Lang::Zh, Mode::Plain);
        assert_eq!(
            zh,
            "要件定义.md   已批准 v12（2026-11-20 10:21，owner）  当前 v12  与基线一致\n\
             #  条款     标题      行范围  批注  任务\n\
             -  -------  --------  ------  ----  ----\n\
             1  SPEC-11  同步流程  3-14    0     0\n\
             2  SPEC-12  复测流程  15-30   2     3\n\
             下一步: 批注 要件定义.md #2 \"文字\""
        );
        let mut differs = sample_outline(true);
        differs.baseline_matches = false;
        differs.rows[0].title = "Sync flow".into();
        differs.rows[1].title = "Retest flow".into();
        let en = render_outline(&differs, Lang::En, Mode::Plain);
        assert_eq!(
            en,
            "要件定义.md   Approved v12 (2026-11-20 10:21, owner)  Current v12  differs from baseline\n\
             #  Clause   Title        Lines  Notes  Tasks\n\
             -  -------  -----------  -----  -----  -----\n\
             1  SPEC-11  Sync flow    3-14   0      0\n\
             2  SPEC-12  Retest flow  15-30  2      3\n\
             Next: 批注 要件定义.md #2 \"文字\""
        );
        let mut empty = sample_outline(false);
        empty.current_version = 3;
        empty.rows.clear();
        empty.next_step = "use 要件定义.md".into();
        assert_eq!(
            render_outline(&empty, Lang::Zh, Mode::Plain),
            "要件定义.md   当前 v3  无基线\n（无条款）\n下一步: use 要件定义.md"
        );
    }

    fn tc916_trace_screen_sections_and_golden() {
        let filled = render_trace(&sample_trace(true), Lang::Zh, Mode::Plain);
        assert_eq!(
            filled,
            "追踪 要件定义.md SPEC-12  复测流程\n\
             锚点 有效，基线已绑定\n\
             \n\
             批注\n\
             \x20 N-5 [共享] 文字\n\
             \x20 N-6 [仅本地] local note\n\
             \n\
             下游任务\n\
             \x20 T-12 [执行中] 复测流程  方案: 已批准  依赖: T-9\n\
             \n\
             关联\n\
             \x20 [单元测试] 复测用例 -> tests/x.rs\n\
             \n\
             Agent 变更与提交\n\
             \x20 方案 P1  版本 v12  事件 evt-0042  批准人 owner\n\
             \n\
             测试证据\n\
             \x20 [测试运行] 通过 账本: cargo test\n\
             \n\
             回滚链\n\
             \x20 v11 -> v12\n\
             \x20 回滚: ladon restore v11 --approve\n\
             \n\
             最近事件\n\
             \x20 #42 dt.commit.applied owner 2026-11-25T10:00:00+09:00\n\
             \n\
             下一步: 追踪 要件定义.md SPEC-12 --events"
        );
        let empty = render_trace(&sample_trace(false), Lang::En, Mode::Plain);
        assert!(empty
            .starts_with("Trace 要件定义.md SPEC-12  复测流程\nanchor valid, baseline bound\n"));
        assert_eq!(empty.matches("  (none)").count(), 7);
        assert!(empty.contains("\nRollback chain\n  (none)\n"));
        assert!(empty.ends_with("Next: 追踪 要件定义.md SPEC-12"));
    }

    fn tc917_approval_screen_hashes_and_baseline_line() {
        let differs = render_approval(
            &sample_approval(BaselineCheck::Differs),
            Lang::Zh,
            Mode::Plain,
        );
        let lines: Vec<&str> = differs.lines().collect();
        assert!(lines.contains(&"期望哈希 expected_hash aaaaaaaaaaaa"));
        assert!(lines.contains(&"结果哈希 result_hash bbbbbbbbbbbb"));
        assert_eq!(lines.iter().filter(|line| line.contains("基线")).count(), 1);
        assert!(lines.contains(&"目标章节与基线：不同"));
        assert!(lines.contains(&"-old"));
        assert!(lines.contains(&"+new"));
        assert!(lines.contains(&"  1. 保留原文"));
        assert!(lines.contains(&"  2. 拆分为两条"));
        assert_eq!(
            lines.last().copied(),
            Some("批准请输入任务号 T-12 并回车；驳回请输入 n 再说明原因；直接回车跳过")
        );
        let mut short = sample_approval(BaselineCheck::NoBaseline);
        short.expected_hash = "abcdef12".into();
        short.result_hash = "x".repeat(64);
        let none = render_approval(&short, Lang::En, Mode::Plain);
        assert!(none.contains("\nexpected_hash abcdef12\n"));
        assert!(none.contains("\nTarget file has no baseline\n"));
        assert!(none.contains("\nresult_hash xxxxxxxxxxxx\n"));
        let zh_none = render_approval(
            &sample_approval(BaselineCheck::NoBaseline),
            Lang::Zh,
            Mode::Plain,
        );
        assert!(zh_none.contains("\n目标文件无基线\n"));
        let mut bare = sample_approval(BaselineCheck::Matches);
        bare.other_plans.clear();
        bare.dependencies.clear();
        let bare_zh = render_approval(&bare, Lang::Zh, Mode::Plain);
        assert!(bare_zh.contains("同一文件上的其他已批准方案: （无）\n"));
        assert!(bare_zh.contains("依赖: （无）\n"));
        let mut many = sample_approval(BaselineCheck::Matches);
        many.options = (1..=6).map(|n| format!("option {n}")).collect();
        let many_out = render_approval(&many, Lang::En, Mode::Plain);
        assert!(many_out.contains("  5. option 5"));
        assert!(!many_out.contains("option 6"));
        let mut escaped = sample_approval(BaselineCheck::Matches);
        escaped.diff = vec!["\u{1b}-old".into()];
        assert!(render_approval(&escaped, Lang::En, Mode::Plain).contains("\n<U+001B>-old\n"));
    }

    fn tc918_plain_mode_has_no_ansi_and_color_is_sgr_only() {
        let mut outline = sample_outline(true);
        outline.rows[0].title = hostile();
        let mut trace = sample_trace(true);
        trace.annotations[0].text = hostile();
        let mut approval = sample_approval(BaselineCheck::Matches);
        approval.summary = hostile();
        approval.diff = vec![hostile()];
        let error = sample_error(&hostile());
        let table = Table {
            headers: vec!["h".into()],
            rows: vec![vec![hostile()]],
        };
        for lang in Lang::ALL {
            let plain = [
                render_outline(&outline, *lang, Mode::Plain),
                render_trace(&trace, *lang, Mode::Plain),
                render_approval(&approval, *lang, Mode::Plain),
                render_error(&error, *lang, Mode::Plain),
                table.render(Mode::Plain),
            ];
            for output in &plain {
                assert!(!output.contains('\u{1b}'), "escape in plain output");
                assert!(!output.contains('\u{202E}'));
                assert!(!output.contains('\u{200B}'));
            }
            let coloured = [
                render_outline(&outline, *lang, Mode::Color),
                render_trace(&trace, *lang, Mode::Color),
                render_approval(&approval, *lang, Mode::Color),
                render_error(&error, *lang, Mode::Color),
                table.render(Mode::Color),
            ];
            for output in &coloured {
                assert_sgr_only(output);
                assert!(!output.contains('\u{202E}'));
                assert!(output.contains("<U+001B>"));
            }
        }
        assert_eq!(detect_mode(true, false), Mode::Color);
        assert_eq!(detect_mode(true, true), Mode::Plain);
        assert_eq!(detect_mode(false, false), Mode::Plain);
        assert_eq!(detect_mode(false, true), Mode::Plain);
    }

    #[test]
    fn tc919_lang_fill_lookup_cell_and_imports() {
        assert_eq!(Lang::parse("zh"), Some(Lang::Zh));
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("EN"), None);
        assert_eq!(Lang::parse("ja"), None);
        assert_eq!(Lang::parse(""), None);
        assert_eq!(Lang::Zh.as_str(), "zh");
        assert_eq!(Lang::En.as_str(), "en");
        assert_eq!(
            fill("{a}-{b}-{c}", &[("a", "{b}"), ("b", "x")]),
            "{b}-x-{c}"
        );
        assert_eq!(text(Lang::Zh, "missing.key"), "missing.key");
        assert_eq!(cell("a\u{202E}\tb\nc"), "a<U+202E> b c");
        assert_eq!(short_hash(&"a".repeat(64)).len(), 12);
        assert_eq!(short_hash(&"a".repeat(11)).len(), 11);
        assert_eq!(short_hash("abc"), "abc");
        assert_eq!(short_hash(&"a".repeat(12)).len(), 12);
        assert_eq!(short_hash(&"b".repeat(13)), "bbbbbbbbbbbb");
        assert_eq!(cell("a\rb"), "a<U+000D>b");
        assert_eq!(fill("a {b", &[("b", "x")]), "a {b");
        let source = include_str!("doctask_render.rs");
        for forbidden in [
            concat!("crate::", "store"),
            concat!("crate::", "engine"),
            concat!("super::", "store"),
            concat!("super::", "engine"),
        ] {
            assert!(!source.contains(forbidden), "forbidden import {forbidden}");
        }
    }

    #[test]
    fn ut_145_control_and_bidi_rejection() {
        tc914_error_rendering_and_sanitisation();
    }

    #[test]
    fn ut_151_cjk_width_and_table_alignment() {
        tc910_display_width_and_padding();
        tc911_table_alignment_and_shape();
    }

    #[test]
    fn ut_152_screen_golden_and_message_keys() {
        tc912_catalogue_key_sets_identical_and_complete();
        tc915_outline_screen_golden();
        tc916_trace_screen_sections_and_golden();
        tc917_approval_screen_hashes_and_baseline_line();
        tc918_plain_mode_has_no_ansi_and_color_is_sgr_only();
    }
}
