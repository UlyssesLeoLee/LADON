# Technical Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `38e8d8ba9b758f6b4274b945aafe34526eada7f133041e38a9a1edd1a431a7e3`

## Responsibility

`Store` is the persistence layer of the workspace ledger. It operates on one local Turso database file, passed to `Store::new` (the engine passes `.ladon/meta.turso`), and exposes async operations for: schema initialisation from `sql/001_initial.sql` and `sql/002_collaboration_shell_agent.sql` (`init`); the document-task schema from `sql/004_document_tasks.sql` (`ensure_schema`); resources (one row per tracked relative path); immutable versions; an append-only per-file event ledger protected by a SHA-256 hash chain; archive object records per provider; file tags; document annotations; document tasks; and governance events.

Atomicity: `append_version` writes resources, versions and events inside one transaction, `mark_deleted` writes its event and the resource flag inside one transaction, and `set_tags` upserts all of its tags inside one transaction; each commits only after every statement succeeded. `add_annotation`, `create_task` and `set_task_status` perform two separate writes on separate connections (the record, then a governance event) and are not atomic as a pair. Each public method opens its own connection with `PRAGMA foreign_keys = ON`.

## Dependencies

- `turso` pinned to `=0.8.3` with `default-features = false` and `features = ["mimalloc"]`. The crate's default features are `mimalloc` and `fts`; the `fts` feature is not enabled, because this module issues no full-text queries. `mimalloc` stays enabled as the default allocator.
- `chrono` (UTC RFC 3339 timestamps), `uuid` v4 (record ids), `serde_json` (changed-section lists, hash-chain payload, JSON views), `anyhow` (errors), `async-trait` (the `Opener` trait), `rand` (retry jitter), `tokio::time` (retry backoff), `tempfile` in unit tests only.
- `crate::crypto::digest` for the hash chain, `crate::model` record types (`FileRecord`, `VersionRecord`, `EventRecord`, `StorageObject`).
- `sql/001_initial.sql`, `sql/002_collaboration_shell_agent.sql` and `sql/004_document_tasks.sql`, embedded with `include_str!` and executed statement by statement.
- Downstream callers: `engine::Engine` (capture, scan, archive_version, restore; `content_is_excluded` and `paths_for_hash`; opens `.ladon/meta.turso`), `agent_api::AgentApi` (history, folder listing, `allow_remote_read`), the `ladon` CLI (`tag-folder`), the document-task CLI commands in `src/doctask_cli.rs` (`ensure_schema` with the accept flag, `file_by_id`), the document-task modules `src/doctask_service.rs`, `src/doctask_flow.rs` and `src/doctask_reconcile.rs` (`ensure_schema(false)`, `file_by_id`), `src/doctask_mcp.rs` (downcasts `StoreBusy`), `src/doctask_store.rs` (uses `Store`, `StoreBusy` and `backoff_ms`), and the Slint GUI crate in `ui-prototype` (`ui-prototype/src/app.rs`), which calls `list_files`, `mark_deleted`, `version`, `find_path`, `versions`, `verify_chain` and `events` through `engine.store`.

## Public Contract

Parameter binding: every value passed to `params!` or a positional array is a `&str` or an `i64`. Each `String` value is converted with `.as_str()` before binding, and a borrowed `&String` is never bound directly, because the turso `IntoValue` trait accepts types implementing `TryInto<Value>` and `&String` is not one of them. Booleans are stored as `i64` 0 or 1. The columns `updated_at`, `created_at` and `timestamp` are bound as `Utc::now().to_rfc3339()` strings; `verified_at` is bound as supplied by the caller (the engine supplies an RFC 3339 string); `schema_migrations.applied_at` is written by the SQL function `datetime('now')`. Values are always bound, never interpolated, so quotes, newlines, Unicode and SQL metacharacters round-trip verbatim. The only interpolated SQL fragment is the table name in `table_counts`, taken from the `COUNTED_TABLES` constant.

Operations:

