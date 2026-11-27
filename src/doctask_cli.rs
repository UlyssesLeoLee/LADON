//! One-shot command layer of the document-task module: the `ladon doc` command tree, the
//! human-only confirmation flow, text and JSON output, exit codes and the legacy `annotate` and
//! `task` helpers. The module holds no SQL. Every read and write goes through the service
//! layers; the console is reached only through the [`DocIo`] seam.
use crate::{
    doctask_flow::{DocFlow, PlanSpec},
    doctask_model::{
        evidence_hash, parse_section_selector, sha256_hex, ActorKind, AnchorState, DocError,
        ErrorCode, LinkKind, ProposalState, SourceKind, TargetKind, TaskRecord, TaskState,
        Visibility,
    },
    doctask_outline::{self, Outline},
    doctask_reconcile::Reconciler,
    doctask_render::{self, Lang, Mode},
    doctask_report::{
        self, ExportOptions, ReportAnnotation, ReportClause, ReportDependency, ReportFile,
        ReportInput, ReportLink, ReportVersion, Scope,
    },
    doctask_service::{
        confirm_yes, hash_prefix_matches, task_content_hash, Caller, DocTaskService, Shown,
        TaskEdit, TaskSpec, TaskView,
    },
    doctask_store::{DocStore, NewEvent},
    engine::Engine,
    model::VersionRecord,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::Duration,
};

/// Actor name used when `--actor` is absent.
const DEFAULT_ACTOR: &str = "human";
/// Rows printed by `doc list` when `--limit` is absent.
const DEFAULT_LIST_LIMIT: usize = 200;
/// Events printed by `doc show` and `doc trace`.
const RECENT_EVENTS: usize = 10;
/// Events printed by `doc history` when `--limit` is absent.
const DEFAULT_HISTORY_LIMIT: usize = 20;

/// The `doc` command line: the options shared by every subcommand, then the subcommand.
#[derive(Args)]
pub struct DocArgs {
    /// Writes one JSON object on standard output instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    /// Writes plain text; no colour is used in any case.
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,
    /// Message language: `zh` or `en`. Defaults to `LADON_LANG` when it is `en`, otherwise `zh`.
    #[arg(long, global = true, value_parser = ["zh", "en"])]
    pub lang: Option<String>,
    /// Name of the human principal written to the chain. Defaults to `human`.
    #[arg(long, global = true)]
    pub actor: Option<String>,
    #[command(subcommand)]
    pub cmd: DocCmd,
}

/// The subcommands of `ladon doc`.
#[derive(Subcommand)]
pub enum DocCmd {
    /// Lists the items waiting for a person; `--watch` refreshes every two seconds.
    Inbox {
        #[arg(long)]
        watch: bool,
    },
    /// Registers, lists and revokes approved baselines.
    #[command(subcommand)]
    Baseline(BaselineCmd),
    /// Prints the clause outline of a file version.
    Outline {
        file: String,
        #[arg(long)]
        version: Option<i64>,
    },
    /// Prints the text of one clause.
    Clause {
        file: String,
        selector: String,
        #[arg(long)]
        version: Option<i64>,
    },
    /// Creates an annotation on a clause of the baseline.
    Note {
        file: String,
        selector: String,
        text: String,
        #[arg(long)]
        private: bool,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        quote: Option<String>,
    },
    /// Lists annotations.
    Notes {
        file: Option<String>,
        #[arg(long)]
        section: Option<String>,
    },
    /// Makes an annotation shared with the agent.
    NoteShare {
        id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Makes an annotation local to the owner.
    NoteUnshare {
        id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Creates a task on a clause.
    New {
        file: String,
        selector: String,
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long)]
        depends: Option<String>,
        #[arg(long)]
        private: bool,
    },
    /// Edits the attributes or the visibility of a task.
    Edit {
        handle: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long, conflicts_with = "share")]
        private: bool,
        #[arg(long)]
        share: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Lists tasks.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        proposed: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Shows the card of one task.
    Show { handle: String },
    /// Shows the traceability screen of a task, or of the tasks of one clause.
    Trace {
        #[arg(required = true, num_args = 1..=2)]
        target: Vec<String>,
    },
    /// Lists the events of the chain.
    History {
        handle: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Moves a task between analysis, working and verification.
    Status {
        handle: String,
        target: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Plans of a task.
    #[command(subcommand)]
    Plan(PlanCmd),
    /// Approves the submitted plan of a task (human only).
    Approve {
        handle: String,
        #[arg(long)]
        option: Option<usize>,
        #[arg(long)]
        comment: Option<String>,
    },
    /// Rejects the submitted plan of a task (human only).
    Reject {
        handle: String,
        #[arg(long)]
        comment: String,
    },
    /// Accepts the verification of a task (human only).
    Close {
        handle: String,
        #[arg(long)]
        waive: Option<String>,
        #[arg(long = "accept-agent-evidence")]
        accept_agent_evidence: Option<String>,
    },
    /// Cancels a task (human only).
    Cancel {
        handle: String,
        #[arg(long)]
        reason: String,
    },
    /// Reopens a cancelled or done task (human only).
    Reopen {
        handle: String,
        #[arg(long)]
        reason: String,
    },
    /// Adopts a task proposed by the agent (human only).
    Adopt {
        handle: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Dismisses a task proposed by the agent (human only).
    Dismiss {
        handle: String,
        #[arg(long)]
        reason: String,
    },
    /// Dependencies between tasks.
    #[command(subcommand)]
    Dep(DepCmd),
    /// Links from a task to designs, commits, URLs or other tasks.
    #[command(subcommand)]
    Link(LinkCmd),
    /// Evidence of a task.
    #[command(subcommand)]
    Evidence(EvidenceCmd),
    /// Prints the anchor report of the files, or runs the check with `--check`.
    Anchors {
        file: Option<String>,
        #[arg(long)]
        check: bool,
    },
    /// Confirms the reviewed clause of a task (human only).
    Ack {
        handle: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Moves the anchor of a task to another clause (human only).
    Relocate {
        handle: String,
        selector: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Rebases the anchor of a task on the current version (human only).
    Rebase {
        handle: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// File-move lineage.
    #[command(subcommand)]
    Lineage(LineageCmd),
    /// Prints the batches and conflicts of the tasks.
    Schedule,
    /// Verifies the chain and the invariants.
    Verify { handle: Option<String> },
    /// Writes the .NEMA traceability graph or the audit report.
    #[command(subcommand)]
    Export(ExportCmd),
    /// Upgrades the schema and reruns the backfill.
    Migrate {
        #[arg(long = "import-legacy")]
        import_legacy: bool,
        #[arg(long = "accept-unverified-backup")]
        accept_unverified_backup: bool,
    },
}

/// Baseline subcommands.
#[derive(Subcommand)]
pub enum BaselineCmd {
    /// Registers a version of a file as an approved baseline (human only).
    Add {
        file: String,
        #[arg(long)]
        version: Option<i64>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Lists baselines.
    List { file: Option<String> },
    /// Revokes a baseline (human only).
    Revoke {
        id: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
}

/// Plan subcommands. Edit options and `--diff` belong to the edit-plan package.
#[derive(Subcommand)]
pub enum PlanCmd {
    /// Submits an analysis-only plan.
    New {
        handle: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        impact: Option<String>,
        #[arg(long, default_value = "medium")]
        risk: String,
        #[arg(long = "option")]
        option: Vec<String>,
    },
    /// Shows the plans of a task.
    Show { handle: String, number: Option<i64> },
}

/// Dependency subcommands.
#[derive(Subcommand)]
pub enum DepCmd {
    /// Makes `handle` depend on `depends_on`.
    Add { handle: String, depends_on: String },
    /// Removes a dependency (human only).
    Rm {
        handle: String,
        depends_on: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Lists the dependencies of a task.
    List { handle: String },
}

/// Link subcommands.
#[derive(Subcommand)]
pub enum LinkCmd {
    /// Adds a link; exactly one of `--file`, `--commit`, `--url` and `--task` is required.
    Add {
        handle: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        section: Option<String>,
        #[arg(long)]
        commit: Option<String>,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        label: Option<String>,
    },
    /// Removes a link by its identifier (human only).
    Rm {
        link_id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Lists the links of a task.
    List { handle: String },
}

/// Evidence subcommands.
#[derive(Subcommand)]
pub enum EvidenceCmd {
    /// Adds evidence to a task.
    Add {
        handle: String,
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "info")]
        result: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// Lists the evidence of a task with its position numbers.
    List { handle: String },
}

/// File-move lineage subcommands.
#[derive(Subcommand)]
pub enum LineageCmd {
    /// Proposes lineage for deleted files.
    Scan,
    /// Lists the lineage rows.
    List,
    /// Confirms a proposed lineage (human only).
    Confirm {
        id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
    /// Rejects a proposed lineage (human only).
    Reject {
        id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long = "expect-hash")]
        expect_hash: Option<String>,
    },
}

/// Export subcommands. Without a scope option the export covers every record.
#[derive(Subcommand)]
pub enum ExportCmd {
    /// Writes the .NEMA traceability graph.
    Nema {
        #[command(flatten)]
        scope: ExportScope,
        #[command(flatten)]
        output: ExportOutput,
    },
    /// Writes the audit report.
    Report {
        #[command(flatten)]
        scope: ExportScope,
        /// Report format: `md` or `json`.
        #[arg(long, default_value = "md", value_parser = ["md", "json"])]
        format: String,
        #[command(flatten)]
        output: ExportOutput,
    },
}

/// The scope of an export.
#[derive(Args)]
pub struct ExportScope {
    /// Exports one task.
    #[arg(long, conflicts_with_all = ["file", "all"])]
    pub task: Option<String>,
    /// Exports the tasks of one file.
    #[arg(long, conflicts_with = "all")]
    pub file: Option<String>,
    /// Restricts `--file` to one section.
    #[arg(long, requires = "file")]
    pub section: Option<String>,
    /// Exports every record (the default).
    #[arg(long)]
    pub all: bool,
}

/// The destination and visibility switches of an export.
#[derive(Args)]
pub struct ExportOutput {
    /// Destination file; the default is `.ladon/exports/<kind>-<time>.<extension>`.
    #[arg(long)]
    pub out: Option<String>,
    /// Replaces an existing destination file.
    #[arg(long)]
    pub force: bool,
    /// Includes `local_only` annotations and tasks.
    #[arg(long = "include-local-only")]
    pub include_local_only: bool,
    /// Includes the content of files that the transfer gate refuses.
    #[arg(long = "include-sensitive")]
    pub include_sensitive: bool,
    /// Time written into the output; defaults to the current local time.
    #[arg(long = "generated-at")]
    pub generated_at: Option<String>,
}

/// The console seam: the standard streams, the console test and the watch pause.
pub trait DocIo {
    /// True when standard input is a real Windows console.
    fn stdin_is_console(&self) -> bool;
    /// True when standard output is a real Windows console.
    fn stdout_is_console(&self) -> bool;
    /// Reads one line without its line terminator; `None` at end of input.
    fn read_line(&mut self) -> Option<String>;
    /// Writes text on standard output.
    fn out(&mut self, text: &str);
    /// Writes text on standard error.
    fn err(&mut self, text: &str);
    /// Waits before the next watch refresh; false stops the watch.
    fn wait_refresh(&mut self) -> bool;
}

/// The process standard streams.
pub struct SystemIo;

#[cfg(windows)]
mod console {
    use std::ffi::c_void;

    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(id: u32) -> *mut c_void;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
    }

    /// True when the standard handle is a console. Pipes and pseudo-terminals fail the query.
    fn is_console(id: u32) -> bool {
        // SAFETY: `GetStdHandle` takes a plain id and returns a handle or an error value; the
        // returned handle is only passed to `GetConsoleMode`, which writes into `mode`.
        unsafe {
            let handle = GetStdHandle(id);
            if handle.is_null() || handle as isize == -1 {
                return false;
            }
            let mut mode = 0u32;
            GetConsoleMode(handle, &mut mode) != 0
        }
    }

    pub fn stdin_is_console() -> bool {
        is_console(STD_INPUT_HANDLE)
    }

    pub fn stdout_is_console() -> bool {
        is_console(STD_OUTPUT_HANDLE)
    }
}

#[cfg(not(windows))]
mod console {
    pub fn stdin_is_console() -> bool {
        false
    }

    pub fn stdout_is_console() -> bool {
        false
    }
}

impl DocIo for SystemIo {
    fn stdin_is_console(&self) -> bool {
        console::stdin_is_console()
    }

    fn stdout_is_console(&self) -> bool {
        console::stdout_is_console()
    }

    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
        }
    }

    fn out(&mut self, text: &str) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn err(&mut self, text: &str) {
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.flush();
    }

    fn wait_refresh(&mut self) -> bool {
        std::thread::sleep(Duration::from_secs(2));
        true
    }
}

/// Maps an error code to the process exit code of design 9.3. The match has no wildcard arm.
pub fn exit_code(code: ErrorCode) -> i32 {
    match code {
        ErrorCode::NotFound | ErrorCode::Ambiguous | ErrorCode::SectionNotFound => 3,
        ErrorCode::InvalidArg
        | ErrorCode::NoBaseline
        | ErrorCode::FileMissing
        | ErrorCode::TransitionDenied
        | ErrorCode::GuardFailed
        | ErrorCode::DepCycle
        | ErrorCode::DepDangling
        | ErrorCode::AnchorBroken
        | ErrorCode::UnsupportedFile
        | ErrorCode::PlanTooLarge
        | ErrorCode::Quota
        | ErrorCode::RateLimit
        | ErrorCode::SchemaOutdated
        | ErrorCode::MigrationBackup => 4,
        ErrorCode::HumanOnly | ErrorCode::NotInteractive => 5,
        ErrorCode::Conflict
        | ErrorCode::StalePlan
        | ErrorCode::StaleSubject
        | ErrorCode::FileChanged => 6,
        ErrorCode::StoreBusy => 7,
        ErrorCode::PolicyDenied => 8,
        ErrorCode::Integrity => 9,
        ErrorCode::Internal => 1,
    }
}

/// The success envelope of `--json`.
pub fn json_ok(data: &Value) -> String {
    json!({"ok": true, "data": data}).to_string()
}

/// The failure envelope of `--json`.
pub fn json_error(error: &DocError) -> String {
    json!({
        "ok": false,
        "code": error.code.as_str(),
        "message": error.message_zh,
        "hint": error.hint_zh,
    })
    .to_string()
}

/// Output of one command: the text lines, the data of the JSON form and the next-step line.
struct Reply {
    lines: Vec<String>,
    data: Value,
    next: Option<String>,
    /// True when the command printed its own output (the watch loop).
    quiet: bool,
    /// A failure whose result is printed as well (`doc verify` with findings).
    fail: Option<ErrorCode>,
}

impl Reply {
    fn new(lines: Vec<String>, data: Value) -> Reply {
        Reply {
            lines,
            data,
            next: None,
            quiet: false,
            fail: None,
        }
    }

    fn next(mut self, text: impl Into<String>) -> Reply {
        self.next = Some(text.into());
        self
    }
}

/// The state of one command: the engine, the principal, the language and the console.
struct Ctx<'a> {
    engine: &'a Engine,
    actor: String,
    lang: Lang,
    mode: Mode,
    json: bool,
    io: &'a mut dyn DocIo,
}

/// Builds an error that the command layer prints and maps to an exit code.
fn fail<T>(code: ErrorCode, message: &str, hint: &str) -> Result<T> {
    Err(DocError::new(code, message, hint).into())
}

impl<'a> Ctx<'a> {
    fn caller(&self) -> Caller {
        Caller::human(&self.actor, SourceKind::Cli)
    }

    fn t(&self, zh: &str, en: &str) -> String {
        match self.lang {
            Lang::Zh => zh.to_string(),
            Lang::En => en.to_string(),
        }
    }

    fn store(&self) -> DocStore {
        DocStore::new(self.engine.store.clone())
    }

    async fn svc(&self) -> Result<DocTaskService<'a>> {
        DocTaskService::attach(self.engine, self.caller()).await
    }

    async fn flow(&self) -> Result<DocFlow<'a>> {
        DocFlow::attach(self.engine, self.caller()).await
    }

    async fn reconciler(&self) -> Result<Reconciler<'a>> {
        Reconciler::attach(self.engine, self.caller()).await
    }

    /// Fails with `E_NOT_INTERACTIVE` unless both standard streams are real consoles.
    fn require_console(&self) -> Result<()> {
        if self.io.stdin_is_console() && self.io.stdout_is_console() {
            return Ok(());
        }
        fail(
            ErrorCode::NotInteractive,
            "此操作只能在真实的控制台中进行",
            "在控制台窗口中运行，管道与伪终端不能确认人工操作",
        )
    }

    fn ask(&mut self, prompt: &str) -> String {
        self.io.err(prompt);
        self.io.read_line().unwrap_or_default()
    }

    fn show_lines(&mut self, lines: &[String]) {
        for line in lines {
            self.io.err(&format!("{line}\n"));
        }
    }

    /// Typed confirmation: the answer must equal the task handle exactly.
    fn confirm_typed(&mut self, handle: &str, lines: &[String]) -> bool {
        self.show_lines(lines);
        let prompt = self.t(
            &format!("输入 {handle} 以确认，其他输入放弃: "),
            &format!("Type {handle} to confirm, anything else abandons: "),
        );
        self.ask(&prompt) == handle
    }

    /// Reason confirmation: the caller has checked that the reason is not empty.
    fn confirm_yes_no(&mut self, lines: &[String]) -> bool {
        self.show_lines(lines);
        let prompt = self.t(
            "确认？输入 y 确认，其他输入放弃: ",
            "Confirm? Type y to confirm, anything else abandons: ",
        );
        self.ask(&prompt) == "y"
    }

    /// Object confirmation: `--yes --expect-hash` confirms the subject shown by its prefix;
    /// otherwise the object is shown and the answer is `y`.
    fn confirm_object(
        &mut self,
        yes: bool,
        expect: Option<&str>,
        subject_hash: &str,
        lines: &[String],
    ) -> Result<bool> {
        confirm_yes(yes, expect)?;
        if yes {
            let prefix = expect.unwrap_or_default();
            if !hash_prefix_matches(prefix, subject_hash) {
                return fail(
                    ErrorCode::StaleSubject,
                    "展示后主体已变化，未写入",
                    "重新运行以查看当前主体与哈希",
                );
            }
            return Ok(true);
        }
        Ok(self.confirm_yes_no(lines))
    }

    fn abandoned(&self) -> Reply {
        Reply::new(
            vec![self.t(
                "已放弃，未写入任何内容。",
                "Abandoned; nothing was written.",
            )],
            json!({"written": false}),
        )
    }

    fn next_hint(&self, zh: &str, en: &str) -> String {
        self.t(zh, en)
    }
}

