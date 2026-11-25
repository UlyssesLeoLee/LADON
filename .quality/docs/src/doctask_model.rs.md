# Technical Design: `src/doctask_model.rs`

Source: `src/doctask_model.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`doctask_model` is the pure model of the document-task module. It holds the task state machine (seven states, three principals, eleven transitions T1 to T11 with their guard inputs), the operation authorization table, the stable error codes and `DocError`, the record types of the governance schema, the handle and section-selector grammars, the three hash functions with their tuple encodings, the field limits and text validation, the display sanitiser, the governance-chain break check and the dependency schedule. It performs no I/O and holds no database connection. Persistence, the service layer, file access and the policy gate belong to later modules.

## Dependencies

- `crate::nema::{topo_batches, Dependency}` for layering the dependency graph in `schedule_batches`. The `nema` module depends only on `anyhow` and `serde`.
- `sha2::Sha256` and `serde_json::to_vec` for the hash functions. Both crates are already declared by the root crate.
- The standard library for character classes, parsing and collections.
- The module does not import `crate::store`, `crate::engine`, `crate::doctask_store`, `crate::doctask_service` or any module that does. Later modules depend on this one, never the reverse.
- Callers: `doctask_store` (record types, `GENESIS_PREV_HASH`, hash functions, `first_chain_break`), `doctask_flow` (`lookup_transition`, `check_guards`, `authorize`, `schedule_batches`), `doctask_service` (handles, selectors, text validation, `DocError`), `doctask_cli`, `doctask_mcp`, `doctask_render` (`sanitize_display`, `ErrorCode`), `doctask_report` (`EventRecord`, `first_chain_break`).

## Public Contract

### Enumerations

Each value domain is a Rust enum with `as_str()` returning the stored string, `parse(&str) -> Option<Self>` for the exact stored string, and `ALL` listing every variant in declaration order. The domains are `TaskState` (`open`, `analysis`, `approval`, `working`, `verification`, `done`, `cancelled`), `Principal` (`human`, `agent`, `system`), `ActorKind` (`human`, `agent`, `system`, `unknown`), `ProposalState`, `SourceKind`, `AnchorState` (`valid`, `needs_review`, `relocated`, `ambiguous`, `orphaned`, `unknown`, `file_missing`, `unchecked`), `Visibility`, `PlanStatus`, `EditKind` (`""` for analysis-only, `replace_section`, `insert_after_section`), `Risk`, `ApprovalKind`, `ApprovalDecision`, `EvidenceKind`, `EvidenceResult`, `EvidenceSource`, `LinkKind`, `TargetKind` and `LineageState`. The values are those of `sql/004_document_tasks.sql` and section 3.3 of the design, and `TaskState` matches the seven legacy strings exactly.

`AnchorState::is_settled(baseline_bound, reviewed)` answers the anchor rule of guard T3-3 and T6-3: the anchor is bound to a baseline and is `valid`, or is `needs_review`, `relocated` or `unknown` and `reviewed` is true. Every other state is not settled.

### Transitions

`Transition` has variants `T1` to `T11` and `code()` returns `"T1"` to `"T11"`. `TRANSITIONS` is a static table of `TransitionSpec { id, from, to, actors, event_kind }` with sixteen rows: T1 open to analysis (human, agent); T2 analysis to approval (human, agent); T3 approval to working (human); T4 approval to analysis (human); T5 working to verification (human, agent); T6 verification to done (human); T7 verification to working (human, agent); T8 working to analysis (human, agent); T9 from each of open, analysis, approval, working and verification to cancelled (human); T10 from done and from cancelled to open (human); T11 approval to analysis (system). The `event_kind` of T10 is `dt.task.reopened`; every other row carries `dt.task.transitioned`.

`lookup_transition(from, to, principal) -> Result<Transition, DocError>`:
1. No row for the pair `(from, to)`: `E_TRANSITION_DENIED`.
2. A row for the pair lists the principal: the row's transition.
3. Otherwise, when some row for the pair is human-only and the principal is not human: `E_HUMAN_ONLY`.
4. Otherwise: `E_TRANSITION_DENIED`.

`allowed_from(from, principal) -> Vec<(TaskState, Transition)>` lists the transitions the principal may take from a state. The `E_TRANSITION_DENIED` detail carries this list.

Guard inputs and evaluation: `GuardInput<'a>` carries the principal and the facts a guard reads: `proposal_accepted`, `anchor`, `baseline_bound`, `anchor_reviewed`, `summary`, `reason`, `plan_applied_or_no_edit`, `latest_evidence_failed`, `has_applying_plan`, `latest_plan_submitted`, `subject_hash_matches`, `dependencies_done`, `plan_stale` and `evidence_accepted`. `check_guards(t, &GuardInput) -> Result<(), DocError>` evaluates the pure guards in the order of section 4:

