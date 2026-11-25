# Technical Design: `src/doctask_cli.rs`

Source: `src/doctask_cli.rs`

Source-SHA256: `ab810ef314aa5ae26038d524a4a4fea3c150f908dbce1a601bb5c3be7b0b6ff4`

## Responsibility

`doctask_cli` is the one-shot command layer of the document-task module. It owns the `ladon doc` command tree (clap definitions), the mapping from each command to a service call, the human-only confirmation flow, the text and JSON output, the mapping from error codes to process exit codes, the `doc export` writer and the two legacy helpers `annotate_legacy` and `task_legacy` that `entry.rs` calls for the commands `annotate` and `task`.

The module holds no SQL. Every read and write goes through `DocTaskService`, `DocFlow`, `Reconciler`, `DocStore` (read accessors and `append_event`), `Engine` (file and version records) or the pure modules `doctask_outline`, `doctask_render` and `doctask_report`. It never opens the database directly and never reads environment variables other than `LADON_LANG`.

Console access is injected through the `DocIo` trait, so every confirmation path is testable in process. Only `SystemIo` touches the operating system: standard input and output, and the sleep between watch refreshes.

## Dependencies

- `crate::engine::Engine`: `store`, `root`, `capture`, `load_version`; `Store::list_files` and `Store::versions` for the export input.
- `crate::doctask_service`: `DocTaskService::attach` and `open`, `Caller`, `Shown`, `TaskSpec`, `TaskEdit`, `TaskView`, `task_content_hash`, `hash_prefix_matches`, `confirm_yes`.
- `crate::doctask_flow`: `DocFlow::attach`, `PlanSpec`.
- `crate::doctask_reconcile`: `Reconciler::attach`.
- `crate::doctask_store::{DocStore, NewEvent}`: the read accessors (`tasks`, `annotations`, `anchors`, `links`, `plans`, `approvals`, `evidence`, `events`, `dependency_edges`, `baselines`), `append_event`, `backfill`, `schema_current`.
- `crate::doctask_model`: `ErrorCode`, `DocError`, `Visibility`, `TaskState`, `ProposalState`, `Risk`, `ActorKind`, `AnchorState`, `LinkKind`, `TargetKind`, `SourceKind`, `TaskRecord`, `evidence_hash`, `sha256_hex`, `parse_task_handle`, `parse_annotation_handle`, `parse_section_selector`.
- `crate::doctask_outline` for `outline`, `resolve` and `clause_bytes`.
- `crate::doctask_render` for `Lang`, `Mode`, `detect_mode`, `render_error`, `short_hash`, `display_width`, `pad_to`, `render_trace`, `render_approval`, `render_outline`.
- `crate::doctask_report` for `ReportInput`, `ExportOptions`, `Scope`, `ReportFile`, `ReportClause`, `ReportAnnotation`, `ReportLink`, `ReportDependency`, `ReportVersion`, `render_markdown`, `render_json`, `render_nema`.
- `clap` (derive), `serde_json`, `anyhow`, `chrono` (local time for default export names and `--generated-at`), `std::fs`, `std::path`, `std::io`, `std::time::Duration`, and the Windows `kernel32` console query declared in this module (`extern "system"`, same form as `google_oauth.rs`).

## Public Contract

### Command tree

`DocArgs` is an `Args` struct, carried by the `Command::Doc` variant of `entry::Command`. It holds the options that apply to every `doc` command, propagated to the subcommands with `global = true`:

| Option | Meaning |
|---|---|
| `--json` | One JSON object on standard output (see Output). |
| `--no-color` | Plain text rendering; the output never contains ANSI sequences. |
| `--lang zh\|en` | Message language. Default: `LADON_LANG` when it is `en`, otherwise `zh`. |
| `--actor NAME` | Human principal name written to the chain. Default `human`. |

`DocCmd` is the subcommand enum. The variants and their arguments follow 57 section 9.2:

