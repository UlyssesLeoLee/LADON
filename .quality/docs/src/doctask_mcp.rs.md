# Technical Design: `src/doctask_mcp.rs`

Source: `src/doctask_mcp.rs`

Source-SHA256: `6f780cce040c42406090f890c96f7c0f5dd8a873a0f9e6b94918cd7948bc93ab`

## Responsibility

`doctask_mcp` is the tool host of `ladon mcp-serve`. It offers the thirteen tools of design 57 section 10.2, validates the arguments of each call, maps the call to the operations of `DocTaskService` and `DocFlow` for the agent principal, writes the disclosure record before any content leaves the process, wraps each call in a panic guard and turns the outcome into the result envelope of design 57 section 10.3. It also runs the stdio server loop: it reads the process arguments already parsed by `entry`, opens the engine lazily on the first `tools/call`, installs the panic hook and returns the exit code.

The module holds no SQL. Storage is reached through `Engine`, `DocStore` (the disclosure insert only, through `insert_row`) and the services. The protocol framing, lifecycle and version negotiation belong to `mcp.rs`; this module implements `mcp::ToolHost` and passes the result object back unchanged.

The module writes nothing to standard output. Its single diagnostic line per call goes to standard error through `std::io::Write`, never through the print macros, and it never contains arguments or content.

Not covered here: the operations themselves (`doctask_service`, `doctask_flow`, `doctask_reconcile`), the edit plan tool `doc_plan_submit`, the one-shot command layer (`doctask_cli`), the Hermes launcher (`hermes`).

## Dependencies

- `crate::mcp`: `Server`, `ServerInfo`, `ToolHost`, `ToolCall`.
- `crate::engine::Engine`: `open`, `store` (`list_files`, `versions`, `latest_version`, `version`), `load_version`, the gate methods through `EngineGate`.
- `crate::doctask_service`: `DocTaskService` (`attach`, `resolve_file`, `list_tasks`, `show_task`, `list_baselines`, `list_annotations`, `create_task`, `add_annotation`), `Caller`, `TaskSpec`, `EngineGate`, `TransferGate`, `TaskView`, `TaskCard`, `AnnotationView`.
- `crate::doctask_flow`: `DocFlow` (`attach`, `schedule`, `admit_proposal`, `start_analysis`, `rework`, `report_work`, `return_to_work`, `add_dependency`, `add_evidence`, `add_link`).
- `crate::doctask_store`: `DocStore` (`new`, `insert_row` into `doc_agent_disclosures`, `links`, `evidence`, `approvals`).
- `crate::doctask_outline`: `outline`, `resolve`, `clause_bytes`, `Outline`, `Clause`.
- `crate::doctask_model`: `DocError`, `ErrorCode`, `parse_section_selector`, `sha256_hex`, `ActorKind`, `Principal` (through `Caller`).
- `crate::store::StoreBusy`: maps a lock failure to `E_STORE_BUSY`.
- External crates: `serde_json`, `anyhow`, `chrono` (disclosure timestamp), `std::panic`, `std::collections::HashSet`.

## Public Contract

### Constants

- `TOOL_COUNT: usize = 13`: the tools this module delivers.
- `READ_ONLY_TOOLS`: the eight names with `annotations.readOnlyHint = true`: `doc_file_find`, `doc_clause_list`, `doc_clause_read`, `doc_mentions_find`, `doc_task_list`, `doc_task_get`, `doc_trace_get`, `doc_schedule_get`.
- `MUTATING_TOOLS`: the five names without the annotation: `doc_task_create`, `doc_annotation_add`, `doc_task_transition`, `doc_evidence_add`, `doc_link_add`.
- `RESULT_LIMIT_BYTES: usize = 32 * 1024`: the largest serialized result text, in UTF-8 bytes.
- `LAUNCH_ID_MAX: usize = 64`.
- `MENTION_FILE_LIMIT: usize = 2000` and `MENTION_SIZE_LIMIT: i64 = 1_048_576`: scan bounds of `doc_mentions_find`.
- `CLAUSE_TEXT_LIMIT: usize = 3000`: characters of the clause text included in `doc_task_get`.

