# Technical Design: `src/doctask_report.rs`

Source: `src/doctask_report.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`doctask_report` is the pure export layer of the document-task module. It turns plain records of the governance store into three texts: the audit report in Markdown, the audit report as JSON, and the `.NEMA` graph of traceability and task dependencies. It performs no I/O, opens no database, reads no environment variable and does not read the clock: the caller supplies the `generated_at` text, the records and the export options. Writing the text to a file (temporary file, rename, refusal to overwrite, the `dt.export.generated` event) belongs to the command layer (WP11). The module decides what may appear in an export; it does not decide where the export is written.

## Dependencies

- `crate::doctask_model` for the record types (`TaskRecord`, `PlanRecord`, `ApprovalRecord`, `EvidenceRecord`, `EventRecord`), the enumerations (`TaskState`, `PlanStatus`, `AnchorState`, `Visibility`, `LinkKind`, `TargetKind`, `EvidenceKind`, `EvidenceResult`, `EvidenceSource`, `ActorKind`, `ApprovalKind`, `ApprovalDecision`, `Risk`, `SourceKind`), `DocError` and `ErrorCode`, `GENESIS_PREV_HASH`, `first_chain_break` and `sanitize_display`.
- `crate::nema` for `parse_graph_edges`, `topo_batches` and `Dependency`. The writer validates its own output by reading it back through these two functions.
- `serde` (derive) and `serde_json` for the JSON form, both already in the root manifest. The module adds no crate.
- The module does not import `crate::store`, `crate::engine`, `crate::doctask_store`, `crate::doctask_service` or `crate::doctask_render`.
- Callers: `doctask_cli` (`doc export report`, `doc export nema`) and the console report screen, if any.

## Public Contract

### Input

`ReportInput` (derives `Default`) holds plain vectors; the caller fills them from the store and may pass them in any order:

| Field | Element | Meaning |
|---|---|---|
| `generated_at` | text | The `--generated-at` value, or the current time when the option is absent. Printed in the Markdown header and the JSON `generated_at` field; never used in the NEMA text. |
| `files` | `ReportFile { id, path, sensitive }` | Tracked files. `sensitive` is true when the external-transfer gate refuses the file. |
| `clauses` | `ReportClause { file_id, section, anchor_id, ordinal, number, title }` | Clause outline rows. `anchor_id` is the anchor id used for NEMA node names. |
| `annotations` | `ReportAnnotation { id, no, file_id, section, text, visibility, author_kind, anchor_state }` | Annotations. |
| `tasks`, `plans`, `approvals`, `evidence` | `TaskRecord`, `PlanRecord`, `ApprovalRecord`, `EvidenceRecord` | Store records of WP02. |
| `plan_stats` | `ReportPlanStat { plan_id, added, removed }` | Line counts of the plan diff, computed by the caller. A plan without a stat shows `未计算`. |
| `links` | `ReportLink { task_id, kind, target_kind, target, target_file_id, label }` | Task links. `target_file_id` is empty when the target is not a file. |
| `dependencies` | `ReportDependency { prerequisite, dependent }` | Task ids; `prerequisite` is the `from` side of the edge. |
| `versions` | `ReportVersion { id, file_id, version_no, content_hash, ledger_event_id, created_at }` | Ledger versions of files. |
| `commits` | `ReportCommit { task_id, plan_id, file_id, version_id, prev_version_id, ledger_event_id }` | Applied plans. `prev_version_id` is empty when the file had no earlier version. |
| `events` | `EventRecord` | The whole governance chain in any order. |

### Options

`Scope` has the variants `All`, `Task(String)` (a task id) and `File { file_id, section: Option<String> }`. `ExportOptions { scope, include_local_only, include_sensitive }` has no default; the caller sets all three.

Scope membership:
- `All`: every task, annotation, clause, link, version and commit.
- `Task(t)`: the task `t`, its clause (the file and section of the task), the annotations on that clause, its links, plans, approvals, evidence and commits, and the versions of its file.
- `File { f, section }`: the tasks of file `f` (restricted to the section when given), the clauses and annotations of the same section filter, the links, plans and so on of those tasks, and the versions of `f`.

A `Task` scope whose id is absent from `tasks` returns `E_NOT_FOUND`. A `Task` scope whose task is hidden (local-only without `include_local_only`, or on a gated file without `include_sensitive`) returns the same `E_NOT_FOUND`. A `File` scope whose file is absent, or is sensitive while `include_sensitive` is false, returns `E_NOT_FOUND` as well. The hint of both names the flag that would make the object visible; the answer does not distinguish the causes, so a hidden object cannot be told from an absent one.

### Visibility

A task or annotation is hidden when:
1. its file is sensitive and `include_sensitive` is false (counted as `gated`), or
2. its visibility is `LocalOnly` and `include_local_only` is false (counted as `local_only`).

Hidden items are omitted from every section, from the NEMA text and from the timeline, together with their plans, approvals, evidence, links, commits, dependencies and timeline events. A hidden item contributes only to the counters of the `omitted` object (`local_only` and `gated`, for tasks and annotations; `dependencies` for dependencies that cannot be shown). Event payloads are never printed, so the event chain cannot leak text that the store kept out of the payloads.

### Cutoff and chain

`cutoff_seq` is the largest `seq` among the events whose kind is not `dt.export.generated`, or 0 when there is none. The chain check runs `first_chain_break` over the events with `seq <= cutoff_seq`. The head hash is the `hash` of the event at `cutoff_seq`, or `GENESIS_PREV_HASH` when there is none. Events with `seq` above the cutoff are not used anywhere in the output. The timeline excludes every `dt.export.generated` event, so an export never changes the text of a later export of the same data.

A dependency is shown when both of its tasks are visible and at least one of them is exported. A dependency whose prerequisite is absent from `tasks`, or hidden, is omitted and counted in `omitted.dependencies`; its handle is never printed. A shown dependency whose other task is visible but outside the scope appears in section 9 by handle and is not counted.

Timeline events are the events with `seq <= cutoff_seq` that refer to a visible item: a visible task (`task_id`), a visible file (`file_id`) with no task, or a file-level event of an in-scope file. Events that refer to a hidden item (through `task_id`, `subject_id` or `file_id`) are omitted. Events without a task and without a file appear only in the `All` scope.

### Output

`render_markdown(input, options) -> Result<String, DocError>`, `render_json(input, options) -> Result<String, DocError>` and `render_nema(input, options) -> Result<String, DocError>`. Each function builds the same selection and returns the text without a trailing newline in Markdown and JSON, and with one trailing newline in NEMA.

**Markdown** has a title, a header block (generated time, cutoff, scope, chain result with the event count and the head hash, counts of omitted items) and these sections in order: 1 summary, 2 clauses and annotations, 3 tasks and timeline, 4 approvals, 5 plans and differences, 6 commits and ledger events, 7 test evidence with its source label, 8 links, 9 dependencies and batches, 10 rollback chain, 11 anomalies, 12 data of unknown origin. Headings and labels are Chinese; identifiers, hashes, codes and kinds are ASCII. An empty section shows `（无）`.

**JSON** is `serde_json::to_string_pretty` of a `Report` value with `schema` set to `ladon.doctask.report.v1`. Its field order follows the struct declarations, so the output is fixed by the code, not by a map iteration order.

### NEMA

The text has two domains, in this order:
- Domain 1, header ` \>1 1`: node rows and the trace edges. Node rows: clause nodes `C-<token>` for clauses that carry a task in the export, task nodes `T-<task_no>` for every exported task, and file nodes `F-<token>` for every file that an edge refers to.
- Domain 2, header ` \>2 1`: only the `Blocks` edges between exported tasks.

Tokens: the anchor or file id with every character outside `[0-9a-fA-F]` removed (so the hyphens of UUIDs drop out), lowercased, then its first 8 characters. Ids whose tokens collide are lengthened by 4 characters at a time (12, 16, and so on up to the full filtered id) until every token of that node kind is distinct. An id that is empty after filtering, or whose filtered form equals another id's, returns `E_INTERNAL`.

Labels: tab, carriage return, line feed, U+3000, U+2028 and U+2029 become spaces; `sanitize_display` is applied; the result is trimmed and cut to 80 characters; an empty result becomes `-`.

Order of the text. Domain 1 node rows: clause nodes in clause order (file path, then `ordinal`), then task nodes in `task_no` order, then file nodes in file path order. Domain 1 edge rows: for each exported task in `task_no` order, its `Traces` edge when its clause has a node, then its link edges sorted by the link kind text and then by target. Domain 2 edge rows: the `Blocks` edges between exported tasks, sorted by `(prerequisite task_no, dependent task_no)`. `E<n>` is numbered from 1 in this order over the whole text.

Edges (`from>E<n>>to label`):
- clause to task: `Traces`;
- task to a file whose link kind is `design` or `detail`: `Refines`; `code`: `Implements`; `ut`, `it` or `st`: `VerifiedBy`; `doc`, `target` or `other`: `Targets`. Only links whose target file is an exported file produce an edge, and a file node is written only when an edge refers to it; links to URLs, commits or tasks appear in the Markdown and JSON only;
- prerequisite to dependent: `Blocks`, domain 2, between exported tasks only.

Read-back validation, before the text is returned: `parse_graph_edges(text)` must succeed; its edge list must equal the emitted list in order (`from`, `edge_id`, `to`, `label`); and `topo_batches` over all node tokens and all parsed edges must succeed. A parse or round-trip failure returns `E_INTERNAL`. The layering of the exported tasks, which is `topo_batches` over the task node tokens and the exported `Blocks` edges, gives the batches of section 9. A cycle among the exported `Blocks` edges returns `E_DEP_CYCLE`, and so does any other failure of that layering.

### Markdown and JSON text

User text is sanitised with `sanitize_display` (control and bidirectional characters become `<U+XXXX>`). Line breaks in single-line contexts become spaces. In Markdown, inline text escapes the characters `\`, `` ` ``, `*`, `[`, `]`, `<`, `>`, `|`, `#` and `~` with a backslash. Annotation text, which may span lines, is printed as a blockquote with one `> ` prefix per line. JSON strings are the sanitised text, encoded by `serde_json`.

