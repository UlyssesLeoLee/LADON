# Test Design: `src/doctask_cli.rs`

Source: `src/doctask_cli.rs`

Source-SHA256: `ab810ef314aa5ae26038d524a4a4fea3c150f908dbce1a601bb5c3be7b0b6ff4`

## Test Strategy

The module is covered at three levels. The unit tests in `src/doctask_cli.rs` (`tc930` to `tc936`) test the command tree, the exit mapping, the JSON envelopes, the confirmation rules and the analysis-only plan path, with a scripted `DocIo` and a workspace in a temporary directory. The integration tests in `tests/it_doctask_cli.rs` (`it_061_*`, `it_938_*`, `it_939_*`) run the legacy helpers, the entry-point source guard and the one-shot flow in process. The binary tests in `tests/it_doctask_cli_binary.rs` (`it_060_*`, `it_066_*`) run the built `ladon` executable with `CARGO_BIN_EXE_ladon`.

The scripted `DocIo` records standard output and standard error, answers the console questions from a queue, and reports the console state from its fields. No test uses a real console, so the confirmation paths run in process. The built binary runs on pipes; it therefore only exercises the refusal of human-only commands and the non-human-only commands. The human-only steps that a binary export or JSON test needs are run in process first, and the workspace is released before the binary runs.

Each test workspace is a `tempfile` directory with `Engine::init`. Each one runs `doc migrate` before the first doc command, because a freshly initialised database is below schema 4 and the read and write commands attach without migrating.

## Test Cases

| TC | Test function | Location | Input | Expected |
|---|---|---|---|---|
| TC-930 | `tc930_command_tree_is_valid` | `src/doctask_cli.rs` | `entry::Cli::command().debug_assert()`; `try_parse_from` of every in-scope `doc` form of 57 section 9.2, including the global options after the subcommand; `doc apply T-1`, `doc recover`, `doc plan show T-1 --diff`, the edit options of `doc plan new`, and `--lang fr`; `scan --actor x` and `annotate ... --actor x` | The tree is valid; every in-scope form parses; the out-of-scope forms fail to parse; the legacy `--actor` options keep their own meaning |
| TC-931 | `tc931_exit_code_mapping` | `src/doctask_cli.rs` | Every `ErrorCode` variant (27 rows) | `exit_code` returns the value of the table in the design, row by row |
| TC-932 | `tc932_json_envelopes` | `src/doctask_cli.rs` | `json_ok` with an object; `json_error` with a `DocError` that has a hint | `json_ok` parses to `{"ok":true,"data":{…}}`; `json_error` parses to exactly the keys `ok`, `code`, `message`, `hint` with `ok` false and `code` equal to the `E_` string |
| TC-933 | `tc933_yes_requires_expect_hash` | `src/doctask_cli.rs` | `confirm_yes(true, None)`; prefixes of 5, 2 and 7 characters; the non-hex prefix `zz`; the prefixes `deadbeef` and `abcdef01` (eight characters); `confirm_yes(false, None)`; `hash_prefix_matches` with `deadbeef` against an upper-case hash and against `cafebabe…` | `None`, the 5-, 2-, 7-character and non-hex prefixes are `E_INVALID_ARG`; eight-character hex prefixes and the absent `--yes` are accepted; the case-insensitive match is accepted; the mismatch is rejected |
| TC-934 | `tc934_human_only_refused_without_console` | `src/doctask_cli.rs` | Piped `FakeIo` (both console flags false); `approve T-1 --comment x`, `baseline add a.md`, `edit T-1 --title x --yes --expect-hash deadbeef`, `reject T-1 --comment x` | Each returns exit 5 before any service call; the number of events is unchanged; no prompt is read |
| TC-935 | `tc935_typed_and_reason_confirmation` | `src/doctask_cli.rs` | Console answers: `t-1` and `T-1 ` (approve, not exact); `T-1` (approve, exact); `reject --comment ""` with a `y` answer queued; `cancel` with `n`; `dismiss` with `Y`; `cancel` with `y` | Non-exact typed answers abandon with `"written":false` and no event; the exact handle approves the task to `working`; an empty reason is `E_INVALID_ARG` (exit 4) with no prompt read; `n` and the capital `Y` abandon; `y` cancels the task to `cancelled` |
| TC-936 | `tc936_plan_new_approve_reject_analysis_only` | `src/doctask_cli.rs` | Workspace with one baseline; tasks `T-1` and `T-2` each moved to `analysis` and given `plan new --summary` (no edit options); `approve T-1` with the typed handle; `reject T-2 --comment` with `y`; `plan show T-1 1` | Both plans have `plan_no` 1 and no target file; the approved task is `working`; the rejected task returns to `analysis` (T4); the command runs with no `EditPlanSupport` registered |
| TC-937 | `it_061_legacy_annotate_task_lines` | `tests/it_doctask_cli.rs` | `annotate_legacy` on `a.md` at `ROOT`; `annotate_legacy` at the unknown section `Nope`; `task_legacy` with a title and a description | Each annotate call returns 0 and prints `annotation id: ` on standard output, including the unknown section; the task call returns 0, prints `task id: ` on standard output and the `T-1` hint with `ladon doc show T-1` on standard error |
| TC-938 | `it_938_entry_arms_are_single_calls` | `tests/it_doctask_cli.rs` | Source text of `src/entry.rs` and `src/main.rs` read through `CARGO_MANIFEST_DIR`; the bodies of the `Command::Annotate`, `Command::Task` and `Command::Doc(args)` arms | Each arm calls `doctask_cli::` and contains none of the tokens `store.`, `SELECT`, `INSERT`, `UPDATE`, `DELETE` or `sql`; `main.rs` makes one call to `main_entry` |
| TC-939 | `it_939_one_shot_flow_in_process` | `tests/it_doctask_cli.rs` | Workspace with migration and the baseline; `note`; `new`; `status analysis`; `plan new`; `approve` (typed); `evidence add`; `status verification --summary`; `close` (typed); `verify`; `history`; `list --all`; `show --json`; `anchors`; `export nema --all` | Every step exits 0; `list --all` shows `T-1`; the JSON of `show` parses with `ok` true and contains the status `done`; one `.nema` file is written under `.ladon/exports` |
| IT-060 | `it_060_binary_flow_json_and_pipe_refusal` | `tests/it_doctask_cli_binary.rs` | Baseline and migration in process, then the binary: `doc new --json`, `doc list --json`, `doc approve --json` on a pipe, `doc baseline add --json --yes --expect-hash deadbeef` on a pipe | `new` and `list` exit 0 with `ok` true on standard output; the piped `approve` exits 5 with `ok` false and `E_NOT_INTERACTIVE` in the JSON; the piped `baseline add` with `--yes` exits 5; the `list` output is byte-identical before and after the refused calls |
| IT-066 | `it_066_binary_export_and_read_back` | `tests/it_doctask_cli_binary.rs` | Baseline, a task completed in process (analysis, plan, approve, evidence, verification, close); then the binary: `doc export nema --all` into the default folder and `doc export report --format json --all --generated-at … --out <exports>/report-check.json` | The `.nema` file parses with `parse_graph_edges` and `topo_batches` succeeds; the JSON report has `cutoff_seq` equal to the largest sequence of an event that is not `dt.export.generated`; the export event is in the chain; the report lists the task with status `done` |

