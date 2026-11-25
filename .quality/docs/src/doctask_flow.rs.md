# Technical Design: `src/doctask_flow.rs`

Source: `src/doctask_flow.rs`

Source-SHA256: `3ab7bdfb81393b6dffff8ca30d123e56ce04dfa13e4a219598f7dcb3a5b21f70`

## Responsibility

`doctask_flow` is the workflow layer of the document-task module. It moves a task through the state machine of design section 4.2 (T1 to T11), records the approval and rejection of plans, adopts and dismisses agent proposals, manages dependencies, links, evidence, the agent quotas, the owner inbox, the parallel schedule and the verification of the governance chain.

Every state change is one call to `DocStore::commit_step`: the compare-and-set of the task status, the row writes and the governance events are applied in one `BEGIN IMMEDIATE` transaction. Event ids, and the ids of rows that refer to them, are chosen before the transaction starts, so a retried transaction writes the same rows. A refused operation writes nothing.

The module decides what may run. Principal checks use `doctask_model::authorize` for the operations of the authorization table and `doctask_model::lookup_transition` for transitions. Guards are evaluated with `doctask_model::check_guards` on a `GuardInput` filled from the stored rows. The module holds no SQL: every table access goes through `DocStore`. It writes no file: the controlled write of an edit plan belongs to the plan module (WP14), reached through `EditPlanSupport`.

`approve` runs its guards on the stored state first, so a stale plan still takes the stale path of guard 4 (T11) exactly as before. When those guards pass, the flow reconciles the anchor of the task with `doctask_reconcile::reconcile_decision` (System actor, one commit per changed anchor, design 5.5 item 3), reloads its snapshot and evaluates the guards again on the reconciled state, so an anchor whose file changed since the plan was shown is refused with `E_ANCHOR_BROKEN` until the owner acknowledges or relocates it. `close` reconciles the anchor before its guards, after the principal, state and adoption checks. A refused transition on the principal or state checks writes nothing.

Not covered here: anchor acknowledgement, relocation and rebase (WP08), the byte-level splice and the edit component itself (WP14), the command-line and console presentation (WP11, WP17), the MCP tool surface and the creation call of an agent task (WP13).

## Dependencies

- `crate::engine::Engine`: `store` (`file_by_id`, `find_path`, `latest_version`, `version`), `root`, and `load_version` for the section check of a file link and the stale check of an edit plan.
- `crate::doctask_service::{Caller, EngineGate, TransferGate}`: the caller that fixes the principal, and the external-transfer gate. The gate decides whether an agent sees a file; a refused file is `E_NOT_FOUND`.
- `crate::doctask_store`: `DocStore` with `tasks`, `plans`, `approvals`, `evidence`, `links`, `dependency_edges`, `anchor_reviews`, `events`, `baselines`, `verify_chain`, `status_drift`, `duplicate_baselines`, `commit_step`, and the row types `RowWrite`, `StatusChange`, `StepEvent`, `NewEvent`, `TaskRow`, `LinkRecord`, `AnchorReview`.
- `crate::doctask_model`: the enumerations, `lookup_transition`, `check_guards`, `GuardInput`, `Operation`, `authorize`, `parse_task_handle`, `Handle`, `plan_hash`, `PlanFields`, `evidence_hash`, `schedule_batches`, `Schedule`, `SchedTask`, the text limits, `validate_text`, `required_text`, `validate_plan_text`, `normalize_new_text`, `parse_section_selector`, `sha256_hex`, `DocError`, `ErrorCode`.
- `crate::doctask_outline::{outline, resolve}` for the section check of a `file_section` link.
- `serde_json` for payloads, `chrono` for the rate window (RFC 3339 parsing), `uuid` for identifiers chosen before a transaction, `async-trait` and `anyhow`.

## Public Contract

### Edit component

