# Test Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `8a0eadfd5ab375e4eb41c452518f6de95c661faa52a38b74fd50e2abd0c432ae`

## Test Strategy

Integration tests drive the public `Store` API against a real Turso database file inside a `tempfile::tempdir`, with no mocks. Each test creates its own directory, so tests are isolated and can run in parallel. `Store::new(dir/.ladon/meta.turso)` followed by `init()` is the shared fixture. Where the Store API exposes no reader (tag `source` and `rule_version`, existence of a table) or a test must change the schema (dropping a table), the test opens a separate raw `turso` connection on the same file; every Store method drops its own connection before returning, so a raw connection opened by a test never overlaps a Store connection. The engine-level tests in `tests/it_history.rs` and `tests/it_nema_task.rs` exercise the same code through `Engine`. Every public method of `Store` is called directly by a test except `add_governance_event`, which runs only inside `add_annotation`, `create_task` and `set_task_status`; its rows are checked through `governance_events` (TC-108), and `paths_for_hash`, which runs only inside `Engine::content_is_excluded`, which `Engine::archive_version`, `AgentApi::allow_remote_read` and the `tag-folder` command call; it has no direct test and is covered through the engine (TC-114, engine test TC-324 and agent test TC-410). The cases cover new and existing resources in `append_version` (TC-101, TC-102), deleted and live records in `mark_deleted` and `append_version` (TC-104, TC-105), and valid and invalid task states in `set_task_status` (TC-108, TC-109). Two branches have no test: the `missing version` error in `mark_deleted` (`src/store.rs` line 121) and the `Ok(false)` returns of `verify_chain` (`src/store.rs` lines 210 and 212, for a broken previous-hash link or a mismatched ledger hash), so every `verify_chain` assertion expects `true`. Assertions cover returned values, stored columns, error texts and the order of events, governance events, tags and archive records. A test is Pass when its exact name is listed as passing in the final test run or in an earlier recorded run named in docs/04_tests/46; every other case, including one covered only by a file-level count and one with no test function, is Not Run.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-101 | append_version for a new path with hash h1, size 3, one changed section, actor human | Returns three non-empty ids; find_path shows version_no 1 and hash h1; versions has one row with the given actor, reason, byte_size 3 and sections; events has one row of kind created | Pass;tests/it_store.rs::tc101_append_version_round_trip |
| TC-102 | Append hash h1 and then hash h2 to the same path, then append h2 again | The h2 append gives version_no 2, event kind modified and a ledger link to the first event; the repeated hash returns an error containing no content change; verify_chain is true | Pass;tests/it_store.rs::tc102_second_version_and_no_change_rejected |
| TC-103 | Append with sensitive true, then a non-sensitive version of another path | find_path reports sensitive true for the first; is_hash_sensitive is true for its hash and false for an unknown hash | Pass;tests/it_store.rs::tc103_sensitive_flag_round_trip |
| TC-104 | mark_deleted on a tracked record, then again with the refreshed record | Record is deleted; a deleted event is appended and the chain verifies; the second call adds no event | Pass;tests/it_store.rs::tc104_mark_deleted_appends_event_and_is_idempotent |
| TC-105 | Append a new hash for a path whose record is deleted; append the same hash again for a second deleted path | New event kind is restored in both cases; find_path shows deleted false and the next version number | Pass;tests/it_store.rs::tc105_append_after_delete_is_restored_event |
| TC-106 | archive_record for one hash and provider twice with different locator, encrypted flag and verified_at; then a second provider with a later verified_at | One record with the second locator, encrypted false and the second timestamp; after the second provider is added the list is s3 then nas, newest first | Pass;tests/it_store.rs::tc106_archive_record_upsert |
| TC-107 | set_tags with b, a, a blank tag, an 81-character tag and an 80-character tag, then again with source ai for tag a only | tags returns a, b and the 80-character tag sorted; blank and oversize tags absent; tag a carries source ai and rule version v2 while tag b keeps rule and v1 | Pass;tests/it_store.rs::tc107_set_tags_filters_and_sorts |
| TC-108 | add_annotation, create_task, set_task_status done, then list_tasks and governance_events | Task listed with status done and its title; governance events appear in order annotation.created, task.created, task.state_changed | Pass;tests/it_store.rs::tc108_annotation_task_and_governance_flow |
| TC-109 | set_task_status with state bogus; with an unknown task id | Errors containing invalid task state and task missing respectively | Pass;tests/it_store.rs::tc109_invalid_task_state_and_unknown_task |
| TC-110 | find_path, latest_version, version, versions, events, archives, tags, governance_events and list_tasks on unknown keys; is_hash_sensitive and verify_chain on an unknown id; list_files on an empty store | None for single lookups, empty lists for collections, false from is_hash_sensitive, true from verify_chain, no errors | Pass;tests/it_store.rs::tc110_missing_records_return_none |
| TC-111 | Reason text with quotes, a newline, Japanese text and a quote followed by DROP TABLE resources | Stored and read back byte-identical; the resources table still works afterwards | Pass;tests/it_store.rs::tc111_special_characters_bound_verbatim |
| TC-112 | After the first init drop the migration-2 table agent_grants through a raw connection, then call init twice | The table exists after the first init and stays absent after the later init calls, and earlier data remains readable | Pass;tests/it_store.rs::tc112_init_is_idempotent |
| TC-113 | Capture a file through Engine, delete it from disk and run scan | The resource is marked deleted, its file id is unchanged and the hash chain verifies | Pass;tests/it_history.rs::it_04_detect_delete_and_keep_history (engine-level) |
| TC-114 | Engine-level: a version whose content hash is also held by a path the current rules exclude. Cases: secret.pem (excluded) and x.md with identical bytes, then archive of the x.md version; and a file captured under restricted/ that is moved to plain/ after a rule covers restricted/, then archive of the plain/ version | Archive refused with sensitive version blocked from external storage; nothing is uploaded | Case 1 Pass;tests/it_engine.rs::tc314_archive_blocks_hash_shared_with_sensitive_file; case 2 Pass;tests/it_engine.rs::tc_moved_file_keeps_sensitivity_after_rule (engine-level) |