- `Inbox { watch }`
- `Baseline(BaselineCmd)`: `Add { file, version, note, yes, expect_hash }`, `List { file }`, `Revoke { id, reason, yes, expect_hash }`
- `Outline { file, version }`, `Clause { file, selector, version }`
- `Note { file, selector, text, private, rationale, quote }`, `Notes { file, section }`
- `NoteShare { id, yes, expect_hash }`, `NoteUnshare { id, yes, expect_hash }`
- `New { file, selector, title, description, owner, priority, depends, private }`
- `Edit { handle, title, description, owner, priority, private, share, yes, expect_hash }`
- `List { status, file, owner, proposed, all, limit }`, `Show { handle }`, `Trace { target }` (one handle, or a file and a selector), `History { handle, file, limit }`
- `Status { handle, target, reason, summary }`
- `Plan(PlanCmd)`: `New { handle, summary, rationale, impact, risk, option }`, `Show { handle, number }` (`number` is the optional plan number)
- `Approve { handle, option, comment }`, `Reject { handle, comment }`
- `Close { handle, waive, accept_agent_evidence }`, `Cancel { handle, reason }`, `Reopen { handle, reason }`
- `Adopt { handle, yes, expect_hash }`, `Dismiss { handle, reason }`
- `Dep(DepCmd)`: `Add { handle, depends_on }`, `Rm { handle, depends_on, yes, expect_hash }`, `List { handle }`
- `Link(LinkCmd)`: `Add { handle, kind, file, section, commit, url, task, label }`, `Rm { link_id, yes, expect_hash }`, `List { handle }`
- `Evidence(EvidenceCmd)`: `Add { handle, kind, result, summary, version }` (`version` is a version id text), `List { handle }`
- `Anchors { file, check }`, `Ack { handle, yes, expect_hash }`, `Relocate { handle, selector, yes, expect_hash }`, `Rebase { handle, yes, expect_hash }`
- `Lineage(LineageCmd)`: `Scan`, `List`, `Confirm { id, yes, expect_hash }`, `Reject { id, yes, expect_hash }`
- `Schedule`, `Verify { handle }`
- `Export(ExportCmd)`: `Nema { scope, output }`, `Report { scope, format, output }`. `ExportScope` holds `--task`, `--file`, `--section` (requires `--file`) and `--all`; `--task` conflicts with `--file` and `--all`, and `--file` conflicts with `--all`. `ExportOutput` holds `--out`, `--force`, `--include-local-only`, `--include-sensitive` and `--generated-at`. `--format` accepts `md` or `json` and defaults to `md`.
- `Migrate { import_legacy, accept_unverified_backup }`

Forms outside this package are not variants: `doc apply`, `doc recover`, `doc plan show --diff`, and the edit options of `doc plan new` (`--file --section --kind --text-file`). clap rejects them as unknown arguments.

Two options of 57 section 9.2 are not accepted, because the flow functions they would pass to take no such parameter: `doc close --comment` (`DocFlow::close` has no comment) and `doc evidence add --ref` (`DocFlow::add_evidence` has no reference).

### Entry points

- `pub async fn run_doc(engine: &Engine, args: &DocArgs, io: &mut dyn DocIo) -> anyhow::Result<i32>`: runs one `doc` command and returns the process exit code. A `DocError` is printed (text on standard error, or the envelope on standard output with `--json`) and mapped to its exit code. Any other error is returned as `Err` and becomes the runtime's `Error: ...` with exit 1.
- `pub async fn annotate_legacy(engine: &Engine, actor: &str, file: &str, section: &str, text: &str, io: &mut dyn DocIo) -> anyhow::Result<i32>`: opens the service with `DocTaskService::open`, calls `legacy_annotate` and prints `annotation id: <id>` on standard output.
- `pub async fn task_legacy(engine: &Engine, actor: &str, file: &str, section: &str, title: &str, description: &str, io: &mut dyn DocIo) -> anyhow::Result<i32>`: opens the service with `DocTaskService::open`, calls `legacy_task`, prints `task id: <id>` on standard output and `任务编号 T-<n>，查看: ladon doc show T-<n>` on standard error (`Task number T-<n>, view: ladon doc show T-<n>` when `LADON_LANG` is `en`). The legacy helpers have no `--lang` option. They print no next-step line.
- `pub fn exit_code(code: ErrorCode) -> i32`: the mapping of 57 section 9.3. The match is exhaustive and has no wildcard arm.

### Console seam

```rust
pub trait DocIo {
    fn stdin_is_console(&self) -> bool;
    fn stdout_is_console(&self) -> bool;
    fn read_line(&mut self) -> Option<String>;   // None at end of input
    fn out(&mut self, text: &str);
    fn err(&mut self, text: &str);
    fn wait_refresh(&mut self) -> bool;          // true to refresh again, false to stop watching
}
pub struct SystemIo;                             // the process standard streams
```