```rust
#[async_trait]
pub trait EditPlanSupport: Send + Sync {
    async fn prepare(&self, target: &FileRecord, section: &str, kind: EditKind, new_text: &str) -> anyhow::Result<EditSubject>;
}
pub struct EditSubject { pub expected_version_id: String, pub expected_hash: String, pub result_hash: String }
```

The component validates the target (file type, section, size) and computes the three subject fields from the ledger and the bytes on disk. The flow stores them and does not recompute them. The flow refuses an edit whose `expected_hash` equals `result_hash` (design 8.1). WP07 registers no component: an edit plan without a registered component fails with `E_INVALID_ARG`, the message that the edit component is not registered, and the hint to submit an analysis-only plan. Analysis-only plans never need the component (design 4.2, T2).

### Flow

`DocFlow<'a>` holds the engine reference, a `DocStore`, the `Caller`, a boxed `TransferGate` (`EngineGate` unless replaced) and an optional boxed `EditPlanSupport`.

- `DocFlow::open(engine, caller)` applies the schema and runs the legacy backfill, as `DocTaskService::open` does. `DocFlow::attach(engine, caller)` writes nothing and fails with `E_SCHEMA_OUTDATED` when migration 4 is not recorded.
- `with_gate(gate)` replaces the gate; `with_edit_support(component)` registers the edit component.

Result types: `Step { task_id, handle, from, to, events }`, where `events` lists the `seq` of the events written, in order, and `from` and `to` are empty for operations that do not change the status; `PlanRef { id, plan_no, plan_hash, existing, step }`, where `step` is absent when `existing` is true; `ScheduleView { schedule, hidden }`; `Inbox { approvals, verifications, proposals }` of `TaskBrief { handle, title, status, plan_hash }`; `VerifyReport { findings, ok }` of `Finding { code, level, task, seq, detail }`.

### Principal and visibility

Each operation first runs `authorize` (operations of the authorization table) or `lookup_transition` (transitions), so a principal outside the allowed list receives `E_HUMAN_ONLY` whatever the state of the task. For the agent principal every task reference resolves only among tasks whose visibility is `shared`, whose file the gate allows and whose base version the gate allows. A `local_only` task, a task on a refused file and a missing task return the same `E_NOT_FOUND` with no candidate. An ambiguous handle returns `E_AMBIGUOUS`. For an agent operation the gate's rule set must be usable, otherwise `E_POLICY_DENIED`.

Human-only operations: approve, reject, close, cancel, reopen, adopt, dismiss, remove dependency, remove link, inbox and verify. Operations open to both principals: start analysis (T1), submit plan (T2), report work (T5), return to work (T7, the agent only when the latest evidence failed), rework (T8), add dependency (the agent only on its own task, see Dependencies), add link, add evidence, and schedule (agent view). The system principal is refused by every operation of this module.

### Proposal precondition

A task whose `proposal_state` is not `accepted` refuses every transition and every plan submission with `E_TRANSITION_DENIED`, and the hint says that the owner must adopt the proposal (`owner must adopt`). Adoption (P1) and dismissal (P2) are human-only.

### Operations

Each row lists the transition, the principal, the guards, the rows written in the same transaction, and the events in order.

