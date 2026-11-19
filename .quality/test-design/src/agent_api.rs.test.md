# Test Design: `src/agent_api.rs`

Source: `src/agent_api.rs`

Source-SHA256: `20b7d5f52db5aa6b29c7e20bcb4158e7126996f0122be8d96dc28192ee18a48a`

## Test Strategy

Integration tests build a real `Engine` over a `tempfile::tempdir` and call `AgentApi` methods directly, with no mocks. Files are created with `std::fs`; tracked files are captured through the engine so that ledger references exist. The symbolic-link test creates a real link and returns early without failing when the platform refuses link creation because of missing privileges (common on Windows without developer mode), so it never produces a false failure. No test uses process-wide environment variables. The pre-existing `it_agent_folder_snapshot_history` test and the inline `ut_agent_capability_status` test remain in place.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-401 | Workspace with b.md, tracked a.md, directory sub, .git and the default .LADONignore; list_folder on the root | Names are a.md, b.md, sub in that order; kinds file, file, directory; a.md has a tracked reference and the right byte size; b.md has none; internal entries hidden | tests/it_agent_folder.rs::tc401_list_folder_filters_internal_entries_and_sorts |
| TC-402 | limit 0, limit 1001, and limit 2 over three entries | Both invalid limits fail with limit must be 1..=1000; the third call returns exactly two entries | tests/it_agent_folder.rs::tc402_list_folder_limit_bounds |
| TC-403 | list_folder on a regular file, on a parent reference, and on a missing folder | Error not a directory for the file; an error for the parent reference and for the missing folder, and nothing outside the workspace is listed | tests/it_agent_folder.rs::tc403_list_folder_rejects_file_escape_and_missing |
| TC-404 | A symbolic link to a.md inside the workspace | The link does not appear in the listing; the test returns early when links cannot be created | tests/it_agent_folder.rs::tc404_list_folder_skips_symlink_entries |
| TC-405 | read_file_local with max_bytes 0, above 8 MiB, below the file size, and a sufficient limit; also a path outside the workspace | Errors read limit must be 1..8MiB twice, file exceeds caller limit, then the exact bytes for the sufficient limit; the outside path fails | tests/it_agent_folder.rs::tc405_read_file_local_limits |
| TC-406 | history for an untracked name; grep_local for a pattern present on two lines with limit 1 | History fails with file not found in ledger; grep returns one hit with the right line number | tests/it_agent_folder.rs::tc406_history_unknown_and_grep_limit |
| TC-407 | allow_remote_read for a normal file, then for a file added with add_sensitive | Ok for the normal file; error containing denies external data transfer for the sensitive file | tests/it_agent_folder.rs::tc407_allow_remote_read_respects_ignore_rules |
| TC-408 | Capture a file, list the root, read history | Folder entry exists; history has one version and a valid chain | tests/it_agent_folder.rs::it_agent_folder_snapshot_history |

## Boundary & Failure Cases

- Limits exactly 1 and 1000 are accepted, 0 and 1001 are rejected (TC-402).
- Read limit exactly 8 MiB is the upper bound (TC-405).
- A path that resolves outside the workspace through a parent reference is denied (TC-403, TC-405).
- Symbolic links are skipped, never followed (TC-404).
- Tracked and untracked entries in the same folder are distinguished by the ledger reference (TC-401).

## Regression Scope

`engine.rs` (resolve, file_by_path, store access), `store.rs` (find_path, versions, verify_chain), `policy.rs` and `search.rs` feed this module. The `agent-list` CLI command uses `list_folder`. `tests/it_history.rs`, `tests/it_store.rs` and `tests/it_engine.rs` must stay green.
