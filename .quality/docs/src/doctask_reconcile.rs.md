# Technical Design: `src/doctask_reconcile.rs`

Source: `src/doctask_reconcile.rs`

Source-SHA256: `a6fda4bbbe0a0349c55097b4987f15efb7ddf60931db8d6cc11d5b61a823ff6e`

## Responsibility

`doctask_reconcile` keeps the anchors of document tasks true to the latest version of their files. It reconciles every anchor against the outline of the latest version (design 57 section 5.4), records each state, key or version change as one governance event, and carries the human decisions that move an anchor out of a review state: acknowledgement (`ack`), relocation to a clause (`relocate`) and re-pinning to the active baseline (`rebase`). It also owns the file-move lineage: it proposes candidates for a file that was deleted and a file that appeared with the same content or with overlapping sections, and it confirms or rejects them, human only.

Every write is one `DocStore::commit_step` call: the row update of the anchor, task or lineage row and its event share one `BEGIN IMMEDIATE` transaction. Event identifiers are chosen before the transaction, so a retried transaction writes the same rows. Reconciliation is idempotent: an anchor whose state, key and version already match the latest version produces no write and no event, and an anchor on the same version whose state was computed for that version reads no bytes.

The module holds no SQL. Every table access goes through `DocStore`, including the readers `anchors` and `lineage`. It reads bytes only through `Engine::cas` and the ledger through `Engine::store`; it writes no file in the working tree.

Principal rules follow `doctask_model::authorize`. Reconciliation writes are System writes (actor `ladon`, actor kind `system`) and are allowed for the owner and the system; the human decisions (`ack`, `relocate`, `rebase`, lineage confirmation and rejection) are allowed for the owner only; the agent gets `E_HUMAN_ONLY` from every entry point of this module. The agent view of reconciliation (in-memory, not stored) belongs to the MCP layer (WP13) and uses `classify` without writing.

Not covered here: the decision guards themselves (`doctask_flow`), the byte-level splice of an edit plan (WP14), the command-line and console presentation (WP11, WP17).

## Dependencies

- `crate::engine::Engine`: `store` (`find_path`, `file_by_id`, `list_files`, `latest_version`, `version`), `cas` (`has`, `load_bytes`, `local_object_path`), `root` (the live file of a file record).
- `crate::crypto::digest`: SHA-256 hex of the live file bytes, compared with `resources.current_hash`.
- `crate::doctask_outline`: `outline`, `classify_anchor`, `AnchorRef`, `LatestVersion`, `Method`, `Verdict`, `Outline`, `Clause`, `ClauseKind`, `resolve`, `RELOCATION_MIN_LINES`, `RELOCATION_MIN_CHARS`.
- `crate::doctask_model`: `Operation`, `authorize`, `Principal`, `ActorKind`, `AnchorState`, `LineageState`, `SourceKind`, `Handle`, `parse_task_handle`, `parse_section_selector`, `SectionSelector`, `DocError`, `ErrorCode`, `MAX_TARGET_FILE_BYTES`.
- `crate::doctask_store`: `DocStore` with `anchors`, `lineage`, `tasks`, `baselines`, `ensure_anchor_clause`, `commit_step`, `events`; the row types `AnchorRecord`, `LineageRow`, `TaskRow`, `BaselineRow`, `AnchorFingerprint`, `StepEvent`, `NewEvent`, `RowWrite`, `EventRecord`.
- `crate::doctask_service::Caller`: the principal, name and session of the human decision.
- `crate::doctask_flow` calls `reconcile_decision` before the guards of approval and acceptance; `crate::doctask_service` calls `follow_file` when it lists tasks.

## Public Contract

### Types