| Operation | Transition | Principal | Guards | Rows written | Events (in order) |
|---|---|---|---|---|---|
| `start_analysis` | T1 open to analysis | Human, Agent | `check_guards(T1)`: anchor not orphaned, file missing or ambiguous | none | `dt.task.transitioned` |
| `submit_plan` | T2 analysis to approval | Human, Agent | valid fields; an edit plan only with a registered component and `expected_hash` different from `result_hash`; a repeat of the latest submitted plan returns it | plan `submitted` with the next `plan_no`; earlier `submitted` plans `superseded` | `dt.plan.submitted`, `dt.task.transitioned` |
| `approve` | T3 approval to working | Human | `check_guards(T3)`: latest plan submitted, subject hash equal to its `plan_hash`, dependencies done, anchor settled, plan not stale | approval row (`plan`, `approved`) whose `event_id` is the approval event; plan `approved`; `chosen_option` when given; `last_approval_id` | `dt.approval.recorded`, `dt.task.transitioned` |
| `reject` | T4 approval to analysis | Human | `check_guards(T4)`: reason required | approval row (`plan`, `rejected`); plan `rejected` | `dt.approval.recorded`, `dt.task.transitioned` |
| `report_work` | T5 working to verification | Human, Agent | `check_guards(T5)`: summary required; the plan is analysis-only or `applied` | evidence row (`note`, `info`, source `reported`) whose `event_id` is the transition event | `dt.task.transitioned` |
| `close` | T6 verification to done | Human | `check_guards(T6)`: evidence accepted (rule below), dependencies done, anchor settled, `expected_subject_hash` equal to `evidence_hash` | approval row (`result`, `approved`); when waived, evidence row (`waiver`, `info`, `reported`) whose `event_id` is the approval event; `closed_at`; `last_approval_id` | `dt.approval.recorded`, `dt.task.transitioned` |
| `return_to_work` | T7 verification to working | Human; Agent only when the latest evidence has result `fail` | `check_guards(T7)`: reason required | for the human, approval row (`result`, `rejected`) whose `event_id` is the transition event | `dt.task.transitioned` |
| `rework` | T8 working to analysis | Human, Agent | `check_guards(T8)`: reason required; no applying plan | plans `approved` become `superseded`; applied plans stay | `dt.task.transitioned` |
| `cancel` | T9 open, analysis, approval, working, verification to cancelled | Human | `check_guards(T9)`: reason required; no applying plan | plans `submitted` and `approved` become `superseded`; `cancel_reason`; `closed_at` | `dt.task.transitioned` |
| `reopen` | T10 done or cancelled to open | Human | `check_guards(T10)`: reason required | `closed_at` cleared | `dt.task.reopened` |
| `approve`, stale path | T11 approval to analysis (system) | System, committed inside `approve` | `check_guards(T11)` with the plan stale | the plan `superseded` | `dt.plan.superseded`, `dt.task.transitioned` with actor `system` |
| `adopt` | P1 | Human | the task is a proposal | `proposal_state` `accepted`, decided by and at | `dt.proposal.accepted` |
| `dismiss` | P2 | Human | the task is a proposal in `open`; `check_guards(T9)`: reason required | `proposal_state` `dismissed`; `cancel_reason`; `closed_at` | `dt.proposal.dismissed`, `dt.task.transitioned` (open to cancelled) |

Payloads: approval events carry the approval id, the kind, the decision, the subject hash and the comment; the close event also carries `accepted_agent_evidence` (the accepted ids) and `evidence_waived`. Transition events carry the reason where the transition has one, and `evidence_waived` for close.

`approve` evaluates the guards before any write. When the only failing guard is guard 4 (`StalePlan`), the flow commits the stale path of T11 in the same transaction as the supersession of the plan, and then returns `E_STALE_PLAN`. The committed state is the result: the plan is `superseded`, the task is in `analysis`, and the transition event has actor `system`.

Guard 4 applies to an edit plan only. It fails when the ledger's latest version of the target is not `expected_version_id`, when the file cannot be read, when it is larger than 8 MiB, or when the digest of its bytes is not `expected_hash`.

Acceptance (T6). An evidence row qualifies when its kind is `test_run`, `report` or `note`, its result is `pass`, no evidence row with result `fail` follows it in chain order, and its actor is human or its id is listed in `accept_agent`. Chain order is the `seq` of the event that wrote the row. An id in `accept_agent` must name an evidence row of this task with those kinds, result `pass` and actor agent; otherwise the call fails with `E_INVALID_ARG`. A waiver reason must be non-empty.

The anchor is settled (guard 3) when `baseline_id` is set and the anchor state is `valid`, or the state is `needs_review`, `relocated` or `unknown` and `reviewed_version_id` equals `current_version_id` (`AnchorState::is_settled`). Any other state, and a task without an anchor row, is not settled.