/// Resolves the language: the `--lang` value, otherwise `LADON_LANG`, otherwise Chinese.
fn resolve_lang(flag: Option<&str>) -> Lang {
    let from_env = std::env::var("LADON_LANG").ok();
    let chosen = flag.or(from_env.as_deref()).unwrap_or("zh");
    Lang::parse(chosen).unwrap_or(Lang::Zh)
}

/// Runs one `doc` command and returns its exit code. A `DocError` is printed (text on standard
/// error, or the envelope on standard output with `--json`) and mapped to its exit code; any
/// other error is returned to the caller.
pub async fn run_doc(engine: &Engine, args: &DocArgs, io: &mut dyn DocIo) -> Result<i32> {
    let lang = resolve_lang(args.lang.as_deref());
    let mode = doctask_render::detect_mode(io.stdout_is_console(), args.no_color);
    let mut cx = Ctx {
        engine,
        actor: args
            .actor
            .clone()
            .unwrap_or_else(|| DEFAULT_ACTOR.to_string()),
        lang,
        mode,
        json: args.json,
        io,
    };
    match Box::pin(dispatch(&mut cx, &args.cmd)).await {
        Ok(reply) => Ok(emit(&mut cx, reply)),
        Err(error) => match error.downcast::<DocError>() {
            Ok(doc) => {
                emit_error(&mut cx, &doc);
                Ok(exit_code(doc.code))
            }
            Err(other) => Err(other),
        },
    }
}

/// Prints a reply and returns the exit code it carries.
fn emit(cx: &mut Ctx<'_>, reply: Reply) -> i32 {
    if reply.quiet {
        return 0;
    }
    if cx.json {
        cx.io.out(&format!("{}\n", json_ok(&reply.data)));
    } else {
        for line in &reply.lines {
            cx.io.out(&format!("{line}\n"));
        }
        if let Some(next) = &reply.next {
            let label = cx.t("下一步", "Next");
            cx.io.out(&format!("{label}: {next}\n"));
        }
    }
    match reply.fail {
        Some(code) => exit_code(code),
        None => 0,
    }
}

/// Prints a `DocError` in the active form.
fn emit_error(cx: &mut Ctx<'_>, error: &DocError) {
    if cx.json {
        cx.io.out(&format!("{}\n", json_error(error)));
    } else {
        let text = doctask_render::render_error(error, cx.lang, cx.mode);
        cx.io.err(&format!("{text}\n"));
    }
}

/// Runs the legacy `annotate` command: a legacy annotation on a clause, with no baseline
/// required. Prints `annotation id: <id>` on standard output.
pub async fn annotate_legacy(
    engine: &Engine,
    actor: &str,
    file: &str,
    section: &str,
    text: &str,
    io: &mut dyn DocIo,
) -> Result<i32> {
    let lang = resolve_lang(None);
    let mode = doctask_render::detect_mode(io.stdout_is_console(), false);
    let mut cx = Ctx {
        engine,
        actor: actor.to_string(),
        lang,
        mode,
        json: false,
        io,
    };
    let result = async {
        let svc = DocTaskService::open(engine, cx.caller()).await?;
        svc.legacy_annotate(file, section, text).await
    }
    .await;
    match result {
        Ok(created) => {
            cx.io.out(&format!("annotation id: {}\n", created.id));
            Ok(0)
        }
        Err(error) => match error.downcast::<DocError>() {
            Ok(doc) => {
                emit_error(&mut cx, &doc);
                Ok(exit_code(doc.code))
            }
            Err(other) => Err(other),
        },
    }
}

/// Runs the legacy `task` command: a legacy task on a clause, accepted and shared, with no
/// baseline required. Prints `task id: <id>` on standard output and the short handle with the
/// next step on standard error.
pub async fn task_legacy(
    engine: &Engine,
    actor: &str,
    file: &str,
    section: &str,
    title: &str,
    description: &str,
    io: &mut dyn DocIo,
) -> Result<i32> {
    let lang = resolve_lang(None);
    let mode = doctask_render::detect_mode(io.stdout_is_console(), false);
    let mut cx = Ctx {
        engine,
        actor: actor.to_string(),
        lang,
        mode,
        json: false,
        io,
    };
    let result = async {
        let svc = DocTaskService::open(engine, cx.caller()).await?;
        svc.legacy_task(file, section, title, description).await
    }
    .await;
    match result {
        Ok(created) => {
            cx.io.out(&format!("task id: {}\n", created.id));
            let hint = match cx.lang {
                Lang::Zh => format!(
                    "任务编号 {}，查看: ladon doc show {}\n",
                    created.handle, created.handle
                ),
                Lang::En => format!(
                    "Task number {}, view: ladon doc show {}\n",
                    created.handle, created.handle
                ),
            };
            cx.io.err(&hint);
            Ok(0)
        }
        Err(error) => match error.downcast::<DocError>() {
            Ok(doc) => {
                emit_error(&mut cx, &doc);
                Ok(exit_code(doc.code))
            }
            Err(other) => Err(other),
        },
    }
}

/// Routes one parsed subcommand to its handler.
async fn dispatch(cx: &mut Ctx<'_>, cmd: &DocCmd) -> Result<Reply> {
    match cmd {
        DocCmd::Inbox { watch } => inbox(cx, *watch).await,
        DocCmd::Baseline(sub) => baseline(cx, sub).await,
        DocCmd::Outline { file, version } => outline(cx, file, *version).await,
        DocCmd::Clause {
            file,
            selector,
            version,
        } => clause(cx, file, selector, *version).await,
        DocCmd::Note {
            file,
            selector,
            text,
            private,
            rationale,
            quote,
        } => {
            note(
                cx,
                file,
                selector,
                text,
                *private,
                rationale.as_deref(),
                quote.as_deref(),
            )
            .await
        }
        DocCmd::Notes { file, section } => notes(cx, file.as_deref(), section.as_deref()).await,
        DocCmd::NoteShare {
            id,
            yes,
            expect_hash,
        } => note_visibility(cx, id, Visibility::Shared, *yes, expect_hash.as_deref()).await,
        DocCmd::NoteUnshare {
            id,
            yes,
            expect_hash,
        } => note_visibility(cx, id, Visibility::LocalOnly, *yes, expect_hash.as_deref()).await,
        DocCmd::New {
            file,
            selector,
            title,
            description,
            owner,
            priority,
            depends,
            private,
        } => {
            new_task(
                cx,
                file,
                selector,
                title,
                description.as_deref(),
                owner.as_deref(),
                *priority,
                depends.as_deref(),
                *private,
            )
            .await
        }
        DocCmd::Edit {
            handle,
            title,
            description,
            owner,
            priority,
            private,
            share,
            yes,
            expect_hash,
        } => {
            let visibility = if *private {
                Some(Visibility::LocalOnly)
            } else if *share {
                Some(Visibility::Shared)
            } else {
                None
            };
            let edit = TaskEdit {
                title: title.clone(),
                description: description.clone(),
                owner: owner.clone(),
                priority: *priority,
                visibility,
            };
            edit_task(cx, handle, &edit, *yes, expect_hash.as_deref()).await
        }
        DocCmd::List {
            status,
            file,
            owner,
            proposed,
            all,
            limit,
        } => {
            list_tasks(
                cx,
                status.as_deref(),
                file.as_deref(),
                owner.as_deref(),
                *proposed,
                *all,
                *limit,
            )
            .await
        }
        DocCmd::Show { handle } => show(cx, handle).await,
        DocCmd::Trace { target } => trace(cx, target).await,
        DocCmd::History {
            handle,
            file,
            limit,
        } => history(cx, handle.as_deref(), file.as_deref(), *limit).await,
        DocCmd::Status {
            handle,
            target,
            reason,
            summary,
        } => status(cx, handle, target, reason.as_deref(), summary.as_deref()).await,
        DocCmd::Plan(sub) => plan(cx, sub).await,
        DocCmd::Approve {
            handle,
            option,
            comment,
        } => approve(cx, handle, *option, comment.as_deref()).await,
        DocCmd::Reject { handle, comment } => reject(cx, handle, comment).await,
        DocCmd::Close {
            handle,
            waive,
            accept_agent_evidence,
        } => {
            close(
                cx,
                handle,
                waive.as_deref(),
                accept_agent_evidence.as_deref(),
            )
            .await
        }
        DocCmd::Cancel { handle, reason } => {
            reason_step(cx, handle, reason, ReasonAction::Cancel).await
        }
        DocCmd::Reopen { handle, reason } => {
            reason_step(cx, handle, reason, ReasonAction::Reopen).await
        }
        DocCmd::Adopt {
            handle,
            yes,
            expect_hash,
        } => adopt(cx, handle, *yes, expect_hash.as_deref()).await,
        DocCmd::Dismiss { handle, reason } => {
            reason_step(cx, handle, reason, ReasonAction::Dismiss).await
        }
        DocCmd::Dep(sub) => dep(cx, sub).await,
        DocCmd::Link(sub) => link(cx, sub).await,
        DocCmd::Evidence(sub) => evidence(cx, sub).await,
        DocCmd::Anchors { file, check } => anchors(cx, file.as_deref(), *check).await,
        DocCmd::Ack {
            handle,
            yes,
            expect_hash,
        } => {
            anchor_decision(
                cx,
                handle,
                AnchorAction::Ack,
                None,
                *yes,
                expect_hash.as_deref(),
            )
            .await
        }
        DocCmd::Relocate {
            handle,
            selector,
            yes,
            expect_hash,
        } => {
            anchor_decision(
                cx,
                handle,
                AnchorAction::Relocate,
                Some(selector),
                *yes,
                expect_hash.as_deref(),
            )
            .await
        }
        DocCmd::Rebase {
            handle,
            yes,
            expect_hash,
        } => {
            anchor_decision(
                cx,
                handle,
                AnchorAction::Rebase,
                None,
                *yes,
                expect_hash.as_deref(),
            )
            .await
        }
        DocCmd::Lineage(sub) => lineage(cx, sub).await,
        DocCmd::Schedule => schedule(cx).await,
        DocCmd::Verify { handle } => verify(cx, handle.as_deref()).await,
        DocCmd::Export(sub) => export(cx, sub).await,
        DocCmd::Migrate {
            import_legacy,
            accept_unverified_backup,
        } => migrate(cx, *import_legacy, *accept_unverified_backup).await,
    }
}

