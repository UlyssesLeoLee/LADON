# Technical Design: `src/doctask_store.rs`

Source: `src/doctask_store.rs`

Source-SHA256: `486d5dc46289698767fc30200b60e6136fed3949096a4acc4d98ecc476bee5dc`

## Responsibility

`doctask_store` is the persistence layer of the document-task module. It owns every write to the document-task tables created by migration 004 and every read that the verification and backfill paths need. Each write runs in one `BEGIN IMMEDIATE` transaction together with the governance-chain append that records it, so a change and its event are committed together or not at all. The module holds no business rules beyond the stored value domains and the record shapes; permission, transition rules and text validation belong to the service layer, and the pure model supplies the hashes, the state strings and the error codes.

## Dependencies

- `crate::store::{Store, StoreBusy, backoff_ms}`: the database path and `Store::connect` (which retries lock errors on open), the `E_STORE_BUSY` error type and the backoff schedule. `Store::ensure_schema` must have run before any method here is called; this module does not apply migrations.
- `crate::doctask_model`: `event_hash` and `EventFields`, `GENESIS_PREV_HASH`, `first_chain_break` and `EventRecord`, `sha256_hex`, `validate_priority`, the enumerations `TaskState`, `ActorKind`, `ProposalState`, `SourceKind`, `Visibility`, `AnchorState`, and `DocError` with `ErrorCode`.
- `turso` (`Connection`, `Value`, `params!`, `transaction::{Transaction, TransactionBehavior}`, `Error`): statements, rows and transactions. `Transaction::new_unchecked` takes a shared connection reference, so the begin, commit and rollback calls do not hold a mutable borrow.
- `anyhow` for the error type of the public methods; `chrono` for timestamps in RFC 3339; `uuid` (v4) for identifiers; `serde_json` for payload construction and validation; `rand` for lock backoff jitter; `tokio::time::sleep` for backoff. Every crate is already declared by the root crate.
- The standard library for collections.
- The module does not import `crate::engine`, `crate::policy`, `crate::doctask_service` or any later document-task module. The modules that depend on it are `doctask_service` (WP06), `doctask_flow` (WP07) and `doctask_reconcile` (WP08), which use its write methods and verification queries.

## Public Contract

### Constants

`DOC_TABLES` lists the fourteen tables that migration 004 creates: `doc_counters`, `doc_baselines`, `doc_anchors`, `doc_task_meta`, `doc_annotation_meta`, `doc_task_deps`, `doc_task_events`, `doc_task_plans`, `doc_task_approvals`, `doc_task_links`, `doc_task_evidence`, `doc_file_lineage`, `doc_agent_sessions` and `doc_agent_disclosures`. `COUNTERS` lists the counter names `task_no`, `annotation_no` and `backfill_v4`.

Retry policy: one driver, `with_retry`, runs every write and every read. Each attempt opens a new connection and runs one body with its own transaction; a body that fails is rolled back before the error is classified. A write is retried as a whole (new connection, new transaction, same identifiers and timestamps) when the chain append fails with the unique-index violation described below, at most 3 times after the first attempt. A lock error (from `BEGIN IMMEDIATE`, or from a statement inside the transaction) is retried with `backoff_ms` (25 ms doubling to 400 ms, plus 0 to 50 percent jitter) up to 6 attempts in total; the sixth failure returns `StoreBusy`, which displays as `E_STORE_BUSY`. Any other error ends the attempt: the transaction is rolled back explicitly and the error is returned.

### Types

`DocStore` wraps a `Store` and is `Clone`. `DocStore::new(store: Store)` and `DocStore::store(&self) -> &Store`.

`NewEvent` describes one chain event before storage. Its public fields are `task_id`, `file_id`, `subject_kind`, `subject_id`, `kind`, `from_status`, `to_status` (empty strings where the event has no status), `actor`, `actor_kind: ActorKind`, `session`, `claimed_session` and `payload_json`. Every method that appends an event generates the event id (UUID v4) and the timestamp (`Utc::now()` in RFC 3339) once per call, before its first attempt, so a retried attempt stores the same identifier and timestamp.

`NewTask` has `file_id`, `anchor_id`, `title`, `description`, `owner`, `priority: u8`, `proposal_state: ProposalState`, `visibility: Visibility`, `source_kind: SourceKind`, `created_by`, `created_by_kind: ActorKind` and `session`.

`CreatedTask` has `task_id: String`, `task_no: i64` and `event_seq: i64`.

`NewAnnotation` has `file_id`, `anchor_id`, `text`, `author`, `author_kind: ActorKind`, `rationale`, `visibility: Visibility`, `quote`, `line_start: i64`, `line_end: i64` and `session`.

`CreatedAnnotation` has `annotation_id: String`, `annotation_no: i64` and `event_seq: i64`.

`StatusDrift` has `task_id`, `status` (the value of `doc_tasks.status`) and `replayed: Option<String>` (the `to_status` of the last event that carries a status, `None` when there is none).

`BackfillReport` (`Default`) has `anchors_created`, `tasks_imported`, `annotations_imported`, `events_imported`, `status_events_added`, all `i64`.

