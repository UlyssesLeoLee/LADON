# Test Design: `src/doctask_reconcile.rs`

Source: `src/doctask_reconcile.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Test Strategy

The module is tested through its public contract in `tests/it_doctask_reconcile.rs` against a real temporary library: `Engine::init` on a temporary directory, files written with `std::fs` and recorded with `Engine::capture`, baselines registered and tasks created through `DocTaskService` (owner), tasks moved to an approval through `DocFlow` where a guard is exercised, and the reconciler opened with `Reconciler::open` (owner or system) or with `Caller::agent` (refused). Each test asserts on returned values, on the rows read back through `DocStore` (`anchors`, `tasks`, `lineage`, `baselines`), on the chain read through `DocStore::events` (kind, subject, payload fields, in `seq` order), and on the error codes compared as `DocError` codes. Shown subjects are read from the stored anchor, which is what the owner is shown.

Raw statements on the database file reach the two states that no public operation writes: a legacy anchor with an empty fingerprint (`unchecked`) and a deleted file with a still-recorded anchor. Cold bytes are reached by removing the object file of the latest version from the content store (`Engine::cas.local_object_path`).

The pure rules (the section-overlap score) are tested in the `tests` module of the source file (TC-909).

Verification runs in the CI job `release.yml` (`cargo test --locked --all-targets` on `windows-latest`), where the integration tests run against turso as built there.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-900 | `a.md` with heading `# A` and a two-line body of at least 40 characters, baseline on version 1, task on `key:H1:A#1`, analysis started, plan submitted. The heading is renamed to `# Renamed` with the same body, captured as version 2. | Reconciliation writes `dt.anchor.reconciled` with `to_state` `relocated`, `method` `body_hash`, `to_key` `H1:Renamed#1`. `approve` returns `E_ANCHOR_BROKEN`. After `ack` with the stored version and hash, `approve` succeeds and the task is `working`; `base_version_id` stays version 1. | tests/it_doctask_reconcile.rs::tc900_renamed_heading_relocates_and_gates_approval |
| TC-901 | (a) `a.md` with `# A`, a section with body `Alpha ...` and a second `# A` before it is inserted with different text, captured. The task key `H1:A#1` now has different text. (b) `s.md` with `# Old` and body `Short`, baseline, task on `key:H1:Old#1`, heading renamed to `# New`, captured. | (a) The anchor becomes `relocated` with key `H1:A#2` and `method` `full_hash`. (b) The anchor becomes `ambiguous`, key stays `H1:Old#1`, no `relocated` state and `method` `none`. | tests/it_doctask_reconcile.rs::tc901_duplicate_heading_uses_full_hash_and_short_body_is_ambiguous |
| TC-902 | `a.md` with a long body, baseline, task, analysis, plan submitted, approved, reported as work. The body sentence changes, captured as version 2. | Reconciliation gives `needs_review`, `method` `none`, key unchanged. `close` before acknowledgement returns `E_ANCHOR_BROKEN`. After `ack` with the stored version and `current_hash`, `reviewed_version_id` equals version 2, `base_version_id` is still version 1, and `close` with a human test run and the evidence hash succeeds (`done`). | tests/it_doctask_reconcile.rs::tc902_changed_body_needs_review_ack_then_close |
| TC-903 | `a.md` with sections `# A` and `# B`, baseline, task on `key:H1:A#1`. Section A is removed and the file is captured as version 2. | Reconciliation gives `orphaned`. `report(None)` lists the anchor with `verdict_state` `orphaned`, `would_change` false and writes no event. `relocate` with `expected_version_id` version 1 returns `E_STALE_SUBJECT`; with version 2 and a wrong hash returns `E_STALE_SUBJECT`; with version 2 and the hash of `H1:B#1` in version 2 succeeds, state `relocated`, `current_key` `H1:B#1`, `reviewed_version_id` version 2, event `dt.anchor.relocated` with method `manual`. A selector that resolves to a virtual `ROW` clause of the same file is refused with `E_GUARD_FAILED` and writes nothing. | tests/it_doctask_reconcile.rs::tc903_orphaned_relocates_only_with_matching_subject |
| TC-904 | `a.md` with a long body, baseline, task, body changed and captured as version 2 (`needs_review`). Then: (a) `ack` by `Caller::agent` (b) `ack` with version 1 (c) `ack` with a wrong hash (d) the file is written again with other text without capture, and `ack` with the stored version and hash. (e) `rebase` with the baseline version and a wrong content hash. | (a) `E_HUMAN_ONLY`, nothing written. (b) and (c) `E_STALE_SUBJECT`, nothing written. (d) `E_FILE_CHANGED`, `reviewed_version_id` unchanged. (e) `E_STALE_SUBJECT`. Each refused call leaves the chain length unchanged after the first reconciliation. | tests/it_doctask_reconcile.rs::tc904_decision_guards_refuse_stale_subject_live_change_and_agent |
| TC-905 | `a.md` with a task on a valid anchor, plan submitted. `Store::mark_deleted` on `a.md`. | Reconciliation gives `file_missing`, key and version unchanged, event with `to_state` `file_missing`. `approve` returns `E_ANCHOR_BROKEN`. `ack` returns `E_GUARD_FAILED` (a deleted file accepts no acknowledgement). `tasks()` shows the anchor state `file_missing`. | tests/it_doctask_reconcile.rs::tc905_deleted_file_is_file_missing_and_refuses_approval |
| TC-906 | `a.md` with a valid anchor on version 1; the file is changed and captured as version 2; the object file of version 2 is removed from the content store. | Reconciliation gives `unknown`, `current_version_id` is version 2, `current_key` and `current_hash` are the values stored before, no fingerprint is written, event payload `method` `none`. No anchor row has an empty `current_hash` after reconciliation of a valid anchor. | tests/it_doctask_reconcile.rs::tc906_cold_latest_bytes_are_unknown_without_fabrication |
| TC-907 | (a) Reconciliation twice on a stable file. (b) Raw statement sets `full_hash`, `body_hash` and `state` to empty and `unchecked` on the anchor of a task whose base version is local. (c) Baseline registered on a changed version 2; `rebase` with version 2 and its `content_hash`. | (a) The second call returns no changes and the chain length does not change. (b) Reconciliation fills the fingerprint (`full_hash` non-empty) and sets `state` `valid`, one event. (c) `rebase` returns a new anchor on version 2, `doc_tasks.base_version_id` is version 2, the event `dt.anchor.rebased` names both anchors; a second `rebase` returns `event_seq` `None` and writes nothing. | tests/it_doctask_reconcile.rs::tc907_reconcile_is_idempotent_fills_legacy_fingerprints_and_rebases |
| TC-908 | `a.md` with a task on a valid anchor. `a.md` is deleted (`mark_deleted`), and `moved.md` with identical bytes is captured. (a) `scan_lineage` by `Caller::agent`. (b) `scan_lineage` by the owner. (c) `confirm_lineage` by the agent. (d) `confirm_lineage` with a wrong new hash. (e) `confirm_lineage` with the right ids and hash. (f) `DocTaskService::list_tasks` as the owner. | (a) `E_HUMAN_ONLY`, nothing written. (b) one proposal with evidence `same_hash` and score 1000, event `dt.lineage.proposed`. (c) `E_HUMAN_ONLY`. (d) `E_STALE_SUBJECT`, state `proposed`. (e) state `confirmed`, `decided_by` the owner, event `dt.lineage.confirmed`. (f) the task's `path` is `moved.md`, and after `reconcile_all` its anchor is `valid` on the new file. | tests/it_doctask_reconcile.rs::tc908_same_content_move_needs_human_and_list_follows |
| TC-909 | Five-section `big.md` with a task on its first section, deleted. `moved.md` with four identical sections and one changed. `few.md` with three identical sections and two other. Scan. Then `reject_lineage` on the `moved.md` candidate, and a second scan. | First scan proposes `moved.md` only, evidence `section_overlap`, score 800; `few.md` (600) is not proposed. `reject_lineage` gives `rejected`, event `dt.lineage.rejected`; the second scan proposes nothing for that pair and the lineage rows count does not grow. `reject_lineage` by the agent returns `E_HUMAN_ONLY`. The unit test `tc909_overlap_permille_rounds_down_and_is_pure` of the source file asserts `overlap_permille` for 4 of 5 (800), 3 of 5 (600), 5 of 5 (1000), 0 of 5 (0), 0 of 0 (0), and 4 of 7 (571). | tests/it_doctask_reconcile.rs::tc909_section_overlap_threshold_and_rejection_is_final; src/doctask_reconcile.rs::tests::tc909_overlap_permille_rounds_down_and_is_pure |