/// The reason-taking step commands.
#[derive(Clone, Copy)]
enum ReasonAction {
    Cancel,
    Reopen,
    Dismiss,
}

/// The anchor decisions of section 5.6.
#[derive(Clone, Copy)]
enum AnchorAction {
    Ack,
    Relocate,
    Rebase,
}

/// The inbox buckets that carry a plan hash.
#[derive(Clone, Copy)]
enum Bucket {
    Approval,
    Verification,
}

fn brief_json(handle: &str, title: &str, status: &str, plan_hash: &str) -> Value {
    json!({"handle": handle, "title": title, "status": status, "plan_hash": plan_hash})
}

fn step_json(step: &crate::doctask_flow::Step) -> Value {
    json!({
        "handle": step.handle,
        "from": step.from,
        "to": step.to,
        "events": step.events,
    })
}

fn step_reply(cx: &Ctx<'_>, step: &crate::doctask_flow::Step) -> Reply {
    let line = format!("{}: {} → {}", step.handle, step.from, step.to);
    Reply::new(vec![line], step_json(step)).next(cx.next_hint(
        &format!("ladon doc show {}", step.handle),
        &format!("ladon doc show {}", step.handle),
    ))
}

fn short(hash: &str) -> String {
    doctask_render::short_hash(hash)
}

/// The canonical handle of a task, `T-n`, after a read through the service.
async fn canonical(svc: &DocTaskService<'_>, handle: &str) -> Result<TaskView> {
    Ok(svc.show_task(handle).await?.task)
}

/// The prerequisites of a task as stored ids, the input of the content hash.
async fn prerequisites(store: &DocStore, task_id: &str) -> Result<Vec<String>> {
    Ok(store
        .dependency_edges()
        .await?
        .into_iter()
        .filter(|(task, _)| task == task_id)
        .map(|(_, depends_on)| depends_on)
        .collect())
}

/// The content hash that the service compares for a task edit or adoption.
async fn content_hash(store: &DocStore, task: &TaskView) -> Result<String> {
    let deps = prerequisites(store, &task.id).await?;
    Ok(task_content_hash(
        &task.title,
        &task.description,
        &task.anchor_id,
        task.priority,
        &deps,
    ))
}

/// The plan hash that the inbox shows for a task in one bucket.
async fn subject_from_inbox(cx: &Ctx<'_>, handle: &str, bucket: Bucket) -> Result<String> {
    let flow = cx.flow().await?;
    let inbox = flow.inbox().await?;
    let rows = match bucket {
        Bucket::Approval => inbox.approvals,
        Bucket::Verification => inbox.verifications,
    };
    match rows.into_iter().find(|row| row.handle == handle) {
        Some(row) => Ok(row.plan_hash),
        None => match bucket {
            Bucket::Approval => fail(
                ErrorCode::TransitionDenied,
                "该任务当前没有待批准的方案",
                "先运行 doc plan new 提交方案",
            ),
            Bucket::Verification => fail(
                ErrorCode::TransitionDenied,
                "该任务当前没有待验收的结果",
                "先运行 doc status 进入 verification",
            ),
        },
    }
}

/// The subject that `DocFlow::close` checks: the evidence hash of the latest plan hash and the
/// ids of the task's evidence. The inbox shows the plan hash alone, so the command computes the
/// same value from the stored records.
async fn verification_subject(cx: &Ctx<'_>, task_id: &str) -> Result<String> {
    let store = cx.store();
    let plan_hash = store
        .plans()
        .await?
        .into_iter()
        .filter(|plan| plan.task_id == task_id)
        .max_by_key(|plan| plan.plan_no)
        .map_or(String::new(), |plan| plan.plan_hash);
    let ids: Vec<String> = store
        .evidence()
        .await?
        .into_iter()
        .filter(|row| row.task_id == task_id)
        .map(|row| row.id)
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    Ok(evidence_hash(&plan_hash, &refs))
}

/// The version that a command addresses: the number given, or the latest version.
async fn pick_version(cx: &Ctx<'_>, file_id: &str, number: Option<i64>) -> Result<VersionRecord> {
    let versions = cx.engine.store.versions(file_id).await?;
    let chosen = match number {
        Some(wanted) => versions.into_iter().find(|v| v.number == wanted),
        None => versions.into_iter().max_by_key(|v| v.number),
    };
    match chosen {
        Some(version) => Ok(version),
        None => fail(
            ErrorCode::NotFound,
            "没有找到该版本",
            "用 doc outline 或 history 查看可用的版本号",
        ),
    }
}

fn extension_of(path: &str) -> &str {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
}

/// The outline of one version of a file.
async fn outline_of(cx: &Ctx<'_>, path: &str, version: &VersionRecord) -> Result<Outline> {
    let bytes = cx.engine.load_version(&version.id, None).await?;
    Ok(doctask_outline::outline(&bytes, extension_of(path)))
}

async fn inbox(cx: &mut Ctx<'_>, watch: bool) -> Result<Reply> {
    loop {
        let flow = cx.flow().await?;
        let inbox = flow.inbox().await?;
        let empty = inbox.approvals.is_empty()
            && inbox.verifications.is_empty()
            && inbox.proposals.is_empty();
        let mut lines = Vec::new();
        let groups = [
            (
                cx.t("待批准方案", "Plans awaiting approval"),
                &inbox.approvals,
            ),
            (
                cx.t("待验收任务", "Tasks awaiting verification"),
                &inbox.verifications,
            ),
            (
                cx.t("待采纳提案", "Proposals awaiting adoption"),
                &inbox.proposals,
            ),
        ];
        for (title, rows) in groups.iter() {
            lines.push(format!("{title}: {}", rows.len()));
            for row in rows.iter() {
                lines.push(format!(
                    "  {}  {}  {}  {}",
                    row.handle,
                    row.status,
                    short(&row.plan_hash),
                    row.title
                ));
            }
        }
        if empty {
            lines.push(cx.t("没有等待处理的事项。", "Nothing is waiting."));
        }
        let data = json!({
            "approvals": inbox.approvals.iter().map(|r| brief_json(&r.handle, &r.title, &r.status, &r.plan_hash)).collect::<Vec<Value>>(),
            "verifications": inbox.verifications.iter().map(|r| brief_json(&r.handle, &r.title, &r.status, &r.plan_hash)).collect::<Vec<Value>>(),
            "proposals": inbox.proposals.iter().map(|r| brief_json(&r.handle, &r.title, &r.status, &r.plan_hash)).collect::<Vec<Value>>(),
        });
        if !watch {
            return Ok(Reply::new(lines, data)
                .next(cx.t("doc show T-n 查看任务卡", "doc show T-n shows a task card")));
        }
        let reply = Reply::new(lines, data);
        emit(cx, reply);
        if !cx.io.wait_refresh() {
            break;
        }
    }
    let mut quiet = Reply::new(Vec::new(), Value::Null);
    quiet.quiet = true;
    Ok(quiet)
}

async fn baseline(cx: &mut Ctx<'_>, sub: &BaselineCmd) -> Result<Reply> {
    match sub {
        BaselineCmd::Add {
            file,
            version,
            note,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let svc = cx.svc().await?;
            let record = svc.resolve_file(file).await?;
            let version = pick_version(cx, &record.id, *version).await?;
            let shown = Shown {
                version_id: version.id.clone(),
                hash: version.content_hash.clone(),
            };
            let lines = vec![format!(
                "{} {} v{} {}",
                cx.t("登记基线", "Register baseline"),
                record.relative_path,
                version.number,
                short(&version.content_hash)
            )];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &version.content_hash, &lines)? {
                return Ok(cx.abandoned());
            }
            let row = svc
                .register_baseline(
                    file,
                    Some(version.number),
                    note.as_deref().unwrap_or(""),
                    &shown,
                )
                .await?;
            let line = format!(
                "{} {} v{} {}",
                cx.t("基线已登记", "Baseline registered"),
                row.id,
                row.version_no,
                short(&row.content_hash)
            );
            let data = json!({"id": row.id, "version_no": row.version_no, "content_hash": row.content_hash});
            Ok(Reply::new(vec![line], data).next(cx.t(
                "doc outline 文件 查看条款",
                "doc outline FILE shows the clauses",
            )))
        }
        BaselineCmd::List { file } => {
            let svc = cx.svc().await?;
            let listing = svc.list_baselines(file.as_deref()).await?;
            let mut lines = Vec::new();
            let mut items = Vec::new();
            for row in &listing.items {
                let state = match &row.revoked_at {
                    Some(_) => cx.t("已撤销", "revoked"),
                    None => cx.t("有效", "active"),
                };
                lines.push(format!(
                    "{}  v{}  {}  {}  {}",
                    row.id,
                    row.version_no,
                    short(&row.content_hash),
                    row.approver,
                    state
                ));
                items.push(json!({
                    "id": row.id,
                    "file_id": row.file_id,
                    "version_id": row.version_id,
                    "version_no": row.version_no,
                    "content_hash": row.content_hash,
                    "approver": row.approver,
                    "revoked": row.revoked_at.is_some(),
                }));
            }
            if listing.hidden > 0 {
                lines.push(format!("{} {}", cx.t("已隐藏", "hidden"), listing.hidden));
            }
            Ok(Reply::new(
                lines,
                json!({"items": items, "hidden": listing.hidden}),
            ))
        }
        BaselineCmd::Revoke {
            id,
            reason,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let svc = cx.svc().await?;
            let listing = svc.list_baselines(None).await?;
            let row = match listing.items.into_iter().find(|r| r.id == *id) {
                Some(row) => row,
                None => {
                    return fail(
                        ErrorCode::NotFound,
                        "没有找到该基线",
                        "用 doc baseline list 查看",
                    )
                }
            };
            let shown = Shown {
                version_id: row.version_id.clone(),
                hash: row.content_hash.clone(),
            };
            let lines = vec![format!(
                "{} {} v{} {}",
                cx.t("撤销基线", "Revoke baseline"),
                row.id,
                row.version_no,
                short(&row.content_hash)
            )];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &row.content_hash, &lines)? {
                return Ok(cx.abandoned());
            }
            let revoked = svc.revoke_baseline(id, reason, &shown).await?;
            let line = format!("{} {}", cx.t("基线已撤销", "Baseline revoked"), revoked.id);
            Ok(Reply::new(vec![line], json!({"id": revoked.id})))
        }
    }
}

async fn outline(cx: &mut Ctx<'_>, file: &str, version: Option<i64>) -> Result<Reply> {
    let svc = cx.svc().await?;
    let record = svc.resolve_file(file).await?;
    let chosen = pick_version(cx, &record.id, version).await?;
    let outline = outline_of(cx, &record.relative_path, &chosen).await?;
    let notes = svc.list_annotations(Some(file)).await?.items;
    let tasks = svc.list_tasks().await?.items;
    let baselines = svc.list_baselines(Some(file)).await?.items;
    let active = baselines
        .iter()
        .filter(|b| b.revoked_at.is_none())
        .max_by_key(|b| b.version_no);
    let status = match active {
        None => cx.t("没有基线", "no baseline"),
        Some(b) if b.version_no == chosen.number => cx.t("与基线一致", "matches the baseline"),
        Some(_) => cx.t("与基线不同", "differs from the baseline"),
    };
    let mut lines = vec![format!(
        "{} v{}  {}",
        record.relative_path, chosen.number, status
    )];
    let mut rows = Vec::new();
    for clause in outline.clauses() {
        let note_count = notes.iter().filter(|n| n.section == clause.key).count();
        let task_count = tasks
            .iter()
            .filter(|t| t.path == record.relative_path && t.section == clause.key)
            .count();
        let number = clause.number.clone().unwrap_or_default();
        lines.push(format!(
            "{:>3}  {:<10} {}  {}-{}  N:{} T:{}",
            clause.ordinal,
            number,
            clause.title,
            clause.first_line,
            clause.last_line,
            note_count,
            task_count
        ));
        rows.push(json!({
            "ordinal": clause.ordinal,
            "key": clause.key,
            "number": number,
            "title": clause.title,
            "first_line": clause.first_line,
            "last_line": clause.last_line,
            "notes": note_count,
            "tasks": task_count,
        }));
    }
    let data = json!({
        "file": record.relative_path,
        "version": chosen.number,
        "status": status,
        "rows": rows,
    });
    Ok(Reply::new(lines, data).next(cx.t(
        "doc clause 文件 选择器 查看条款文本",
        "doc clause FILE SELECTOR shows the clause text",
    )))
}

