# Test Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `38e8d8ba9b758f6b4274b945aafe34526eada7f133041e38a9a1edd1a431a7e3`

## Test Strategy

Integration tests drive the public `Store` API against a real Turso database file inside a `tempfile::tempdir`, with no mocks. Each test creates its own directory, so tests are isolated and can run in parallel. `Store::new(dir/.ladon/meta.turso)` is the fixture. A library in the 0.0.3 shape is produced by `Store::init` and the existing Store methods (migrations 1 and 2 only); migration 4 is applied only by `ensure_schema`.

Where the Store API exposes no reader (row contents of the legacy tables, table existence, the `doc_task_events` chain), a test opens a separate raw `turso` connection on the same file. Every Store method drops its own connection before returning, so a raw connection never overlaps a Store connection.

Connection retry is tested at unit level (`src/store.rs`, `mod tests`) with an injected `Opener` whose scripted calls return `turso::Error::Busy` or a non-lock error; the `BusyThen` script delegates to `TursoOpener` on a real file after its lock errors. Only the retry loop and its backoff are under test there; the backoff sleep is real and totals at most about 1.2 seconds per test. The cross-process lock is observed by IT-048, which re-runs the test binary as a child process; IT-048 asserts only that the child exits successfully.

Compile-time freezing of the kernel signatures used by `ui-prototype` is IT-068 in `tests/it_kernel_api_freeze.rs`.