### Tool descriptors

`pub fn tool_descriptors() -> Vec<Value>` returns the thirteen descriptors in the order of the table below. Each descriptor is `{name, description, inputSchema}` plus `annotations: {readOnlyHint: true}` for the read-only tools only. `description` is one Chinese sentence. `inputSchema` is generated from the same field table that validates the arguments, so the advertised schema and the enforced rules cannot diverge. Its root is `{"type":"object","additionalProperties":false,"properties":{…},"required":[…]}` (`required` may be empty). Every property carries only `type`, `description`, `enum`, `minLength`, `maxLength`, `minimum`, `maximum`, `items` and `maxItems`, and every `description` is one sentence. No `$ref`, `oneOf`, `anyOf`, `nullable` or `outputSchema` appears. Strings are advertised with `minLength` only when the minimum is above zero.

### Argument validation

`pub fn validate_arguments(tool: &str, arguments: &Value) -> Result<Map<String, Value>, DocError>` checks the arguments of a known tool with the field table. It returns the accepted object with the supplied values only; defaults are applied by the handlers. Every failure is `E_INVALID_ARG` with a message naming the property. The rules are:

| Field kind | Accepted value |
| --- | --- |
| text | a JSON string whose length in characters lies in `[min, max]`; control characters other than line feed and tab are refused |
| integer | a JSON integer (a float such as `2.0` is refused) in `[min, max]` |
| boolean | a JSON boolean |
| choice | one of the listed strings, exactly |
| list | a JSON array of at most `max_items` strings, each within the text rules of its item bound |

An absent optional property is not refused. Unknown properties are refused. A non-object `arguments` value is refused, and `null` is read as an empty object. Cross-property rules are checked after the field rules and are listed under each tool.

### Result envelopes

`pub fn envelope_ok(data: Value, summary_zh: &str, next_zh: &str) -> Value` returns `{"content":[{"type":"text","text":T}]}` where `T` is the compact JSON of `{"ok":true,"data":…,"summary_zh":…,"next_zh":…}`. The result object has no `isError` member.

`pub fn envelope_refused(code: ErrorCode, message: &str, hint: &str) -> Value` returns the same shape with `T = {"ok":false,"code":…,"message":…,"hint":…}`. It has no `isError` member and no member named `error`. Business refusals, invalid arguments, `E_STORE_BUSY` and `E_SCHEMA_OUTDATED` use this form and do not count toward the host's failure counter.

`pub fn internal_failure() -> Value` returns the same shape with `code = E_INTERNAL`, the fixed message and hint of that code, and `"isError": true`. It is used for the panic guard and for any error that is neither a `DocError` nor a `StoreBusy`.

Every string of every envelope passes `sanitize` first, which removes control characters other than line feed and tab. If the serialized text exceeds `RESULT_LIMIT_BYTES`, the list payloads were already cut by `fit_list` (below); a text that still exceeds the limit is replaced by `E_INTERNAL` with `isError`.

`pub fn fit_list(items: Vec<Value>, budget: usize) -> (Vec<Value>, bool)` keeps the longest prefix whose serialized size stays within `budget` bytes and reports whether items were dropped. The list tools call it with `RESULT_LIMIT_BYTES` minus 1024 bytes of envelope overhead, and report the dropped state as `truncated`.

### The host

`pub struct DocToolHost` implements `mcp::ToolHost`. Its fields are private: the workspace root, the launch id, the optional global rule file, the engine (absent until the first valid `tools/call`) and the disclosure keys already written by this process.

- `DocToolHost::new(root: PathBuf, launch_id: String, global_ignore: Option<PathBuf>) -> DocToolHost`: stores the parts; opens nothing.
- `tools()` returns `tool_descriptors()`.
- `call(name, arguments)`: returns `None` for a name outside the thirteen tools (the protocol layer then answers -32602 and writes nothing else). For a listed name it returns a `CatchPanic` future around the call.

The call sequence of a listed tool follows design 57 section 10.4 for the parts this module performs:

