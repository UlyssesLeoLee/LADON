# Technical Design: `src/doctask_cli.rs`

Source: `src/doctask_cli.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`doctask_cli` is the one-shot command layer of the document-task module. It owns the `ladon doc` command tree (clap definitions), the mapping from each command to a service call, the human-only confirmation flow, the text and JSON output, and the mapping from error codes to process exit codes. It also provides the two legacy helpers `annotate_legacy` and `task_legacy` that `entry.rs` calls for the commands `annotate` and `task`.

The module holds no SQL. Every read and write goes through `DocTaskService`, `DocFlow`, `Reconciler`, `DocStore` (read accessors only), `Engine` (file bytes and versions) or the pure modules `doctask_outline`, `doctask_render` and `doctask_report`. It never opens the database directly and never reads environment variables other than `LADON_LANG`.

Console access is injected through the `DocIo` trait, so every confirmation path is testable in process. Only `SystemIo` touches the operating system: standard input and output, and the sleep between watch refreshes.

## Dependencies

- `crate::engine::Engine` for `Engine::open` results, `load_version`, `store` and `root`.
- `crate::doctask_service`: `DocTaskService::attach`, `Caller`, `Shown`, `TaskSpec`, `TaskEdit`, `task_content_hash`, `hash_prefix_matches`, `confirm_yes`.
- `crate::doctask_flow`: `DocFlow::attach`, `PlanSpec`, `EditSpec` (not used in this package), `Step`, `PlanRef`, `Inbox`, `ScheduleView`, `VerifyReport`.
- `crate::doctask_reconcile`: `Reconciler::attach`, `ReconcileReport`, `MoveLine`, `AnchorStep`, `LineageRow`.
- `crate::doctask_store::DocStore` read accessors (`tasks`, `annotations`, `baselines`, `plans`, `approvals`, `evidence`, `links`, `anchors`, `events`, `dependency_edges`, `lineage`, `versions` through the engine) and the write accessors `backfill` and `schema_current`.
- `crate::doctask_model`: `ErrorCode`, `DocError`, `Visibility`, `TaskState`, `ProposalState`, `Risk`, `sha256_hex`, `parse_task_handle`, `parse_annotation_handle`, `parse_section_selector`, `MIN_HEX_PREFIX_CHARS`.
- `crate::doctask_outline` for `outline`, `resolve` and `clause_bytes` (the outline of a version and the text of a clause).
- `crate::doctask_render` for `Lang`, `Mode`, `detect_mode`, `render_error`, `short_hash`, `display_width`, `pad_to`, `render_trace`, `render_approval`, `render_outline`.
- `crate::doctask_report` for `ReportInput`, `ExportOptions`, `Scope`, `render_markdown`, `render_json`, `render_nema`.
- `clap` (derive), `serde_json`, `anyhow`, `std::io`, `std::time::Duration`, and the Windows `kernel32` console query declared in this module (`extern "system"`, same form as `google_oauth.rs`).

## Public Contract

### Command tree

`DocArgs` is an `Args` struct, flattened into the `Command::Doc` variant of `entry::Command`. It carries the options that apply to every `doc` command and that are propagated to the subcommands with `global = true`:

| Option | Meaning |
|---|---|
| `--json` | One JSON object on standard output, no other standard output (see Output). |
| `--no-color` | Plain text rendering (the output never contains ANSI sequences in any case). |
| `--lang zh\|en` | Message language. Default: `LADON_LANG` when it is `en`, otherwise `zh`. |
| `--actor NAME` | Human principal name written to the chain. Default `human`. |

`DocCmd` is the subcommand enum. Its variants and their arguments follow 57 section 9.2:

- `Inbox { watch }`
- `Baseline(BaselineCmd)`: `Add { file, version, note, yes, expect_hash }`, `List { file }`, `Revoke { id, reason, yes, expect_hash }`
- `Outline { file, version }`, `Clause { file, selector, version }`
- `Note { file, selector, text, private, rationale, quote }`, `Notes { file, section }`
- `NoteShare { id, yes, expect_hash }`, `NoteUnshare { id, yes, expect_hash }`
- `New { file, selector, title, description, owner, priority, depends, private }`
- `Edit { handle, title, description, owner, priority, private, share, yes, expect_hash }`
- `List { status, file, owner, proposed, all, limit }`, `Show { handle }`, `Trace { target }` (one handle, or a file and a selector), `History { handle, file, limit }`
- `Status { handle, target, reason, summary }`
- `Plan(PlanCmd)`: `New { handle, summary, rationale, impact, risk, option }`, `Show { handle, version }`
- `Approve { handle, option, comment }`, `Reject { handle, comment }`
- `Close { handle, waive, accept_agent_evidence }`, `Cancel { handle, reason }`, `Reopen { handle, reason }`
- `Adopt { handle, yes, expect_hash }`, `Dismiss { handle, reason }`
- `Dep(DepCmd)`: `Add { handle, depends_on }`, `Rm { handle, depends_on, yes, expect_hash }`, `List { handle }`
- `Link(LinkCmd)`: `Add { handle, kind, file, section, commit, url, task, label }`, `Rm { handle, link_id, yes, expect_hash }`, `List { handle }`
- `Evidence(EvidenceCmd)`: `Add { handle, kind, result, summary, version }`, `List { handle }`
- `Anchors { file, check }`, `Ack { handle, yes, expect_hash }`, `Relocate { handle, selector, yes, expect_hash }`, `Rebase { handle, yes, expect_hash }`
- `Lineage(LineageCmd)`: `Scan`, `List`, `Confirm { id, yes, expect_hash }`, `Reject { id, yes, expect_hash }`
- `Schedule`, `Verify { handle }`
- `Export(ExportCmd)`: `Nema { scope, out, force, include_local_only, include_sensitive, generated_at }`, `Report { scope, format, out, force, include_local_only, include_sensitive, generated_at }`
- `Migrate { import_legacy, accept_unverified_backup }`

`doc apply`, `doc recover`, `doc plan show --diff` and the edit options of `doc plan new` (`--file --section --kind --text-file`) are not variants of this enum. They belong to the edit-plan package and are rejected by clap as unknown arguments.

### Entry points

- `pub async fn run_doc(engine: &Engine, args: &DocArgs, io: &mut dyn DocIo) -> anyhow::Result<i32>`: runs one `doc` command and returns the process exit code. A `DocError` is printed (text on stderr, or JSON on standard output) and mapped to its exit code. Any other error is returned as `Err` and becomes the runtime's `Error: ...` with exit 1.
- `pub async fn annotate_legacy(engine: &Engine, actor: &str, file: &str, section: &str, text: &str, io: &mut dyn DocIo) -> anyhow::Result<i32>`: calls `DocTaskService::legacy_annotate` and prints `annotation id: <id>` on standard output.
- `pub async fn task_legacy(engine: &Engine, actor: &str, file: &str, section: &str, title: &str, description: &str, io: &mut dyn DocIo) -> anyhow::Result<i32>`: calls `DocTaskService::legacy_task`, prints `task id: <id>` on standard output and `任务编号 T-<n>，查看: ladon doc show T-<n>` on standard error (English form with `--lang en`).
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

- Text: the ASCII columns first and the free text after them. A successful command ends with one `下一步: …` line (`Next: …` in English). The legacy helpers print no next-step line.
- JSON: exactly one object on standard output, `{"ok":true,"data":…}` on success or `{"ok":false,"code":"E_…","message":"…","hint":"…"}` on a `DocError`. Keys are emitted by `serde_json` with the field names above. Standard error stays empty in JSON mode.
- Errors in text mode: `render_error` output (`错误[CODE]: 消息` and `提示: …`) on standard error.

### Exit codes

| Code | Error codes |
|---|---|
| 0 | success, and an abandoned confirmation (`"written": false`) |
| 1 | not a `DocError` (returned as `Err`) |
| 3 | `E_NOT_FOUND`, `E_AMBIGUOUS`, `E_SECTION_NOT_FOUND` |
| 4 | `E_INVALID_ARG`, `E_NO_BASELINE`, `E_FILE_MISSING`, `E_TRANSITION_DENIED`, `E_GUARD_FAILED`, `E_DEP_CYCLE`, `E_DEP_DANGLING`, `E_ANCHOR_BROKEN`, `E_UNSUPPORTED_FILE`, `E_PLAN_TOO_LARGE`, `E_QUOTA`, `E_RATE_LIMIT`, `E_SCHEMA_OUTDATED`, `E_MIGRATION_BACKUP` |
| 5 | `E_HUMAN_ONLY`, `E_NOT_INTERACTIVE` |
| 6 | `E_CONFLICT`, `E_STALE_PLAN`, `E_STALE_SUBJECT`, `E_FILE_CHANGED` |
| 7 | `E_STORE_BUSY` |
| 8 | `E_POLICY_DENIED` |
| 9 | `E_INTEGRITY` |
| 1 | `E_INTERNAL` |

## Behaviour

### Human-only confirmation

Every command that writes a human-only operation runs in this order:

1. Validate the arguments: `confirm_yes` (`--yes` needs `--expect-hash` of at least eight hexadecimal characters) and a non-empty reason where one is required. A failure is `E_INVALID_ARG`.
2. If `stdin_is_console` or `stdout_is_console` is false, return `E_NOT_INTERACTIVE` (exit 5). No service call has been made.
3. Load the subject through the attached service and build the display.
4. Ask for the confirmation of its class and act only on an exact answer.

The three classes follow 57 section 9.5:

- Typed (`approve`, `close`): the display shows the task handle, the title and the subject hash (`plan_hash`, first eight characters). The user must type the task handle exactly (for example `T-12`). `--yes` is not an option of these commands. Any other input abandons the operation.
- Reason and `y/N` (`reject`, `cancel`, `reopen`, `dismiss`): the reason comes from the command line and must not be empty. The user answers `y` to write. Any other answer abandons the operation.
- Object and `y/N` (`baseline add`, `baseline revoke`, `note-share`, `note-unshare`, `edit`, `dep rm`, `link rm`, `adopt`, `ack`, `relocate`, `rebase`, `lineage confirm`, `lineage reject`, `export` to a path outside `.ladon/exports`, `migrate --accept-unverified-backup`): the display shows the object, its version or content hash (first eight characters) and the subject hash. `--yes --expect-hash <prefix>` writes without the prompt. The prefix must match the current subject hash by `hash_prefix_matches`; a mismatch is `E_STALE_SUBJECT`. The prefix only confirms the subject the user saw; the service receives the subject it reads now and performs its own comparison.

An abandoned confirmation returns exit 0 with `已放弃，未写入任何内容。` (`Abandoned; nothing was written.`) on standard error in text mode, and `{"ok":true,"data":{"written":false}}` in JSON mode. It makes no service write.

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
| `plan new` | `DocFlow::submit_plan` with `PlanSpec { edit: None }` |
| `plan show` | `DocStore::plans` and `approvals` filtered by task; `version` selects a plan number |
| `approve` | `DocFlow::approve` with the plan hash from `DocFlow::inbox` (typed confirmation) |
| `reject` | `DocFlow::reject` |
| `close` | `DocFlow::close` with the plan hash from `DocFlow::inbox` (typed confirmation); `--accept-agent-evidence` numbers are the `#k` positions of `evidence list` |
| `cancel`, `reopen` | `DocFlow::cancel`, `DocFlow::reopen` |
| `adopt`, `dismiss` | `DocFlow::adopt`, `DocFlow::dismiss` |
| `dep add`, `dep rm`, `dep list` | `DocFlow::add_dependency`, `DocFlow::remove_dependency`, `DocStore::dependency_edges` |
| `link add`, `link rm`, `link list` | `DocFlow::add_link`, `DocFlow::remove_link` (the task handle is found from `DocStore::links`), `DocStore::links` |
| `evidence add`, `evidence list` | `DocFlow::add_evidence`; `DocStore::evidence` ordered by creation time |
| `anchors` | `Reconciler::report`; with `--check`, `Reconciler::reconcile_file` or `reconcile_all` |
| `ack`, `relocate`, `rebase` | `Reconciler::ack`, `relocate`, `rebase`; expected version and hash from the task's anchor in `DocStore::anchors` |
| `lineage scan`, `list`, `confirm`, `reject` | `Reconciler::scan_lineage`, `lineage`, `confirm_lineage`, `reject_lineage` |
| `schedule` | `DocFlow::schedule` |
| `verify` | `DocFlow::verify`; any finding of level `error` returns `E_INTEGRITY` after the findings are printed |
| `export nema`, `export report` | `DocStore` and `Engine` reads assembled into `ReportInput`; `render_nema` or `render_markdown` or `render_json`; the output is written to a temporary file and renamed; an existing file requires `--force`; a path outside `.ladon/exports` needs the object confirmation; device names and non-regular files are refused with `E_INVALID_ARG`; the `dt.export.generated` event is appended through `DocStore::append_event` after the write |
| `migrate` | `Store::ensure_schema(accept_unverified_backup)` and, with `--import-legacy`, `DocStore::backfill`; prints the backup state and the backfill counts |

Read-only commands attach with `DocTaskService::attach` or `DocFlow::attach`. `attach` never migrates, so a database older than schema 4 returns `E_SCHEMA_OUTDATED` (exit 4) until `doc migrate` runs.

### Legacy commands

`annotate` and `task` in `entry.rs` call `annotate_legacy` and `task_legacy`. They use the service legacy paths, which require no baseline and create legacy-sourced records. The file argument and the `--section` default `ROOT` are unchanged. `--actor` defaults to `human`.

### Handles

Task handles (`T-12`, `12`, a full UUID, or a hexadecimal prefix of at least eight characters) and annotation handles (`N-3`) are parsed by `parse_task_handle` and `parse_annotation_handle`. Section selectors are parsed by `parse_section_selector`. The file argument accepts a relative path, an absolute path or a file id prefix of at least eight hexadecimal characters, through `DocTaskService::resolve_file`.

## Change Notes

- The confirmation order (validation, console check, subject load, prompt) makes the non-console refusal independent of the workspace state.
- The console check uses `GetConsoleMode` rather than the standard library's terminal test, which also accepts MSYS pseudo-terminals.
- Human-only confirmation for the object classes does not re-enter the service with a display token. The service re-reads the subject it compares against, as the service operations define.
