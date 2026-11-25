# Test Design: `src/doctask_flow.rs`

Source: `src/doctask_flow.rs`

Source-SHA256: `3ab7bdfb81393b6dffff8ca30d123e56ce04dfa13e4a219598f7dcb3a5b21f70`

## Test Strategy

The flow is tested through its public contract in `tests/it_doctask_flow.rs` against a real temporary library: `Engine::init` on a temporary directory, files written with `std::fs` and recorded with `Engine::capture`, baselines and tasks created through `DocTaskService` (WP06) so that the rows are the ones the application writes, and the flow opened with `DocFlow::open` (owner) or `DocFlow::attach` (agent). Each test asserts on returned values, on the rows read back through `DocStore` (`tasks`, `plans`, `approvals`, `evidence`, `links`, `dependency_edges`), on the chain read through `DocStore::events` (kind, `from_status`, `to_status`, actor kind, in `seq` order), and on the error codes, compared as `DocError` codes.

Three states are reached without a public operation that writes them: a status bypass through the legacy `Store::set_task_status` (detected as `STATUS_DRIFT`), a forged transition written through `DocStore::commit_step` with an agent actor, and an anchor state set to `relocated` by a raw statement on a second connection. An edit component is a fixed fake of `EditPlanSupport`; the stale path is reached by changing the target file and capturing a new version after the plan is submitted.