1. Validate the arguments (`validate_arguments`). A failure returns `envelope_refused(E_INVALID_ARG)` at once, before the engine is opened.
2. Open the engine when it is not open yet (`Engine::open` on the root; the engine is kept for later calls). A failed open is `E_STORE_BUSY` for a lock failure, `E_POLICY_DENIED` when the global rule file named by `--global-ignore` cannot be read, and `E_INTERNAL` otherwise. A failed open is not cached; the next call tries again.
3. Run the handler with the agent caller `Caller::agent(launch_id)`. Every service operation performs the principal check, the file and task resolution, the transfer gate and the projection of design 57 section 13.1, so the handler does not repeat them.
4. For read tools that return bytes or text derived from bytes, write the disclosure rows before the result is built (see Disclosure).
5. Build the envelope, sanitize it, write one stderr line (`mcp-serve tool=<name> ms=<elapsed> code=<ok|E_…>`) and return.

### Handlers

Read tools. Every read tool attaches with `DocTaskService::attach` (or `DocFlow::attach` for `doc_schedule_get`), which returns `E_SCHEMA_OUTDATED` below schema 4 and writes nothing. Defaults: `limit` 20 for `doc_file_find` and `doc_task_list`, 10 for `doc_mentions_find`; `events` 10; `version` the latest; `offset` 0; `max_chars` 3000.

| Tool | Arguments | Data |
| --- | --- | --- |
| `doc_file_find` | `query?` (1–100 characters), `has_tasks?` (boolean), `limit?` (1–50) | `{files:[{file_ref, path, version_no, baseline_version_no, open_tasks}], truncated}`. Files are listed when not deleted and allowed by the file gate. `query` is a case-insensitive substring of the path. `baseline_version_no` is null when no baseline is active. `open_tasks` counts the visible tasks of the path whose status is neither `done` nor `cancelled`. |
| `doc_clause_list` | `file_ref` (1–200), `version?` (integer ≥ 1) | `{file_ref, version_no, rows:[{ordinal, number, title, level, first_line, last_line, notes, tasks}], truncated}`. `notes` counts the shared annotations of the clause, `tasks` the visible tasks of the path and clause key. Rows are the outline of the version (`Outline::clauses`); a file without an outline has one `ROOT` row. |
| `doc_clause_read` | `file_ref` (1–200), `section` (1–200), `version?`, `offset?` (≥ 0), `max_chars?` (200–6000) | `{text, version_no, hash, truncated, next_offset}`. `offset` and `max_chars` count characters. `next_offset` is the offset of the next window or null. `hash` is the SHA-256 of the clause bytes of the whole clause. |
| `doc_mentions_find` | `token` (2–80), `limit?` (1–30) | `{matches:[{file_ref, path, section, line}], truncated}`. The search is a literal, case-sensitive substring test on the latest version of each tracked `md`, `nema` or `txt` file of at most 1 MiB, in path order, over at most 2000 candidate files. `section` is the key of the clause containing the line, or `ROOT`. `line` is 1-based. `truncated` is true when the limit or the file bound stopped the scan. |
| `doc_task_list` | `status?` (array of the seven states, at most 7), `file_ref?` (1–200), `owner?` (1–200), `proposed_only?` (boolean), `limit?` (1–50), `cursor?` (1–20 characters) | `{tasks:[{task, id, title, status, priority, owner, file_ref, section, anchor_state, proposal_state, blocked_by, updated_at}], hidden, next_cursor, truncated}`. The cursor is the decimal offset of the next item of the filtered, visible list; an invalid cursor is `E_INVALID_ARG`. `file_ref` matches the file id or the path. |
| `doc_task_get` | `task` (1–20), `events?` (0–30) | `{task:{…task fields of doc_task_list, description}, events:[{seq, kind, from_status, to_status, actor, actor_kind, timestamp}], last_decision, clause_text, clause_truncated, annotations}`. `events` is the last N events of the chain in `seq` order; payloads are never present. `last_decision` is `{decision, comment, by, at}` of the latest approval row of the task, or null. `clause_text` is the clause of the task section in its base version, cut to `CLAUSE_TEXT_LIMIT` characters, or null when the base version is refused or the section does not resolve. `annotations` are the shared annotations of the section, cut to half of the list budget, with `annotations_truncated` set when items were dropped. |
| `doc_trace_get` | `task?` (1–20), `file_ref?` (1–200), `section?` (1–200) | `{traces:[{task, status, links:[…], evidence:[…]}], hidden, truncated}`. Either `task`, or both `file_ref` and `section`, is required; otherwise `E_INVALID_ARG`. A link to a task that the caller cannot see is counted in `hidden` and not named. Evidence is `{kind, result, source, actor_kind, created_at}`. |
| `doc_schedule_get` | none | `{batches, conflicts:[{file, tasks}], blocked:[{task, reason}], hidden}` from `DocFlow::schedule`. |

