# Test Design: `src/doctask_service.rs`

Source: `src/doctask_service.rs`

Source-SHA256: `ced75da560e8bee97ec5ff4992bffa988869c84bd757d91c73302d8ccdc3434d`

## Test Strategy

The service is tested through its public contract in `tests/it_doctask_service.rs` against a real temporary library: `Engine::init` on a temporary directory, files written with `std::fs` and recorded with `Engine::capture`, the schema applied by `DocTaskService::open`. Each test asserts on returned values, on the rows read back through `DocStore` and the `ds.events()` chain, and on the error codes compared as whole `DocError` values, so the detail and the message are part of the assertion.

The transfer gate is replaced in some tests by a small fake that refuses named file ids, version ids, or the whole rule set. One test uses the real `EngineGate` with a `.LADONignore` rule written before `Engine::init`, so the real decision path is covered.

Pure helpers (`confirm_yes`, `hash_prefix_matches`, `task_content_hash`) are tested in the `tests` module of `src/doctask_service.rs` (TC-878).

The existing legacy test `it_05_task_annotation_bind_approved_version` in `tests/it_nema_task.rs` is strengthened in place (TC-870).

Verification runs in the CI job `release.yml` (`cargo test --locked --all-targets` on `windows-latest`). A local gnullvm build is attempted only when committed memory allows.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-870 | Library with `a.md` holding `# REQ-001` and `Approved`, captured as version 1. `create_task` and `add_annotation` on `H1:REQ-001#1` before any baseline. Baseline registered for version 1 with the displayed version id and hash. `a.md` edited and captured as version 2. Task `Review` created on `H1:REQ-001#1`, and annotation `add review` on the same section. | Before the baseline both calls return `E_NO_BASELINE`. The task's `base_version_id` equals version 1, not version 2, and its anchor section is `H1:REQ-001#1`. The annotation reads back with the text `add review`. `base_stale` is true. | tests/it_doctask_service.rs::tc870_create_requires_baseline_and_pins_base_version; tests/it_nema_task.rs::it_05_task_annotation_bind_approved_version (strengthened: asserts `base_version_id`, section and annotation read-back after `open`) |
| TC-871 | Library with `a.md` captured as version 1 and a baseline. (a) `register_baseline` with a `Shown` whose hash is wrong. (b) Correct `Shown`, then the same call again. (c) `a.md` rewritten on disk without capture, then `register_baseline` for a new version captured earlier. (d) `revoke_baseline` with an empty reason, with a 501-character reason, then with a 500-character reason and the correct `Shown`; revoke of an unknown baseline id. (e) Re-register version 1 after the revocation. | (a) `E_STALE_SUBJECT`, no row written. (b) First call returns the row; the second returns the same id and writes no event. (c) `E_FILE_CHANGED`. (d) Empty and 501-character reasons are `E_INVALID_ARG`; the unknown id is `E_NOT_FOUND`; with the 500-character reason the baseline has `revoked_at` set and a `dt.baseline.revoked` event. (e) A new row with a `dt.baseline.registered` event whose payload contains `replaces_revoked` equal to the revoked id. | tests/it_doctask_service.rs::tc871_baseline_register_revoke_and_guards |
| TC-872 | Library with a baseline, a task and an annotation. The agent principal from `Caller::agent`. Each of: `register_baseline`, `revoke_baseline`, `edit_task`, `set_annotation_visibility`, and the legacy `legacy_task` and `legacy_annotate`, all attempted by the agent with valid arguments. | Each call returns `E_HUMAN_ONLY`. The event count of `ds.events()` and the baseline and task rows are unchanged. | tests/it_doctask_service.rs::tc872_agent_refused_for_human_only_operations |
| TC-873 | Human baseline. Agent creates task `Agent idea` on `H1:A#1` with priority 1 and `create_task` again with the same title. Agent adds annotation `agent note` twice. Human creates `Private idea` with `private: true`. Agent `create_task` with priority 4, and with a 201-character title, both refused. Agent `list_tasks`. | First agent task: status `open`, proposal `proposed`, visibility `shared`, source `hermes`, creator `hermes`. Second call returns the same id with `existing: true`. Second annotation returns `existing: true` and the same handle. The two invalid creations are `E_INVALID_ARG` and write nothing. The agent list omits `Private idea` and reports `hidden` 1. | tests/it_doctask_service.rs::tc873_agent_proposals_and_natural_keys |
| TC-874 | Library with `ok.md` and `secret.md`, each with a baseline and one task and annotation. (a) Fake gate that refuses the file id of `secret.md`. (b) The agent lists tasks and annotations. (c) Agent `show_task` on the refused file's task, on a `local_only` task, and on `T-999`; agent `create_task` on `secret.md`; agent `add_annotation` with a file ref equal to a path. (d) `resolve_file` for the agent with a path, with the file id, and with the refused id. (e) Fake policy refusal for the agent list. (f) Real `EngineGate` with `.LADONignore` containing `secret.md` written before `Engine::init`. | (b) Only `ok.md` records are listed; `hidden` counts the refused and `local_only` records. (c) Each of the refused, `local_only` and missing references returns an identical `DocError` (equal code, message, hint and empty detail) and the file operations return the same `E_NOT_FOUND`. (d) Path returns `E_NOT_FOUND` for the agent; id of `ok.md` resolves; id of `secret.md` returns the same `E_NOT_FOUND`. (e) `E_POLICY_DENIED`. (f) The real gate refuses `secret.md` for the agent and allows `ok.md`. | tests/it_doctask_service.rs::tc874_agent_view_hides_denied_files_uniformly; tc874_resolve_file_for_agent_accepts_ids_only; tc874_policy_unusable_is_policy_denied; tc874_engine_gate_refuses_excluded_path |
| TC-875 | File `h.md` with a baseline on version 1 (`# Alpha`, `old body`), then version 2 captured with a changed body, a second task on version 2 and a second annotation on version 2. Fake gate refuses version 1 by id. | Agent list shows the version-2 task and annotation and omits the version-1 task and annotation; `hidden` counts both. The agent card for the version-1 task returns `E_NOT_FOUND`. The human sees all records. | tests/it_doctask_service.rs::tc875_historical_version_gate_hides_task_and_annotation |
| TC-876 | Human creates task `Event probe` and changes its priority with `edit_task`. Agent calls `show_task` for the handle and the human calls `show_task`. | Agent events: at least two, each with `payload` absent, `kind` and `actor` present, `actor_kind` `human` for the creation and the edit. Human events carry `payload` present. | tests/it_doctask_service.rs::tc876_agent_events_omit_payload |
| TC-877 | (a) `Engine::init` without any `open`: `attach` on the engine. (b) `open`, then a legacy row written with the raw store `add_annotation` after `open` (no meta). (c) Agent `attach`, `list_tasks`, `list_annotations`, then compare counts of `doc_task_meta`, `doc_annotation_meta` and `ds.events()` before and after. | (a) `attach` returns `E_SCHEMA_OUTDATED` and the schema version 4 is still not recorded. (b) and (c) `attach` succeeds; the counts are unchanged, the row written without meta is not listed for the agent and is not given a meta row by `attach`. | tests/it_doctask_service.rs::tc877_attach_requires_schema_and_writes_nothing |
| TC-878 | Task `Secret Title`, human edit with a wrong `expected_hash`, then with the right one (from `task_content_hash` of the card's values, computed by the test from the same fields). Annotation share with a wrong text hash, then the right one. Unit tests of the rule: `confirm_yes(true, None)`, `confirm_yes(true, Some("abc12"))`, `confirm_yes(true, Some("zz"))`, `confirm_yes(true, Some("deadbeef"))`, `confirm_yes(false, None)`, `hash_prefix_matches("abcdef12", full)` with `full` starting `abcdef1234`. | Wrong hashes: `E_STALE_SUBJECT`, no event. Right hash: the title is updated; the event `dt.task.edited` payload contains the SHA-256 and length of the old and new title and no plain title; the annotation visibility is `shared` after the right hash. `confirm_yes`: the first, second and third are `E_INVALID_ARG`, the fourth and fifth are `Ok`. The prefix check is true. | tests/it_doctask_service.rs::tc878_expected_subject_guards; src/doctask_service.rs::tests::tc878_confirm_rule_and_prefix_match |
| TC-879 | File `legacy.md` with `# A` and `body`, no baseline. Legacy `annotate` on `H1:A#1` and on `H1:Missing`; legacy `task` on `H1:A#1` and on `H1:Missing`. Human `list_tasks`, `list_annotations`, `show_task`. | Known section: anchor state `valid`. Unknown section: anchor `unchecked`, accepted, and `unbound` true. Legacy task: source `legacy`, proposal `accepted`, visibility `shared`, creator `human`. The human lists all four records with no hidden count. | tests/it_doctask_service.rs::tc879_legacy_path_keeps_unchecked_and_legacy_semantics |

## Acceptance Traceability

| Acceptance item (WBS WP06) | TC | Input | Assertion |
| --- | --- | --- | --- |
| Creation without a baseline returns `E_NO_BASELINE` | TC-870 | Task and annotation before registration | Both errors are `E_NO_BASELINE` |
| The task is pinned to the baseline version and the section is read from its bytes | TC-870 | Version 2 captured after the baseline on version 1 | `base_version_id` equals version 1; section key resolved from version 1 bytes |
| Agent calls to baseline, edit and public annotation return `E_HUMAN_ONLY` | TC-872 | Agent with valid arguments | `E_HUMAN_ONLY` and no row or event change |
| Agent view omits `local_only` and refused-file records and counts them in `hidden` | TC-874 | Fake gate; `local_only` task | Only `ok.md` records; `hidden` equals the omitted count |
| Refused, `local_only` and missing references give identical `E_NOT_FOUND`, no candidates | TC-874 | Refused task, `local_only` task, `T-999` | Whole-value equality of `DocError`; empty detail |
| Historical versions are judged version by version | TC-875 | Version-1 task and annotation refused; version-2 records allowed | Version-1 records omitted and counted; version-2 records listed |
| Agent events carry no payload | TC-876 | Event list for the agent view | `payload` absent |
| `attach` writes no meta and no reconciliation | TC-877 | Counts before and after an agent read | Counts unchanged; legacy row without meta stays without meta |
| Expected subject changes give `E_STALE_SUBJECT` | TC-871, TC-878 | Wrong `Shown` on registration; wrong hashes on edit and share | `E_STALE_SUBJECT`, no write |
| Baseline registration on a file not captured gives `E_FILE_CHANGED` | TC-871 | File rewritten on disk without capture | `E_FILE_CHANGED`, no row |
| `ack` and `rebase` give `E_FILE_CHANGED` | — | Deferred to WP08, which reuses the guard of TC-871 | Stated in the WP08 test design |
| `--yes` without `--expect-hash` is refused | TC-878 | `confirm_yes(true, None)` | `E_INVALID_ARG` |
| Revocation followed by re-registration of the same version works | TC-871 | Revoke, then register the version again | New row; payload `replaces_revoked` |
| Legacy create keeps `unchecked` and `legacy` semantics | TC-879 | Unknown section; legacy task | Anchor `unchecked`; source `legacy` |
| IT-005 reads back `base_version_id`, section and annotation | TC-870 | Strengthened `it_05` after `open` | Read-back values equal the inputs |
| Task creation and editing including visibility | TC-873, TC-878 | Human private task; agent proposal; human edit | Visibility and proposal values as listed |
| Read views for tasks, annotations and baselines | TC-879, TC-873 | Human lists and card | Records and counts as listed |

## Boundary & Failure Cases

- Priority 0 and 4 on task creation are `E_INVALID_ARG`; a title of 201 characters is `E_INVALID_ARG` (TC-873).
- A reason of 501 characters on revocation is `E_INVALID_ARG`; a baseline id that does not exist is `E_NOT_FOUND` (TC-871).
- A section selector that matches nothing is `E_SECTION_NOT_FOUND` on the strict path of creation (TC-870 with `H1:Missing` on a new annotation), while the legacy path accepts it as `unchecked` (TC-879).

## Regression Scope

- `tests/it_doctask_store.rs` (IT-049, IT-050) and `tests/it_doctask_migration.rs` (IT-047, IT-048) run unchanged against the `DocStore` methods that gain new public items.
- `tests/it_nema_task.rs` (IT-005, IT-006) and the legacy tests of `tests/it_store.rs` keep their assertions.
- The task list follows a confirmed lineage move to the successor's path (TC-908 in `tests/it_doctask_reconcile.rs`); without a confirmed edge the list is unchanged.
- `tests/it_kernel_api_freeze.rs` compiles against the engine and store signatures used by the GUI; this package adds no engine or store signature.