async fn clause(
    cx: &mut Ctx<'_>,
    file: &str,
    selector: &str,
    version: Option<i64>,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let record = svc.resolve_file(file).await?;
    let chosen = pick_version(cx, &record.id, version).await?;
    let outline = outline_of(cx, &record.relative_path, &chosen).await?;
    let wanted = parse_section_selector(Some(selector))?;
    let found = doctask_outline::resolve(&outline, &wanted)?;
    let bytes = cx.engine.load_version(&chosen.id, None).await?;
    let text = match doctask_outline::clause_bytes(&bytes, found) {
        Some(slice) => String::from_utf8_lossy(slice).to_string(),
        None => {
            return fail(
                ErrorCode::SectionNotFound,
                "条款范围不在版本内容之中",
                "用 doc outline 查看条款范围",
            )
        }
    };
    let number = found.number.clone().unwrap_or_default();
    let data = json!({"number": number, "title": found.title, "text": text});
    Ok(Reply::new(vec![text.trim_end().to_string()], data))
}

#[allow(clippy::too_many_arguments)]
async fn note(
    cx: &mut Ctx<'_>,
    file: &str,
    selector: &str,
    text: &str,
    private: bool,
    rationale: Option<&str>,
    quote: Option<&str>,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let created = svc
        .add_annotation(
            file,
            selector,
            text,
            rationale.unwrap_or(""),
            quote.unwrap_or(""),
            private,
        )
        .await?;
    let verb = if created.existing {
        cx.t("批注已存在", "Annotation already exists")
    } else {
        cx.t("批注已创建", "Annotation created")
    };
    let line = format!("{verb} {}", created.handle);
    Ok(Reply::new(
        vec![line],
        json!({"id": created.id, "handle": created.handle, "existing": created.existing}),
    )
    .next(cx.t("doc notes 列出批注", "doc notes lists annotations")))
}

async fn notes(cx: &mut Ctx<'_>, file: Option<&str>, section: Option<&str>) -> Result<Reply> {
    let svc = cx.svc().await?;
    let listing = svc.list_annotations(file).await?;
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for view in listing
        .items
        .iter()
        .filter(|v| section.map_or(true, |s| v.section == s))
    {
        lines.push(format!(
            "{}  {}  {}  {}  {}",
            view.handle, view.visibility, view.section, view.author, view.text
        ));
        items.push(json!({
            "handle": view.handle,
            "id": view.id,
            "section": view.section,
            "visibility": view.visibility,
            "author": view.author,
            "text": view.text,
        }));
    }
    Ok(Reply::new(
        lines,
        json!({"items": items, "hidden": listing.hidden}),
    ))
}

async fn note_visibility(
    cx: &mut Ctx<'_>,
    id: &str,
    visibility: Visibility,
    yes: bool,
    expect_hash: Option<&str>,
) -> Result<Reply> {
    confirm_yes(yes, expect_hash)?;
    cx.require_console()?;
    let svc = cx.svc().await?;
    let wanted = id.trim_start_matches("N-").trim_start_matches("n-");
    let listing = svc.list_annotations(None).await?;
    let view = match listing
        .items
        .into_iter()
        .find(|v| v.handle.trim_start_matches("N-") == wanted || v.id == id)
    {
        Some(view) => view,
        None => {
            return fail(
                ErrorCode::NotFound,
                "没有找到该批注",
                "用 doc notes 列出批注",
            )
        }
    };
    let text_hash = sha256_hex(view.text.as_bytes());
    let lines = vec![format!(
        "{} {} {}",
        cx.t("批注", "Annotation"),
        view.handle,
        short(&text_hash)
    )];
    if !cx.confirm_object(yes, expect_hash, &text_hash, &lines)? {
        return Ok(cx.abandoned());
    }
    let updated = svc
        .set_annotation_visibility(&view.handle, visibility, &text_hash)
        .await?;
    let line = format!(
        "{} {}: {}",
        cx.t("批注可见性", "Annotation visibility"),
        updated.handle,
        updated.visibility
    );
    Ok(Reply::new(
        vec![line],
        json!({"handle": updated.handle, "visibility": updated.visibility}),
    ))
}

#[allow(clippy::too_many_arguments)]
async fn new_task(
    cx: &mut Ctx<'_>,
    file: &str,
    selector: &str,
    title: &str,
    description: Option<&str>,
    owner: Option<&str>,
    priority: Option<i64>,
    depends: Option<&str>,
    private: bool,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let spec = TaskSpec {
        title: title.to_string(),
        description: description.unwrap_or("").to_string(),
        owner: owner.unwrap_or("").to_string(),
        priority: priority.unwrap_or(2),
        private,
    };
    let created = svc.create_task(file, selector, &spec).await?;
    let mut lines = vec![format!(
        "{} {}",
        if created.existing {
            cx.t("任务已存在", "Task already exists")
        } else {
            cx.t("任务已创建", "Task created")
        },
        created.handle
    )];
    if let Some(list) = depends {
        let flow = cx.flow().await?;
        for prerequisite in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let step = flow.add_dependency(&created.handle, prerequisite).await?;
            lines.push(format!(
                "{} {} → {}",
                cx.t("依赖", "Dependency"),
                step.handle,
                prerequisite
            ));
        }
    }
    let data = json!({"id": created.id, "handle": created.handle, "existing": created.existing});
    Ok(Reply::new(lines, data).next(format!("ladon doc show {}", created.handle)))
}

async fn edit_task(
    cx: &mut Ctx<'_>,
    handle: &str,
    edit: &TaskEdit,
    yes: bool,
    expect_hash: Option<&str>,
) -> Result<Reply> {
    confirm_yes(yes, expect_hash)?;
    if edit.title.is_none()
        && edit.description.is_none()
        && edit.owner.is_none()
        && edit.priority.is_none()
        && edit.visibility.is_none()
    {
        return fail(
            ErrorCode::InvalidArg,
            "没有要修改的字段",
            "给出 --title、--description、--owner、--priority、--private 或 --share 之一",
        );
    }
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let store = cx.store();
    let hash = content_hash(&store, &task).await?;
    let lines = vec![format!(
        "{} {} {}",
        cx.t("编辑任务", "Edit task"),
        task.handle,
        short(&hash)
    )];
    if !cx.confirm_object(yes, expect_hash, &hash, &lines)? {
        return Ok(cx.abandoned());
    }
    let updated = svc.edit_task(&task.handle, edit, &hash).await?;
    let line = format!("{} {}", cx.t("任务已更新", "Task updated"), updated.handle);
    Ok(Reply::new(
        vec![line],
        json!({"handle": updated.handle, "title": updated.title}),
    ))
}

async fn list_tasks(
    cx: &mut Ctx<'_>,
    status: Option<&str>,
    file: Option<&str>,
    owner: Option<&str>,
    proposed: bool,
    all: bool,
    limit: Option<usize>,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let file_record = match file {
        Some(reference) => Some(svc.resolve_file(reference).await?),
        None => None,
    };
    let statuses: Vec<&str> = status
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let listing = svc.list_tasks().await?;
    let mut rows: Vec<TaskView> = listing
        .items
        .into_iter()
        .filter(|t| {
            if proposed {
                t.proposal_state == "proposed"
            } else {
                t.proposal_state == "accepted"
            }
        })
        .filter(|t| all || (t.status != "done" && t.status != "cancelled"))
        .filter(|t| statuses.is_empty() || statuses.contains(&t.status.as_str()))
        .filter(|t| match &file_record {
            Some(record) => t.path == record.relative_path,
            None => true,
        })
        .filter(|t| owner.map_or(true, |o| t.owner == o))
        .collect();
    rows.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.task_no.cmp(&b.task_no)));
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for task in rows.iter().take(limit) {
        lines.push(format!(
            "{}  {}  P{}  {}  {}  {}",
            task.handle, task.status, task.priority, task.path, task.section, task.title
        ));
        items.push(json!({
            "handle": task.handle,
            "status": task.status,
            "priority": task.priority,
            "path": task.path,
            "section": task.section,
            "title": task.title,
            "owner": task.owner,
        }));
    }
    let next = cx.t("doc show T-n 查看任务卡", "doc show T-n shows a task card");
    Ok(Reply::new(lines, json!({"items": items, "hidden": listing.hidden})).next(next))
}

async fn show(cx: &mut Ctx<'_>, handle: &str) -> Result<Reply> {
    let svc = cx.svc().await?;
    let card = svc.show_task(handle).await?;
    let task = &card.task;
    let store = cx.store();
    let evidence = store
        .evidence()
        .await?
        .into_iter()
        .filter(|e| e.task_id == task.id)
        .collect::<Vec<_>>();
    let links = store
        .links()
        .await?
        .into_iter()
        .filter(|l| l.task_id == task.id && l.removed_at.is_none())
        .collect::<Vec<_>>();
    let mut lines = vec![
        format!("{}  {}", task.handle, task.title),
        format!(
            "{} {}  {} {}  {} {}",
            cx.t("状态", "Status"),
            task.status,
            cx.t("提案", "Proposal"),
            task.proposal_state,
            cx.t("可见性", "Visibility"),
            task.visibility
        ),
        format!(
            "{} {} {}  {} {}",
            cx.t("文件", "File"),
            task.path,
            task.section,
            cx.t("锚点", "Anchor"),
            task.anchor_state
        ),
        format!(
            "{} P{}  {} {}",
            cx.t("优先级", "Priority"),
            task.priority,
            cx.t("负责人", "Owner"),
            task.owner
        ),
        format!(
            "{} {}",
            cx.t("阻塞于", "Blocked by"),
            task.blocked_by.join(", ")
        ),
        format!("{} {}", cx.t("描述", "Description"), task.description),
        format!("{} {}", cx.t("证据", "Evidence"), evidence.len()),
        format!("{} {}", cx.t("关联", "Links"), links.len()),
    ];
    let start = card.events.len().saturating_sub(RECENT_EVENTS);
    for event in &card.events[start..] {
        lines.push(format!(
            "  #{} {} {}→{} {}",
            event.seq, event.kind, event.from_status, event.to_status, event.actor
        ));
    }
    let events: Vec<Value> = card.events[start..]
        .iter()
        .map(|e| json!({"seq": e.seq, "kind": e.kind, "from": e.from_status, "to": e.to_status, "actor": e.actor}))
        .collect();
    let data = json!({
        "handle": task.handle,
        "title": task.title,
        "status": task.status,
        "proposal_state": task.proposal_state,
        "visibility": task.visibility,
        "path": task.path,
        "section": task.section,
        "anchor_state": task.anchor_state,
        "priority": task.priority,
        "owner": task.owner,
        "blocked_by": task.blocked_by,
        "evidence": evidence.len(),
        "links": links.len(),
        "events": events,
    });
    let next = cx.t(
        "doc plan show T-n 查看方案",
        "doc plan show T-n shows the plans",
    );
    Ok(Reply::new(lines, data).next(next))
}

async fn trace(cx: &mut Ctx<'_>, target: &[String]) -> Result<Reply> {
    let svc = cx.svc().await?;
    let handles: Vec<String> = if target.len() == 1 {
        vec![canonical(&svc, &target[0]).await?.handle]
    } else {
        let record = svc.resolve_file(&target[0]).await?;
        let wanted = parse_section_selector(Some(&target[1]))?;
        let outline = outline_of(
            cx,
            &record.relative_path,
            &pick_version(cx, &record.id, None).await?,
        )
        .await?;
        let clause = doctask_outline::resolve(&outline, &wanted)?;
        svc.list_tasks()
            .await?
            .items
            .into_iter()
            .filter(|t| t.path == record.relative_path && t.section == clause.key)
            .map(|t| t.handle)
            .collect()
    };
    let store = cx.store();
    let mut lines = Vec::new();
    let mut traces = Vec::new();
    for handle in &handles {
        let card = svc.show_task(handle).await?;
        let task = card.task.clone();
        let downstream: Vec<String> = store
            .dependency_edges()
            .await?
            .into_iter()
            .filter(|(_, depends_on)| *depends_on == task.id)
            .map(|(dependent, _)| dependent)
            .collect();
        let dependents: Vec<String> = svc
            .list_tasks()
            .await?
            .items
            .into_iter()
            .filter(|t| downstream.contains(&t.id))
            .map(|t| t.handle)
            .collect();
        let notes = svc
            .list_annotations(Some(task.path.as_str()))
            .await?
            .items
            .into_iter()
            .filter(|n| n.section == task.section)
            .map(|n| n.handle)
            .collect::<Vec<String>>();
        let evidence: Vec<String> = store
            .evidence()
            .await?
            .into_iter()
            .filter(|e| e.task_id == task.id)
            .map(|e| format!("{} {}", e.kind.as_str(), e.result.as_str()))
            .collect();
        let links: Vec<String> = store
            .links()
            .await?
            .into_iter()
            .filter(|l| l.task_id == task.id && l.removed_at.is_none())
            .map(|l| format!("{} {}", l.link_kind, l.target_ref))
            .collect();
        let rollbacks: Vec<String> = cx
            .engine
            .store
            .versions(&task.file_ref)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|v| format!("v{} {} {}", v.number, short(&v.content_hash), v.reason))
            .collect();
        lines.push(format!("== {}  {}", task.handle, task.title));
        lines.push(format!(
            "{} {}",
            cx.t("批注", "Annotations"),
            notes.join(", ")
        ));
        lines.push(format!(
            "{} {}",
            cx.t("下游任务", "Downstream"),
            dependents.join(", ")
        ));
        lines.push(format!("{} {}", cx.t("关联", "Links"), links.join("; ")));
        lines.push(format!(
            "{} {}",
            cx.t("证据", "Evidence"),
            evidence.join("; ")
        ));
        lines.push(format!(
            "{} {}",
            cx.t("回滚链", "Rollback chain"),
            rollbacks.join("; ")
        ));
        traces.push(json!({
            "handle": task.handle,
            "status": task.status,
            "notes": notes,
            "downstream": dependents,
            "links": links,
            "evidence": evidence,
            "rollbacks": rollbacks,
        }));
    }
    Ok(Reply::new(lines, json!({"traces": traces})))
}

