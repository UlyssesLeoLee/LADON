# Test Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `a36b22395a6703671e5d6be00f21435b92c8ce85f238e8d68dc614a4a3d92bee`

## Test Strategy

Integration tests drive the public `Store` API against a real Turso database file inside a `tempfile::tempdir`, with no mocks. Each test creates its own directory, so tests are isolated and can run in parallel. `Store::new(dir/.ladon/meta.turso)` followed by `init()` is the shared fixture. The existing engine-level tests in `tests/it_history.rs` and `tests/it_nema_task.rs` exercise the same code through `Engine` and remain part of the suite. The goal is every public method plus both branches of each conditional (existing or new resource, deleted or not deleted, valid or invalid state).

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-101 | append_version for a new path with hash h1, size 3, one changed section, actor human | Returns three non-empty ids; find_path shows version_no 1 and hash h1; versions has one row with the given actor, reason, byte_size 3 and sections; events has one row of kind created | tests/it_store.rs::tc101_append_version_round_trip |
| TC-102 | Append hash h2 to the same path, then append h2 again | Second append gives version_no 2 and event kind modified; the repeated hash returns an error containing no content change; verify_chain is true | tests/it_store.rs::tc102_second_version_and_no_change_rejected |
| TC-103 | Append with sensitive true, then a non-sensitive version of another path | find_path reports sensitive true for the first; is_hash_sensitive is true for its hash and false for an unknown hash | tests/it_store.rs::tc103_sensitive_flag_round_trip |
| TC-104 | mark_deleted on a tracked record, then again with the refreshed record | Record is deleted; a deleted event is appended and the chain verifies; the second call adds no event | tests/it_store.rs::tc104_mark_deleted_appends_event_and_is_idempotent |
| TC-105 | Append a new hash for a path whose record is deleted | New event kind is restored; find_path shows deleted false and the next version number | tests/it_store.rs::tc105_append_after_delete_is_restored_event |
| TC-106 | archive_record twice for the same hash and provider with different locators | archives(hash) returns exactly one record carrying the second locator and encrypted true | tests/it_store.rs::tc106_archive_record_upsert |
| TC-107 | set_tags with b, a, a blank tag and an 81-character tag, then again with source ai | tags returns a and b sorted; blank and oversize tags absent; no duplicates after the second call | tests/it_store.rs::tc107_set_tags_filters_and_sorts |
| TC-108 | add_annotation, create_task, set_task_status done, then list_tasks and governance_events | Task listed with status done and its title; governance events appear in order annotation.created, task.created, task.state_changed | tests/it_store.rs::tc108_annotation_task_and_governance_flow |
| TC-109 | set_task_status with state bogus; with an unknown task id | Errors containing invalid task state and task missing respectively | tests/it_store.rs::tc109_invalid_task_state_and_unknown_task |
| TC-110 | find_path, latest_version, version, versions, events, archives and tags on unknown keys | None for single lookups, empty lists for collections, no errors | tests/it_store.rs::tc110_missing_records_return_none |
| TC-111 | Reason text with quotes, a newline, Japanese text and a quote followed by DROP TABLE resources | Stored and read back byte-identical; the resources table still works afterwards | tests/it_store.rs::tc111_special_characters_bound_verbatim |
| TC-112 | Call init twice on the same file | Both succeed and previously stored data remains readable | tests/it_store.rs::tc112_init_is_idempotent |
| TC-113 | Capture, delete and re-capture a file through Engine | Events appended in order with a valid chain and the deleted flag set after scan | tests/it_history.rs::it_04_detect_delete_and_keep_history |

## Boundary & Failure Cases

- Tag length exactly 80 bytes is accepted and 81 bytes is skipped (TC-107).
- Re-appending an unchanged hash after a deletion is accepted as a restore, while an unchanged hash on a live record is rejected (TC-102, TC-105).
- Empty previous ledger hash for the first event and non-empty for later events (TC-101, TC-102).
- Unknown identifiers return empty results instead of failing (TC-110).
- Hostile text is never interpreted as SQL (TC-111).
- A second `init` must not duplicate migration bookkeeping (TC-112).

## Regression Scope

`engine.rs` (capture, scan, archive_version, restore), `agent_api.rs` (history and folder listing through find_path and versions) and the `ladon` CLI all call these methods. The engine-level suites `tests/it_history.rs`, `tests/it_nema_task.rs` and `tests/it_agent_folder.rs` must stay green. The SQL files are untouched, so schema compatibility is checked by TC-112 and by opening a fresh database in every test.
