# Test Design: `src/doctask_cli.rs`

Source: `src/doctask_cli.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Test Strategy

The module is covered at three levels. The unit tests in `src/doctask_cli.rs` (`tc930`～`tc936`) test the command tree, the exit mapping, the JSON envelopes, the confirmation rules and the analysis-only plan path with an in-memory fake `DocIo` and a workspace in a temporary directory. The integration tests in `tests/it_doctask_cli.rs` (`it_061_*`, `it_939_*`, `it_938_*`) run the parsed command line through `entry::Cli` and `doctask_cli::run_doc` in process, and read `src/entry.rs` for the token scan. The binary tests in `tests/it_doctask_cli_binary.rs` (`it_060_*`, `it_066_*`) run the built `ladon` executable with `CARGO_BIN_EXE_ladon`; they cover the piped human-only refusal and the export read-back.

The fake `DocIo` records standard output and standard error, answers the console questions from a scripted queue, and reports the console state from its fields. No test uses a real console. The human-only refusal on pipes is shown by the binary tests; the confirmation paths are shown in process.

Tests that need a store run in `tempfile`-style directories created under `std::env::temp_dir()` with a unique name per test, and each creates its own workspace with `Engine::init`.

## Test Cases

| TC | Test function | Location | Input | Expected |
|---|---|---|---|---|
| TC-930 | `tc930_command_tree_is_valid` | `src/doctask_cli.rs` | `entry::Cli::command().debug_assert()`; `try_parse_from` of every `doc` form in 57 section 9.2 that is in scope; `doc apply T-1`, `doc recover`, `doc plan show T-1 --diff` | The tree is valid; every in-scope form parses; the three out-of-scope forms fail to parse. The global options `--json`, `--lang`, `--no-color`, `--actor` parse after the subcommand; `scan --actor x` and `annotate` still parse with their own `--actor` |
| TC-931 | `tc931_exit_code_mapping` | `src/doctask_cli.rs` | Every `ErrorCode` variant (27 rows) | `exit_code` returns the value of the table in the design: 3, 4, 5, 6, 7, 8, 9 and 1 for `E_INTERNAL`, row by row |
| TC-932 | `tc932_json_envelopes` | `src/doctask_cli.rs` | `json_ok` with an object; `json_error` with a `DocError` that has a hint | `json_ok` parses to `{"ok":true,"data":{…}}`; `json_error` parses to exactly the keys `ok`, `code`, `message`, `hint` with `ok` false and `code` equal to the `E_` string |
| TC-933 | `tc933_yes_requires_expect_hash` | `src/doctask_cli.rs` | `confirm_yes(true, None)`; `confirm_yes(true, Some("abc12"))`; `confirm_yes(true, Some("zz"))`; `confirm_yes(true, Some("deadbeef"))`; `confirm_yes(false, None)`; subject check with prefix `deadbeef` against hash `DEADBEEF00…` and against `cafebabe…` | Rows 1 to 3 are `E_INVALID_ARG`; row 4 and row 5 are accepted; the case-insensitive match is accepted; the mismatch returns `E_STALE_SUBJECT` (exit 6) from the subject check |
| TC-934 | `tc934_human_only_refused_without_console` | `src/doctask_cli.rs` | Fake `DocIo` with both console flags false; commands `approve T-1 --comment x`, `baseline add a.md`, `edit T-1 --title x --yes --expect-hash deadbeef`, `reject T-1 --comment x` | Each returns exit 5 (`E_NOT_INTERACTIVE`) before any service call; the chain has no new event (event count before and after is equal); no prompt is read |
| TC-935 | `tc935_typed_and_reason_confirmation` | `src/doctask_cli.rs` | Console fake with answers: `T-1` (approve, exact), `t-1` (approve, not exact), `T-1 ` (close, not exact), reject with empty `--comment` (no prompt), cancel with reason and `y`, cancel with reason and `n`, dismiss with reason and `Y` | Exact `T-1` proceeds; `t-1` and `T-1 ` abandon with `written:false` and no event; empty reason is `E_INVALID_ARG` (exit 4) with no prompt; `y` proceeds; `n` abandons; `Y` (capital) abandons because the answer must be the lowercase `y` |
| TC-936 | `tc936_plan_new_approve_reject_analysis_only` | `src/doctask_cli.rs` | Workspace with one file and baseline; `new T-*` through `run_doc` with `plan new --summary s`; `approve` with typed handle; second task with `plan new` and `reject --comment` with `y` | Plan is recorded with `plan_no` 1 and the analysis-only edit absent; approve with the plan hash from the inbox moves the task to `working`; reject moves the second task to `open` (the transition the flow defines); no edit-plan support is required (the command runs with no `EditPlanSupport` registered) |
| TC-937 | `it_061_legacy_annotate_task_lines` | `tests/it_doctask_cli.rs` | `Cli::try_parse_from(["ladon","--root",R,"annotate","a.md","--section","ROOT","reviewed"])` and the same for `task` with a title and `--description`, run through `entry::run` with a fake host; `annotate` with section `Nope` | First stdout line starts with `annotation id: ` and `task id: `; `task` writes the `任务编号 T-1，查看: ladon doc show T-1` line to standard error through the `DocIo` used by the helper; the unknown section annotation is stored with anchor state `unchecked` (read back through `DocStore`) |
| TC-938 | `it_938_entry_arms_are_single_calls` | `tests/it_doctask_cli.rs` | Source text of `src/entry.rs` read through `CARGO_MANIFEST_DIR`; the bodies of the `Command::Doc`, `Command::Annotate` and `Command::Task` arms | Each arm is one call into `doctask_cli`; the bodies contain no `store.`, `SELECT`, `INSERT`, `UPDATE`, `DELETE` or `sql` tokens; `src/main.rs` is unchanged (one call) |
| TC-939 | `it_939_one_shot_flow_in_process` | `tests/it_doctask_cli.rs` | Init a workspace; `file` with two sections; `baseline add` (confirmed through the fake console); `note`; `new`; `plan new` (analysis-only); `approve` (typed); `evidence add`; `close` (typed); `verify`; `history`; `list --all`; `show`; `anchors` | Every step exits 0; the close moves the task to `done`; `verify` reports ok; `list --all` shows the task; `show` contains the status `done`; the JSON form of `show` parses |
| IT-060 | `it_060_binary_flow_json_and_pipe_refusal` | `tests/it_doctask_cli_binary.rs` | Built `ladon` binary: `init`, `doc baseline add` with `--json` and `--yes --expect-hash`, `doc new --json`, `doc list --json`, `doc approve` on a pipe | `--json` stdout parses and has `ok` true; the piped `approve` exits 5 with `E_NOT_INTERACTIVE` in the JSON on stdout; no state change after the refused call |
| IT-066 | `it_066_binary_export_and_read_back` | `tests/it_doctask_cli_binary.rs` | Built `ladon`: a completed task (as in TC-939), `doc export nema --out` inside `.ladon/exports`, `doc export report --format json`, `doc export nema` read back through `nema::parse_graph_edges` | The NEMA file parses and `topo_batches` succeeds; the JSON report is parseable and `cutoff_seq` equals the last chain sequence before the export event; the completion status matches the `show` output |

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
| `annotate` and `task` keep positional arguments and first output lines | TC-937 (`it_061_*`) |
| The new `entry.rs` branches are single library calls with no SQL | TC-938 |
| The one-shot flow from `init` to export runs without Hermes | TC-939, IT-066 |

## Boundary & Failure Cases

- Hash prefix of seven characters and of eight characters: seven is refused with `E_INVALID_ARG`, eight is accepted when it matches.
- Task handle `12` and `T-12` address the same task; a handle with a non-hex prefix shorter than eight characters is `E_INVALID_ARG`.
- A confirmation answer of an empty line abandons.
- `--depends` referring to a missing task returns `E_NOT_FOUND`; the new task remains and the output names it (documented behaviour).
- `doc list --limit 0` prints no rows and the next-step line.
- `doc export report --format xml` is refused with `E_INVALID_ARG` before any read.
- `--out` with a device name (`CON`, `NUL`) or a path to an existing directory is refused with `E_INVALID_ARG`.
- `doc watch` (`inbox --watch`) with `wait_refresh` returning false after one iteration prints the inbox once and returns exit 0.

## Regression Scope

- `tests/it_entry_legacy.rs` (the `annotate` and `task` transcripts) and `tests/it_cli_binary_smoke.rs` continue to pass with the stderr line of `task` added to its transcript.
- `tc108`, `tc109` and `it_05` in the store tests keep passing because `Store::add_annotation`, `create_task`, `set_task_status` and `list_tasks` are unchanged.
- The `doctask_service`, `doctask_flow`, `doctask_reconcile` and `doctask_report` unit tests are not changed by this package.