async fn history(
    cx: &mut Ctx<'_>,
    handle: Option<&str>,
    file: Option<&str>,
    limit: Option<usize>,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let task_id = match handle {
        Some(h) => Some(canonical(&svc, h).await?.id),
        None => None,
    };
    let file_id = match file {
        Some(f) => Some(svc.resolve_file(f).await?.id),
        None => None,
    };
    let store = cx.store();
    let mut events: Vec<_> = store
        .events()
        .await?
        .into_iter()
        .filter(|e| task_id.as_ref().map_or(true, |id| e.task_id == *id))
        .filter(|e| file_id.as_ref().map_or(true, |id| e.file_id == *id))
        .collect();
    let limit = limit.unwrap_or(DEFAULT_HISTORY_LIMIT);
    let start = events.len().saturating_sub(limit);
    events.drain(..start);
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for event in &events {
        lines.push(format!(
            "#{}  {}  {}  {}({})  {}→{}  {}",
            event.seq,
            event.timestamp,
            event.kind,
            event.actor,
            event.actor_kind,
            event.from_status,
            event.to_status,
            short(&event.hash)
        ));
        items.push(json!({
            "seq": event.seq,
            "timestamp": event.timestamp,
            "kind": event.kind,
            "actor": event.actor,
            "actor_kind": event.actor_kind,
            "from": event.from_status,
            "to": event.to_status,
            "hash": event.hash,
        }));
    }
    Ok(Reply::new(lines, json!({"events": items})))
}

async fn status(
    cx: &mut Ctx<'_>,
    handle: &str,
    target: &str,
    reason: Option<&str>,
    summary: Option<&str>,
) -> Result<Reply> {
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let flow = cx.flow().await?;
    let step = match target {
        "analysis" => {
            if task.status == "working" {
                let text = require_text(reason, "--reason")?;
                flow.rework(&task.handle, &text).await?
            } else {
                flow.start_analysis(&task.handle).await?
            }
        }
        "working" => {
            let text = require_text(reason, "--reason")?;
            flow.return_to_work(&task.handle, &text).await?
        }
        "verification" => {
            let text = require_text(summary, "--summary")?;
            flow.report_work(&task.handle, &text).await?
        }
        _ => {
            return fail(
                ErrorCode::InvalidArg,
                "目标状态只能是 analysis、working 或 verification",
                "重新运行并选择其中之一",
            )
        }
    };
    Ok(step_reply(cx, &step))
}

/// A required text option: a missing or empty value is `E_INVALID_ARG`.
fn require_text(value: Option<&str>, option: &str) -> Result<String> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(text) => Ok(text.to_string()),
        None => fail(
            ErrorCode::InvalidArg,
            &format!("{option} 必须非空"),
            &format!("给出 {option} 后重试"),
        ),
    }
}

async fn plan(cx: &mut Ctx<'_>, sub: &PlanCmd) -> Result<Reply> {
    match sub {
        PlanCmd::New {
            handle,
            summary,
            rationale,
            impact,
            risk,
            option,
        } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let flow = cx.flow().await?;
            let spec = PlanSpec {
                summary: summary.clone(),
                rationale: rationale.clone().unwrap_or_default(),
                impact: impact.clone().unwrap_or_default(),
                risk: risk.clone(),
                options: option.clone(),
                edit: None,
            };
            let submitted = flow.submit_plan(&task.handle, &spec).await?;
            let mut lines = vec![format!(
                "{} P-{} {}",
                cx.t("方案已提交", "Plan submitted"),
                submitted.plan_no,
                short(&submitted.plan_hash)
            )];
            if let Some(step) = &submitted.step {
                lines.push(format!("{}: {} → {}", step.handle, step.from, step.to));
            }
            let data = json!({
                "id": submitted.id,
                "plan_no": submitted.plan_no,
                "plan_hash": submitted.plan_hash,
                "existing": submitted.existing,
            });
            Ok(Reply::new(lines, data).next(format!("ladon doc approve {}", task.handle)))
        }
        PlanCmd::Show { handle, number } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let store = cx.store();
            let mut plans: Vec<_> = store
                .plans()
                .await?
                .into_iter()
                .filter(|p| p.task_id == task.id)
                .filter(|p| number.map_or(true, |n| p.plan_no == n))
                .collect();
            plans.sort_by_key(|p| p.plan_no);
            let mut lines = Vec::new();
            let mut items = Vec::new();
            for plan in &plans {
                lines.push(format!(
                    "P-{}  {}  {}  {}  {}",
                    plan.plan_no,
                    plan.status.as_str(),
                    plan.risk.as_str(),
                    short(&plan.plan_hash),
                    plan.summary
                ));
                for (index, option) in plan.options.iter().enumerate() {
                    lines.push(format!("  {}. {}", index + 1, option));
                }
                items.push(json!({
                    "plan_no": plan.plan_no,
                    "status": plan.status.as_str(),
                    "risk": plan.risk.as_str(),
                    "plan_hash": plan.plan_hash,
                    "summary": plan.summary,
                    "rationale": plan.rationale,
                    "impact": plan.impact,
                    "options": plan.options,
                    "chosen_option": plan.chosen_option,
                }));
            }
            Ok(Reply::new(
                lines,
                json!({"handle": task.handle, "plans": items}),
            ))
        }
    }
}

async fn approve(
    cx: &mut Ctx<'_>,
    handle: &str,
    option: Option<usize>,
    comment: Option<&str>,
) -> Result<Reply> {
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let subject = subject_from_inbox(cx, &task.handle, Bucket::Approval).await?;
    let lines = vec![
        format!(
            "{} {}  {}",
            cx.t("批准方案", "Approve plan"),
            task.handle,
            task.title
        ),
        format!("{} {}", cx.t("方案哈希", "Plan hash"), short(&subject)),
    ];
    if !cx.confirm_typed(&task.handle, &lines) {
        return Ok(cx.abandoned());
    }
    let flow = cx.flow().await?;
    let step = flow
        .approve(&task.handle, &subject, option, comment.unwrap_or(""))
        .await?;
    Ok(step_reply(cx, &step))
}

async fn reject(cx: &mut Ctx<'_>, handle: &str, comment: &str) -> Result<Reply> {
    let reason = require_text(Some(comment), "--comment")?;
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let lines = vec![format!(
        "{} {}  {}",
        cx.t("驳回方案", "Reject plan"),
        task.handle,
        task.title
    )];
    if !cx.confirm_yes_no(&lines) {
        return Ok(cx.abandoned());
    }
    let flow = cx.flow().await?;
    let step = flow.reject(&task.handle, &reason).await?;
    Ok(step_reply(cx, &step))
}

async fn close(
    cx: &mut Ctx<'_>,
    handle: &str,
    waive: Option<&str>,
    accept: Option<&str>,
) -> Result<Reply> {
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    subject_from_inbox(cx, &task.handle, Bucket::Verification).await?;
    let subject = verification_subject(cx, &task.id).await?;
    let store = cx.store();
    let mut evidence: Vec<_> = store
        .evidence()
        .await?
        .into_iter()
        .filter(|e| e.task_id == task.id)
        .collect();
    evidence.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut accepted = Vec::new();
    if let Some(list) = accept {
        for part in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let position: usize = match part.parse() {
                Ok(value) => value,
                Err(_) => {
                    return fail(
                        ErrorCode::InvalidArg,
                        "证据序号必须是正整数",
                        "用 doc evidence list 查看序号",
                    )
                }
            };
            match position.checked_sub(1).and_then(|i| evidence.get(i)) {
                Some(row) => accepted.push(row.id.clone()),
                None => {
                    return fail(
                        ErrorCode::NotFound,
                        "没有该序号的证据",
                        "用 doc evidence list 查看序号",
                    )
                }
            }
        }
    }
    let lines = vec![format!(
        "{} {}  {}",
        cx.t("验收任务", "Close task"),
        task.handle,
        short(&subject)
    )];
    if !cx.confirm_typed(&task.handle, &lines) {
        return Ok(cx.abandoned());
    }
    let flow = cx.flow().await?;
    let step = flow.close(&task.handle, &subject, &accepted, waive).await?;
    Ok(step_reply(cx, &step))
}

async fn reason_step(
    cx: &mut Ctx<'_>,
    handle: &str,
    reason: &str,
    action: ReasonAction,
) -> Result<Reply> {
    let text = require_text(Some(reason), "--reason")?;
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let title = match action {
        ReasonAction::Cancel => cx.t("取消任务", "Cancel task"),
        ReasonAction::Reopen => cx.t("重开任务", "Reopen task"),
        ReasonAction::Dismiss => cx.t("忽略提案", "Dismiss proposal"),
    };
    let lines = vec![format!("{} {}  {}", title, task.handle, task.title)];
    if !cx.confirm_yes_no(&lines) {
        return Ok(cx.abandoned());
    }
    let flow = cx.flow().await?;
    let step = match action {
        ReasonAction::Cancel => flow.cancel(&task.handle, &text).await?,
        ReasonAction::Reopen => flow.reopen(&task.handle, &text).await?,
        ReasonAction::Dismiss => flow.dismiss(&task.handle, &text).await?,
    };
    Ok(step_reply(cx, &step))
}

async fn adopt(
    cx: &mut Ctx<'_>,
    handle: &str,
    yes: bool,
    expect_hash: Option<&str>,
) -> Result<Reply> {
    confirm_yes(yes, expect_hash)?;
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let hash = content_hash(&cx.store(), &task).await?;
    let lines = vec![format!(
        "{} {}  {}",
        cx.t("采纳提案", "Adopt proposal"),
        task.handle,
        short(&hash)
    )];
    if !cx.confirm_object(yes, expect_hash, &hash, &lines)? {
        return Ok(cx.abandoned());
    }
    let flow = cx.flow().await?;
    let step = flow.adopt(&task.handle).await?;
    Ok(step_reply(cx, &step))
}

async fn dep(cx: &mut Ctx<'_>, sub: &DepCmd) -> Result<Reply> {
    match sub {
        DepCmd::Add { handle, depends_on } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let prerequisite = canonical(&svc, depends_on).await?;
            let flow = cx.flow().await?;
            let step = flow
                .add_dependency(&task.handle, &prerequisite.handle)
                .await?;
            let schedule = flow.schedule().await?;
            let mut lines = vec![format!(
                "{} {} → {}",
                cx.t("依赖已添加", "Dependency added"),
                task.handle,
                prerequisite.handle
            )];
            for (index, batch) in schedule.schedule.batches.iter().enumerate() {
                lines.push(format!(
                    "{} {}: {}",
                    cx.t("批次", "Batch"),
                    index + 1,
                    batch.join(" ")
                ));
            }
            let data = json!({"step": step_json(&step), "batches": schedule.schedule.batches});
            Ok(Reply::new(lines, data).next(cx.t(
                "doc schedule 查看并行批次",
                "doc schedule shows the batches",
            )))
        }
        DepCmd::Rm {
            handle,
            depends_on,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let prerequisite = canonical(&svc, depends_on).await?;
            let hash = content_hash(&cx.store(), &task).await?;
            let lines = vec![format!(
                "{} {} → {}  {}",
                cx.t("移除依赖", "Remove dependency"),
                task.handle,
                prerequisite.handle,
                short(&hash)
            )];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &hash, &lines)? {
                return Ok(cx.abandoned());
            }
            let flow = cx.flow().await?;
            let step = flow
                .remove_dependency(&task.handle, &prerequisite.handle)
                .await?;
            Ok(step_reply(cx, &step))
        }
        DepCmd::List { handle } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let store = cx.store();
            let edges = store.dependency_edges().await?;
            let tasks = svc.list_tasks().await?.items;
            let handle_of = |id: &str| -> String {
                tasks
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.handle.clone())
                    .unwrap_or_else(|| id.to_string())
            };
            let mut lines = Vec::new();
            let mut needs = Vec::new();
            for (dependent, prerequisite) in &edges {
                if *dependent == task.id {
                    let name = handle_of(prerequisite);
                    lines.push(format!(
                        "{} {} {}",
                        task.handle,
                        cx.t("依赖", "depends on"),
                        name
                    ));
                    needs.push(name);
                }
            }
            for (dependent, prerequisite) in &edges {
                if *prerequisite == task.id {
                    let name = handle_of(dependent);
                    lines.push(format!(
                        "{} {} {}",
                        name,
                        cx.t("依赖", "depends on"),
                        task.handle
                    ));
                }
            }
            if lines.is_empty() {
                lines.push(cx.t("没有依赖。", "No dependencies."));
            }
            Ok(Reply::new(
                lines,
                json!({"handle": task.handle, "depends_on": needs}),
            ))
        }
    }
}

