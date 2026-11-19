# Technical Design: `src/store.rs`

Source: `src/store.rs`

Source-SHA256: `a36b22395a6703671e5d6be00f21435b92c8ce85f238e8d68dc614a4a3d92bee`

## Responsibility

`Store` is the persistence layer of the workspace ledger. It owns one local Turso database file (`.ladon/meta.turso`) and exposes async operations for: schema initialisation from `sql/001_initial.sql` and `sql/002_collaboration_shell_agent.sql`; resources (one row per tracked relative path); immutable versions; an append-only per-file event ledger protected by a SHA-256 hash chain; archive object records per provider; file tags; document annotations; document tasks; and governance events.

Every write path that touches more than one table (`append_version`, `mark_deleted`, `set_tags`) runs inside a single transaction and commits only after all statements succeed. Each public method opens its own connection with `PRAGMA foreign_keys = ON`.

## Dependencies

- `turso` pinned to `=0.8.3` with `default-features = false` and `features = ["mimalloc"]`. The `fts` feature is not enabled: this module issues no full-text queries, and the 0.8.3 `fts` sources need nightly-only allocator APIs that a stable toolchain cannot compile. `mimalloc` stays enabled as the default allocator.
- `chrono` (UTC RFC 3339 timestamps), `uuid` v4 (record ids), `serde_json` (changed-section lists, hash-chain payload, JSON views), `anyhow` (errors).
- `crate::crypto::digest` for the hash chain, `crate::model` record types (`FileRecord`, `VersionRecord`, `EventRecord`, `StorageObject`).
- `sql/001_initial.sql` and `sql/002_collaboration_shell_agent.sql`, embedded with `include_str!` and executed statement by statement.
- Downstream callers: `engine::Engine` (capture, scan, archive, restore), `agent_api::AgentApi` (history, folder listing), the `ladon` CLI.

## Public Contract

Parameter binding: every value passed to `params!` or a positional array is a `&str`, an owned `String`, or an `i64`. Borrowed `&String` values are converted with `.as_str()` before binding, because the turso `IntoValue` trait accepts types implementing `TryInto<Value>` and `&String` is not one of them. Booleans are stored as `i64` 0 or 1. Timestamps are `Utc::now().to_rfc3339()` strings. Values are always bound, never interpolated, so quotes, newlines, Unicode and SQL metacharacters round-trip verbatim.

Operations:

- `Store::new(path)`; `init()` creates the parent directory, applies migration 1 and, when `schema_migrations` has no row for version 2, migration 2. Calling `init` repeatedly is safe.
- `find_path(path) -> Option<FileRecord>`, `list_files()` ordered by relative path, `latest_version(file_id)`, `version(version_id)`, `versions(file_id)` newest first, `events(file_id)` in ledger order. Unknown keys yield `None` or empty lists, never errors.
- `append_version(PendingVersion) -> (file_id, version_id, event_id)`: creates the resource on first sight (version 1, event kind `created`), otherwise increments `version_no`. The event kind is `restored` when the previous row was deleted and `modified` otherwise. Returns the error `no content change` when the hash equals the stored hash of a non-deleted resource. The new event stores `ledger_prev_hash` (hash of the file's previous event, empty for the first) and `ledger_hash = event_hash(...)`.
- `mark_deleted(file, actor, reason)`: no-op when `file.deleted` is already true; otherwise sets `deleted = 1` and appends a `deleted` event bound to the latest version, extending the chain. Fails with `missing version` when the resource has no version.
- `archive_record(StorageObject)` upserts on `(content_hash, provider)`; `archives(hash)` lists records newest `verified_at` first; `is_hash_sensitive(hash)` is true when any version with that content hash is flagged sensitive.
- `set_tags(file_id, tags, source, rule_version)` trims each tag, skips empty tags and tags longer than 80 bytes, and upserts on `(file_id, tag)`; `tags(file_id)` returns tags sorted ascending.
- `add_annotation`, `create_task` (status `open`), `add_governance_event`, `governance_events` (ledger order), `list_tasks` (by `created_at`), `set_task_status` which accepts only `open`, `analysis`, `approval`, `working`, `verification`, `done`, `cancelled` (error `invalid task state` otherwise, error `task missing` for an unknown id) and records a `task.state_changed` governance event.
- `verify_chain(file_id) -> bool` recomputes every event hash in order and returns false at the first broken `ledger_prev_hash` link or hash mismatch.

Invariants: `event_hash` is SHA-256 over the JSON serialisation of the 12-tuple `(prev, event id, file id, version id, kind, old path, new path, timestamp, actor, reason, previous hash, current hash)`. Version numbers per file are contiguous starting at 1.

## Change Notes

All `params!` call sites in `append_version`, `mark_deleted`, `archive_record`, `set_tags`, `add_annotation`, `create_task`, `add_governance_event` and `set_task_status` bind `&str` or `i64` values only, which makes the module compile and behave correctly against turso 0.8.3. Query text, schema, transactions and the hash-chain format are unchanged, so existing `.ladon/meta.turso` files stay readable and verifiable. The Cargo manifest carries the dependency feature set described under Dependencies; no other module depends on turso directly.
