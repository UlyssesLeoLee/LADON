# Technical Design: `src/doctask_service.rs`

Source: `src/doctask_service.rs`

Source-SHA256: `d8d232cf164973cdf259bee9ee36c7b74b81ee1f106023bfefbe73da22ec23fe`

## Responsibility

`doctask_service` is the service core of the document-task module. It is the only read and write entry point of the module's data. The one-shot command, the console and the MCP server call it with a `Caller` that fixes the principal. Each operation checks the principal against the authorization table of `doctask_model::authorize`, resolves the file or task reference, applies the external-transfer gate for the agent principal, validates the text fields, checks the expected subject shown to the human, and then writes through `DocStore`. The service holds no SQL: every table access goes through `DocStore`.

Covered here: opening and attaching, the transfer gate and its trait, the agent view projection, file resolution, baseline registration and revocation, annotations, task creation and editing, annotation visibility, read views, the legacy annotate and task path, and the `--yes` confirmation rule.

Not covered here: acknowledgement, relocation and rebase of anchors (WP08), transitions, proposals, dependencies, links, evidence, quota and the inbox (WP07), edit plans (WP14), the disclosure record (WP13), and the command-line confirmation prompts (WP11).

## Dependencies

- `crate::engine::Engine`: the root path, the content-addressed store (`cas.has`, `cas.load_bytes`), the store handle, and `remote_read_allowed`, `remote_version_allowed`, `refresh_policy_if_stale` (the transfer gate of design 13.1). `Engine::root` is read to locate the live file for the digest comparison.
- `crate::store::Store`: `ensure_schema`, `file_by_id`, `find_path`, `latest_version`, `version`, `versions`.
- `crate::doctask_store::{DocStore, NewEvent, NewTask, NewAnnotation, RowWrite, AnchorFingerprint, BaselineRow, TaskRow, AnnotationRow, CreatedTask, CreatedAnnotation}`: every write and every read of the document-task tables.
- `crate::doctask_outline`: `outline`, `resolve`, `Clause`, `ClauseKind` for section resolution on version bytes.
- `crate::doctask_model`: principals, operations and `authorize`, `ActorKind`, `SourceKind`, `Visibility`, `ProposalState`, `AnchorState`, `TaskState`, `Handle`, `parse_task_handle`, `parse_annotation_handle`, `parse_section_selector`, validators, `sha256_hex`, `DocError` and `ErrorCode`.
- `crate::crypto::digest`: SHA-256 hex of file bytes, identical to the content hash stored by the content-addressed store.
- `crate::model::{FileRecord, VersionRecord}`.
- `async_trait` for the gate trait, `chrono` for the timestamp of baseline rows, `serde_json` for payloads, `anyhow` for the error type of public methods (each `DocError` is wrapped, and callers downcast it).

## Public Contract

### Caller and subject

- `Caller { principal: Principal, name: String, session: String, source: SourceKind }`.
  - `Caller::human(name, source)`: principal `Human`, session empty.
  - `Caller::agent(launch_id)`: principal `Agent`, name `hermes`, session `hermes:<launch_id>`, source `Hermes`.
- `Shown { version_id: String, hash: String }`: the subject displayed to the human before a confirmation. Every human-only operation that changes a subject takes one, or the subject's own hash for edits and visibility changes.
- `Listing<T> { items: Vec<T>, hidden: u64 }`: a list with the number of records omitted for the agent principal.

### Gate

- `#[async_trait] pub trait TransferGate: Send + Sync` with three methods:
  - `policy_usable(&self) -> bool`;
  - `file_allowed(&self, file: &FileRecord) -> bool`;
  - `version_allowed(&self, version: &VersionRecord) -> bool`.
- `EngineGate<'a>` implements the trait over `&Engine`. `policy_usable` is `refresh_policy_if_stale().is_ok()`. The two decision methods return `remote_read_allowed` and `remote_version_allowed`; an error from the engine is treated as a refusal (`false`).
- The gate never returns an error: a failure of the rule set is a refusal, which is the fail-closed rule of design 13.1.

### Service

- `DocTaskService<'a> { engine: &'a Engine, ds: DocStore, gate: Box<dyn TransferGate + 'a>, caller: Caller }`.
- `open(engine, caller)`: `Store::ensure_schema(false)`, then `DocStore::backfill()`. Used by the one-shot command and the console. Runs migration and import; may write.
- `attach(engine, caller)`: `DocStore::schema_current()`; a library below Schema 4 returns `E_SCHEMA_OUTDATED`. Writes nothing: no migration, no backfill, no `meta` row, no reconciliation.
- `with_gate(self, gate)`: replaces the gate. Used by tests and by callers that hold a different gate.
- `resolve_file(file_ref) -> FileRecord`.
  - Human: a relative path (backslashes are read as slashes) or a resource id.
  - Agent: a resource id only (the `file_ref` of design 3.4), and only when the gate allows the file. Any other input, a denied file and a missing file return the same `E_NOT_FOUND`, with the same message and hint and an empty detail.
  - For the agent principal, `policy_usable() == false` returns `E_POLICY_DENIED` before any lookup.