## Boundary & Failure Cases

- Tag length exactly 80 bytes is accepted and 81 bytes is skipped (TC-107).
- Re-appending an unchanged hash after a deletion is accepted as a restore, while an unchanged hash on a live record is rejected (TC-102, TC-105).
- Empty previous ledger hash for the first event and non-empty for later events (TC-101, TC-102).
- Unknown identifiers return empty results instead of failing (TC-110).
- Hostile text is never interpreted as SQL (TC-111).
- Migration 2 is applied once: a table dropped after the first `init` is not recreated by later `init` calls (TC-112).
- An archive upsert replaces the locator, the encrypted flag and `verified_at`, including a false encrypted flag (TC-106); a tag upsert replaces `source` and `rule_version` (TC-107).
- `paths_for_hash` includes deleted and moved paths and lists each path once; it is exercised through archive refusal (TC-114) and through `Engine::content_is_excluded` (engine test TC-324) and through the remote read check (agent test TC-410).
- Known limitation: `LadonPolicy::excluded` does not normalise `..` components, so a path that contains a `..` component is not rejected by its outside-workspace check, whether the path is relative or an absolute path under the root.

## Regression Scope

`engine.rs` (capture, scan, archive_version through `paths_for_hash`, restore), `agent_api.rs` (history and folder listing through find_path and versions, and the remote read check through `paths_for_hash`) and the `ladon` CLI all call these methods. The engine-level suites `tests/it_history.rs`, `tests/it_nema_task.rs` and `tests/it_agent_folder.rs` must stay green. Schema compatibility is checked by TC-112 and by opening a fresh database in every test.