async fn link(cx: &mut Ctx<'_>, sub: &LinkCmd) -> Result<Reply> {
    match sub {
        LinkCmd::Add {
            handle,
            kind,
            file,
            section,
            commit,
            url,
            task,
            label,
        } => {
            let chosen = [
                file.as_ref().map(|v| ("file", v.as_str())),
                commit.as_ref().map(|v| ("commit", v.as_str())),
                url.as_ref().map(|v| ("url", v.as_str())),
                task.as_ref().map(|v| ("task", v.as_str())),
            ];
            let present: Vec<(&str, &str)> = chosen.iter().flatten().copied().collect();
            if present.len() != 1 {
                return fail(
                    ErrorCode::InvalidArg,
                    "关联必须且只能给出 --file、--commit、--url、--task 之一",
                    "只保留一个目标选项后重试",
                );
            }
            let (target_kind, target) = present[0];
            let svc = cx.svc().await?;
            let task_ref = canonical(&svc, handle).await?;
            let flow = cx.flow().await?;
            let step = flow
                .add_link(
                    &task_ref.handle,
                    kind,
                    target_kind,
                    target,
                    section.as_deref(),
                    label.as_deref().unwrap_or(""),
                )
                .await?;
            Ok(step_reply(cx, &step))
        }
        LinkCmd::Rm {
            link_id,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let store = cx.store();
            let row = match store.links().await?.into_iter().find(|l| l.id == *link_id) {
                Some(row) => row,
                None => {
                    return fail(
                        ErrorCode::NotFound,
                        "没有找到该关联",
                        "用 doc link list T-n 查看",
                    )
                }
            };
            let svc = cx.svc().await?;
            let task = svc.show_task(&row.task_id).await.map(|c| c.task);
            let handle = match task {
                Ok(view) => view.handle,
                Err(_) => {
                    let tasks = svc.list_tasks().await?.items;
                    match tasks.into_iter().find(|t| t.id == row.task_id) {
                        Some(view) => view.handle,
                        None => {
                            return fail(
                                ErrorCode::NotFound,
                                "关联所属的任务不存在",
                                "用 doc list --all 查看",
                            )
                        }
                    }
                }
            };
            let lines = vec![format!(
                "{} {} {} {}",
                cx.t("移除关联", "Remove link"),
                handle,
                row.link_kind,
                row.target_ref
            )];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &row.id, &lines)? {
                return Ok(cx.abandoned());
            }
            let flow = cx.flow().await?;
            let step = flow.remove_link(&handle, link_id).await?;
            Ok(step_reply(cx, &step))
        }
        LinkCmd::List { handle } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let rows: Vec<_> = cx
                .store()
                .links()
                .await?
                .into_iter()
                .filter(|l| l.task_id == task.id && l.removed_at.is_none())
                .collect();
            let lines = rows
                .iter()
                .map(|l| {
                    format!(
                        "{}  {}  {}  {}",
                        l.id, l.link_kind, l.target_kind, l.target_ref
                    )
                })
                .collect::<Vec<String>>();
            let items: Vec<Value> = rows
                .iter()
                .map(|l| json!({"id": l.id, "kind": l.link_kind, "target_kind": l.target_kind, "target": l.target_ref}))
                .collect();
            Ok(Reply::new(
                lines,
                json!({"handle": task.handle, "links": items}),
            ))
        }
    }
}

async fn evidence(cx: &mut Ctx<'_>, sub: &EvidenceCmd) -> Result<Reply> {
    match sub {
        EvidenceCmd::Add {
            handle,
            kind,
            result,
            summary,
            version,
        } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let flow = cx.flow().await?;
            let step = flow
                .add_evidence(&task.handle, kind, result, summary, version.as_deref())
                .await?;
            Ok(step_reply(cx, &step))
        }
        EvidenceCmd::List { handle } => {
            let svc = cx.svc().await?;
            let task = canonical(&svc, handle).await?;
            let mut rows: Vec<_> = cx
                .store()
                .evidence()
                .await?
                .into_iter()
                .filter(|e| e.task_id == task.id)
                .collect();
            rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            let mut lines = Vec::new();
            let mut items = Vec::new();
            for (index, row) in rows.iter().enumerate() {
                lines.push(format!(
                    "#{}  {}  {}  {}  {}",
                    index + 1,
                    row.kind.as_str(),
                    row.result.as_str(),
                    row.source.as_str(),
                    row.created_at
                ));
                items.push(json!({
                    "position": index + 1,
                    "kind": row.kind.as_str(),
                    "result": row.result.as_str(),
                    "source": row.source.as_str(),
                    "created_at": row.created_at,
                }));
            }
            Ok(Reply::new(
                lines,
                json!({"handle": task.handle, "evidence": items}),
            ))
        }
    }
}

async fn anchors(cx: &mut Ctx<'_>, file: Option<&str>, check: bool) -> Result<Reply> {
    let reconciler = cx.reconciler().await?;
    if check {
        let report = match file {
            Some(reference) => reconciler.reconcile_file(reference).await?,
            None => reconciler.reconcile_all().await?,
        };
        let line = format!(
            "{} {}  {} {}",
            cx.t("检查锚点", "Anchors examined"),
            report.examined,
            cx.t("变更", "changes"),
            report.changes.len()
        );
        let changes: Vec<Value> = report
            .changes
            .iter()
            .map(|c| json!({"anchor": c.anchor_id, "from": c.from_state, "to": c.to_state, "method": c.method}))
            .collect();
        return Ok(Reply::new(
            vec![line],
            json!({"examined": report.examined, "changes": changes}),
        ));
    }
    let rows = reconciler.report(file).await?;
    let lines = rows
        .iter()
        .map(|r| {
            format!(
                "{}  {}  {} → {}",
                r.section_key,
                r.state,
                r.verdict_state,
                if r.would_change {
                    "changed"
                } else {
                    "unchanged"
                }
            )
        })
        .collect::<Vec<String>>();
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "anchor": r.anchor_id,
                "file": r.file_id,
                "section": r.section_key,
                "state": r.state,
                "verdict": r.verdict_state,
                "would_change": r.would_change,
                "method": r.method,
            })
        })
        .collect();
    Ok(Reply::new(lines, json!({"rows": items})))
}

async fn anchor_decision(
    cx: &mut Ctx<'_>,
    handle: &str,
    action: AnchorAction,
    selector: Option<&str>,
    yes: bool,
    expect_hash: Option<&str>,
) -> Result<Reply> {
    confirm_yes(yes, expect_hash)?;
    cx.require_console()?;
    let svc = cx.svc().await?;
    let task = canonical(&svc, handle).await?;
    let anchor = match cx
        .store()
        .anchors()
        .await?
        .into_iter()
        .find(|a| a.id == task.anchor_id)
    {
        Some(anchor) => anchor,
        None => {
            return fail(
                ErrorCode::NotFound,
                "任务的锚点不存在",
                "用 doc anchors 查看",
            )
        }
    };
    let lines = vec![format!(
        "{} {}  {}  {}",
        cx.t("锚点决定", "Anchor decision"),
        task.handle,
        anchor.section_key,
        short(&anchor.current_hash)
    )];
    if !cx.confirm_object(yes, expect_hash, &anchor.current_hash, &lines)? {
        return Ok(cx.abandoned());
    }
    let reconciler = cx.reconciler().await?;
    let step = match action {
        AnchorAction::Ack => {
            reconciler
                .ack(
                    &task.handle,
                    &anchor.current_version_id,
                    &anchor.current_hash,
                )
                .await?
        }
        AnchorAction::Relocate => {
            let target = selector.unwrap_or("");
            reconciler
                .relocate(
                    &task.handle,
                    target,
                    &anchor.current_version_id,
                    &anchor.current_hash,
                )
                .await?
        }
        AnchorAction::Rebase => {
            reconciler
                .rebase(
                    &task.handle,
                    &anchor.current_version_id,
                    &anchor.current_hash,
                )
                .await?
        }
    };
    let line = format!(
        "{} {} {}",
        task.handle,
        step.state,
        step.event_seq
            .map_or(String::from("-"), |s| format!("#{s}"))
    );
    Ok(Reply::new(
        vec![line],
        json!({"anchor": step.anchor_id, "state": step.state, "event_seq": step.event_seq}),
    ))
}

async fn lineage(cx: &mut Ctx<'_>, sub: &LineageCmd) -> Result<Reply> {
    let reconciler = cx.reconciler().await?;
    match sub {
        LineageCmd::Scan => {
            let rows = reconciler.scan_lineage().await?;
            let lines = rows
                .iter()
                .map(|r| {
                    format!(
                        "{}  {}  {}  {}",
                        r.id, r.state, r.score_permille, r.evidence
                    )
                })
                .collect::<Vec<String>>();
            Ok(Reply::new(lines, json!({"rows": rows.len()})))
        }
        LineageCmd::List => {
            let rows = reconciler.lineage().await?;
            let lines = rows
                .iter()
                .map(|r| {
                    format!(
                        "{}  {}  {}  {}",
                        r.id, r.state, r.score_permille, r.evidence
                    )
                })
                .collect::<Vec<String>>();
            let items: Vec<Value> = rows
                .iter()
                .map(|r| json!({"id": r.id, "old_file": r.old_file_id, "new_file": r.new_file_id, "state": r.state}))
                .collect();
            Ok(Reply::new(lines, json!({"rows": items})))
        }
        LineageCmd::Confirm {
            id,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let row = find_lineage(&reconciler, id).await?;
            let new_file = match cx.engine.store.file_by_id(&row.new_file_id).await? {
                Some(file) => file,
                None => {
                    return fail(
                        ErrorCode::NotFound,
                        "新文件不存在",
                        "用 doc lineage list 查看",
                    )
                }
            };
            let lines = vec![format!(
                "{} {}  {}",
                cx.t("确认谱系", "Confirm lineage"),
                row.id,
                short(&new_file.current_hash)
            )];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &new_file.current_hash, &lines)? {
                return Ok(cx.abandoned());
            }
            let confirmed = reconciler
                .confirm_lineage(
                    &row.id,
                    &row.old_file_id,
                    &row.new_file_id,
                    &new_file.current_hash,
                )
                .await?;
            let line = format!(
                "{} {}",
                cx.t("谱系已确认", "Lineage confirmed"),
                confirmed.id
            );
            Ok(Reply::new(
                vec![line],
                json!({"id": confirmed.id, "state": confirmed.state}),
            ))
        }
        LineageCmd::Reject {
            id,
            yes,
            expect_hash,
        } => {
            confirm_yes(*yes, expect_hash.as_deref())?;
            cx.require_console()?;
            let row = find_lineage(&reconciler, id).await?;
            let lines = vec![format!("{} {}", cx.t("驳回谱系", "Reject lineage"), row.id)];
            if !cx.confirm_object(*yes, expect_hash.as_deref(), &row.id, &lines)? {
                return Ok(cx.abandoned());
            }
            let rejected = reconciler.reject_lineage(&row.id).await?;
            let line = format!("{} {}", cx.t("谱系已驳回", "Lineage rejected"), rejected.id);
            Ok(Reply::new(
                vec![line],
                json!({"id": rejected.id, "state": rejected.state}),
            ))
        }
    }
}

async fn find_lineage(
    reconciler: &Reconciler<'_>,
    id: &str,
) -> Result<crate::doctask_store::LineageRow> {
    match reconciler.lineage().await?.into_iter().find(|r| r.id == id) {
        Some(row) => Ok(row),
        None => fail(
            ErrorCode::NotFound,
            "没有找到该谱系记录",
            "用 doc lineage list 查看",
        ),
    }
}

async fn schedule(cx: &mut Ctx<'_>) -> Result<Reply> {
    let flow = cx.flow().await?;
    let view = flow.schedule().await?;
    let mut lines = Vec::new();
    for (index, batch) in view.schedule.batches.iter().enumerate() {
        lines.push(format!(
            "{} {}: {}",
            cx.t("批次", "Batch"),
            index + 1,
            batch.join(" ")
        ));
    }
    for conflict in &view.schedule.conflicts {
        lines.push(format!(
            "{} {}: {}",
            cx.t("冲突", "Conflict"),
            conflict.file,
            conflict.tasks.join(" ")
        ));
    }
    for blocked in &view.schedule.blocked {
        lines.push(format!(
            "{} {}: {}",
            cx.t("阻塞", "Blocked"),
            blocked.task,
            blocked.reason
        ));
    }
    if lines.is_empty() {
        lines.push(cx.t("没有可调度的任务。", "Nothing to schedule."));
    }
    let data = json!({
        "batches": view.schedule.batches,
        "conflicts": view.schedule.conflicts.iter().map(|c| json!({"file": c.file, "tasks": c.tasks})).collect::<Vec<Value>>(),
        "blocked": view.schedule.blocked.iter().map(|b| json!({"task": b.task, "reason": b.reason})).collect::<Vec<Value>>(),
        "hidden": view.hidden,
    });
    Ok(Reply::new(lines, data))
}

async fn verify(cx: &mut Ctx<'_>, handle: Option<&str>) -> Result<Reply> {
    let flow = cx.flow().await?;
    let task_handle = match handle {
        Some(h) => Some(canonical(&cx.svc().await?, h).await?.handle),
        None => None,
    };
    let report = flow.verify(task_handle.as_deref()).await?;
    let lines = report
        .findings
        .iter()
        .map(|f| {
            format!(
                "[{}] {} {} {}",
                f.level,
                f.code,
                f.task.clone().unwrap_or_default(),
                f.detail
            )
        })
        .collect::<Vec<String>>();
    let data = json!({
        "ok": report.ok,
        "findings": report.findings.iter().map(|f| json!({
            "code": f.code,
            "level": f.level,
            "task": f.task,
            "seq": f.seq,
            "detail": f.detail,
        })).collect::<Vec<Value>>(),
    });
    let mut reply = Reply::new(lines, data);
    if !report.ok {
        reply.fail = Some(ErrorCode::Integrity);
    } else {
        reply.lines.push(cx.t("校验通过。", "Verification passed."));
    }
    Ok(reply)
}

async fn migrate(cx: &mut Ctx<'_>, import_legacy: bool, accept_unverified: bool) -> Result<Reply> {
    if accept_unverified {
        cx.require_console()?;
        let lines = vec![cx.t(
            "备份无法验证时继续升级 Schema",
            "Continue the schema upgrade when the backup cannot be verified",
        )];
        if !cx.confirm_yes_no(&lines) {
            return Ok(cx.abandoned());
        }
    }
    let outcome = cx.engine.store.ensure_schema(accept_unverified).await?;
    let mut lines = vec![format!("{} {:?}", cx.t("Schema", "Schema"), outcome)];
    let mut data = json!({"schema": format!("{outcome:?}")});
    if import_legacy {
        let report = cx.store().backfill().await?;
        lines.push(format!(
            "{} anchors {} tasks {} annotations {} events {} status {}",
            cx.t("回填", "Backfill"),
            report.anchors_created,
            report.tasks_imported,
            report.annotations_imported,
            report.events_imported,
            report.status_events_added
        ));
        data["backfill"] = json!({
            "anchors_created": report.anchors_created,
            "tasks_imported": report.tasks_imported,
            "annotations_imported": report.annotations_imported,
            "events_imported": report.events_imported,
            "status_events_added": report.status_events_added,
        });
    }
    Ok(Reply::new(lines, data).next(cx.t("doc verify 检查链", "doc verify checks the chain")))
}