| Transition | Check and error |
|---|---|
| T1 | `proposal_accepted` else `E_TRANSITION_DENIED` (hint: the owner must adopt the proposal); anchor `orphaned`, `file_missing` or `ambiguous` else `E_ANCHOR_BROKEN` |
| T2 | none (plan field validation is `validate_plan_text`) |
| T3 | principal human else `E_HUMAN_ONLY`; `latest_plan_submitted` else `E_GUARD_FAILED`; `subject_hash_matches` else `E_STALE_SUBJECT`; `dependencies_done` else `E_GUARD_FAILED`; anchor settled else `E_ANCHOR_BROKEN`; `plan_stale` gives `E_STALE_PLAN` (the caller commits T11 in the same transaction before returning) |
| T4, T7, T8, T9, T10 | `reason` non-empty after trimming and at most 500 characters else `E_INVALID_ARG`; T7 by agent requires `latest_evidence_failed` else `E_GUARD_FAILED`; T8 and T9 require `!has_applying_plan` else `E_GUARD_FAILED` |
| T5 | `summary` non-empty and at most 4000 characters else `E_INVALID_ARG`; `plan_applied_or_no_edit` else `E_GUARD_FAILED` |
| T6 | `evidence_accepted` else `E_GUARD_FAILED`; `dependencies_done` else `E_GUARD_FAILED`; anchor settled else `E_ANCHOR_BROKEN`; `subject_hash_matches` else `E_STALE_SUBJECT` |
| T11 | `plan_stale` else `E_GUARD_FAILED` |

### Operations and authorization

`Operation` lists the operations of section 10.5 and section 4.2 that are human-only (approve, reject, close, cancel, reopen, adopt, dismiss, apply, recover, baseline register and revoke, edit task attributes, share annotation, anchor acknowledge, relocate and rebase, lineage confirm and reject, remove dependency, remove link, export to file, verify, restore version, control Hermes). It also lists the operations open to agents (read, create proposal, add annotation, submit plan, add link, add dependency) and the system operations (reconcile, backfill, migrate, expire plan).

`allowed_principals(op)` returns `[Human]` for human-only operations, `[Human, Agent]` for agent operations and `[Human, System]` for system operations. `authorize(principal, op) -> Result<(), DocError>` succeeds when the principal is allowed, and otherwise returns `E_HUMAN_ONLY` with the allowed list in the detail. A disallowed agent or system principal therefore always receives `E_HUMAN_ONLY`.

### Errors

`ErrorCode` has the 27 codes of section 14.1, each with `as_str()` (for example `E_STALE_PLAN`), `exit_code()` (the value of section 14.1 and section 9.3) and `ALL`. `DocError { code, message_zh, hint_zh, detail }` is built with `DocError::new(code, message_zh, hint_zh)` and refined with `with_detail`. Messages are Chinese; English texts are supplied by the render module from the code.

### Handles and selectors

`Handle` is `Number(i64)`, `Uuid(String)` (lowercase canonical form) or `Prefix(String)` (lowercase hexadecimal).

`parse_task_handle(input, principal)` and `parse_annotation_handle(input, principal)` accept, after trimming:
- `T-n` (task) or `N-n` (annotation), with `n` a decimal integer of at least one digit and at most `i64::MAX`, for every principal. The other prefix is `E_INVALID_ARG`.
- For the human principal only: a decimal string, read as a number; a full UUID in the 8-4-4-4-12 hexadecimal form, read case-insensitively; a hexadecimal string of at least 8 characters that contains a letter from `a` to `f`, read as a prefix.
- Any other input, and for agents any input other than the prefixed number, is `E_INVALID_ARG`.

`SectionSelector` is `Root`, `Ordinal(usize)`, `Clause(String)` (uppercase), `Key(String)` or `Text(String)`. `parse_section_selector(Option<&str>)` maps an absent or blank input to `Root`. `#n` with `n >= 1` is `Ordinal`. `key:` followed by a non-empty exact key is `Key`. A whole input matching `[A-Za-z][A-Za-z0-9]*-[0-9]+` is `Clause`. Any other non-blank trimmed input is `Text`. Resolution against an outline belongs to the outline module.

### Limits and text validation

Lengths count Unicode scalar values. The constants are: task title 1 to 200; task description at most 4000; task-create rationale at most 1000; plan summary 1 to 500; plan rationale at most 4000; plan impact at most 2000; plan options at most 5 items of 1 to 500 characters; transition reason at most 500; verification summary 1 to 4000; annotation text 1 to 4000; annotation quote at most 500; link label at most 120; read references at most 10; agent dependencies at most 10; priority 1 to 3; `new_text` at most 262144 bytes; target file at most 1 MiB; hexadecimal prefix at least 8 characters.

`validate_text(field, value, min, max)` rejects a length outside the bounds with `E_INVALID_ARG`, and rejects any character that is a control character other than newline and tab (this includes NUL and ESC), and any bidirectional embedding, override or isolate control (U+202A to U+202E and U+2066 to U+2069), with `E_INVALID_ARG`.