Mutating tools. None of them writes a disclosure row. Each records its result through the service or flow it calls.

| Tool | Arguments | Behaviour and data |
| --- | --- | --- |
| `doc_task_create` | `file_ref` (1–200), `section` (1–200), `title` (1–200), `description?` (≤ 4000), `priority?` (1–3), `depends_on?` (≤ 10 items, each 1–20), `rationale?` (≤ 1000), `hermes_session?` (≤ 64) | `DocFlow::admit_proposal` (quota and rate), then `DocTaskService::create_task` with `TaskSpec` (priority default 2, shared visibility, no owner), then `DocFlow::add_dependency` for each entry. Data `{id, handle, existing}`. `rationale` and `hermes_session` are validated and not stored, because the service and flow layers take no such parameter. `next_zh` asks the owner to adopt the proposal in the console. |
| `doc_annotation_add` | `file_ref` (1–200), `section` (1–200), `text` (1–4000), `rationale?` (≤ 1000), `quote?` (≤ 500), `hermes_session?` (≤ 64) | `DocTaskService::add_annotation` with `private = false`. Data `{id, handle, existing}`. `hermes_session` is validated and not stored. |
| `doc_task_transition` | `task` (1–20), `to` (`analysis`, `verification`, `working`), `reason?` (≤ 4000 when `to` is `verification`, otherwise ≤ 500), `hermes_session?` (≤ 64) | Transition plan of the table below. A reason is required for `verification` (the work summary) and for the moves that return to `analysis` or `working`. Data `{handle, from, to, events}`. |
| `doc_evidence_add` | `task` (1–20), `kind` (`test_run`, `report`, `note`, `version`), `result?` (1–40, default `info`), `summary` (1–1000), `ref?` (≤ 500), `version?` (1–64), `hermes_session?` (≤ 64) | `DocFlow::add_evidence`. When `ref` is given, the flow receives `summary` followed by a line `参考: <ref>`; the combined text must stay within 1000 characters (`E_INVALID_ARG` otherwise). `version` is the version id. Data `{handle, from, to, events}`. |
| `doc_link_add` | `task` (1–20), `kind` (1–64), `file_ref?` (1–200), `section?` (1–200), `commit?` (1–64), `url?` (1–2000), `label?` (≤ 120), `hermes_session?` (≤ 64) | Exactly one of `file_ref`, `commit` and `url` must be given, otherwise `E_INVALID_ARG`. `DocFlow::add_link` with target kind `file`, `commit` or `url`. Data `{handle, from, to, events}`. |

Transition plan (`doc_task_transition`), decided from the current status of the visible task:

| `to` | Status now | Operation | Reason |
| --- | --- | --- | --- |
| `analysis` | `open` | `start_analysis` (T1) | not used |
| `analysis` | `working` | `rework` (T8) | required |
| `verification` | `working` | `report_work` (T5) | required (the summary) |
| `working` | `verification` | `return_to_work` (T7) | required |

Any other combination returns `E_TRANSITION_DENIED`, and its hint lists the moves available from the current status: `open` → `analysis`; `analysis` → the plan submission of WP14; `working` → `analysis`, `verification`; `verification` → `working`. The flow layer still applies its own guards (for example that T7 by the agent requires a failing latest evidence).

### Disclosure