/// Writes the .NEMA graph or the audit report of the selected scope, then records the export.
/// The file goes to `.ladon/exports` unless `--out` names another path, which needs the object
/// confirmation of a human.
async fn export(cx: &mut Ctx<'_>, sub: &ExportCmd) -> Result<Reply> {
    let (kind, scope, output, extension) = match sub {
        ExportCmd::Nema { scope, output } => ("nema", scope, output, "nema".to_string()),
        ExportCmd::Report {
            scope,
            format,
            output,
        } => ("report", scope, output, format.clone()),
    };
    let exports_dir = normalise(&absolute(&cx.engine.root.join(".ladon").join("exports"))?);
    let exports_real = locate(&exports_dir)?;
    let destination = match &output.out {
        Some(requested) => {
            if is_device_path(requested) {
                return fail(
                    ErrorCode::InvalidArg,
                    "目标是设备名或设备命名空间，拒绝写入",
                    "给出普通文件路径",
                );
            }
            let path = normalise(&absolute(Path::new(requested))?);
            check_destination(cx, &path, &exports_dir, output.force)?;
            path
        }
        None => default_destination(&exports_dir, kind, &extension),
    };
    if !locate(&destination)?.starts_with(&exports_real) {
        cx.require_console()?;
        let lines = vec![format!(
            "{} {}",
            cx.t(
                "导出到 .ladon/exports 之外的路径:",
                "Export outside .ladon/exports:"
            ),
            destination.display()
        )];
        if !cx.confirm_yes_no(&lines) {
            return Ok(cx.abandoned());
        }
    }
    let generated_at = match &output.generated_at {
        Some(text) => {
            if text.trim().is_empty() || text.chars().any(char::is_control) {
                return fail(
                    ErrorCode::InvalidArg,
                    "--generated-at 为空或含控制字符",
                    "给出一行文本，例如 2026-11-25T09:00:00+09:00",
                );
            }
            text.clone()
        }
        None => chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
    };
    let report_scope = resolve_scope(cx, scope).await?;
    let input = build_input(cx, generated_at).await?;
    let options = ExportOptions {
        scope: report_scope,
        include_local_only: output.include_local_only,
        include_sensitive: output.include_sensitive,
    };
    let text = match extension.as_str() {
        "nema" => doctask_report::render_nema(&input, &options)?,
        "json" => doctask_report::render_json(&input, &options)?,
        _ => doctask_report::render_markdown(&input, &options)?,
    };
    write_atomic(&destination, text.as_bytes())?;
    let digest = sha256_hex(text.as_bytes());
    let shown = destination.display().to_string();
    let event = NewEvent {
        task_id: String::new(),
        file_id: String::new(),
        subject_kind: "export".to_string(),
        subject_id: digest.clone(),
        kind: "dt.export.generated".to_string(),
        from_status: String::new(),
        to_status: String::new(),
        actor: cx.actor.clone(),
        actor_kind: ActorKind::Human,
        session: String::new(),
        claimed_session: String::new(),
        payload_json: json!({"kind": kind, "path": shown, "sha256": digest}).to_string(),
    };
    cx.store().append_event(&event).await?;
    let lines = vec![
        cx.t(&format!("导出完成: {shown}"), &format!("Exported: {shown}")),
        format!("sha256: {digest}"),
    ];
    Ok(Reply::new(
        lines,
        json!({"kind": kind, "path": shown, "sha256": digest, "bytes": text.len()}),
    ))
}

/// The export command's `--out` value must not name a device.
fn is_device_path(path: &str) -> bool {
    if path.starts_with(r"\\.\") || path.starts_with(r"\\?\") || path.starts_with("//./") {
        return true;
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let bytes = stem.as_bytes();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (bytes.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && (b'1'..=b'9').contains(&bytes[3]))
}

/// The absolute form of a path; a relative path is taken from the current directory.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// The location of a path for containment checks: the longest existing ancestor is canonicalised
/// (symbolic links and the workspace root spelling resolved) and the rest is appended.
fn locate(path: &Path) -> Result<PathBuf> {
    let absolute_path = normalise(&absolute(path)?);
    let mut existing = absolute_path.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = fs::canonicalize(&existing) {
            let mut located = canonical;
            for part in rest.iter().rev() {
                located.push(part);
            }
            return Ok(located);
        }
        match existing.file_name() {
            Some(name) => {
                rest.push(name.to_os_string());
                existing.pop();
            }
            None => return Ok(absolute_path),
        }
    }
}

/// Removes `.` and resolves `..` lexically, so that prefix checks see the final location.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Refuses an existing non-file, an existing file without `--force`, and a path inside
/// `.ladon` other than the exports folder.
fn check_destination(
    cx: &Ctx<'_>,
    destination: &Path,
    exports_dir: &Path,
    force: bool,
) -> Result<()> {
    if let Ok(meta) = fs::metadata(destination) {
        if !meta.is_file() {
            return fail(
                ErrorCode::InvalidArg,
                "目标已存在且不是普通文件",
                "给出一个文件路径",
            );
        }
        if !force {
            return fail(
                ErrorCode::InvalidArg,
                "目标文件已存在",
                "加上 --force 以覆盖",
            );
        }
    }
    let ledger = locate(&cx.engine.root.join(".ladon"))?;
    let real = locate(destination)?;
    if real.starts_with(&ledger) && !real.starts_with(locate(exports_dir)?) {
        return fail(
            ErrorCode::InvalidArg,
            "拒绝写入 .ladon 下的其他路径",
            "导出到 .ladon/exports 或工作区之外",
        );
    }
    Ok(())
}