Every legal path ends with `verify()` reporting no error-level finding (TC-880, TC-890, TC-891, TC-892, TC-893, TC-894, TC-899). Verification runs in the CI job `release.yml` (`cargo test --locked --all-targets` on `windows-latest`). The pure rules of the module are tested in its `tests` module (TC-899).

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-880 | Owner: start, submit an analysis-only plan, approve with the shown hash, report work, add a human pass test run, close with the evidence hash. | Each step writes exactly the events of design 4.2 in order: `dt.task.transitioned` (open to analysis); `dt.plan.submitted`, `dt.task.transitioned` (analysis to approval); `dt.approval.recorded`, `dt.task.transitioned` (approval to working); `dt.task.transitioned` (working to verification); `dt.approval.recorded`, `dt.task.transitioned` (verification to done). `verify` reports ok. | tests/it_doctask_flow.rs::tc880_every_legal_edge_writes_its_events |
| TC-881 | Owner on an open task, then on an analysis task: approve, reopen, rework, report work, submit plan, start analysis again, close. | Each illegal call returns `E_TRANSITION_DENIED`; the chain length is unchanged by the refused calls. | tests/it_doctask_flow.rs::tc881_illegal_edges_are_refused_without_writes |
| TC-882 | Task in analysis. `DocStore::commit_step` with a compare-and-set from approval (the stored status is analysis), then with a valid compare-and-set and an update of a plan row that does not exist. | The first returns `E_CONFLICT` and writes nothing. The second returns `E_NOT_FOUND`; the chain length and the stored status (analysis) are unchanged, so the whole step rolled back. | tests/it_doctask_flow.rs::tc882_a_failed_step_writes_nothing |
| TC-883 | Task in verification whose only evidence is the report note (result info). Close with the shown hash. Add a human pass note, add a dependency on an open task, close again, remove the dependency, close. | First close: `E_GUARD_FAILED`. Second close, with the dependency open: `E_GUARD_FAILED`. After removing the dependency: success, approval and transition events written. | tests/it_doctask_flow.rs::tc883_close_requires_evidence_and_dependencies |
| TC-884 | Task in verification; the agent adds a `test_run` pass. Owner closes without acceptance, with an unknown id, then with the agent row id. | Without acceptance: `E_GUARD_FAILED`. Unknown id: `E_INVALID_ARG`. The agent row id: success, and the approval event payload contains that id in `accepted_agent_evidence`. | tests/it_doctask_flow.rs::tc884_agent_pass_needs_explicit_acceptance |
| TC-885 | Task in verification. Close with a waiver reason of only spaces, then with the reason `accepted risk`. | Blank waiver: `E_INVALID_ARG`. Waiver: success; one evidence row of kind `waiver`, result `info`, actor human; the transition payload carries `"evidence_waived":true`. | tests/it_doctask_flow.rs::tc885_written_waiver_and_chain_order |
| TC-886 | Owner submits a plan (task in approval). The agent calls approve, reject, close, cancel, reopen, adopt, dismiss, remove dependency, remove link, inbox, verify, and start analysis. | Every human-only call returns `E_HUMAN_ONLY`. Start analysis on an approval task returns `E_TRANSITION_DENIED` (only an open task can start). The chain length is unchanged. | tests/it_doctask_flow.rs::tc886_agent_is_refused_every_human_operation |
| TC-887 | Two agent proposals created with `DocStore::create_task` in session `hermes:L1`. The agent starts `T-1` before adoption. The owner adopts `T-1`; the agent starts it; the owner's inbox lists `T-2`; the owner dismisses `T-2` with a reason; the owner adopts `T-2`. | Before adoption the agent gets `E_TRANSITION_DENIED` whose hint contains `owner must adopt`, and nothing is written. Adoption writes `dt.proposal.accepted`. Dismissal writes `dt.proposal.dismissed` and `dt.task.transitioned` (open to cancelled), and `proposal_state` is `dismissed`. Adopting a dismissed proposal returns `E_TRANSITION_DENIED`. | tests/it_doctask_flow.rs::tc887_proposals_wait_for_the_owner |
| TC-888 | Three tasks `T-1`, `T-2`, `T-3` on `a.md`, each started and linked with a `target` link to a file: `T-1` and `T-3` to `a.md`, `T-2` to `b.md`. | `schedule()` returns one batch `[T-1, T-2, T-3]`, one conflict group on the `a.md` file id with `T-1` and `T-3`, no blocked task, no unknown target, and `hidden` 0. | tests/it_doctask_flow.rs::tc888_uat05_one_conflict_group_and_one_parallel_task |
| TC-889 | Task in analysis; `Store::set_task_status` sets the stored status to approval. Then the status is repaired, a plan is submitted, and `DocStore::commit_step` writes an approval-to-working transition with an agent actor. | `verify(Some("T-1"))` reports `STATUS_DRIFT` as an error and is not ok. After the forged transition, `verify` reports `HUMAN_ONLY_FORGED` as an error. | tests/it_doctask_flow.rs::tc889_verify_detects_bypass_and_forgery |
| TC-890 | Analysis-only lifecycle with a rejection: start, plan one, reject with reason, plan two with options `alpha` and `beta`, approve with option 1, report, add a human report pass, close. | Plan one is `rejected`; plan two is `approved` with `chosen_option` `alpha`; plan two has `plan_no` 2; the close returns `done`; `verify` is ok. | tests/it_doctask_flow.rs::tc890_analysis_only_lifecycle_open_to_done |
| TC-891 | Owner with a fixed edit component on `a.md` (section `key:H1:A#1`). An edit plan is submitted. An owner without the component submits the same edit. The target is changed and captured; approve with the shown hash. | Without the component: `E_INVALID_ARG`, chain unchanged. Approve: `E_STALE_PLAN`; the plan is `superseded`; the task is `analysis`; the last two events are `dt.plan.superseded` and `dt.task.transitioned` with actor kind `system`. `verify` is ok. | tests/it_doctask_flow.rs::tc891_stale_plan_commits_t11_and_returns_error |
| TC-892 | Anchor state set to `relocated` (raw statement), then `valid` again, then `relocated` before close. | Approve with relocated anchor: `E_ANCHOR_BROKEN`, nothing written. With valid anchor the approve succeeds. Close with relocated anchor: `E_ANCHOR_BROKEN`; with valid anchor the close succeeds. | tests/it_doctask_flow.rs::tc892_relocated_anchor_blocks_approve_and_close |
| TC-893 | Task in verification after a report note. The agent returns the work with no failed evidence, then adds a failed report, then returns the work. The owner reworks. | The first return: `E_GUARD_FAILED`. After the failed report the agent's return succeeds and writes no approval row (one approval row remains from the approval). Rework moves the task to analysis and supersedes the approved plan. `verify` is ok. | tests/it_doctask_flow.rs::tc893_return_and_rework_paths |
| TC-894 | Task one is started then cancelled with a reason; cancelling again fails; reopening without a reason fails; reopening with a reason writes `dt.task.reopened`; cancelling again. Task two depends on task one. | Repeated cancel: `E_TRANSITION_DENIED`. Empty reason: `E_INVALID_ARG`. Reopen: the last event is `dt.task.reopened` with to-status `open`. Schedule: task two is blocked with the reason `dependency cancelled: T-1`. | tests/it_doctask_flow.rs::tc894_cancel_and_reopen_and_cancelled_dependency |
| TC-895 | Tasks one and two. Owner adds `T-2` on `T-1`; self dependency; missing target; the cycle `T-1` on `T-2`; the duplicate. Owner removes the edge. An agent-created task `T-3` in session `hermes:L1` depends on `T-1`; the agent adds `T-1` on `T-2`. | Self: `E_INVALID_ARG`. Missing: `E_DEP_DANGLING`. Cycle: `E_DEP_CYCLE`. Duplicate: `E_INVALID_ARG`. Removal empties the edge list. Agent on its own task: success. Agent on an owner task: `E_HUMAN_ONLY`. | tests/it_doctask_flow.rs::tc895_dependencies_cycles_and_ownership |
| TC-896 | Owner task `T-1` and `T-2`. Valid links: a file, a git commit (`abcdef0123`), an https address, a task. Invalid: a git commit `abc`, an `ftp` address, an https address with a 121-character label, an empty `file_section` selector, a task `T-99` that does not exist. Owner removes the file link. | Valid links are written. Invalid ones return `E_INVALID_ARG`, the missing task `E_NOT_FOUND`. After removal the link row has `removed_at` set. | tests/it_doctask_flow.rs::tc896_links_validate_each_target_kind |
| TC-897 | Analysis, then approved and working. Evidence: a `test_run` pass; `commit` kind; `version` without id; `version` of the baseline version; `version` of the `b.md` version; `version` of a later `a.md` version with result `pass`; the same with result `info`. | Analysis refuses evidence (`E_TRANSITION_DENIED`). Refusals as listed (`E_INVALID_ARG` for kind, missing id, baseline number and result; `E_NOT_FOUND` for another file). The later version with `info` is accepted. The rows in chain order are `reported`, then `ledger`. | tests/it_doctask_flow.rs::tc897_evidence_states_kinds_and_version_rules |
| TC-898 | Nine proposals in session `hermes:L1`, then a tenth; an eleventh admission. Session `hermes:L2` with 39 events, then one more. Session `hermes:L3` with 40 events. | With nine pending proposals the admission succeeds; with ten it returns `E_QUOTA`. `hermes:L2`: admission succeeds with 39 events, returns `E_RATE_LIMIT` with 40. `hermes:L3`: a change call returns `E_RATE_LIMIT`; `schedule` still succeeds. | tests/it_doctask_flow.rs::tc898_proposal_quota_and_rate_window |
| TC-899 | Fresh library, one task started; `verify`, `inbox` and `schedule` on it. Module tests: `evidence_accepts` on a human pass, an agent pass with and without acceptance, a fail after a pass, a fail before a pass, a waiver row; `is_acceptable_kind` on report and version. | `verify` reports no findings. The inbox is empty and the schedule has one batch with `T-1`. The pure rules return the expected booleans. | tests/it_doctask_flow.rs::tc899_fresh_library_verifies_clean_and_pure_rules_hold; src/doctask_flow.rs (tests module, `tc899_*`) |