### Errors

| Condition | Code |
|---|---|
| Scope task or file absent (or file gated without `include_sensitive`) | `E_NOT_FOUND` |
| Cycle among exported dependencies | `E_DEP_CYCLE` |
| Token collision that cannot be resolved; read-back or topo failure of the NEMA text | `E_INTERNAL` |

Dangling dependencies (a prerequisite absent from the input or hidden) are not errors: the edge is omitted and counted in `omitted.dependencies`.

## Determinism

Every collection is sorted before it is rendered: tasks by `task_no`; annotations by `no`; clauses by file path then `ordinal`; plans by `(task_no, plan_no)`; approvals, evidence and versions by `created_at` then `id`; links by `(task_no, kind text, target)`; dependencies by `(prerequisite task_no, dependent task_no)`; commits by `(task_no, plan_no)`; events by `seq`. Maps are `BTreeMap` or `BTreeSet` only. The output therefore depends on the content of the input and on `generated_at` alone. Two calls with the same input and options return identical bytes, and an export that appends only a `dt.export.generated` event returns the same bytes as before.

## Change Notes

The module is new. It is the only module that writes `.NEMA` text and report text; the command layer calls it and writes the result. `lib.rs` gains one `pub mod doctask_report;` line, placed after the `doctask_render` line. No other module changes. The crate gains no dependency, and `Cargo.toml`, `Cargo.lock` and `ui-prototype/Cargo.lock` stay unchanged.