For each read tool that returns bytes or text derived from bytes, the handler collects `(file_id, version_id, content_hash)` for every version it read, then inserts one row per pair into `doc_agent_disclosures` with `DocStore::insert_row`, columns `launch_id`, `file_id`, `version_id`, `content_hash`, `first_at` (UTC, RFC 3339). The handler skips a pair that this process has already written (the set `disclosed`), since the key includes the launch id and the process is the only writer of that launch. Rows are written before the envelope is built. A failed insert returns `E_STORE_BUSY` as a refusal, and no content is returned. Tools that return metadata only (`doc_file_find`, `doc_task_list`, `doc_schedule_get`, `doc_trace_get`) write no disclosure row. `doc_mentions_find` discloses the versions in which a match was found. `doc_task_get` discloses the base version only when `clause_text` is returned.

### Panic guard

`pub struct CatchPanic<'a>` wraps a `ToolCall<'a>`. Each `poll` runs the inner poll inside `std::panic::catch_unwind(AssertUnwindSafe(…))`. A panic completes the future with `Some(internal_failure())`, and the server loop continues with the next line. The wrapper does not require the inner future to be `Send`, and it spawns no task.

`pub fn panic_report(location: Option<(&str, u32)>) -> String` formats the one line that the installed panic hook writes to standard error: `mcp-serve panic at <file>:<line> E_INTERNAL`, or `mcp-serve panic E_INTERNAL` when no location is known. It never includes the panic payload, because the payload may contain text from the workspace.

### Serving

`pub async fn serve(root: &Path, launch_id: &str, global_ignore: Option<&Path>) -> anyhow::Result<i32>`:

1. `launch_id` must be 1–64 characters from ASCII letters, digits and `-`. Otherwise it writes one line to standard error and returns `1`.
2. The file `<root>/.ladon/meta.turso` must exist. Otherwise it writes one line (`mcp-serve: workspace not initialized`) and returns `1`. The database itself is not opened here.
3. When `global_ignore` is given, the process variable `LADON_GLOBAL_IGNORE` is set to that path before any engine is opened.
4. The previous panic hook is saved, and the panic hook of `panic_report` is installed.
5. `Server::new(ServerInfo{name:"ladon", version: CARGO_PKG_VERSION}, DocToolHost::new(...))` serves standard input to standard output until end of input. End of input returns `0`.
6. The previous panic hook is restored, and the result is `0` on end of input. An I/O error on the streams is returned as an error.

`initialize` and `tools/list` never reach the host's engine code, so they work when the database is locked or outdated.

## Errors

| Condition | Result |
| --- | --- |
| Unknown tool name | `None` from `call`; protocol answer -32602 |
| Argument rule violated, cross-property rule violated | `E_INVALID_ARG`, refusal form |
| Quota, rate, guard, state or not-found from a service | the code of the `DocError`, refusal form |
| Refused or absent file, version, task or annotation | `E_NOT_FOUND`, refusal form, same message for every reason |
| Rule set unusable | `E_POLICY_DENIED`, refusal form |
| Database locked, disclosure insert failed by lock | `E_STORE_BUSY`, refusal form |
| Schema below 4 | `E_SCHEMA_OUTDATED`, refusal form |
| Binary or non-UTF-8 file read as text | `E_UNSUPPORTED_FILE`, refusal form |
| Panic, unexpected error | `E_INTERNAL`, `isError: true` |

## Behavioural Rules

- Read tools never write to the event chain, the meta table, the anchor table or the task rows. The one write is the disclosure row.
- The agent principal is fixed to `hermes` with session `hermes:<launch_id>`. No argument can set a principal.
- Refused objects are indistinguishable from missing ones: an error does not reveal whether a refused file exists.
- Tool names and descriptions contain no human-only operation.

## Change Notes

- The module offers the thirteen tools of design 57 section 10.2 and the stdio serve loop behind `ladon mcp-serve`.
- It depends on existing services only and changes none of their signatures. The disclosure rows use the generic `DocStore::insert_row`, so `doctask_store` gains no method.
- The command entry gains one subcommand, `mcp-serve`, that calls `serve` before any engine is opened. Existing commands are unchanged.
- Compatibility: the tool names, argument names and envelope keys are the contract with Hermes configurations; changing them requires a new design revision and a new snapshot in TC-950.