`SystemIo::stdin_is_console` and `stdout_is_console` call `GetConsoleMode` on `STD_INPUT_HANDLE` and `STD_OUTPUT_HANDLE` on Windows. Named pipes and pseudo-terminals of MSYS or Cygwin fail that query and count as non-consoles. On other platforms both return `false`. `wait_refresh` sleeps two seconds and returns `true`.

### Output

- Text: the successful result lines, then one `下一步: …` line (`Next: …` in English) when the command names a next step. The legacy helpers print no next-step line.
- JSON: exactly one object on standard output, `{"ok":true,"data":…}` on success or `{"ok":false,"code":"E_…","message":"…","hint":"…"}` on a `DocError`. An abandoned confirmation is a success with `{"ok":true,"data":{"written":false}}`.
- Confirmation prompts are written to standard error in both modes, and only on a console, because a prompt is reached only when both streams are consoles.
- A human-only command refused on a pipe writes no prompt. Its refusal is the `E_NOT_INTERACTIVE` envelope on standard output in JSON mode, and in text mode the `render_error` text on standard error.
- Errors in text mode: `render_error` output (`错误[CODE]: 消息` and `提示: …`) on standard error.

### Exit codes

| Code | Error codes |
|---|---|
| 0 | success, and an abandoned confirmation (`"written": false`) |
| 1 | not a `DocError` (returned as `Err`), and `E_INTERNAL` |
| 3 | `E_NOT_FOUND`, `E_AMBIGUOUS`, `E_SECTION_NOT_FOUND` |
| 4 | `E_INVALID_ARG`, `E_NO_BASELINE`, `E_FILE_MISSING`, `E_TRANSITION_DENIED`, `E_GUARD_FAILED`, `E_DEP_CYCLE`, `E_DEP_DANGLING`, `E_ANCHOR_BROKEN`, `E_UNSUPPORTED_FILE`, `E_PLAN_TOO_LARGE`, `E_QUOTA`, `E_RATE_LIMIT`, `E_SCHEMA_OUTDATED`, `E_MIGRATION_BACKUP` |
| 5 | `E_HUMAN_ONLY`, `E_NOT_INTERACTIVE` |
| 6 | `E_CONFLICT`, `E_STALE_PLAN`, `E_STALE_SUBJECT`, `E_FILE_CHANGED` |
| 7 | `E_STORE_BUSY` |
| 8 | `E_POLICY_DENIED` |
| 9 | `E_INTEGRITY` |

## Behaviour

### Human-only confirmation

Every command that writes a human-only operation runs in this order:

1. Validate the arguments: `confirm_yes` (`--yes` needs `--expect-hash` of at least eight hexadecimal characters) and a non-empty text where one is required (`--comment`, `--reason`). A failure is `E_INVALID_ARG`.
2. If `stdin_is_console` or `stdout_is_console` is false, return `E_NOT_INTERACTIVE` (exit 5). No service call has been made.
3. Load the subject through the attached service and build the display.
4. Ask for the confirmation of its class and act only on an exact answer.

The three classes follow 57 section 9.5:

- Typed (`approve`, `close`): the display shows the task handle, the title and the subject hash (first eight characters). The user must type the task handle exactly (for example `T-12`). `--yes` is not an option of these commands. Any other input abandons the operation. The subject of `approve` is the latest plan hash from `DocFlow::inbox`. The subject of `close` is the evidence hash of the latest plan hash and the ids of the task's evidence, computed by the command from the stored records, because the inbox shows only the plan hash and `DocFlow::close` compares the evidence hash.
- Reason and `y/N` (`reject`, `cancel`, `reopen`, `dismiss`): the reason comes from the command line and must not be empty. The user answers `y` to write. Any other answer abandons the operation.
- Object and `y/N` (`baseline add`, `baseline revoke`, `note-share`, `note-unshare`, `edit`, `dep rm`, `link rm`, `adopt`, `ack`, `relocate`, `rebase`, `lineage confirm`, `lineage reject`, `export` to a path outside `.ladon/exports`, `migrate --accept-unverified-backup`): the display shows the object, its version or content hash (first eight characters) and the subject hash. `--yes --expect-hash <prefix>` writes without the prompt. The prefix must match the current subject hash by `hash_prefix_matches`; a mismatch is `E_STALE_SUBJECT`. The prefix only confirms the subject the user saw; the service receives the subject it reads now and performs its own comparison. `confirm_yes` runs before the console check, so a malformed `--expect-hash` is `E_INVALID_ARG` on a pipe as well.