## Acceptance Traceability

Every clause of the WP07 acceptance column of `docs/05_operations/58_文档任务与HermesCLI开发WBS.md` maps to at least one case above.

| Acceptance clause | Cases |
| --- | --- |
| Every legal edge writes its documented events in the same transaction | TC-880, TC-882, TC-890 |
| Every illegal edge is refused and writes nothing | TC-881, TC-882, TC-886 |
| Acceptance needs evidence or a written waiver, prerequisite tasks done, and a healthy anchor | TC-883, TC-884, TC-885, TC-892 |
| Agent cannot approve, reject, close, cancel, reopen, adopt or dismiss at the service layer | TC-886 |
| Agent task is a proposal and cannot transition before adoption | TC-887, TC-898 |
| UAT-05: one conflict group plus one parallel task | TC-888 |
| `verify` detects `STATUS_DRIFT` from a bypass of `set_task_status` | TC-889 |
| Analysis-only open to done lifecycle runs through the service layer | TC-880, TC-890 |
| T3 guard 4 failure: plan `superseded`, task `analysis`, state committed, `E_STALE_PLAN` | TC-891 |
| Agent-written pass does not satisfy T6; `--accept-agent-evidence` and written waiver each pass, with ids in the approval payload | TC-884, TC-885 |
| `relocated` not confirmed: T3 and T6 refused | TC-892 |
| Unadopted proposal: transitions and plan submission give `E_TRANSITION_DENIED` with the adopt hint | TC-887 |
| Events of each edge are consistent with `verify` | TC-880, TC-890, TC-893, TC-894, TC-899 |
| Dependencies, cycles, dangling references | TC-895 |
| Links and their target rules | TC-896 |
| Evidence states, kinds and version rules | TC-897 |
| Quotas and the rate window | TC-898 |
| Inbox, schedule and verify as the owner and agent views | TC-886, TC-888, TC-889, TC-898, TC-899 |
| `EditPlanSupport` rejected when not registered | TC-891 |

## Boundary & Failure Cases

- Nine and ten pending proposals, and 39 and 40 events in the rate window (TC-898).
- Blank and non-blank waiver reasons (TC-885); empty reasons on reopen (TC-894).
- Handles `T-n` resolve; an unknown handle returns `E_NOT_FOUND` (TC-895, TC-896).
- A failed compare-and-set and a failed row write both roll the whole step back (TC-882).
- A stale edit plan is detected before any write other than the T11 commit (TC-891).
- A cycle is detected before the dependency row is written (TC-895).

## Regression Scope

- `tests/it_doctask_reconcile.rs` (TC-900, TC-902, TC-905) covers the decision-time reconciliation before `approve` and `close`: a changed anchor refuses the guard with `E_ANCHOR_BROKEN` until it is acknowledged, and a deleted anchor file stays refused. `approve` evaluates the stored state first, so the stale path of TC-891 is unchanged. The flow tests above create anchors whose fingerprints match their file, so the reconciliation writes nothing for them and their chain assertions hold.
- `tests/it_doctask_store.rs` and `tests/it_doctask_service.rs` (WP05, WP06) must pass unchanged; the store gains the `RowWrite` variants `UpdateWhere` and `Delete`, which the existing matches do not need to cover because the store validates them in its own functions.
- `tests/it_doctask_migration.rs` (IT-050) and `tests/it_nema_task.rs` (IT-005) are unaffected.
