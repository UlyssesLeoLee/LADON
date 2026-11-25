# Test Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `38e8d8ba9b758f6b4274b945aafe34526eada7f133041e38a9a1edd1a431a7e3`

## Test Strategy

Integration tests drive the public `Store` API against a real Turso database file inside a `tempfile::tempdir`, with no mocks. Each test creates its own directory, so tests are isolated and can run in parallel. `Store::new(dir/.ladon/meta.turso)` is the fixture. A library in the 0.0.3 shape is produced by `Store::init` and the existing Store methods (migrations 1 and 2 only); migration 4 is applied only by `ensure_schema`.

Where the Store API exposes no reader (row contents of the legacy tables, table existence, the `doc_task_events` chain), a test opens a separate raw `turso` connection on the same file. Every Store method drops its own connection before returning, so a raw connection never overlaps a Store connection.

Connection retry is tested at unit level (`src/store.rs`, `mod tests`) with an injected `Opener` that returns `turso::Error::Busy` for a chosen number of attempts and then delegates to `TursoOpener` on a real file. Only the retry loop and its backoff are under test there; the backoff sleep is real and totals at most about 1.2 seconds per test. The cross-process lock is tested by IT-048 with a child process of the test binary.

Compile-time freezing of the kernel signatures used by `ui-prototype` is IT-068 in `tests/it_kernel_api_freeze.rs`.

Every new test is Pass when its exact name is listed as passing in the final test run; every other case is Not Run.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-820 | Fresh library: `Store::init`, then `ensure_schema(false)`. Append one version for `a.md`. | `ensure_schema` returns `Applied`; `schema_migrations` holds exactly the versions 2 and 4; `doc_task_events` exists and is empty apart from the `dt.migration.applied` row; `file_by_id(id)` for the id returned by `append_version` equals `find_path("a.md")`; `file_by_id("unknown")` is `None` | Not Run;tests/it_doctask_migration.rs::it_047_fresh_library_records_versions_2_and_4 |
| TC-821 | Three libraries: one with `Store::init` only; one with `Engine::init` only; one with `Store::init` followed by `Store::ensure_schema` | In the first two, `schema_migrations` holds only version 2 and `doc_task_events` does not exist; the third holds versions 2 and 4 | Not Run;tests/it_doctask_migration.rs::it_047_init_and_engine_init_do_not_apply_004 |
| TC-822 | 0.0.3-shaped library: `Store::init`, two `append_version` calls on `a.md` and `b.md`, `add_annotation`, `create_task`, `set_task_status`, `add_governance_event`. Snapshot every row of `resources`, `versions`, `events`, `archive_objects`, `file_tags`, `annotations`, `doc_tasks`, `governance_events` and `schema_migrations` as text. | After `ensure_schema(false)` the snapshots are identical row for row and column for column; `annotations`, `doc_tasks` and `governance_events` have the same row counts; the existing Store API still returns the same `versions` and `verify_chain` result | Not Run;tests/it_doctask_migration.rs::it_047_upgrade_keeps_legacy_rows_unchanged |
| TC-823 | `ensure_schema(false)` on a library, then `DROP TABLE doc_counters` on a raw connection, then `ensure_schema(false)` twice | The second and third calls return `Current`; `doc_counters` is not recreated; exactly one `dt.migration.applied` row exists in `doc_task_events` | Not Run;tests/it_doctask_migration.rs::it_047_recorded_version_not_reapplied |
| TC-824 | The text of `sql/004_document_tasks.sql`, read with `include_str!` | Split on `;`, every non-empty statement starts with `CREATE TABLE IF NOT EXISTS ` or `CREATE INDEX IF NOT EXISTS ` or `CREATE UNIQUE INDEX IF NOT EXISTS `; the text contains no `--`, no `/*`, and no case-insensitive occurrence of `TRIGGER`, `ALTER`, `CHECK`, `WITH`; no index statement contains `WHERE`; the text contains 30 statements: 14 `CREATE TABLE IF NOT EXISTS` and 16 `CREATE [UNIQUE] INDEX IF NOT EXISTS` | Not Run;tests/it_doctask_migration.rs::it_047_sql_004_uses_only_allowed_statements |
| TC-825 | Library with data (as TC-822). Record the source row counts of the five tables before the call. Call `ensure_schema(false)`. | Result `Applied` with `Verified { path }`; `path` equals `<dir>/.ladon/backup/meta.turso.pre-004`; the backup opens and its five row counts equal the recorded source counts; when the source `meta.turso-wal` existed before the call, `meta.turso.pre-004-wal` exists afterwards; the `dt.migration.applied` payload has `backup` `verified` and `backup_verified` true | Not Run;tests/it_doctask_migration.rs::it_073_backup_pair_copied_and_counted |
| TC-826 | Library with data. A directory is created at `.ladon/backup/meta.turso.pre-004`, so the copy cannot be written. | `ensure_schema(false)` fails with an error whose text starts with `E_MIGRATION_BACKUP`; `schema_migrations` has no version 4; `doc_task_events` does not exist. Then `ensure_schema(true)` returns `Applied` with `Unverified { reason }`, the reason is non-empty; `schema_migrations` holds version 4; the `dt.migration.applied` payload has `backup` `unverified`, `backup_verified` false and `backup_reason` equal to the reason | Not Run;tests/it_doctask_migration.rs::it_073_unverified_backup_refuses_then_accepts_with_flag |
| TC-827 | Empty library: `Store::init` only, no rows | `ensure_schema(false)` returns `Applied { backup: NotNeeded }` without an error; `.ladon/backup` does not exist; the payload has `backup` `not_needed` and `backup_verified` true | Not Run;tests/it_doctask_migration.rs::it_073_empty_library_has_no_backup |
| TC-828 | `Store` over a temp file; an injected `Opener` returns `Busy("database is locked")` for the first two attempts and then delegates to `TursoOpener` | `connect_via` returns `Ok`; the opener was called 3 times; the elapsed time of the call is at least 75 ms (the two base sleeps) and below 1500 ms (the bounds with full jitter are 37 and 75 ms plus scheduling slack); `backoff_ms` for retries 1 to 5 with jitter 0 is 25, 50, 100, 200, 400; with jitter 500 it is 37, 75, 150, 300, 600; the connection is usable afterwards | Not Run;src/store.rs::ut_162_connect_retry_with_injected_opener |
| TC-829 | Injected `Opener` returning `Busy` on every call; a second opener returning `Error::Error("no such table")` on its first call; a third returning `Busy` once and then a non-lock error | First case: `Err` after exactly 6 calls, the error downcasts to `StoreBusy { attempts: 6 }` and its text starts with `E_STORE_BUSY`; second case: `Err` after exactly 1 call with no sleep; third case: `Err` after exactly 2 calls, the non-lock error is returned unchanged | Not Run;src/store.rs::ut_162_connect_exhaustion_and_non_lock_errors_are_not_retried |
| IT-048 | Turso behaviour probe, see the section below | Observations recorded in this document | Not Run;tests/it_doctask_migration.rs::it_048_* |
| IT-068 | Compile-only kernel signature freeze | Build succeeds; the test body is empty at run time | Not Run;tests/it_kernel_api_freeze.rs::it_068_kernel_signatures_are_frozen |