## Acceptance Traceability

| Acceptance item (57 section 9.2, WBS WP11) | Test cases |
|---|---|
| Every in-scope command parses and the out-of-scope forms are rejected | TC-930 |
| Exit codes follow 9.3 | TC-931, IT-060 |
| `--json` prints exactly one JSON object on stdout | TC-932, IT-060 |
| `--yes` requires `--expect-hash` and the hash prefix is checked | TC-933 |
| Human-only commands exit 5 on pipes and without a console; nothing is written | TC-934, IT-060 |
| Typed and reason confirmations accept only exact answers | TC-935 |
| `plan new`, `approve`, `reject` work in analysis-only mode | TC-936, TC-939 |
| `annotate` and `task` keep positional arguments and first output lines | TC-937 (`it_061_*`), and the transcript of `tests/it_entry_legacy.rs` |
| The new `entry.rs` branches are single library calls with no SQL | TC-938 |
| The one-shot flow from `init` to export runs without Hermes | TC-939, IT-066 |
| `doc export nema` and `doc export report` write a deterministic file and record the export | IT-066, TC-939 |

## Boundary & Failure Cases

- Hash prefix of seven characters and of eight characters: seven is refused with `E_INVALID_ARG`, eight is accepted when it matches.
- Task handle `12` and `T-12` address the same task; a handle with a non-hex prefix shorter than eight characters is `E_INVALID_ARG`.
- A confirmation answer of an empty line abandons.
- `--depends` referring to a missing task returns `E_NOT_FOUND`; the new task remains and the output names it.
- `doc export report --format xml` is refused by clap before any read.
- `--out` with a device name (`CON`, `NUL`, `COM1`) or a path to an existing directory is refused with `E_INVALID_ARG`.
- `--out` to an existing regular file without `--force` is refused with `E_INVALID_ARG`.
- `--out` to a path inside `.ladon` but outside `.ladon/exports` is refused with `E_INVALID_ARG`.
- `--generated-at` with an empty value or a control character is refused with `E_INVALID_ARG`.
- `--section` without `--file` is refused by clap.

## Regression Scope

- `tests/it_entry_legacy.rs` (the `annotate` and `task` transcripts) passes with two recorded changes: the stderr line of `task` (`任务编号 T-1，查看: ladon doc show T-1`), and `annotate` on an untracked file, which now exits 3 with `E_NOT_FOUND` instead of 1. The unit test `entry::tests::init_and_record_commands_through_run` asserts the same exit code 3.
- `tests/it_cli_binary_smoke.rs` continues to pass; the `doc` command is listed in the help output.
- `tc108`, `tc109` and `it_05` in the store tests keep passing because `Store::add_annotation`, `create_task`, `set_task_status` and `list_tasks` are unchanged.
- The `doctask_service`, `doctask_flow`, `doctask_reconcile` and `doctask_report` unit tests are not changed by this package.