- `list_tasks() -> Listing<TaskView>`, `show_task(handle) -> TaskCard`.
- `register_baseline(file_ref, version_no: Option<i64>, note, shown: &Shown) -> BaselineRow` (human only).
- `revoke_baseline(baseline_id, reason, shown: &Shown) -> BaselineRow` (human only).
- `list_baselines(file_ref: Option<&str>) -> Listing<BaselineRow>` (read).
- `add_annotation(file_ref, section, text, rationale, quote, private) -> AnnotationRef` (human and agent; the agent's annotation is always `shared`).
- `list_annotations(file_ref: Option<&str>) -> Listing<AnnotationView>` (read).
- `set_annotation_visibility(handle, visibility: Visibility, expected_text_hash) -> AnnotationView` (human only).
- `create_task(file_ref, section, spec: &TaskSpec) -> TaskRef` (human and agent).
- `edit_task(handle, edit: &TaskEdit, expected_hash) -> TaskView` (human only).
- `legacy_annotate(file_ref, section, text) -> AnnotationRef` and `legacy_task(file_ref, section, title, description) -> TaskRef` (human only; the path of the `annotate` and `task` commands, which keep the 0.0.3 semantics).

Value types: `TaskSpec { title, description, owner, priority: i64, private: bool }` (`TaskSpec::new(title)` sets priority 2); `TaskEdit { title, description, owner, priority, visibility }`, each an `Option` (an absent field is unchanged); `TaskView` (identifier, handle, task number, file reference, path, anchor identifier, section, clause number, anchor state, `unbound` (no baseline binding), base version id and number, `base_stale`, `file_state` (`live` or `missing`), status, title, description, owner, priority, proposal state, visibility, source kind, creator and creator kind, times, and `blocked_by` as handles of visible unfinished prerequisites); `TaskCard { task, events }`; `EventView { seq, kind, from_status, to_status, actor, actor_kind, timestamp, payload: Option<String> }` with `payload` absent for the agent principal; `AnnotationView`, `AnnotationRef { id, handle, existing }`, `TaskRef { id, handle, existing }`.

### Free functions

- `confirm_yes(yes: bool, expect_hash: Option<&str>) -> Result<(), DocError>`: `yes` without an `expect_hash` is `E_INVALID_ARG`; an `expect_hash` shorter than 8 characters or not hexadecimal is `E_INVALID_ARG`; otherwise `Ok`.
- `hash_prefix_matches(prefix, full) -> bool`: the prefix is at least 8 hexadecimal characters and a lowercase prefix of `full`.
- `task_content_hash(title, description, anchor_id, priority, depends_on) -> String`: SHA-256 of the JSON array `[title, description, anchor_id, priority, sorted depends_on]`. It is the subject hash of task edits.

## Behaviour

### Guard order for writes

1. Principal authorization (`authorize`); a principal outside the table gets `E_HUMAN_ONLY` before any lookup.
2. Policy usability for the agent principal (`E_POLICY_DENIED`).
3. Text and priority validation with the limits of `doctask_model` (`E_INVALID_ARG`).
4. File or handle resolution (`E_NOT_FOUND`, `E_AMBIGUOUS`, `E_FILE_MISSING`).
5. Gate for the agent principal: file and version (`E_NOT_FOUND`).
6. Baseline precondition (`E_NO_BASELINE`) and section resolution on the baseline version (`E_SECTION_NOT_FOUND`, `E_AMBIGUOUS`, `E_ANCHOR_BROKEN` when the bytes are cold).
7. Expected subject: `E_STALE_SUBJECT` when the displayed version id, content hash or task content hash differs.
8. Live file for baseline registration: a deleted file, or one that cannot be read at `root/relative_path`, is `E_FILE_MISSING`; a digest that differs from `resources.current_hash` is `E_FILE_CHANGED`.
9. Natural-key deduplication: an existing record is returned with `existing: true`.
10. One `DocStore` write with its governance event (`write_with_event` or the typed `create_task` and `add_annotation`).

The expected subject is compared before the write. The write itself runs in its own Immediate transaction, so the comparison and the write are not one transaction; a change between them is caught by the next expected-subject check of the same kind.

### Agent view

Every agent read is projected through one `Projection` per call. The projection caches the gate result per file id and per version id, and counts omitted records in `hidden`.

- A task is omitted (and counted) when its `visibility` is `local_only`, when its file is refused, or when the version of its baseline is refused.
- An annotation is omitted by the same three rules, with its base version.
- A task's blocked-by reference to an omitted prerequisite is counted, not listed.
- Events are projected to `EventView` with `payload: None`.
- `show_task` and `create_task` on a refused, `local_only` or missing reference return the same `E_NOT_FOUND`; its detail is empty and it lists no candidates.
- Tasks and annotations without a `doc_task_meta` or `doc_annotation_meta` row are invisible to the agent (they are imported by `open`, never by `attach`).

### Baseline

- Registration: the version is the one numbered `version_no`, or the latest version. The displayed `Shown` must equal the version id and the content hash. The live file digest must equal `resources.current_hash`. An active baseline on the same version is returned unchanged. Otherwise a `dt.baseline.registered` event is written with the baseline row. When the version has a revoked baseline, the payload carries `replaces_revoked` with the most recent revoked baseline id.
- Revocation: the reason is required (1 to 500 characters). The `Shown` must equal the baseline's version id and content hash. A revoked baseline is `E_INVALID_ARG`. The event `dt.baseline.revoked` carries the version number, the content hash and the reason.

### Anchors and text

- Section resolution on the strict path: `parse_section_selector(section)`, then `doctask_outline::resolve` on `outline(bytes, extension)` of the version bytes. The bytes come from the content-addressed store and are bounded by `MAX_TARGET_FILE_BYTES`. Cold bytes are `E_ANCHOR_BROKEN`.
- Anchors created by the service carry the clause fingerprint (`AnchorFingerprint`: baseline id, clause key, clause number, title, level, lines, full and body hash) and state `valid`.
- The legacy path keeps the 0.0.3 behaviour: no baseline is required. The base version is the active baseline's version when there is one, and otherwise the latest version (the anchor then has no baseline binding). The section string is first matched as an exact clause key, then as a selector. A section that resolves gets a fingerprinted `valid` anchor. A section that does not resolve, or whose bytes are cold, gets an `unchecked` anchor keyed by the raw section text and is accepted, not refused. Legacy tasks have source `legacy`, proposal `accepted` and visibility `shared`. Legacy annotations are `shared`.

### Tasks

- Human: proposal `accepted`, source kind from the caller (`cli` or `console`), visibility `local_only` when `private`, otherwise `shared`.
- Agent: proposal `proposed`, source `hermes`, visibility `shared`. The agent principal never sets `private`.
- Creation requires the title (1 to 200 characters), an optional description (at most 4000), an owner (at most 200) and a priority of 1 to 3. The title and anchor identify the natural key: an open task on the same anchor with the same title is returned with `existing: true`.
- Edit (human only): the `expected_hash` must equal `task_content_hash` of the current task. The event `dt.task.edited` records, for each changed text field, the SHA-256 and length of the old and new values, and for priority and visibility the plain values. No text of a `local_only` task is stored in plain form by the edit.

### Annotations

- Creation: the agent's annotation is `shared`. The human's is `local_only` when `private`. The natural key is the anchor and the text: an identical text on the same anchor is returned with `existing: true`.
- Visibility change (human only): the `expected_hash` must equal SHA-256 of the annotation text. The event `dt.annotation.visibility_changed` records the old and new visibility and the text hash and length.

### Views

- `list_tasks`, `show_task`, `list_annotations`, `list_baselines`: read operations. The human sees every record; the agent sees the projection.
- `show_task` returns the events of the task in `seq` order.
- `resolve_file`, `list_*` never write.
- The file of a task is resolved with `doctask_reconcile::follow_file`: a file deleted by a confirmed lineage move is shown at its successor's path (design 5.7). Without a confirmed edge the file is the one the task names.

## Errors

Every failure is a `DocError` with the code of design 14.1: `E_HUMAN_ONLY`, `E_NOT_FOUND`, `E_AMBIGUOUS`, `E_NO_BASELINE`, `E_FILE_MISSING`, `E_FILE_CHANGED`, `E_STALE_SUBJECT`, `E_SECTION_NOT_FOUND`, `E_ANCHOR_BROKEN`, `E_INVALID_ARG`, `E_POLICY_DENIED`, `E_SCHEMA_OUTDATED`, and the storage codes passed through from `DocStore`.

## Schema assumptions

- Migration 004 is applied for `open` and checked for `attach`. The tables of `sql/004_document_tasks.sql` have the columns named in `doctask_store`.
- Legacy rows (`doc_tasks`, `annotations`) without meta are imported by `DocStore::backfill`, which `open` runs on every call.

## Deferred operations

- Anchor acknowledgement, relocation and rebase: WP08. The `E_FILE_CHANGED` guard of registration is the same check; WP08 reuses it.
- Transitions, proposals, dependencies, links, evidence, quota: WP07.
- The command-line `--yes` and prompt flow: WP11; `confirm_yes` is the rule it applies.
- The disclosure record of the agent read path: WP13.

## Change Notes

The module is declared in `lib.rs` as a public module. It depends on the gate trait, `doctask_store`, `doctask_outline`, `doctask_model`, `engine` and `store`. It does not depend on the command entry, the console or the MCP layer, which call it.