An abandoned confirmation returns exit 0 with `已放弃，未写入任何内容。` (`Abandoned; nothing was written.`) on standard output in text mode, and `{"ok":true,"data":{"written":false}}` in JSON mode. It makes no service write.

### Command mapping

| Command | Backing call |
|---|---|
| `inbox` | `DocFlow::inbox`; `--watch` repeats while `wait_refresh` returns true |
| `baseline add` | `DocTaskService::register_baseline` with `Shown` built from the current latest version (or `--version`) |
| `baseline list` | `DocTaskService::list_baselines` |
| `baseline revoke` | `DocTaskService::revoke_baseline` |
| `outline`, `clause` | `Engine::load_version` for the chosen version, `doctask_outline::outline`, `resolve`, `clause_bytes`; baseline status from `DocStore::baselines` |
| `note` | `DocTaskService::add_annotation` |
| `notes` | `DocTaskService::list_annotations` |
| `note-share`, `note-unshare` | `DocTaskService::set_annotation_visibility`; expected text hash is the SHA-256 of the annotation text shown |
| `new` | `DocTaskService::create_task`; each `--depends` entry then calls `DocFlow::add_dependency` |
| `edit` | `DocTaskService::edit_task`; expected hash is `task_content_hash` of the task shown |
| `list` | `DocTaskService::list_tasks`, filtered and ordered in the command layer (priority, then task number) |
| `show` | `DocTaskService::show_task`, with evidence and links from `DocStore` reads filtered by task |
| `trace` | `DocTaskService::show_task`, `list_annotations`, `list_tasks` (downstream tasks by dependency), `DocStore` reads for links, evidence and approvals; rendered with `render_trace` |
| `history` | `DocStore::events` filtered by task or file |
| `status` | `DocFlow::start_analysis` (open to analysis), `DocFlow::rework` (working to analysis, with reason), `DocFlow::return_to_work` (verification to working, with reason), `DocFlow::report_work` (working to verification, with summary) |
| `plan new` | `DocFlow::submit_plan` with `PlanSpec { edit: None }`: an analysis-only plan |
| `plan show` | `DocStore::plans` and `approvals` filtered by task; `number` selects a plan number |
| `approve` | `DocFlow::approve` with the plan hash from `DocFlow::inbox` (typed confirmation) |
| `reject` | `DocFlow::reject`; the task returns to `analysis` (T4) |
| `close` | `DocFlow::close` with the subject described under Human-only confirmation (typed confirmation); `--accept-agent-evidence` numbers are the `#k` positions of `evidence list` |
| `cancel`, `reopen` | `DocFlow::cancel`, `DocFlow::reopen` |
| `adopt`, `dismiss` | `DocFlow::adopt`, `DocFlow::dismiss` |
| `dep add`, `dep rm`, `dep list` | `DocFlow::add_dependency`, `DocFlow::remove_dependency`, `DocStore::dependency_edges` |
| `link add`, `link rm`, `link list` | `DocFlow::add_link`, `DocFlow::remove_link`, `DocStore::links` |
| `evidence add`, `evidence list` | `DocFlow::add_evidence`; `DocStore::evidence` ordered by creation time |
| `anchors` | `Reconciler::report`; with `--check`, `Reconciler::reconcile_file` or `reconcile_all` |
| `ack`, `relocate`, `rebase` | `Reconciler::ack`, `relocate`, `rebase`; expected version and hash from the task's anchor in `DocStore::anchors` |
| `lineage scan`, `list`, `confirm`, `reject` | `Reconciler::scan_lineage`, `lineage`, `confirm_lineage`, `reject_lineage` |
| `schedule` | `DocFlow::schedule` |
| `verify` | `DocFlow::verify`; any finding of level `error` returns `E_INTEGRITY` after the findings are printed |
| `export nema`, `export report` | See Export below |
| `migrate` | `Store::ensure_schema(accept_unverified_backup)` and, with `--import-legacy`, `DocStore::backfill`; prints the backup state and the backfill counts |

Read-only commands attach with `DocTaskService::attach` or `DocFlow::attach`. `attach` never migrates, so a database below schema 4 returns `E_SCHEMA_OUTDATED` (exit 4) until `doc migrate` runs.

### Export