`normalize_new_text(text)` rejects more than 262144 bytes with `E_PLAN_TOO_LARGE`, replaces each `\r\n` with `\n`, and rejects any remaining carriage return or other control character except newline and tab with `E_INVALID_ARG`. It does not reject bidirectional characters.

`check_plan_target(extension, bytes)` requires the extension `md`, `markdown`, `nema` or `nemat` (case-insensitive) and returns `E_UNSUPPORTED_FILE` otherwise; more than 1 MiB gives `E_PLAN_TOO_LARGE`; a NUL byte, a UTF-16 byte order mark (`FF FE` or `FE FF` at the start) or bytes that are not valid UTF-8 give `E_UNSUPPORTED_FILE`. A UTF-8 byte order mark is accepted.

`validate_priority(i64) -> Result<u8, DocError>` accepts 1 to 3.

### Display sanitiser

`sanitize_display(text) -> String` replaces every character for which `is_display_hazard` is true with the visible marker `<U+XXXX>` (four uppercase hexadecimal digits). Hazards are the control characters other than newline and tab, zero-width characters (U+200B to U+200D, U+2060, U+FEFF), left-to-right and right-to-left marks (U+200E, U+200F, U+061C) and bidirectional embedding, override and isolate controls. Newline, tab and all other characters pass unchanged.

### Hashes

`sha256_hex(bytes) -> String` returns 64 lowercase hexadecimal digits. `GENESIS_PREV_HASH` is 64 `0` characters.

- `event_hash(&EventFields)`: the SHA-256 of `serde_json::to_vec` of the tuple `(prev_hash, id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp)`, each a string, in that order. The payload text is used as stored.
- `plan_hash(&PlanFields)`: the SHA-256 of `serde_json::to_vec` of `(task_id, plan_no, summary, edit_kind, target_file_id, target_section, new_text, expected_hash, result_hash)`, where `plan_no` is an integer and the others are strings.
- `evidence_hash(plan_hash, evidence_ids)`: the ids are sorted in ascending byte order, and the SHA-256 is taken of `serde_json::to_vec` of `(plan_hash, ids)`.

### Governance chain check

`EventRecord` mirrors the row of `doc_task_events`, with `seq`, the fifteen tuple fields, `prev_hash` and `hash`. `first_chain_break(events) -> Option<i64>` walks the records in `seq` order from the genesis value and returns the `seq` of the first record whose `prev_hash` differs from the preceding hash, or whose stored `hash` differs from `event_hash` of its fields. It returns `None` for an intact chain or an empty slice.

### Records

`TaskRecord`, `PlanRecord`, `ApprovalRecord` and `EvidenceRecord` hold the columns of `doc_tasks` (with `doc_task_meta` fields), `doc_task_plans`, `doc_task_approvals` and `doc_task_evidence` that the pure logic reads. Identifiers are strings; numbers are `i64`; optional timestamps are `Option<String>`.

### Scheduling

`SchedTask { id, task_no, status, proposal_accepted, depends_on, plan_target, link_target }` describes one task. `schedule_batches(&[SchedTask]) -> Result<Schedule, DocError>` returns `Schedule { batches, conflicts, blocked, unknown_targets }`, where every task is written as the handle `T-n`.

1. Duplicate ids are `E_INVALID_ARG`. The dependency graph of all input tasks, with an edge from each known dependency to its dependent, is layered with `nema::topo_batches`. A cycle (including a self-dependency) is `E_DEP_CYCLE`.
2. The schedulable set holds the tasks whose status is `open`, `analysis`, `approval` or `working` and whose proposal is accepted. Other tasks are not scheduled and are not reported.
3. For each schedulable task, a dependency that is `done` is dropped. A dependency that is schedulable is an edge. Any other dependency blocks the task: `cancelled` gives the reason "dependency cancelled", an unaccepted proposal gives "dependency proposed", an unknown id gives "dependency not found" and any other status gives "dependency not schedulable".
4. A schedulable task that depends on a blocked task is blocked with the reason `upstream blocked: T-n`, repeated to a fixed point.
5. The unblocked tasks are layered with `nema::topo_batches`. Each batch is sorted by `task_no`.
6. The write target of a task is `plan_target` when present, otherwise `link_target`, otherwise the task is listed in `unknown_targets` (it stays in its batch).
7. Within a batch, tasks with the same known target form a conflict group. Each group lists its tasks in `task_no` order and the groups are sorted by file, then by their first task number.
8. `blocked` is sorted by `task_no`, and `unknown_targets` likewise.

## Change Notes

The module is new and has no predecessor. Its items are the designed contract of the document-task model. Choices that the design leaves open are fixed here: a disallowed principal on any human-only or system operation receives `E_HUMAN_ONLY`; `Handle` treats an all-digit string as a number, not a prefix; the evidence hash encodes the sorted id list as a JSON array; option items use the same bounds as plan summaries; the transitive blocking rule of scheduling keeps the order guarantee of dependencies; the cycle check covers the whole input graph, including tasks that are not scheduled.

`lib.rs` gains one `pub mod doctask_model;` line. No other module changes. The crate gains no dependency.