- `AnchorChange { anchor_id, from_state, to_state, from_key, to_key, method, event_seq }`: one state, key or version change written by reconciliation. `method` is one of `same_hash`, `full_hash`, `body_hash`, `none` (design 5.4).
- `ReconcileReport { examined: usize, changes: Vec<AnchorChange> }`.
- `AnchorStep { anchor_id, state, event_seq: Option<i64> }`: the result of `ack`, `relocate` and `rebase`. `event_seq` is `None` when the call changed nothing (a rebase onto the anchor already bound).
- `MoveLine { anchor_id, file_id, section_key, state, key, would_change, verdict_state, verdict_key, method }`: one row of the migration report. The report is computed without writes; `would_change` is true when a reconciliation would write this anchor.
- `Reconciler<'a>`: holds the engine, a `DocStore` and the `Caller`.

### Constructors

- `Reconciler::open(engine: &Engine, caller: Caller) -> Result<Reconciler>`: applies migration 4 when needed and imports legacy rows, as `DocFlow::open`. May write.
- `Reconciler::attach(engine: &Engine, caller: Caller) -> Result<Reconciler>`: fails with `E_SCHEMA_OUTDATED` when migration 4 is not recorded. Writes nothing.

### Reconciliation (Operation::Reconcile: owner and system; agent `E_HUMAN_ONLY`)

- `reconcile_all() -> Result<ReconcileReport>`: every anchor, in `anchors()` order.
- `reconcile_file(file_ref: &str) -> Result<ReconcileReport>`: the anchors whose file, or whose followed file, is the file named by a path or an id.
- `report(file_ref: Option<&str>) -> Result<Vec<MoveLine>>`: the migration report of design 5.6, computed without writes.
- `reconcile_decision(engine: &Engine, ds: &DocStore, anchor_id: &str) -> Result<()>` (crate visibility): reconciles one anchor. Used by `DocFlow::approve` and `DocFlow::close`, which then reload their snapshot.
- `follow_file(engine: &Engine, ds: &DocStore, file_id: &str) -> Result<Option<FileRecord>>` (crate visibility): the file reached from `file_id` over confirmed lineage edges, at most `MAX_LINEAGE_HOPS` (8) edges, stopping at a repeated file. Without a confirmed edge it returns the file itself. `None` when the id is unknown.

### Human decisions (Operation::AnchorAck, AnchorRelocate, AnchorRebase: owner only)

Every decision takes the task handle and the subject shown to the owner. Each call first authorizes, then reads the task by handle (`E_NOT_FOUND` when absent), then reconciles the anchor of the task with the system actor, then compares the shown subject with the stored one, then checks the live file, then writes.

- `ack(handle, expected_version_id, expected_hash) -> Result<AnchorStep>`: the anchor state must be `needs_review`, `relocated` or `unknown`, otherwise `E_GUARD_FAILED`. `expected_version_id` must equal the anchor's `current_version_id` and `expected_hash` its `current_hash`, otherwise `E_STALE_SUBJECT`. The live file (not deleted, digest equal to `resources.current_hash`) must match, otherwise `E_FILE_CHANGED`. Sets `reviewed_version_id` to `current_version_id`. Leaves `base_version_id` unchanged. Event `dt.anchor.acknowledged` with the task id.
- `relocate(handle, selector: &str, expected_version_id, expected_hash) -> Result<AnchorStep>`: the anchor state must be `orphaned` or `ambiguous`, otherwise `E_GUARD_FAILED`. The selector is parsed with `parse_section_selector` and resolved in the latest version; a virtual `ROW` clause is refused with `E_GUARD_FAILED`; an unresolvable selector returns `E_SECTION_NOT_FOUND` or `E_AMBIGUOUS`. `expected_version_id` must equal the latest version id and `expected_hash` the selected clause's `full_hash`, otherwise `E_STALE_SUBJECT`. The live file must match, otherwise `E_FILE_CHANGED`. Sets `current_key`, `current_hash`, `current_version_id` and `reviewed_version_id` to the selected clause and the latest version, takes the clause's fingerprint (`title`, `level`, `line_start`, `line_end`, `clause_id`, `full_hash`, `body_hash`) as the anchor's own, and sets `state = relocated`. Event `dt.anchor.relocated` with method `manual`.
- `rebase(handle, expected_version_id, expected_hash) -> Result<AnchorStep>`: the active baseline is the latest of the file's baselines with `revoked_at` absent; `NO_BASELINE` when there is none. `expected_version_id` must equal the baseline's `version_id` and `expected_hash` its `content_hash`, otherwise `E_STALE_SUBJECT`. The live file must match, otherwise `E_FILE_CHANGED`. The baseline version bytes must be local (`E_ANCHOR_BROKEN` otherwise). The anchor's `current_key` must exist in the baseline outline, otherwise `E_SECTION_NOT_FOUND`. The anchor is created or found by `DocStore::ensure_anchor_clause` on the baseline version with the clause fingerprint. When the result differs from the task's anchor, one commit writes `doc_task_meta.anchor_id`, `doc_tasks.base_version_id` and event `dt.anchor.rebased`; when it is the same anchor the call returns `event_seq: None` and writes nothing.