Every new test is Pass when its exact name is listed as passing in the final test run; every other case is Not Run.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location | Status |
| --- | --- | --- | --- | --- |
| TC-820 | Fresh library: `Store::init`, one `append_version` for `a.md` with hash `h1`, then `ensure_schema(false)`. | `ensure_schema` returns `Applied`; `schema_migrations` holds exactly the versions 2 and 4; `doc_task_events` exists and holds exactly one `dt.migration.applied` row; `file_by_id(id)` for the id returned by `append_version` returns the record with the same id as `find_path("a.md")`, with `relative_path` `a.md`, `current_hash` `h1`, `version_no` 1 and `deleted` false; `file_by_id("no-such-id")` is `None` | tests/it_doctask_migration.rs::it_047_fresh_library_records_versions_2_and_4 | Not Run |
| TC-821 | Two libraries: one with `Store::init` only; one with `Engine::init` only. The library with `Store::init` followed by `ensure_schema` is asserted in TC-820. | In both libraries `schema_migrations` holds only version 2 and `doc_task_events` does not exist | tests/it_doctask_migration.rs::it_047_init_and_engine_init_do_not_apply_004 | Not Run |
| TC-822 | 0.0.3-shaped library (`legacy_library` in the test file): `Store::init`; three `append_version` calls (`a.md` with `h1`, `a.md` with `h2`, `b.md` with `hb`); `add_annotation`; `create_task`; `set_task_status` to `working`; `add_governance_event`; `set_tags`. Snapshot every row of `resources`, `versions`, `events`, `archive_objects`, `file_tags`, `annotations`, `doc_tasks` and `governance_events` as text, one string per row with every column rendered. `archive_objects` has no row in this fixture. | After `ensure_schema(false)` every snapshot equals its earlier snapshot row for row and column for column; `verify_chain` for the `a.md` file returns `true` before and after; `versions` for that file returns 2 entries | tests/it_doctask_migration.rs::it_047_upgrade_keeps_legacy_rows_unchanged | Not Run |
| TC-823 | `legacy_library`, then `ensure_schema(false)`; then `DROP TABLE doc_counters` on a raw connection; then two `ensure_schema(false)` calls and one `store.init()` | Both `ensure_schema` calls after the drop return `Current`; `doc_counters` is not recreated; exactly one `dt.migration.applied` row exists in `doc_task_events` | tests/it_doctask_migration.rs::it_047_recorded_version_not_reapplied | Not Run |
| TC-824 | The text of `sql/004_document_tasks.sql`, read with `include_str!` | The text contains no `--` and no `/*`; it contains none of `TRIGGER`, `ALTER`, `CHECK`, `WITH` and `WHERE` in any letter case; split on `;`, every non-empty statement starts with `CREATE TABLE IF NOT EXISTS `, `CREATE INDEX IF NOT EXISTS ` or `CREATE UNIQUE INDEX IF NOT EXISTS `; the statements are 14 table statements and 16 index statements, 30 in total | tests/it_doctask_migration.rs::it_047_sql_004_uses_only_allowed_statements | Not Run |
| TC-825 | Library with data (as TC-822). Record the source row counts of the five counted tables and whether `meta.turso-wal` exists. Call `ensure_schema(false)`. | Result `Applied` with `Verified { path }`; `path` equals `<dir>/.ladon/backup/meta.turso.pre-004`; the backup opens and its five row counts equal the recorded source counts; when the source `meta.turso-wal` existed before the call, `meta.turso.pre-004-wal` exists afterwards; the `dt.migration.applied` payload has `version` 4, `backup` `verified`, `backup_verified` true and a null `backup_reason` | tests/it_doctask_migration.rs::it_073_backup_pair_copied_and_counted | Not Run |
| TC-826 | Library with data. A directory is created at `.ladon/backup/meta.turso.pre-004`, so the copy cannot be written. | `ensure_schema(false)` fails with an error whose text starts with `E_MIGRATION_BACKUP` and which downcasts to `MigrationBackupUnverified`; `schema_migrations` holds only version 2; `doc_task_events` does not exist. Then `ensure_schema(true)` returns `Applied` with `Unverified { reason }`, the reason is non-empty; `schema_migrations` holds versions 2 and 4; the `dt.migration.applied` payload has `backup` `unverified`, `backup_verified` false and `backup_reason` equal to the reason | tests/it_doctask_migration.rs::it_073_unverified_backup_refuses_then_accepts_with_flag | Not Run |
| TC-827 | Empty library: `Store::init` only, no rows | `ensure_schema(false)` returns `Applied { backup: NotNeeded }` without an error; `.ladon/backup` does not exist; the payload has `backup` `not_needed` and `backup_verified` true | tests/it_doctask_migration.rs::it_073_empty_library_has_no_backup | Not Run |
| TC-828 | `Store` over a temp file; an injected `Opener` returns `turso::Error::Busy` with the text `Locking error: Failed locking file. File is locked by another process` for the first two calls and then delegates to `TursoOpener` | `connect_via` returns `Ok`; the opener was called 3 times; the elapsed time of the call is at least 75 ms (the two base sleeps) and below 1500 ms (the bounds with full jitter are 37 and 75 ms plus scheduling slack); `backoff_ms` for retries 1 to 5 with jitter 0 is 25, 50, 100, 200, 400; with jitter 500 it is 37, 75, 150, 300, 600; the connection is usable afterwards (a `CREATE TABLE` runs on it) | src/store.rs::tests::ut_162_connect_retry_with_injected_opener | Not Run |
| TC-829 | Three injected `Opener` scripts: (a) `Busy` on every call; (b) `Error::Error("no such table")` on every call; (c) `Busy` on the first call and `Error::Error("no such table")` on every later call | (a) `Err` after exactly 6 calls; the error downcasts to `StoreBusy` with `attempts` 6 and its text starts with `E_STORE_BUSY`; (b) `Err` after exactly 1 call; the error does not downcast to `StoreBusy` and its text contains `no such table`; (c) `Err` after exactly 2 calls; its text contains `no such table` | src/store.rs::tests::ut_162_connect_exhaustion_and_non_lock_errors_are_not_retried | Not Run |
| TC-1050 | A library with data and a file `.ladon/backup/meta.turso.pre-004` that holds the bytes `earlier backup`; `ensure_schema(false)`. | The result is `Applied` with a `Verified` backup whose path is `meta.turso.pre-004-2`; the earlier file still holds `earlier backup`. | tests/it_doctask_migration.rs::it_1050_an_existing_backup_is_never_overwritten | Not Run |
| IT-048 | Turso behaviour probe, see the section below | Each of the four probe tests runs to completion and prints `IT-048 probe:` lines to standard error. `it_048_rows_affected_transactions_and_rollback` asserts that the value read after `ROLLBACK` is the committed value 2; `it_048_check_foreign_key_and_unique_observations` asserts that the duplicate unique index insert fails; `it_048_cross_process_lock_observation` asserts that the child process exits successfully | Not Run;tests/it_doctask_migration.rs::it_048_rows_affected_transactions_and_rollback; tests/it_doctask_migration.rs::it_048_check_foreign_key_and_unique_observations; tests/it_doctask_migration.rs::it_048_autoincrement_after_rollback; tests/it_doctask_migration.rs::it_048_cross_process_lock_observation |
| IT-068 | Compile-only kernel signature freeze | Build succeeds; the test body is empty at run time | Not Run;tests/it_kernel_api_freeze.rs::it_068_kernel_signatures_are_frozen |

## Turso Behaviour Probe (IT-048)

The probe tests in `tests/it_doctask_migration.rs` are named `it_048_*`. Each prints its observations to standard error on lines beginning with `IT-048 probe:`. The cross-process test re-runs the test binary as a child process, which prints its result on a line beginning with `IT-048 child:`; the parent prints that line again with the prefix `IT-048 probe: cross_process`. The probes cover the following points of turso 0.8.3 (pinned in `Cargo.toml`), on which the document-task design (57 section 3.1 and section 2.4) depends:

1. Rows affected (`it_048_rows_affected_transactions_and_rollback`): the value returned by `execute` for an `UPDATE` matching one row, for an `UPDATE` matching no row, and for an `INSERT OR IGNORE` that is ignored. The test asserts only the value read after a `ROLLBACK`, which must be the committed value.
2. Transactions (same test): `BEGIN IMMEDIATE` and `ROLLBACK` run through `execute` on one connection. No `it_048_*` test runs `COMMIT` or reads a committed row from a second connection.
3. `CHECK` constraint and foreign key with `PRAGMA foreign_keys = ON` (`it_048_check_foreign_key_and_unique_observations`): whether the violating insert is accepted or rejected, with the error variant and text when rejected. The test asserts neither outcome; the design relies on neither.
4. Unique index violation (same test): the `turso::Error` variant and text. The test asserts that the duplicate insert fails.
5. `AUTOINCREMENT` sequence after a rolled-back insert (`it_048_autoincrement_after_rollback`): the `seq` values of the committed rows, printed as a comma-separated list. The test asserts nothing; the list shows whether the next `seq` is contiguous with the last committed one, which decides the level of the `SEQ_GAP` check.
6. A second process opening a database held open by the first (`it_048_cross_process_lock_observation`): the error variant and text printed by the child. The test asserts only that the child process exits successfully.

The record of the observed values is the standard error of each run of the `it_doctask_migration` test target.

## Acceptance Traceability

| Acceptance clause (58 WBS, WP01) | Input | Assertion | Case |
| --- | --- | --- | --- |
| Fresh library after `init` and `ensure_schema` has versions 2 and 4 | fresh library | versions equal {2, 4} | TC-820 |
| `Store::init` and `Engine::init` alone do not apply 004 | two libraries | no version 4, no `doc_task_events` | TC-821 |
| 0.0.3 library upgrades with unchanged legacy rows and no new legacy rows | legacy library | row snapshots equal | TC-822 |
| Recorded versions are not re-run; a dropped table is not recreated | drop after migration | `doc_counters` absent after repeated calls | TC-823 |
| 004 has no comment, trigger, `ALTER`, `CHECK`, partial index | sql text | constructs absent | TC-824 |
| Backup pair copied and row counts checked | legacy library with data | backup opens with equal counts | TC-825 |
| Unverifiable backup: 004 not executed; confirmed run records `backup_verified:false` | blocked backup | error, then flag in payload | TC-826 |
| Empty library makes no backup | empty library | no backup directory | TC-827 |
| Retry: 6 attempts, backoff 25 to 400 ms plus jitter, lock error mapped to `E_STORE_BUSY`, non-lock error not retried | injected opener | counts, bounds, error type | TC-828, TC-829 |
| `file_by_id` returns the record for a known id and `None` otherwise | known and unknown ids | equality with `find_path` | TC-820 |
| Probe results recorded | turso behaviour | the probe tests print the six points above as `IT-048 probe:` lines on standard error | IT-048 |
| Kernel signatures used by `ui-prototype` compile | compile | build | IT-068 |

## Boundary & Failure Cases

- A library is treated as non-empty when any of the five counted tables has at least one row. TC-825 covers a library with rows in all five tables and TC-827 covers a library with none. No case covers a library with rows in exactly one table.
- The backup copy is taken while the migrating connection is open, and the verification reads only the copy (TC-825).
- A backup target that cannot be written is reported as unverified and never silently skipped (TC-826).
- A second `ensure_schema` run that finds version 4 recorded returns `Current` before any backup or event is written. TC-823 checks the event count; no case checks the backup directory.
- Backoff jitter is applied on top of the capped base, so the upper bound of each sleep is 1.5 times the base (TC-828).
- Exhausting the retries returns the typed error rather than the last turso message (TC-829).

## Regression Scope

The existing `tests/it_store.rs` cases tc101 to tc112 (12 cases, listed in `docs/04_tests/42_IT详细设计.md`) must stay green; tc112 (a table dropped after migration 2 is not recreated by `init`) is the regression anchor for the migration-recording rule. `tests/it_store.rs` and the engine suites `tests/it_engine.rs`, `tests/it_history.rs`, `tests/it_nema_task.rs` and `tests/it_agent_folder.rs` open fresh databases through `Store::init` or `Engine::init`, so they also cover the unchanged `init` path. The `ui-prototype` kernel build depends on the signatures frozen by IT-068. The `tests/it_doctask_store.rs` cases are designed in `.quality/test-design/src/doctask_store.rs.test.md`; IT-069 in `tests/it_doctask_concurrency.rs`, which asserts that a failed MCP write reply carries code `E_STORE_BUSY`, is designed in `.quality/test-design/src/doctask_mcp.rs.test.md`.
