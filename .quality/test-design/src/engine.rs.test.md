# Test Design: `src/engine.rs`

Source: `src/engine.rs`

Source-SHA256: `7753a8573b635cacb21549e2ca9c28c613ed68c7c350d13bf375b21e5ee7959d`

## Test Strategy

Integration tests use a real `Engine` over a `tempfile::tempdir` workspace, a real Turso database, the real CAS and a `FilesystemProvider` pointed at another temporary directory. There are no mocks. Tests that only touch the ledger and the live tree live in `tests/it_engine.rs`; they assume the environment variable `LADON_GLOBAL_IGNORE` is unset, because the policy reads it, and they run in parallel. Archive tests need the process-wide variable `LADON_ARCHIVE_KEY_HEX`; they live in their own file `tests/it_engine_archive.rs` as a single sequential test, so the variable cannot race with other tests (each integration test file is its own process). The key used is a fixed random 64-hex-digit test value, never a documented example key. The `tests/it_history.rs` cases also exercise the engine.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-301 | Capture req.md, call restore_plan with its version id | Text contains the file id, version 1, the content hash and req.md in the RESTORE PLAN line, plus Target exists true | tests/it_engine.rs::tc301_restore_plan_describes_target |
| TC-302 | restore_plan with an unknown version id | Error containing unknown version | tests/it_engine.rs::tc302_restore_plan_unknown_version |
| TC-303 | Capture docs/a.md, call restore_plan, delete the file and call it again | Both plans show the forward-slash path docs/a.md; the first shows Target exists true and the second Target exists false | tests/it_engine.rs::tc303_restore_plan_nested_path_and_missing_target |
| TC-304 | Capture two versions, edit the live file without capturing, call restore with approve true | Error containing live file changed outside ledger; the live file keeps the edited content | tests/it_engine.rs::tc304_restore_refuses_diverged_live_file |
| TC-305 | Capture, delete the file, scan, then restore version 1 with approve | File exists again with the original bytes; events are created, deleted, restored; chain verifies | tests/it_engine.rs::tc305_restore_recreates_missing_file |
| TC-306 | resolve for a file under .git, a file under .ladon and a file outside the workspace | Error internal file cannot be tracked for the first two; error path escapes the workspace for the third | tests/it_engine.rs::tc306_resolve_rejects_internal_and_outside_paths |
| TC-307 | Engine::open on an empty directory; then init, capture, open and file_by_path with backslash and unknown names | Open fails with not initialized; after init it succeeds and finds the file; unknown path gives file not found in ledger | tests/it_engine.rs::tc307_open_requires_initialized_workspace |
| TC-308 | Capture a.md; append a rule for a.md to .LADONignore; capture again; reload_policy; capture again | sensitive is false before reload and true after reload | tests/it_engine.rs::tc308_reload_policy_applies_new_rules |
| TC-309 | Tree with .git/config, docs/x.md and y.md; run scan | Ledger lists exactly docs/x.md and y.md | tests/it_engine.rs::tc309_scan_skips_internal_directories |
| TC-310 | Capture a file of 9 MiB, which is above the 8 MiB structured-diff limit | Status is captured and the changed sections are exactly LARGE_BINARY_OR_TEXT | tests/it_engine.rs::tc310_capture_large_file_defers_structured_diff |
| TC-311 | Archive with no key, a non-hex key and a short key; then with the valid key archive, repeat, damage the remote object and archive with eviction, restore the object and evict, load without and with provider, edit and capture while cold, archive on a second provider | Key errors name the variable, 64 hex digits and exactly 32 bytes and record nothing; the object is encrypted and named after the provider; the repeat returns the same locator and verified_at; the damaged object gives archive authentication failed and keeps the local copy; after eviction load without provider fails with cold and with provider returns the original bytes; a provider without a record gives no archived location found; the next capture reports DIFF_DEFERRED_PREVIOUS_COLD; a first archive on a new provider fails with local copy unavailable | tests/it_engine_archive.rs::tc311_archive_evict_and_load_lifecycle |
| TC-312 | Capture secret.pem, which the default rules ignore, then archive_version | Error is returned and its text contains denies external data transfer | tests/it_history.rs::it_02_sensitive_prevents_archive |
| TC-313 | Capture, modify, capture; restore with approve false and true | Restore without approve fails; with approve restores and appends a third event | tests/it_history.rs::it_03_archive_restore_with_verification |
| TC-314 | Capture secret.pem and x.md with identical bytes, then archive the version of x.md | The version of x.md is not sensitive itself, and the error contains sensitive version blocked from external storage | tests/it_engine.rs::tc314_archive_blocks_hash_shared_with_sensitive_file |

## Boundary & Failure Cases

- Unchanged content returns status unchanged and creates no version (it_history it_01).
- A file larger than 8 MiB switches to the large-file marker (TC-310).
- Eviction followed by capture exercises the cold-previous branch (TC-311).
- Eviction never happens before the remote object authenticates (TC-311).
- Restore must never overwrite a live file whose hash differs from the ledger (TC-304).
- Restore of a deleted file works without a live target (TC-305).
- Internal paths (`.ladon`, `.git`) and outside-workspace paths cannot be captured (TC-306).
- A missing or malformed archive key stops archiving before any upload (TC-311).
- A version sharing its content hash with a sensitive version is never archived (TC-314).

## Regression Scope

`store.rs` (every ledger write) and `agent_api.rs` (history, folder listing) depend on engine behaviour; `tests/it_store.rs` and `tests/it_agent_folder.rs` must stay green. The CLI commands `capture`, `scan`, `archive`, `plan` and `restore` call these methods. The policy, CAS and crypto modules are exercised indirectly and have their own inline tests.