## Turso Behaviour Probe (IT-048)

The probe tests in `tests/it_doctask_migration.rs` are named `it_048_*`. Each prints its observations to standard error on lines beginning with `IT-048 probe:`. They record the following facts about turso 0.8.3, which the document-task design (57 section 3.1 and section 2.4) depends on:

1. Rows affected: the value returned by `execute` for an `UPDATE` matching one row, for an `UPDATE` matching no row, and for an `INSERT OR IGNORE` that is ignored.
2. `BEGIN IMMEDIATE`, `COMMIT` and `ROLLBACK` run through `execute`, and whether a second connection can read the committed row.
3. `CHECK` constraints and foreign keys with `PRAGMA foreign_keys = ON`: whether a violating insert fails. The design relies on neither, so the probe records the behaviour only.
4. A unique index violation: the `turso::Error` variant and its text.
5. `AUTOINCREMENT` sequence after a rolled-back insert: whether the next `seq` is contiguous with the last committed one. This decides the level of the `SEQ_GAP` check.
6. A second process opening a database held open by the first: the `turso::Error` variant and text. The child process is the test binary re-executed with an environment variable, and it prints the error on a line beginning with `IT-048 child:`.

The observed values are the `IT-048 probe:` and `IT-048 child:` lines of the CI log of the `it_doctask_migration` test target. Each line records one fact in the form given above; the lines are the record of the probe.

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
| Probe results recorded | turso behaviour | observations in this document | IT-048 |
| Kernel signatures used by `ui-prototype` compile | compile | build | IT-068 |

## Boundary & Failure Cases

- A library with rows in exactly one of the five counted tables is treated as non-empty (TC-825 uses a library with rows in all of them; TC-827 uses none).
- The backup copy is taken while the migrating connection is open, and the verification reads only the copy (TC-825).
- A backup target that cannot be written is reported as unverified and never silently skipped (TC-826).
- A second `ensure_schema` run that finds version 4 recorded writes no backup and no event (TC-823).
- Backoff jitter is applied on top of the capped base, so the upper bound of each sleep is 1.5 times the base (TC-828).
- Exhausting the retries returns the typed error rather than the last turso message (TC-829).

## Regression Scope

The existing `tests/it_store.rs` cases tc101 to tc114 must stay green; tc112 (a table dropped after migration 2 is not recreated by `init`) is the regression anchor for the migration-recording rule. `tests/it_store.rs` and the engine suites `tests/it_engine.rs`, `tests/it_history.rs`, `tests/it_nema_task.rs` and `tests/it_agent_folder.rs` open fresh databases through `Store::init` or `Engine::init`, so they also cover the unchanged `init` path. The `ui-prototype` kernel build depends on the signatures frozen by IT-068.