`doc export nema` and `doc export report` build one `ReportInput` and pass it to `render_nema`, `render_markdown` or `render_json` with an `ExportOptions` made from the scope and the two visibility switches. A scope is `--task` (the task is resolved through `DocTaskService::show_task`), `--file` (resolved through `DocTaskService::resolve_file`, with `--section` kept as the selector text after `parse_section_selector` accepts it) or everything.

The input is read as follows:

- files and versions: `Store::list_files`, and `Store::versions` for each file. A version has no ledger event id in its row, so `ReportVersion.ledger_event_id` is empty.
- clauses: `DocStore::anchors`. The clause number is the anchor's clause id and the ordinal is its first line.
- annotations: `DocStore::annotations`, with the anchor state taken from the anchor of the same id; an annotation with no anchor row is given the state `unchecked`.
- tasks: `DocStore::tasks`, converted to `TaskRecord`. A task row does not store its cancel reason, so `TaskRecord.cancel_reason` is empty.
- links: `DocStore::links`, without the removed links.
- dependencies: `DocStore::dependency_edges`, mapped to `(prerequisite, dependent)`.
- plans, approvals, evidence and events: `DocStore::plans`, `approvals`, `evidence` and `events`.
- commits and plan statistics: empty. A committed edit comes from applying an edit plan, which is outside this package (WP14); no commit rows exist before that.

A stored value that cannot be converted to its enum is `E_INTEGRITY` (exit 9) and nothing is written.

The output path:

- Without `--out` the file is `<root>/.ladon/exports/<kind>-<yyyymmdd-hhmmss>.<extension>` (extension `nema`, `md` or `json`), with a numeric suffix when the name is taken in the same second.
- With `--out` a relative path is taken from the current directory. The path is normalised lexically. A device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension) or a `\\.\`, `\\?\` or `//./` path is `E_INVALID_ARG`. An existing directory or other non-regular file is `E_INVALID_ARG`. An existing regular file needs `--force`, otherwise `E_INVALID_ARG`. A path inside `<root>/.ladon` outside `.ladon/exports` is `E_INVALID_ARG`.
- Containment in `.ladon` and `.ladon/exports` is decided on located paths: the longest existing ancestor of the path is canonicalised and the rest is appended, so the workspace root and the `--out` value are compared in the same spelling. The file is written under the normalised path given by the user.
- A path outside `.ladon/exports` needs the object confirmation of a human: the console check, then the `y/N` answer. A refusal is abandoned with nothing written.

`--generated-at` is written verbatim and must not be empty or contain control characters; the default is the local time in the form `%Y-%m-%dT%H:%M:%S%:z`. The text is written to a `.<name>.tmp` sibling and renamed over the destination; a failed rename removes the temporary file. The output is then hashed with `sha256_hex`, and `DocStore::append_event` appends `dt.export.generated` with subject kind `export`, subject id equal to the hash, the principal as actor, and the payload `{"kind","path","sha256"}`. The result lines are the path and `sha256: <hash>`; the JSON data is `{"kind","path","sha256","bytes"}`.

### Legacy commands

`annotate` and `task` in `entry.rs` call `annotate_legacy` and `task_legacy`. Both open the service with `DocTaskService::open`, which runs `ensure_schema(false)` and the idempotent backfill, so these two commands work on a fresh or older workspace. The service legacy paths require no baseline and create legacy-sourced records. The file argument and the `--section` default `ROOT` are unchanged. `--actor` defaults to `human`.

### Handles

Task handles (`T-12`, `12`, a full UUID, or a hexadecimal prefix of at least eight characters) and annotation handles (`N-3`) are parsed by `parse_task_handle` and `parse_annotation_handle`. Section selectors are parsed by `parse_section_selector`. The file argument accepts a relative path, an absolute path or a file id prefix of at least eight hexadecimal characters, through `DocTaskService::resolve_file`.

## Change Notes

- The confirmation order (validation, console check, subject load, prompt) makes the non-console refusal independent of the workspace state.
- The console check uses `GetConsoleMode` rather than the standard library's terminal test, which also accepts MSYS pseudo-terminals.
- Human-only confirmation for the object classes does not re-enter the service with a display token. The service re-reads the subject it compares against, as the service operations define.
- The close subject is computed in the command because the inbox shows the plan hash alone while the flow compares the evidence hash.
- The legacy commands migrate on open. Read-only `doc` commands do not migrate.