- `Store::new(path)`; `init()` creates the parent directory, applies migration 1 and, when `schema_migrations` has no row for version 2, migration 2. Calling `init` repeatedly is safe, and migration 2 is applied once. `init` never applies migration 4.
- `ensure_schema(accept_unverified_backup: bool) -> Result<SchemaOutcome>` is the only path that applies migration 4. It first runs `init`, then reads whether `schema_migrations` records version 4. A recorded version returns `SchemaOutcome::Current` at once. Otherwise:
  1. The row counts of `resources`, `versions`, `doc_tasks`, `annotations` and `governance_events` are read on the open connection. When their sum is zero the library is new: no backup is made and the backup state is `NotNeeded`.
  2. Otherwise the migration backup (next section) runs while the connection stays open. Its state is `Verified { path }` or `Unverified { reason }`.
  3. An `Unverified` state without `accept_unverified_backup` returns `MigrationBackupUnverified` (`E_MIGRATION_BACKUP`) before any migration statement runs and before any event is written. The backup copies written in step 2 remain in `backup/`.
  4. Each statement of `sql/004_document_tasks.sql` is executed in order, each committed on its own. Every statement uses `IF NOT EXISTS`, so a rerun after a failed run creates only the objects still missing.
  5. When no `dt.migration.applied` event exists yet, one is appended to `doc_task_events` (see the event section).
  6. `INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (4, datetime('now'))` runs last.

  The result is `SchemaOutcome::Applied { backup }`. Migration 4 creates only the 14 tables and 16 indexes listed in Migration 4 Schema; it contains no comment, trigger, `ALTER`, `CHECK` constraint or partial index.
