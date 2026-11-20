# Technical Design: `src/engine.rs`

Source: `src/engine.rs`

Source-SHA256: `7753a8573b635cacb21549e2ca9c28c613ed68c7c350d13bf375b21e5ee7959d`

## Responsibility

`Engine` is the orchestration layer for one workspace root. It combines the metadata `Store`, the content-addressed `CasStore` under `.ladon/cas`, and the `.LADONignore` policy. It turns file-system observations into ledger events: capturing a file as a new version, scanning a whole tree (including detecting deletions), archiving a version to an external provider as an encrypted object, loading historical bytes from the local CAS or from a provider, and restoring a historical version to the live path with rollback on failure.

## Dependencies

- `store::Store` and `PendingVersion` (ledger persistence), `archive::CasStore` and `assert_under_workspace` (content store and path containment), `crypto::{digest, load_archive_key, seal, open}` (SHA-256, AES-GCM archive envelope), `diff::changed_sections` (section-level change detection), `policy::LadonPolicy` (ignore rules), `provider::StorageProvider` (external archive transport), `model` records.
- `walkdir` for scans, `chrono` for archive verification timestamps, `uuid` for temporary and backup file names, `anyhow` for errors, `tokio` for the async runtime that callers provide.
- `config/examples/.LADONignore` embedded with `include_str!` as the default rule file. The optional environment variable `LADON_GLOBAL_IGNORE` adds a global rule file to the policy.
- Callers: the `ladon` CLI, `agent_api::AgentApi`, integration tests.

## Public Contract

- `Engine::init(path)` canonicalises the root, creates `.ladon`, initialises the database, writes the default `.LADONignore` when absent and loads the policy. `Engine::open(path)` requires an existing `.ladon/meta.turso` and fails with the error ``workspace not initialized; run `ladon init` `` otherwise.
- `reload_policy()` re-reads all `.LADONignore` files. On failure it marks the policy invalid, which blocks every archive transfer until a later reload succeeds. Captures use the policy loaded at the last init, open or successful reload.
- `resolve(path)` accepts absolute or workspace-relative paths, rejects paths outside the root (error `path escapes the workspace`), a symbolic link as the path itself and non-regular files, and rejects anything under `.ladon` or `.git` (error `internal file cannot be tracked`). It returns the canonical path and the forward-slash relative path.
- `capture(path, actor, reason) -> CaptureResult`: hashes the file into the CAS. When the hash equals the hash of a record that is not marked deleted, the result has status `unchanged` and no version. Otherwise it appends a version with `changed_sections`: `DIFF_DEFERRED_PREVIOUS_COLD` when the previous content is not in the local CAS, `LARGE_BINARY_OR_TEXT` for files above 8 MiB, an empty list when bytes are identical to the previous content, else the structured diff. `sensitive` reflects the policy decision at capture time, also for unchanged files.
- `scan(actor)` captures every regular file under the root except entries named `.ladon` or `.git` at any depth (skipped with their contents) and the root-level `.LADONignore`; a `.LADONignore` in a subdirectory is captured and flagged sensitive. It then marks ledger entries whose files are no longer present as deleted (reason `missing during full scan`).
- `file_by_path(path)` normalises backslashes and returns the ledger record or the error `file not found in ledger`.
- `archive_version(version_id, provider, evict_local) -> StorageObject`: refuses when the policy is invalid, when the policy forbids external transfer for the path (`LADON policy denies external data transfer`), or when the version or any version with the same content hash is sensitive (`sensitive version blocked from external storage`). It requires `LADON_ARCHIVE_KEY_HEX` (64 hex digits; errors name the missing variable, `64 hex digits` or `exactly 32 bytes`). If the provider already holds the object, it downloads and authenticates it, optionally evicts the local copy, and returns the stored record without writing a new one. Otherwise it seals the local bytes, uploads, reads back and authenticates the object, records it, and optionally evicts the local copy. The local copy is evicted only after the remote object authenticated; an object that fails authentication yields `archive authentication failed` and keeps the local copy. A first archive needs the local copy (`local copy unavailable for first archive`).
- `load_version(version_id, provider)`: returns bytes from the local CAS when present; otherwise needs a provider that holds an archive record for the content hash, downloads, decrypts and verifies the hash. Errors: `unknown version`, `historical content is cold; supply matching provider`, `no archived location found`.
- `restore_plan(version_id) -> String` returns a multi-line description beginning `RESTORE PLAN: {file id} version {number} ({content hash}) -> {relative path}` followed by `Target exists: true|false` and the approval requirement. It reads the ledger and checks whether the target exists, and changes nothing.
- `restore(version_id, approve, provider)`: requires `approve`; refuses when the live file's hash differs from the ledger (`live file changed outside ledger; scan before restore`); writes the historical bytes to a temporary file, swaps it into place while keeping a backup, captures the result through `capture` as actor `restore` (a new version with event kind `restored` and status `captured` when the record is marked deleted, a new version with event kind `modified` otherwise when the content differs from the record, and status `unchanged` with no new version when the restored bytes equal the hash of a record that is not marked deleted), deletes the backup on success and restores the previous file when the capture fails.

## Change Notes

`restore_plan` reads the file's path from the ledger record and checks existence on `self.root.join(&file.relative_path)`, which borrows the record and does not consume it. Archive eviction follows successful authentication of the remote object, and ledger semantics, error messages and the CLI-facing text are those specified under Public Contract.