### Plans

`submit_plan(handle, spec)` validates summary, rationale, impact and options with `validate_plan_text`. `risk` is `low`, `medium` or `high`; an empty value means `medium`. An analysis-only plan has an empty edit kind, an empty target, empty new text and empty hashes. An edit plan names a file reference, a section, a kind (`replace_section` or `insert_after_section`) and new text. The new text is normalised with `normalize_new_text`, the target is resolved through the principal's file rule, and the component's `prepare` supplies the subject fields.

`plan_hash` covers `(task_id, plan_no, summary, edit_kind, target_file_id, target_section, new_text, expected_hash, result_hash)`. A submission whose hash, computed with the `plan_no` of the latest submitted plan, equals that plan's stored `plan_hash` is the same plan: it returns the existing plan with `existing` true and writes nothing.

### Dependencies

`add_dependency(handle, depends_on)`:

- The two references must differ (`E_INVALID_ARG`). The target must be visible to the principal (`E_DEP_DANGLING` otherwise). The edge must be new (`E_INVALID_ARG`).
- The agent may add an edge only to a task that its own session created as an agent task (otherwise `E_HUMAN_ONLY`), and at most `MAX_AGENT_DEPENDENCIES` edges per task (otherwise `E_QUOTA`). The creation call of an agent task with its declared dependencies is the creation path of WP13, which calls this operation for each declared dependency right after creating the task; the flow enforces the ownership and the cap.
- The cycle check runs `schedule_batches` over every task with its stored edges and the new edge; a cycle returns `E_DEP_CYCLE`.

`remove_dependency(handle, depends_on)` is human-only. It deletes the edge row and writes `dt.dep.removed`. An edge is written by `dt.dep.added`.

### Links

`add_link(handle, link_kind, target_kind, target, section, label)`:

- `link_kind` is a `LinkKind` value and `target_kind` a `TargetKind` value.
- `file`: the target resolves through the principal's file rule; it is stored by id with its relative path.
- `file_section`: the file resolves as above, the selector is syntactically valid (`parse_section_selector`), and the section exists in the latest version (outline of the version bytes and `resolve`).
- `git_commit`: 7 to 64 hexadecimal characters.
- `url`: scheme `http` or `https`, at most 300 characters.
- `task`: a visible task, stored by id.
- `label` is at most `MAX_LINK_LABEL_CHARS` characters.

`remove_link(handle, link_id)` is human-only. It sets `removed_at` and writes `dt.link.removed`. A link is written by `dt.link.added`.

### Evidence

`add_evidence(handle, kind, result, summary, version_id)`:

- Principals: human and agent. The system principal receives `E_HUMAN_ONLY`.
- The task must be `working` or `verification`; otherwise `E_TRANSITION_DENIED`.
- Kinds `test_run`, `report` and `note` with results `pass`, `fail` or `info`; source `reported`. The summary is 1 to `MAX_VERIFY_SUMMARY_CHARS` characters.
- Kind `version` requires `version_id`: the version exists, belongs to the task's file, has a number greater than the `version_no` of the active baseline, and passes the gate for the agent. The row takes its content hash from the ledger and its source is `ledger`. Its result must be `info`.
- Kinds `commit`, `agent_change` and `waiver` are written only by the application and approval paths; here they return `E_INVALID_ARG`.
- The row's `event_id` is the `dt.evidence.added` event. Evidence rows are never changed.

### Quotas and rate

For the agent principal every change operation first calls `check_agent_rate`. It counts the events of the caller's session with a timestamp in the last 10 minutes, as design 4.3 defines the count over `doc_task_events`. A count of 40 or more returns `E_RATE_LIMIT`. One operation may write two events (a transition and its approval, for example), so the limit is reached after fewer operations than events. Reads do not count.

`admit_proposal` (agent only) first calls `check_agent_rate`, then counts the tasks of the caller's session whose `proposal_state` is `proposed`; ten or more return `E_QUOTA`. The creation path of WP13 calls it before it creates an agent task.