### Write methods

Every write method below runs in one `BEGIN IMMEDIATE` transaction and commits only when all of its statements and its chain append succeed. Payload objects are built with sorted keys, so the stored JSON text is the same for the same inputs.

- `ensure_anchor(file_id, base_version_id, section_key, state: AnchorState) -> Result<String>`: returns the id of the `doc_anchors` row with that `(file_id, base_version_id, section_key)` triple when one exists, otherwise inserts a row with that state, `current_version_id = base_version_id`, `current_key = section_key`, empty fingerprints, `reviewed_version_id = ''`, and `title` and `level` taken from `parse_anchor_key`. It writes no event. An unknown `state` string cannot occur because the parameter is typed.
- `next_counter(name) -> Result<i64>`: `name` must be one of `COUNTERS`, otherwise `E_INVALID_ARG`. Runs its own transaction and returns the new value.
- `append_event(ev: &NewEvent) -> Result<EventRecord>`: `ev.kind` must be non-empty and `ev.payload_json` must parse as JSON, otherwise `E_INVALID_ARG`. Reads the tail hash (or `GENESIS_PREV_HASH` when the chain is empty), computes the event hash, inserts the row and returns the stored record with its `seq`.
- `create_task(new: &NewTask) -> Result<CreatedTask>`: `validate_priority(new.priority)` (`E_INVALID_ARG`). Inside the transaction: the anchor row must exist (`E_NOT_FOUND`), belong to `new.file_id` (`E_INVALID_ARG`), and the file must exist in `resources` (`E_NOT_FOUND`). Then the next `task_no`, the `doc_tasks` row with status `open`, the `doc_task_meta` row with `source_kind`, `visibility`, `proposal_state`, `owner`, `priority`, `created_by`, `created_by_kind` and `session` as given, and `updated_at` set to the event timestamp. The event is `dt.task.created` with `from_status = ''`, `to_status = 'open'`, `subject_kind = 'task'`, `subject_id = task id`, `task_id = task id`, and payload `{"task_no", "anchor_id", "title_sha256", "title_len", "source_kind", "proposal_state", "visibility"}`. The title appears only as its SHA-256 and its length in Unicode scalar values.
- `add_annotation(new: &NewAnnotation) -> Result<CreatedAnnotation>`: same anchor and file checks as `create_task`. Inserts the `annotations` row (`section_key` and `base_version_id` copied from the anchor, `text` stored as given), the `doc_annotation_meta` row with the next `annotation_no`, and the event `dt.annotation.created` with subject `annotation` and payload `{"annotation_no", "anchor_id", "visibility", "text_sha256", "text_len"}`.
- `transition(ev: &NewEvent) -> Result<EventRecord>`: compare-and-set on the task status. `ev.task_id` identifies the task; `ev.from_status` and `ev.to_status` must parse as `TaskState`, and `ev.kind` and `ev.payload_json` are validated as in `append_event` (`E_INVALID_ARG` otherwise). Inside the transaction: read the task row (`E_NOT_FOUND` when absent) and require its status to equal `from_status` (`E_CONFLICT` otherwise); run `UPDATE doc_tasks SET status = to WHERE id = ? AND status = from` and require exactly one changed row (`E_CONFLICT` otherwise); read the status back and require `to` (`E_CONFLICT` otherwise); set `doc_task_meta.updated_at` to the event timestamp; append the event with `file_id` taken from the task row. The caller supplies the kind (for example `dt.task.transitioned`) and the payload JSON. Two concurrent calls from the same status yield one success and one `E_CONFLICT`.
- `insert_row(table, values: &[(&str, Value)]) -> Result<()>`: `table` must be in `DOC_TABLES`, `values` must be non-empty, and every column name must match `^[a-z][a-z0-9_]*$`; otherwise `E_INVALID_ARG`. Executes one `INSERT INTO table (columns) VALUES (?1, …)` in a transaction with the same retry policy. It writes no event; it is the generic row writer for the tables whose behaviour belongs to later packages.
- `backfill() -> Result<BackfillReport>`: one transaction, idempotent. Steps in order:
  1. Anchors: for each distinct `(file_id, base_version_id, anchor_section)` of `doc_tasks` and `(file_id, base_version_id, section_key)` of `annotations` (collected in `BTreeSet` order), call the `ensure_anchor` logic with state `unchecked`. Each newly created anchor counts in `anchors_created`.
  2. Legacy tasks: every `doc_tasks` row with no `doc_task_meta` row (`NOT EXISTS`), ordered by `(created_at, id)`, receives the next `task_no` and a meta row with `anchor_id` from step 1, `owner = ''`, `priority = 2`, `proposal_state = 'accepted'`, `proposal_decided_by = ''`, `proposal_decided_at = NULL`, `visibility = 'local_only'`, `source_kind = 'legacy'`, `source_ref = ''`, `created_by = 'human'`, `created_by_kind = 'unknown'`, `session = ''`, `last_approval_id = ''`, `cancel_reason = ''`, `updated_at` set to the import time and `closed_at = NULL`. Its status in `doc_tasks` is not changed.
  3. Legacy annotations: every `annotations` row with no `doc_annotation_meta` row (`NOT EXISTS`), ordered by `(created_at, id)`, receives the next `annotation_no` and a meta row with `author = 'human'`, `author_kind = 'unknown'`, `rationale = ''`, `visibility = 'local_only'`, `quote = ''`, `line_start = 0`, `line_end = 0`, `session = ''` and the original `created_at`.
  4. Governance events: every `governance_events` row (ordered by `seq`) whose id `legacy:<id>` is not yet a `doc_task_events.id` is appended, with id `legacy:<original id>`, timestamp equal to the original timestamp, `kind = 'legacy.' + original kind`, `subject_kind` equal to the part of the original kind before its first `.`, `subject_id` equal to the original subject id, `task_id` equal to the subject id when the original kind starts with `task.` and empty otherwise, `file_id` from the row, `actor` from the row, `actor_kind = 'unknown'`, empty session fields, `to_status` as given by `legacy_payload`, and the payload given by `legacy_payload`.
  5. Status: for every task whose meta row has `source_kind = 'legacy'`, a `dt.task.imported` event (`subject_kind = 'task'`, `subject_id = task_id = ` the task id, `file_id` from the task row, `from_status = ''`, `actor = 'ladon'`, `actor_kind = 'unknown'`, `to_status = doc_tasks.status`, the import time as timestamp, payload `{"source":"legacy","status":"<status>"}`) is appended when the task has no `dt.task.imported` event yet, or when the `to_status` of its last event carrying a status differs from `doc_tasks.status`. The second case covers a status changed by an earlier binary that wrote no event.
  6. The counter `backfill_v4` is incremented once per run.
  
  Running it again with no new legacy rows and no status drift appends no event and changes no row except the counter. Event hashing and ordering are deterministic for the same inputs; generated identifiers differ between runs only where rows are created.