- Migration backup: the backup directory is `backup` beside the database file, created when missing. The main file is copied to `backup/meta.turso.pre-004`, or, when a file of that name already exists, to the first free name `backup/meta.turso.pre-004-<n>` with n from 2, so an earlier backup is never overwritten. When `meta.turso-wal` exists it is copied to the log of the same pair (`meta.turso.pre-004-wal`, or `meta.turso.pre-004-<n>-wal`); when it does not exist, a stale copy of that log is removed, so the pair always matches the source. The copy is opened with `TursoOpener` through `open_with_retry`, and the five row counts are compared with the source counts. A copy error, an open error, a read error or a count mismatch yields `Unverified` with a reason text; the reason names the failing step and, for a mismatch, the table and both counts.
- `find_path(path) -> Result<Option<FileRecord>>`; `file_by_id(file_id) -> Result<Option<FileRecord>>` (the same columns as `find_path`, keyed by `resources.id`); `list_files()` ordered by relative path; `latest_version(file_id)`; `version(version_id)`; `versions(file_id)` newest first; `events(file_id)` in ledger order. Unknown keys yield `None` or empty lists, never errors.
- `append_version(PendingVersion) -> Result<(file_id, version_id, event_id)>`: creates the resource on first sight (version 1, event kind `created`), otherwise increments `version_no`. The event kind is `restored` when the previous row was deleted and `modified` otherwise. Returns the error `no content change` when the hash equals the stored hash of a non-deleted resource. The new event stores `ledger_prev_hash` (hash of the file's previous event, empty for the first) and `ledger_hash = event_hash(...)`.
- `mark_deleted(file, actor, reason)`: no-op when `file.deleted` is already true; otherwise sets `deleted = 1` and appends a `deleted` event bound to the latest version, extending the chain. Fails with `missing version` when the resource has no version.
- `archive_record(StorageObject)` upserts on `(content_hash, provider)`, replacing the locator, the encrypted flag and `verified_at`; `archives(hash)` lists records newest `verified_at` first; `is_hash_sensitive(hash)` is true when any version with that content hash is flagged sensitive. `paths_for_hash(hash)` returns the distinct `relative_path` values of every resource that has a version with that content hash, live or deleted, sorted ascending; `Engine::content_is_excluded` applies the current rules to each of them, and `Engine::archive_version`, `AgentApi::allow_remote_read` and the CLI `tag-folder` command reach them through that function. Known limitation: `LadonPolicy::excluded` does not normalise `..` components, so a path that contains a `..` component is not rejected by its outside-workspace check, whether the path is relative or an absolute path under the root.
- `set_tags(file_id, tags, source, rule_version)` trims each tag, skips tags that are empty after trimming and tags longer than 80 bytes, and upserts on `(file_id, tag)`, replacing `source` and `rule_version` of an existing tag; `tags(file_id)` returns tags sorted ascending.
- `add_annotation` and `create_task` (status `open`) each insert their record and then append a governance event (`annotation.created` or `task.created`, actor `human`). `add_governance_event(file_id, kind, subject_id, payload, actor)` returns the new event id. `governance_events(file_id)` returns the events in ledger order and `list_tasks(file_id)` returns the tasks ordered by `created_at`; both return JSON objects, with the keys `id`, `kind`, `subject_id`, `payload`, `actor` and `timestamp` for events, and `id`, `section`, `base_version_id`, `title`, `status`, `description` and `created_at` for tasks. `set_task_status` accepts only `open`, `analysis`, `approval`, `working`, `verification`, `done`, `cancelled` (error `invalid task state` otherwise, error `task missing` for an unknown id) and records a `task.state_changed` governance event.
- `verify_chain(file_id) -> Result<bool>` recomputes every event hash in order and returns false at the first broken `ledger_prev_hash` link or hash mismatch.

Errors with a stable identity implement `std::error::Error` and are recovered with `downcast_ref`:

- `StoreBusy { attempts, detail }`, displayed as `E_STORE_BUSY: ...`, returned by the connection retry when every attempt failed with a lock error; `detail` is the text of the last lock error.
- `MigrationBackupUnverified { reason }`, displayed as `E_MIGRATION_BACKUP: ...`, returned by `ensure_schema` when the backup is unverified and not accepted.

Invariants: `event_hash` is SHA-256 over the JSON serialisation of the 12-tuple `(prev, event id, file id, version id, kind, old path, new path, timestamp, actor, reason, previous hash, current hash)`. Version numbers per file are contiguous starting at 1.

## Connection and Retry

`connect()` is `pub(crate)` and calls `connect_via(&TursoOpener)`. `connect_via(opener)`, also `pub(crate)`, calls the free function `open_with_retry(opener, path)`:

- `Opener` is an `async-trait` trait with one method, `open(path) -> Result<turso::Connection, turso::Error>`. `TursoOpener` builds a local database, connects and runs `PRAGMA foreign_keys = ON`.
- A lock error is `turso::Error::Busy(_)`, or any error whose text contains `locked by another process`. Every other error is returned at once without retry.
- At most 6 attempts are made. Before attempt `k + 1` (k from 1 to 5) the task sleeps `backoff_ms(k, jitter)`: `min(25 * 2^(k-1), 400)` milliseconds plus a jitter of 0 to 50 percent of that value, drawn from `rand` in permille (0 to 500).
- When all 6 attempts fail with a lock error, the result is `StoreBusy { attempts: 6 }`.

Every public `Store` method except the constructor `new` opens its connection through `connect()`, directly or through another method, so the retry applies to all of them. The backup copy check in `ensure_schema` calls `open_with_retry` with `TursoOpener`, so the same retry applies to it.

## Migration 4 Schema

`sql/004_document_tasks.sql` contains 30 statements separated by `;`, each `CREATE TABLE IF NOT EXISTS` or `CREATE [UNIQUE] INDEX IF NOT EXISTS`: 14 tables and 16 indexes. It inserts no rows; the rows that `ensure_schema` writes are the `dt.migration.applied` event and the `schema_migrations` row for version 4. The five tables counted by the backup check (`resources`, `versions`, `doc_tasks`, `annotations`, `governance_events`) are created by migration 1 (`sql/001_initial.sql`); migration 4 adds indexes on `doc_tasks` and `annotations` only.

| Table | Key columns and constraints | Referenced by (`src/`) |
| --- | --- | --- |
| `doc_counters` | `name` PRIMARY KEY; `value` | `doctask_store.rs` |
| `doc_baselines` | `id` PRIMARY KEY; `file_id` and `version_id` reference `resources` and `versions`; `approver`, `approver_kind`, `created_at`, `revoked_at`, `revoked_by` | `doctask_service.rs`, `doctask_store.rs` |
| `doc_anchors` | `id` PRIMARY KEY; `file_id` references `resources`; `UNIQUE (file_id, base_version_id, section_key)`; `state`, `current_version_id`, `reviewed_version_id` | `doctask_reconcile.rs`, `doctask_store.rs` |
| `doc_task_meta` | `task_id` PRIMARY KEY references `doc_tasks`; `task_no` UNIQUE; `anchor_id` references `doc_anchors`; `proposal_state`, `visibility`, `created_by`, `closed_at` | `doctask_flow.rs`, `doctask_reconcile.rs`, `doctask_service.rs`, `doctask_store.rs` |
| `doc_annotation_meta` | `annotation_id` PRIMARY KEY references `annotations`; `annotation_no` UNIQUE; `anchor_id` references `doc_anchors`; `author`, `author_kind`, `quote`, `line_start`, `line_end` | `doctask_service.rs`, `doctask_store.rs` |
| `doc_task_deps` | `(task_id, depends_on)` PRIMARY KEY; both columns reference `doc_tasks` | `doctask_flow.rs`, `doctask_store.rs` |
| `doc_task_events` | `seq` INTEGER PRIMARY KEY AUTOINCREMENT; `id` UNIQUE; `prev_hash` and `hash` (hash chain); `payload_json` | `store.rs`, `doctask_model.rs`, `doctask_store.rs` |
| `doc_task_plans` | `id` PRIMARY KEY; `task_id` references `doc_tasks`; `UNIQUE (task_id, plan_no)`; `plan_hash`, `status`, `expected_version_id`, `expected_hash` | `doctask_flow.rs`, `doctask_store.rs` |
| `doc_task_approvals` | `id` PRIMARY KEY; `task_id` references `doc_tasks`; `kind`, `decision`, `subject_hash`, `base_version_id`, `event_id` | `doctask_flow.rs`, `doctask_store.rs` |
| `doc_task_links` | `id` PRIMARY KEY; `task_id` references `doc_tasks`; `link_kind`, `target_kind`, `target_file_id`, `target_section`, `removed_at` | `doctask_flow.rs`, `doctask_store.rs` |
| `doc_task_evidence` | `id` PRIMARY KEY; `task_id` references `doc_tasks`; `kind`, `result`, `source`, `content_hash`, `event_id` | `doctask_flow.rs`, `doctask_store.rs` |
| `doc_file_lineage` | `id` PRIMARY KEY; `old_file_id` and `new_file_id` reference `resources`; `UNIQUE (old_file_id, new_file_id)`; `score_permille`, `state` | `doctask_reconcile.rs`, `doctask_store.rs` |
| `doc_agent_sessions` | `launch_id` PRIMARY KEY; `hermes_slug`, `hermes_exe`, `focus_task_no`, `started_at`, `ended_at`, `exit_code`, `policy_fp` | `doctask_store.rs` |
| `doc_agent_disclosures` | `PRIMARY KEY (launch_id, file_id, version_id)`; `launch_id` references `doc_agent_sessions`; `file_id` references `resources` | `doctask_mcp.rs`, `doctask_store.rs` |

Indexes created by migration 4 (16):

- On tables created by migration 4: `idx_doc_baselines_file` (`file_id`, `created_at`), `idx_doc_baselines_version` (`file_id`, `version_id`), `idx_doc_anchors_file` (`file_id`, `state`), `idx_doc_task_meta_anchor` (`anchor_id`), `idx_doc_task_meta_proposal` (`proposal_state`), `idx_doc_annotation_meta_anchor` (`anchor_id`), `idx_doc_task_deps_rev` (`depends_on`), `idx_doc_task_events_task` (`task_id`, `seq`), `idx_doc_task_events_file` (`file_id`, `seq`), `idx_doc_task_events_prev` (`prev_hash`, UNIQUE), `idx_doc_task_approvals_task` (`task_id`, `created_at`), `idx_doc_task_links_task` (`task_id`), `idx_doc_task_evidence_task` (`task_id`, `created_at`).
- On tables created by migration 1: `idx_doc_tasks_file` (`file_id`, `status`) and `idx_doc_tasks_status` (`status`) on `doc_tasks`; `idx_annotations_file` (`file_id`, `section_key`) on `annotations`.

## Document-Task Event Written by Migration 4

The event is a row of `doc_task_events` (table of migration 4) with: `id` a new UUID v4; `task_id` and `file_id` empty; `subject_kind` `schema`; `subject_id` `4`; `kind` `dt.migration.applied`; `from_status` and `to_status` empty; `actor` `ladon`; `actor_kind` `system`; `session` and `claimed_session` empty; `payload_json` the JSON text below; `timestamp` the RFC 3339 time; `prev_hash` the hash of the table's last row, or 64 zeros when the table is empty; and `hash` the SHA-256 of `serde_json::to_vec` of the 15-tuple `(prev_hash, id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp)`, every value a string.

The payload object has the keys `version` (4), `backup` (`verified`, `not_needed` or `unverified`), `backup_verified` (false only for `unverified`), `backup_reason` (the reason text, or null) and `row_counts` (the five table counts before migration).

## Change Notes

Every `params!` call in `src/store.rs` binds only `&str` or `i64` values, the types turso 0.8.3 accepts. Within `src/`, the `turso` crate is used directly by `store.rs` and by `doctask_store.rs`, `doctask_flow.rs`, `doctask_mcp.rs`, `doctask_reconcile.rs` and `doctask_service.rs`. The integration tests under `tests/` also call the `turso` API directly; `tests/it_store.rs` uses `turso::Builder` and `turso::params!`.