### Lineage (Operation::Reconcile for scan and list; Operation::LineageConfirm and LineageReject for decisions)

- `scan_lineage() -> Result<Vec<LineageRow>>`: proposes candidates for each deleted file that has at least one anchor. The candidate set is the non-deleted files other than the deleted file. A candidate with `current_hash` equal to the content hash of the deleted file's last version gets evidence `same_hash` and score 1000. Otherwise, when both last versions are local and the deleted file has at least one section, a candidate gets evidence `section_overlap` and score `overlap_permille(old, new)` (the number of old sections whose `full_hash` occurs among the new sections, times 1000, divided by the number of old sections, rounded down) when that score is at least 800. A pair that already has a lineage row in any state is not proposed again. Each new row is `proposed` with event `dt.lineage.proposed`. Returns the rows proposed by this call.
- `lineage() -> Result<Vec<LineageRow>>`: every lineage row, in creation order.
- `confirm_lineage(id, expected_old_file_id, expected_new_file_id, expected_new_hash) -> Result<LineageRow>`: the row must be `proposed` (`E_GUARD_FAILED` otherwise). The two file ids must equal the row's, and `expected_new_hash` must equal the new file's `current_hash`, otherwise `E_STALE_SUBJECT`. The old file must be deleted and the new file must not be deleted (`E_GUARD_FAILED`). The old file must not already have a confirmed successor, and the confirmed edges after this one must not form a cycle or exceed `MAX_LINEAGE_HOPS` (`E_GUARD_FAILED`). Sets `state = confirmed`, `decided_by` to the caller's name and `decided_at`. Event `dt.lineage.confirmed` with the caller as actor.
- `reject_lineage(id) -> Result<LineageRow>`: the row must be `proposed`. Sets `state = rejected`. Event `dt.lineage.rejected`.

### Pure functions (module level, unit tested)

- `overlap_permille(old: &[String], new: &[String]) -> i64`: the rule of the section-overlap score; `0` when `old` is empty.
- `assess(engine, lineage, anchor) -> Assessment` (private): the decision of design 5.4 for one anchor. It reads the ledger and the bytes of the latest version of the target and writes nothing. The reconciliation and the migration report both use it, so the report shows exactly what a reconciliation would write.

### Errors

| Condition | Code |
| --- | --- |
| Agent principal calls any entry point | `E_HUMAN_ONLY` |
| Task handle or file reference unknown | `E_NOT_FOUND` |
| Anchor state does not accept the decision | `E_GUARD_FAILED` |
| Shown version, hash, lineage ids or baseline differ from the stored subject | `E_STALE_SUBJECT` |
| Live file missing or digest differs from the ledger | `E_FILE_CHANGED` (missing or deleted file: `E_FILE_MISSING`) |
| Baseline or latest version bytes not local, or not readable | `E_ANCHOR_BROKEN` |
| Key absent from the baseline outline; selector unresolved | `E_SECTION_NOT_FOUND`; ambiguous selector `E_AMBIGUOUS` |
| No active baseline for rebase | `E_NO_BASELINE` |
| Schema not current on `attach` | `E_SCHEMA_OUTDATED` |

## Behaviour

### Reconciliation of one anchor