## Acceptance Traceability

| Acceptance item (WBS 58, WP08) | Cases |
| --- | --- |
| Renamed heading with same body gives `relocated`, records an event, and blocks approval, acceptance and application until `ack` or `relocate` | TC-900, TC-903 |
| Duplicate heading gives `relocated` (`full_hash`); a body below the relocation floor is not relocated automatically | TC-901 |
| Changed body gives `needs_review`; after confirmation acceptance is possible and `base_version_id` is unchanged | TC-902 |
| `ack`, `relocate`, `rebase` and lineage confirmation with mismatched `expected_*` give `E_STALE_SUBJECT`; ledger mismatch gives `E_FILE_CHANGED` | TC-903, TC-904, TC-908 |
| Deleted file gives `file_missing`; cold bytes give `unknown` and no fabricated value | TC-905, TC-906 |
| Repeated reconciliation produces no duplicate event | TC-907 |
| A move candidate is confirmed only by a human; after confirmation the task list follows the new path | TC-908 |
| Section-overlap candidates follow the 800 per mille rule and a rejected pair is not proposed again | TC-909 |

## Boundary & Failure Cases

- Overlap score exactly 800 is proposed; 799 is not (the rule of `overlap_permille`, TC-909).
- A relocated body with one line or fewer than 40 characters is not relocated by body hash (TC-901).
- A relocation target that is a virtual `ROW` clause is refused (TC-903 selector rule).
- A cold latest version gives `unknown` and does not change the stored fingerprint or `current_hash` (TC-906).
- A file deleted with no confirmed successor stays `file_missing` on every reconciliation (TC-905).
- A second confirmed successor for the same old file is refused (TC-908 rule; asserted in the same test after the first confirmation).
- A rebase to a key absent from the baseline outline is refused with `E_SECTION_NOT_FOUND` (TC-907 rule).
- The agent principal reaches no writing or listing entry point (TC-904, TC-908, TC-909).
- Invalid selector text reaches `parse_section_selector` and returns its error unchanged (TC-903 selector rule).

## Regression Scope

- `tests/it_doctask_flow.rs` (TC-880 to TC-899): approval and acceptance now reconcile the task's anchor before the guards; those tests create anchors on the base version with fingerprints, so the reconciliation is a no-op for them.
- `tests/it_doctask_service.rs` (TC-870 to TC-879) and `tests/it_nema_task.rs` (IT-005): the task list resolves its file through `follow_file`, which returns the same file when no lineage is confirmed.
- `tests/it_doctask_store.rs` (TC-860 to TC-869): `ensure_anchor_clause` now also stores `current_hash`; the store tests read anchors through the existing readers only.
- `tests/it_kernel_api_freeze.rs`: the engine and store signatures used by the GUI are unchanged.