### Read and verification methods

The methods of this section write nothing. They retry lock errors with the same schedule as the write methods.

- `events() -> Result<Vec<EventRecord>>`: every row of `doc_task_events` in `seq` order.
- `verify_chain() -> Result<Option<i64>>`: `first_chain_break(&events)`. Returns the `seq` of the first record whose `prev_hash` differs from the preceding hash (the genesis value for the first record) or whose stored `hash` differs from its recomputed hash; `None` for an intact or empty chain.
- `status_drift() -> Result<Vec<StatusDrift>>`: for every `doc_tasks` row (ordered by `id`), the `to_status` of the last event with a non-empty `to_status` for that task. Returns the tasks whose replayed status is `None` or differs from `doc_tasks.status`.
- `dependency_edges() -> Result<Vec<(String, String)>>`: `(task_id, depends_on)` from `doc_task_deps`, ordered by both columns.
- `duplicate_baselines() -> Result<Vec<(String, String, i64)>>`: `(file_id, version_id, count)` for every pair with more than one `doc_baselines` row whose `revoked_at` is `NULL`.

### Helpers with crate visibility

- `parse_anchor_key(key) -> (i64, String)`: `ROOT` gives `(0, "（全文）")`. A key of the form `H<digits>:<title>#<digits>` gives the digits as the level and the title between the first `:` after the level and the last `#`. Any other key gives `(0, "")`.
- `legacy_payload(kind, payload) -> (String, Option<String>)`: for `kind = "task.state_changed"` whose payload is exactly one of the seven state strings, returns `{"legacy":true,"payload":"<state>"}` and `Some(state)`. For every other kind and payload, returns `{"bytes":<n>,"legacy":true,"sha256":"<hex>"}` where `n` is the byte length and `sha256` the digest of the original payload text, and `None`.
- `is_chain_conflict(err: &anyhow::Error) -> bool`: true when the error is a `turso::Error::Constraint` whose text contains `UNIQUE constraint failed` and `doc_task_events.prev_hash`.
- `is_lock_error(err: &turso::Error) -> bool`: true for `Busy`, `BusySnapshot`, and any error whose text contains `locked by another process`.
- `is_doc_table(name) -> bool` and `is_identifier(name) -> bool`: the membership and syntax checks used by `insert_row`.

### Schema assumptions

The module depends on migration 004 as written in `sql/004_document_tasks.sql`, on the unique index `idx_doc_task_events_prev` (which makes two events with the same `prev_hash` impossible), and on the `UNIQUE` constraint of `doc_task_meta.task_no`. Foreign keys are enabled by the store's opener; the module does not rely on foreign-key errors for its own rule checks, because each rule is checked by an explicit query before the write.

## Change Notes

The module adds no dependency and changes no existing table. `lib.rs` gains one `pub mod doctask_store;` line. `store.rs` is used only through its existing public and crate-visible items; the lock-error predicate is repeated here because the store keeps its own private. Legacy rows are never modified by the backfill: it reads `doc_tasks`, `annotations` and `governance_events`, and writes only the new tables and the chain. Verification of the chain relies on the pure `first_chain_break` in the model, so the rule is defined in one place.