### Inbox and schedule

`inbox()` is human-only. It returns the tasks in `approval` with their latest plan hash, the tasks in `verification`, and the tasks whose `proposal_state` is `proposed`, each in task-number order.

`schedule()` builds the input of `schedule_batches` over all tasks: `proposal_accepted` from `proposal_state`; `depends_on` from the stored edges; `plan_target` from the latest plan that is neither `superseded` nor `rejected` and has an edit target; `link_target` from the task's `target` link with no `removed_at`. For the agent principal the result is filtered: tasks not visible to the agent are omitted from `batches`, `blocked` and `unknown_targets`; a conflict group that names an omitted task, or a file the gate refuses, is omitted; a blocked entry whose reason names an omitted task is omitted; the distinct omitted tasks are counted in `hidden`. Conflict files are given by file id.

### Verify

`verify(handle)` is human-only. With a handle, the findings of that task are returned together with the chain-level findings.

| Code | Level | Check |
|---|---|---|
| `CHAIN_BROKEN` | error | `DocStore::verify_chain`: the first failing `seq` |
| `SEQ_GAP` | warning | a gap in the `seq` values: the `seq` after the gap |
| `STATUS_DRIFT` | error | `DocStore::status_drift`: the stored status against the last status event |
| `ILLEGAL_TRANSITION` | error | replay of the status events of each task from `open`, or from its latest reset point (`dt.task.created`, `dt.task.imported`, `legacy.task.state_changed`); each `dt.task.transitioned` and `dt.task.reopened` must start at the replayed state and be allowed by `lookup_transition` for its actor kind; an event whose actor kind is `unknown` is not checked for legality |
| `HUMAN_ONLY_FORGED` | error | a transition whose pair is human-only, executed by an actor that is neither human nor unknown (system T11 is allowed); an event of a human-only kind whose actor is neither human nor unknown; the human-only kinds are `dt.approval.recorded`, `dt.proposal.accepted`, `dt.proposal.dismissed`, `dt.task.reopened`, `dt.task.edited`, `dt.dep.removed`, `dt.link.removed`, `dt.baseline.registered`, `dt.baseline.revoked`, `dt.commit.applied`, `dt.anchor.acknowledged`, `dt.anchor.relocated`, `dt.anchor.rebased`, `dt.lineage.confirmed`, `dt.lineage.rejected` |
| `APPROVAL_ORPHAN` | error | an approval row whose `event_id` is not in the chain, or a `dt.approval.recorded` event that no approval row refers to |
| `PLAN_INCONSISTENT` | error | a plan in `applying` with no `dt.commit.started` event for it |
| `DEP_CYCLE` | error | stored dependency edges that form a cycle (`schedule_batches` over all tasks) |
| `BASELINE_DUPLICATE` | error | `DocStore::duplicate_baselines` |
| `LEGACY_UNAPPROVED` | warning | a task whose `source_kind` is `legacy`, in `approval`, `working`, `verification` or `done`, with no approval row |
| `ANCHOR_UNREVIEWED` | warning | a task anchor in `needs_review`, `relocated`, `ambiguous` or `orphaned` whose `reviewed_version_id` is not its `current_version_id` |

`COMMIT_UNANCHORED` is produced by the application path (WP14). `LEGACY_NEW` is not produced: it counts legacy rows without metadata, which the store's backfill report already states. `VerifyReport::ok` is true when no error-level finding exists; the command line maps a failure to exit code 9.

## Change Notes

The flow is a separate type from `DocTaskService`, because the service keeps its database handle, gate and caller private to its own module; both use the same store, gate and caller types. The store provides the one-transaction primitive `commit_step` (optional status compare-and-set, row writes and several events with caller-chosen ids), the row writes `UpdateWhere` and `Delete`, and the readers `plans`, `approvals`, `evidence`, `links` and `anchor_reviews`, because design 2.1 requires all SQL in `DocStore`.
