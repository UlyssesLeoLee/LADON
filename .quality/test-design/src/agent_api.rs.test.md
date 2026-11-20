# Test Design: `src/agent_api.rs`

Source: `src/agent_api.rs`

Source-SHA256: `20b7d5f52db5aa6b29c7e20bcb4158e7126996f0122be8d96dc28192ee18a48a`

## Test Strategy

Integration tests build a real `Engine` over a `tempfile::tempdir` and call `AgentApi` methods directly, with no mocks. Files are created with `std::fs`; tracked files are captured through the engine so that ledger references exist. The link test creates a real symbolic link and, on Windows, a directory junction through `mklink /J`, which needs no special privilege; when neither can be created the test skips its assertions, unless the environment variable `LADON_REQUIRE_LINK_TESTS` is set, in which case it fails. The listing-order test uses a file name that sorts differently bytewise and in NTFS enumeration order, so a missing sort is detected. No test sets process-wide environment variables. The `it_agent_folder_snapshot_history` test and the inline `ut_agent_capability_status` test cover the basic folder, history and capability paths.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-401 | Workspace with B.md, tracked a.md, directory sub, .git and the default .LADONignore; list_folder on the root | Names are B.md, a.md, sub in bytewise order; a.md is a file of 4 bytes with a tracked reference; B.md is a file of 2 bytes without one; sub is a directory; internal entries hidden | tests/it_agent_folder.rs::tc401_list_folder_filters_internal_entries_and_sorts |
| TC-402 | limit 0, limit 1001, and limits 1, 2 and 1000 over three entries | Both invalid limits fail with limit must be 1..=1000; limit 1 returns one entry, limit 2 returns exactly two entries and limit 1000 returns all three | tests/it_agent_folder.rs::tc402_list_folder_limit_bounds |
| TC-403 | list_folder on a regular file, on a parent reference, on a missing folder and on an existing empty folder outside the workspace given as an absolute path | Error not a directory for the file; an error for the parent reference and the missing folder; error folder escapes workspace for the outside folder | tests/it_agent_folder.rs::tc403_list_folder_rejects_file_escape_and_missing |
| TC-404 | A symbolic link to a.md and a junction to the sub directory inside the workspace | Neither link appears in the listing, which contains exactly a.md and sub; the test skips when neither link type can be created | tests/it_agent_folder.rs::tc404_list_folder_skips_symlink_entries |
| TC-405 | read_file_local with max_bytes 0, above 8 MiB, below the file size, exactly 8 MiB and a sufficient limit; also an absolute path outside the workspace | Errors read limit must be 1..8MiB twice, file exceeds caller limit, then the exact bytes for the sufficient limit and a 10-byte result for the 8 MiB limit; the outside path fails | tests/it_agent_folder.rs::tc405_read_file_local_limits |
| TC-406 | history for an untracked name; grep_local for a pattern present on two lines with limit 1 | History fails with file not found in ledger; grep returns one hit with the right line number and path | tests/it_agent_folder.rs::tc406_history_unknown_and_grep_limit |
| TC-407 | allow_remote_read for a normal file, then for a file added with add_sensitive | Ok for the normal file; error containing denies external data transfer for the sensitive file | tests/it_agent_folder.rs::tc407_allow_remote_read_respects_ignore_rules |
| TC-408 | Capture a file, list the root, read history | Folder entry exists; history has one version, its file reference equals the captured file id and the chain is valid | tests/it_agent_folder.rs::it_agent_folder_snapshot_history |

## Boundary & Failure Cases

- Limits exactly 1 and 1000 are accepted, 0 and 1001 are rejected (TC-402).
- Read limit exactly 8 MiB is accepted and 8 MiB plus one byte is rejected (TC-405).
- A folder outside the workspace given as an absolute path is denied with the containment error, and a parent reference never lists anything (TC-403); an absolute file path outside the workspace is denied for reads (TC-405).
- Symbolic links and junctions are skipped, never followed (TC-404).
- Tracked and untracked entries in the same folder are distinguished by the ledger reference (TC-401).
- Names are ordered bytewise, which differs from NTFS enumeration order (TC-401).

## Regression Scope

`engine.rs` (resolve, file_by_path, store access), `archive.rs` (`assert_under_workspace`, called by `resolve`), `store.rs` (find_path, versions, verify_chain), `policy.rs` and `search.rs` feed this module. The `agent-list` CLI command uses `list_folder`. `tests/it_history.rs`, `tests/it_store.rs` and `tests/it_engine.rs` must stay green.