/// The default destination: `<exports>/<kind>-<time>.<extension>`, with a numeric suffix when the
/// name is taken within the same second.
fn default_destination(exports_dir: &Path, kind: &str, extension: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut candidate = exports_dir.join(format!("{kind}-{stamp}.{extension}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = exports_dir.join(format!("{kind}-{stamp}-{n}.{extension}"));
        n += 1;
    }
    candidate
}

/// Writes the bytes to a temporary sibling and renames it over the destination.
fn write_atomic(destination: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export");
    let temp = destination.with_file_name(format!(".{name}.tmp"));
    fs::write(&temp, bytes)?;
    if let Err(error) = fs::rename(&temp, destination) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

/// The report scope of the command options.
async fn resolve_scope(cx: &Ctx<'_>, scope: &ExportScope) -> Result<Scope> {
    if let Some(handle) = &scope.task {
        let svc = cx.svc().await?;
        let task = canonical(&svc, handle).await?;
        return Ok(Scope::Task(task.id));
    }
    if let Some(file) = &scope.file {
        parse_section_selector(scope.section.as_deref())?;
        let svc = cx.svc().await?;
        let record = svc.resolve_file(file).await?;
        return Ok(Scope::File {
            file_id: record.id,
            section: scope.section.clone(),
        });
    }
    Ok(Scope::All)
}

/// A stored value that the report cannot read is an integrity failure.
fn parsed<T>(value: Option<T>, what: &str) -> Result<T> {
    value.ok_or_else(|| {
        DocError::new(
            ErrorCode::Integrity,
            &format!("存储中的{what}无法识别"),
            "运行 doc verify 检查链与不变量",
        )
        .into()
    })
}

/// Reads every record the report uses. Anchor states of annotations come from their anchors;
/// an annotation with no anchor row is reported as `unchecked`.
async fn build_input(cx: &Ctx<'_>, generated_at: String) -> Result<ReportInput> {
    let engine = cx.engine;
    let store = cx.store();
    let mut input = ReportInput {
        generated_at,
        ..ReportInput::default()
    };
    for file in engine.store.list_files().await? {
        for version in engine.store.versions(&file.id).await? {
            input.versions.push(ReportVersion {
                id: version.id,
                file_id: version.file_id,
                version_no: version.number,
                content_hash: version.content_hash,
                ledger_event_id: String::new(),
                created_at: version.created_at,
            });
        }
        // The gate decides now, so a rule added after the capture counts; the stored flag alone would miss it.
        let refused = !engine.remote_read_allowed(&file).await.unwrap_or(false);
        input.files.push(ReportFile {
            id: file.id,
            path: file.relative_path,
            sensitive: refused,
        });
    }
    let mut anchor_states = BTreeMap::new();
    for anchor in store.anchors().await? {
        anchor_states.insert(anchor.id.clone(), anchor.state.clone());
        input.clauses.push(ReportClause {
            file_id: anchor.file_id,
            section: anchor.section_key,
            anchor_id: anchor.id,
            ordinal: anchor.line_start,
            number: anchor.clause_id,
            title: anchor.title,
        });
    }
    for row in store.annotations().await? {
        let state = anchor_states
            .get(&row.anchor_id)
            .cloned()
            .unwrap_or_else(|| "unchecked".to_string());
        input.annotations.push(ReportAnnotation {
            id: row.id,
            no: row.annotation_no,
            file_id: row.file_id,
            section: row.section_key,
            text: row.text,
            visibility: parsed(Visibility::parse(&row.visibility), "批注可见性")?,
            author_kind: parsed(ActorKind::parse(&row.author_kind), "批注作者类型")?,
            anchor_state: parsed(AnchorState::parse(&state), "锚点状态")?,
        });
    }
    for row in store.tasks().await? {
        let anchor_state = row
            .anchor_state
            .clone()
            .unwrap_or_else(|| "unchecked".to_string());
        input.tasks.push(TaskRecord {
            id: row.id,
            task_no: row.task_no,
            file_id: row.file_id,
            section: row.anchor_section,
            title: row.title,
            description: row.description,
            status: parsed(TaskState::parse(&row.status), "任务状态")?,
            priority: parsed(u8::try_from(row.priority).ok(), "任务优先级")?,
            proposal_state: parsed(ProposalState::parse(&row.proposal_state), "提案状态")?,
            source_kind: parsed(SourceKind::parse(&row.source_kind), "任务来源")?,
            created_by_kind: parsed(ActorKind::parse(&row.created_by_kind), "创建者类型")?,
            visibility: parsed(Visibility::parse(&row.visibility), "任务可见性")?,
            anchor_state: parsed(AnchorState::parse(&anchor_state), "锚点状态")?,
            owner: row.owner,
            cancel_reason: String::new(),
            closed_at: row.closed_at,
        });
    }
    for link in store.links().await? {
        if link.removed_at.is_some() {
            continue;
        }
        input.links.push(ReportLink {
            task_id: link.task_id,
            kind: parsed(LinkKind::parse(&link.link_kind), "关联类型")?,
            target_kind: parsed(TargetKind::parse(&link.target_kind), "关联目标类型")?,
            target: link.target_ref,
            target_file_id: link.target_file_id,
            label: link.label,
        });
    }
    for (dependent, prerequisite) in store.dependency_edges().await? {
        input.dependencies.push(ReportDependency {
            prerequisite,
            dependent,
        });
    }
    input.plans = store.plans().await?;
    input.approvals = store.approvals().await?;
    input.evidence = store.evidence().await?;
    input.events = store.events().await?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use std::collections::VecDeque;

    /// A console stand-in: answers come from a queue, and the console state is fixed.
    #[derive(Default)]
    struct FakeIo {
        stdin: bool,
        stdout: bool,
        answers: VecDeque<String>,
        reads: usize,
        stdout_text: String,
        stderr_text: String,
    }

    impl FakeIo {
        fn console(answers: &[&str]) -> FakeIo {
            FakeIo {
                stdin: true,
                stdout: true,
                answers: answers.iter().map(|a| a.to_string()).collect(),
                ..FakeIo::default()
            }
        }

        fn piped() -> FakeIo {
            FakeIo::default()
        }
    }

    impl DocIo for FakeIo {
        fn stdin_is_console(&self) -> bool {
            self.stdin
        }

        fn stdout_is_console(&self) -> bool {
            self.stdout
        }

        fn read_line(&mut self) -> Option<String> {
            self.reads += 1;
            self.answers.pop_front()
        }

        fn out(&mut self, text: &str) {
            self.stdout_text.push_str(text);
        }

        fn err(&mut self, text: &str) {
            self.stderr_text.push_str(text);
        }

        fn wait_refresh(&mut self) -> bool {
            false
        }
    }

    /// Parses `ladon doc <args>` and returns the doc arguments.
    fn doc_args(args: &[&str]) -> DocArgs {
        let mut argv = vec!["ladon", "doc"];
        argv.extend_from_slice(args);
        match crate::entry::Cli::try_parse_from(argv)
            .expect("doc command parses")
            .cmd
        {
            crate::entry::Command::Doc(doc) => doc,
            _ => panic!("not a doc command"),
        }
    }

    /// Runs one `doc` command line and returns its exit code.
    async fn run(engine: &Engine, args: &[&str], io: &mut FakeIo) -> i32 {
        run_doc(engine, &doc_args(args), io)
            .await
            .expect("the command returns an exit code")
    }

    /// A workspace with `a.md` (two clauses) registered as the baseline.
    async fn baselined() -> (tempfile::TempDir, Engine) {
        let tmp = tempfile::tempdir().expect("temp dir");
        let engine = Engine::init(tmp.path()).await.expect("workspace");
        std::fs::write(
            tmp.path().join("a.md"),
            "# 1 Scope\nbody\n\n# 2 Rules\nbody\n",
        )
        .expect("write a.md");
        engine
            .capture(&tmp.path().join("a.md"), "human", "test")
            .await
            .expect("capture");
        let mut io = FakeIo::piped();
        assert_eq!(
            run(&engine, &["migrate"], &mut io).await,
            0,
            "{}",
            io.stderr_text
        );
        let mut io = FakeIo::console(&["y"]);
        let code = run(&engine, &["baseline", "add", "a.md"], &mut io).await;
        assert_eq!(code, 0, "{}{}", io.stdout_text, io.stderr_text);
        (tmp, engine)
    }

    /// Creates a task on clause 1 and submits an analysis-only plan for it.
    async fn planned_task(engine: &Engine, title: &str) {
        let mut io = FakeIo::console(&[]);
        assert_eq!(run(engine, &["new", "a.md", "1", title], &mut io).await, 0);
        let number = DocStore::new(engine.store.clone())
            .tasks()
            .await
            .expect("tasks")
            .len();
        let handle = format!("T-{number}");
        assert_eq!(
            run(engine, &["status", &handle, "analysis"], &mut io).await,
            0
        );
        assert_eq!(
            run(
                engine,
                &["plan", "new", &handle, "--summary", "summary"],
                &mut io
            )
            .await,
            0
        );
    }

    async fn event_count(engine: &Engine) -> usize {
        DocStore::new(engine.store.clone())
            .events()
            .await
            .expect("events")
            .len()
    }

    async fn status_of(engine: &Engine, task_no: i64) -> String {
        DocStore::new(engine.store.clone())
            .tasks()
            .await
            .expect("tasks")
            .into_iter()
            .find(|task| task.task_no == task_no)
            .expect("task row")
            .status
    }

    #[test]
    fn tc930_command_tree_is_valid() {
        crate::entry::Cli::command().debug_assert();
        let in_scope: &[&[&str]] = &[
            &["inbox", "--watch"],
            &["baseline", "add", "a.md", "--version", "1", "--note", "n"],
            &[
                "baseline",
                "add",
                "a.md",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ],
            &["baseline", "list", "a.md"],
            &["baseline", "revoke", "B-1", "--reason", "r"],
            &["outline", "a.md", "--version", "1"],
            &["clause", "a.md", "1"],
            &[
                "note",
                "a.md",
                "1",
                "text",
                "--private",
                "--rationale",
                "r",
                "--quote",
                "q",
            ],
            &["notes", "a.md", "--section", "1"],
            &["note-share", "N-1", "--yes", "--expect-hash", "deadbeef"],
            &["note-unshare", "N-1"],
            &[
                "new",
                "a.md",
                "1",
                "title",
                "--description",
                "d",
                "--owner",
                "o",
                "--priority",
                "2",
                "--depends",
                "T-1",
                "--private",
            ],
            &[
                "edit",
                "T-1",
                "--title",
                "x",
                "--share",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ],
            &[
                "list",
                "--status",
                "open,working",
                "--file",
                "a.md",
                "--owner",
                "o",
                "--proposed",
                "--all",
                "--limit",
                "5",
            ],
            &["show", "T-1"],
            &["trace", "T-1"],
            &["trace", "a.md", "1"],
            &["history", "T-1", "--limit", "3"],
            &["status", "T-1", "analysis"],
            &["status", "T-1", "working", "--reason", "r"],
            &[
                "plan",
                "new",
                "T-1",
                "--summary",
                "s",
                "--rationale",
                "r",
                "--impact",
                "i",
                "--risk",
                "high",
                "--option",
                "a",
                "--option",
                "b",
            ],
            &["plan", "show", "T-1", "1"],
            &["approve", "T-1", "--option", "1", "--comment", "c"],
            &["reject", "T-1", "--comment", "c"],
            &[
                "close",
                "T-1",
                "--waive",
                "r",
                "--accept-agent-evidence",
                "1,2",
            ],
            &["cancel", "T-1", "--reason", "r"],
            &["reopen", "T-1", "--reason", "r"],
            &["adopt", "T-1", "--yes", "--expect-hash", "deadbeef"],
            &["dismiss", "T-1", "--reason", "r"],
            &["dep", "add", "T-2", "T-1"],
            &[
                "dep",
                "rm",
                "T-2",
                "T-1",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ],
            &["dep", "list", "T-2"],
            &[
                "link",
                "add",
                "T-1",
                "--kind",
                "design",
                "--file",
                "a.md",
                "--section",
                "1",
                "--label",
                "l",
            ],
            &["link", "rm", "L-1", "--yes", "--expect-hash", "deadbeef"],
            &["link", "list", "T-1"],
            &[
                "evidence",
                "add",
                "T-1",
                "--kind",
                "test_run",
                "--result",
                "pass",
                "--summary",
                "s",
                "--version",
                "V-1",
            ],
            &["evidence", "list", "T-1"],
            &["anchors", "a.md", "--check"],
            &["ack", "T-1", "--yes", "--expect-hash", "deadbeef"],
            &["relocate", "T-1", "2", "--yes", "--expect-hash", "deadbeef"],
            &["rebase", "T-1", "--yes", "--expect-hash", "deadbeef"],
            &["lineage", "scan"],
            &["lineage", "list"],
            &[
                "lineage",
                "confirm",
                "LN-1",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ],
            &[
                "lineage",
                "reject",
                "LN-1",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ],
            &["schedule"],
            &["verify", "T-1"],
            &[
                "export", "nema", "--task", "T-1", "--out", "x.nema", "--force",
            ],
            &[
                "export",
                "report",
                "--format",
                "json",
                "--all",
                "--include-local-only",
                "--include-sensitive",
                "--generated-at",
                "2026-11-25T09:00:00+09:00",
            ],
            &["migrate", "--import-legacy", "--accept-unverified-backup"],
            &[
                "list",
                "--json",
                "--lang",
                "en",
                "--no-color",
                "--actor",
                "x",
            ],
        ];
        for line in in_scope {
            let mut argv = vec!["ladon", "doc"];
            argv.extend_from_slice(line);
            assert!(
                crate::entry::Cli::try_parse_from(argv).is_ok(),
                "in-scope form must parse: {line:?}"
            );
        }
        let out_of_scope: &[&[&str]] = &[
            &["apply", "T-1"],
            &["recover"],
            &["plan", "show", "T-1", "--diff"],
            &[
                "plan",
                "new",
                "T-1",
                "--summary",
                "s",
                "--file",
                "a.md",
                "--section",
                "1",
                "--kind",
                "replace",
                "--text-file",
                "t.md",
            ],
            &["list", "--lang", "fr"],
        ];
        for line in out_of_scope {
            let mut argv = vec!["ladon", "doc"];
            argv.extend_from_slice(line);
            assert!(
                crate::entry::Cli::try_parse_from(argv).is_err(),
                "out-of-scope form must be rejected: {line:?}"
            );
        }
        for line in [
            &["scan", "--actor", "x"][..],
            &[
                "annotate",
                "a.md",
                "--section",
                "ROOT",
                "text",
                "--actor",
                "x",
            ][..],
        ] {
            let mut argv = vec!["ladon"];
            argv.extend_from_slice(line);
            assert!(
                crate::entry::Cli::try_parse_from(argv).is_ok(),
                "legacy form keeps its own --actor: {line:?}"
            );
        }
    }

    #[test]
    fn tc933_yes_requires_expect_hash() {
        let invalid = |result: std::result::Result<(), DocError>| {
            result.expect_err("refused").code.as_str().to_string()
        };
        assert_eq!(invalid(confirm_yes(true, None)), "E_INVALID_ARG");
        assert_eq!(invalid(confirm_yes(true, Some("abcdef0"))), "E_INVALID_ARG");
        assert_eq!(invalid(confirm_yes(true, Some("abc12"))), "E_INVALID_ARG");
        assert_eq!(invalid(confirm_yes(true, Some("zz"))), "E_INVALID_ARG");
        assert!(confirm_yes(true, Some("abcdef01")).is_ok());
        assert!(confirm_yes(true, Some("deadbeef")).is_ok());
        assert!(confirm_yes(false, None).is_ok());
        let full = "DEADBEEF00112233445566778899aabbccddeeff00112233445566778899aabb";
        assert!(hash_prefix_matches("deadbeef", full));
        assert!(!hash_prefix_matches(
            "deadbeef",
            "cafebabe00112233445566778899aabbccddeeff00112233445566778899aabb"
        ));
    }

    #[tokio::test]
    async fn tc934_human_only_refused_without_console() {
        let (_tmp, engine) = baselined().await;
        let before = event_count(&engine).await;
        let mut io = FakeIo::piped();
        for line in [
            &["approve", "T-1", "--comment", "x"][..],
            &["baseline", "add", "a.md"][..],
            &[
                "edit",
                "T-1",
                "--title",
                "x",
                "--yes",
                "--expect-hash",
                "deadbeef",
            ][..],
            &["reject", "T-1", "--comment", "x"][..],
        ] {
            assert_eq!(run(&engine, line, &mut io).await, 5, "{line:?}");
        }
        assert_eq!(io.reads, 0, "no prompt is read without a console");
        assert_eq!(event_count(&engine).await, before);
    }

    #[tokio::test]
    async fn tc935_typed_and_reason_confirmation() {
        let (_tmp, engine) = baselined().await;
        planned_task(&engine, "First").await;
        let before = event_count(&engine).await;

        for answer in ["t-1", "T-1 "] {
            let mut io = FakeIo::console(&[answer]);
            assert_eq!(
                run(
                    &engine,
                    &["approve", "T-1", "--comment", "c", "--json"],
                    &mut io
                )
                .await,
                0
            );
            assert!(io.stdout_text.contains(r#""written":false"#), "{answer:?}");
        }
        assert_eq!(event_count(&engine).await, before);
        assert_eq!(status_of(&engine, 1).await, "approval");

        let mut io = FakeIo::console(&["T-1"]);
        assert_eq!(
            run(&engine, &["approve", "T-1", "--comment", "c"], &mut io).await,
            0
        );
        assert_eq!(status_of(&engine, 1).await, "working");

        planned_task(&engine, "Second").await;
        let mut io = FakeIo::console(&["y"]);
        assert_eq!(
            run(&engine, &["reject", "T-2", "--comment", ""], &mut io).await,
            4
        );
        assert_eq!(io.reads, 0, "an empty reason is refused before the prompt");

        let mut io = FakeIo::console(&["n"]);
        assert_eq!(
            run(
                &engine,
                &["cancel", "T-1", "--reason", "r", "--json"],
                &mut io
            )
            .await,
            0
        );
        assert!(io.stdout_text.contains(r#""written":false"#));
        assert_eq!(status_of(&engine, 1).await, "working");

        let mut io = FakeIo::console(&["Y"]);
        assert_eq!(
            run(
                &engine,
                &["dismiss", "T-1", "--reason", "r", "--json"],
                &mut io
            )
            .await,
            0
        );
        assert!(io.stdout_text.contains(r#""written":false"#));

        let mut io = FakeIo::console(&["y"]);
        assert_eq!(
            run(&engine, &["cancel", "T-1", "--reason", "r"], &mut io).await,
            0
        );
        assert_eq!(status_of(&engine, 1).await, "cancelled");
    }

    #[tokio::test]
    async fn tc936_plan_new_approve_reject_analysis_only() {
        let (_tmp, engine) = baselined().await;
        planned_task(&engine, "First").await;
        planned_task(&engine, "Second").await;

        let plans = DocStore::new(engine.store.clone())
            .plans()
            .await
            .expect("plans");
        assert_eq!(plans.len(), 2);
        assert!(plans
            .iter()
            .all(|plan| plan.plan_no == 1 && plan.target_file_id.is_empty()));

        let mut io = FakeIo::console(&["T-1"]);
        assert_eq!(
            run(&engine, &["approve", "T-1", "--comment", "ok"], &mut io).await,
            0
        );
        assert_eq!(status_of(&engine, 1).await, "working");

        let mut io = FakeIo::console(&["y"]);
        assert_eq!(
            run(
                &engine,
                &["reject", "T-2", "--comment", "needs work"],
                &mut io
            )
            .await,
            0
        );
        assert_eq!(status_of(&engine, 2).await, "analysis");

        let mut io = FakeIo::piped();
        assert_eq!(
            run(&engine, &["plan", "show", "T-1", "1"], &mut io).await,
            0
        );
    }

    #[test]
    fn tc931_exit_code_mapping() {
        let rows = [
            (ErrorCode::NotFound, 3),
            (ErrorCode::Ambiguous, 3),
            (ErrorCode::SectionNotFound, 3),
            (ErrorCode::InvalidArg, 4),
            (ErrorCode::NoBaseline, 4),
            (ErrorCode::FileMissing, 4),
            (ErrorCode::TransitionDenied, 4),
            (ErrorCode::GuardFailed, 4),
            (ErrorCode::DepCycle, 4),
            (ErrorCode::DepDangling, 4),
            (ErrorCode::AnchorBroken, 4),
            (ErrorCode::UnsupportedFile, 4),
            (ErrorCode::PlanTooLarge, 4),
            (ErrorCode::Quota, 4),
            (ErrorCode::RateLimit, 4),
            (ErrorCode::SchemaOutdated, 4),
            (ErrorCode::MigrationBackup, 4),
            (ErrorCode::HumanOnly, 5),
            (ErrorCode::NotInteractive, 5),
            (ErrorCode::Conflict, 6),
            (ErrorCode::StalePlan, 6),
            (ErrorCode::StaleSubject, 6),
            (ErrorCode::FileChanged, 6),
            (ErrorCode::StoreBusy, 7),
            (ErrorCode::PolicyDenied, 8),
            (ErrorCode::Integrity, 9),
            (ErrorCode::Internal, 1),
        ];
        assert_eq!(rows.len(), 27);
        for (code, expected) in rows {
            assert_eq!(exit_code(code), expected, "{}", code.as_str());
        }
    }

    #[test]
    fn tc932_json_envelopes() {
        let ok: Value = serde_json::from_str(&json_ok(&json!({"n": 1}))).expect("json");
        assert_eq!(ok["ok"], json!(true));
        assert_eq!(ok["data"]["n"], json!(1));
        let error = DocError::new(ErrorCode::NotFound, "没有找到", "检查参数");
        let failed: Value = serde_json::from_str(&json_error(&error)).expect("json");
        assert_eq!(failed["ok"], json!(false));
        assert_eq!(failed["code"], json!("E_NOT_FOUND"));
        assert_eq!(failed["message"], json!("没有找到"));
        assert_eq!(failed["hint"], json!("检查参数"));
        assert_eq!(failed.as_object().map(|o| o.len()), Some(4));
    }
}