Input: the anchor row, the target file (`follow_file` of the anchor's file id), the target's latest version and its bytes. Steps in order, as design 5.4:

1. Target file deleted: state `file_missing`, key and version unchanged, no bytes read.
2. Latest version absent from the ledger: state `unknown`, key and version unchanged.
3. Anchor state `valid`, `needs_review`, `relocated`, `ambiguous` or `orphaned` and the stored version equals the latest version: no change, no bytes read.
4. Latest version bytes not local or not readable: state `unknown`, current version advanced to the latest version, key and `current_hash` unchanged. No hash is computed.
5. Fingerprint empty (`unchecked`): read the anchor's base version bytes and fill `title`, `level`, `line_start`, `line_end`, `clause_id`, `full_hash`, `body_hash` from the clause of `section_key`. When that fails, the state stays `unchecked` and the anchor is not written.
6. Classify with `doctask_outline::classify_anchor` against the latest outline, using the anchor's `current_key`, `full_hash` and `body_hash`.
7. The stored `current_hash` is the `full_hash` of the verdict's key in the latest outline, or empty when that key is absent.

The anchor is written when its state, key, current version or current hash changes, or when the fingerprint was filled. One write per changed anchor: `doc_anchors` row update (`state`, `current_version_id`, `current_key`, `current_hash`, `updated_at`, and the fingerprint columns when filled) with event `dt.anchor.reconciled` (`task_id` empty, `subject_kind` `anchor`, `subject_id` the anchor id). The payload is `{"anchor_id", "from_state", "to_state", "from_key", "to_key", "method", "from_version_id", "to_version_id", "target_file_id"}`. Actor `ladon`, kind `system`.

The anchor's `reviewed_version_id` is never changed by reconciliation. An anchor is settled for a guard only when its `reviewed_version_id` equals its `current_version_id` (design 4.4), so a changed version requires a new decision.

`reconcile_all` and `reconcile_file` apply this per anchor, each anchor in its own commit. `report` applies the same steps and returns the verdict without writing.

### Anchor fingerprint at creation

`DocStore::ensure_anchor_clause` sets `current_hash` to the clause `full_hash` when it creates the row, so a new anchor on its base version is already reconciled and step 3 applies to it.

### Human decisions

`ack`, `relocate` and `rebase` reconcile the task's anchor before their checks, so the stored subject they compare against is the one a reconciliation of the current file produces. Their events carry the task id and the caller as actor (kind `human`), the caller's session and a payload naming the anchor, the version and the hash.

### Lineage

Confirmed edges are read with `DocStore::lineage` and followed from the old file id, at most `MAX_LINEAGE_HOPS` edges, stopping when a file repeats. The task list resolves its file with `follow_file`, so a confirmed move shows the new path. Reconciliation of an anchor whose file is deleted but has a confirmed successor uses the successor's latest version.

## Errors

See the table in the public contract. Every error is a `DocError` converted to `anyhow::Error`, so a caller can downcast it to read the code.

## Concurrency

Each write is one `BEGIN IMMEDIATE` transaction. The assessment of an anchor is read before its transaction starts, so two reconciliations of the same anchor that race each other can both emit the same change. Both set the same state, key and version on the anchor row, and each emitted event is appended to the chain in its own transaction. No other lock is taken; the owner's decisions reconcile first and then compare the subject they were shown with the stored values in the same call.

## Schema assumptions

Migration 4 as written in `sql/004_document_tasks.sql`: `doc_anchors` (state, current_version_id, current_key, current_hash, reviewed_version_id, fingerprint columns), `doc_task_meta.anchor_id`, `doc_tasks.base_version_id`, `doc_file_lineage` (`UNIQUE (old_file_id, new_file_id)`, state, decided columns), `doc_baselines`.

## Change Notes

The module adds no dependency and changes no table. Its writes are the ones of design 57 sections 5.4 to 5.7. The `classify` steps of section 5.4 are the function `doctask_outline::classify_anchor`; the fingerprint fill and the version rules are in this module. Lineage evidence and the overlap threshold are the rules of section 5.7.
