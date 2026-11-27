# Test Design: `src/entry.rs`

Source: `src/entry.rs`

Source-SHA256: `fad375d62c373ac5e685e4f70203da3201f4cc6afe6865d84b610ea22aa8ebe2`

## Test Strategy

The logic of `entry.rs` runs in process through the `Host` seam. The test module `tests` inside `src/entry.rs` supplies a `FakeHost` that scripts watch events, records browser URLs, token writes, AI calls and shell calls, and returns configured results. Its recorded state is kept in `Mutex` fields because `Host` requires `Sync`. Workspaces are temporary directories created with `tempfile` and initialised with `Engine::init`. The watch tests accept the 350 ms pause of the batch loop. The process-level behaviour of every legacy command is covered by `tests/it_entry_legacy.rs` (TC-1020 to TC-1024), which runs the binary and therefore also exercises `SystemHost`. No test opens a browser or starts a Google sign-in.

Acceptance items of WP00C, mapped to test cases:

| Acceptance item | Test case | Input | Assertion |
| --- | --- | --- | --- |
| Watch batches are handled by an independent function | TC-1027 | `process_batch` with a created file, a removed file, a rule file and an ignored path | Captured version, deleted mark and policy reload are observed; the call returns `Ok`. |
| Watch stop condition and event source are injectable | TC-1027 | `watch` with a scripted source | Stop before the first event returns `Ok` and reads nothing; disconnection returns `watcher disconnected`. |
| Browser opener and token store are injectable | TC-1028 | `FakeHost` with login success, login failure and a fixed settings folder | URL goes to `open_browser` only; the token is saved only after success. |
| HTTP client for `tag-folder` is injectable | TC-1029 | `FakeHost` AI tagger with a remote endpoint and a sensitive file | Excluded files never reach the tagger; tags are sorted and unique. |
| Shell host call is injectable | TC-1029 | `FakeHost` returning each `ShellResult` and an error | Each result maps to its message or call; the error maps to `Reported` with code 1. |

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1027 | `process_batch` on a workspace with the paths `notes.txt` (a file created after `Engine::init`), `docs/old.md` (tracked in the store, absent from disk), `.LADONignore` (the root rule file) and `.ladon/ignored` (an ignored path). | `Ok(())`; `notes.txt` has a tracked record afterwards; `docs/old.md` is marked deleted; the ignored path produces no record. | src/entry.rs::tests::process_batch_captures_marks_and_reloads |
| TC-1027 | `watch` with a scripted source: one batch with a created file, then the source reports disconnection. | Returns the error `watcher disconnected`; the created file is tracked. | src/entry.rs::tests::watch_handles_batch_then_reports_disconnect |
| TC-1027 | `watch` with a scripted source whose first event carries `notify::Error::generic("boom")`. | Returns an error whose message is `boom`. | src/entry.rs::tests::watch_returns_notify_error |
| TC-1027 | `watch` with `stop_requested` returning `true` and a source that would fail if read. | Returns `Ok(())`; the source is not read. | src/entry.rs::tests::watch_stops_when_requested |
| TC-1027 | `watch` where `start_watch` fails. | The error of `start_watch` is returned unchanged; the start line is not printed. | src/entry.rs::tests::watch_propagates_start_error |
| TC-1028 | `run` with `google-login`, a settings folder, and `oauth_login` returning `refresh-fixture`. | `Ok(())`; `save_refresh_token` receives the folder and `refresh-fixture`; `open_browser` receives the URL once. | src/entry.rs::tests::google_login_saves_token_after_success |
| TC-1028 | `run` with `google-login` and `oauth_login` failing with `sign-in timed out`. | Returns that error; no token is saved; the URL was opened once before the failure. | src/entry.rs::tests::google_login_failure_saves_nothing |
| TC-1028 | `run` with `google-login` and no settings folder. | Returns the error `no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR`; `oauth_login` is not called. | src/entry.rs::tests::google_login_without_settings_folder |
| TC-1028 | `run` with `google-logout` and a settings folder, twice. | Both return `Ok(())`; `delete_refresh_token` is called twice with the folder. | src/entry.rs::tests::google_logout_is_repeatable |
| TC-1028 | `run` with `google-logout` and no settings folder. | Returns the no-settings-folder error; `delete_refresh_token` is not called. | src/entry.rs::tests::google_logout_without_settings_folder |
| TC-1029 | `tag_folder` with options for `https://ai.example.test/v1`, model `m`, description `rules` and recursion, on a scanned workspace containing `docs/req.md` and `secret.txt`, where `secret.txt` was made sensitive with `add_sensitive` before the scan. The tagger returns `spec`, `requirements` and `spec`. | The tagger is called for `docs/req.md` only; the count is 2 (printed as `tagged 2 file(s)`); the stored tags of `docs/req.md` include `spec` and `requirements` and are sorted and unique; the stored tags of `secret.txt` include `rule-described` and not `spec`. The stderr line `skipped remote AI tagging for excluded file: secret.txt` is written by the same branch and is not asserted in process. | src/entry.rs::tests::tag_folder_skips_excluded_files_for_remote_endpoint |
| TC-1029 | `tag-folder` with `--endpoint` but without `--model`. | The tagger is never called; the output is `tagged <n> file(s)` with heuristic tags only. | src/entry.rs::tests::tag_folder_without_model_never_calls_ai |
| TC-1029 | `shell-menu <file>` with `spawn_shell_host` returning 0, then returning 3. | Returns `Ok(())` both times; `open_in_explorer` is called only for status 3. | src/entry.rs::tests::shell_menu_opens_explorer_on_failed_host |
| TC-1029 | `shell-host-internal <file>` with `run_isolated_host` returning `ExecutedOrDismissed`, `ViewHistory`, `DocumentTask` and `OpenExplorer`. | Each returns `Ok(())`; `OpenExplorer` calls `open_in_explorer` with the canonical path. | src/entry.rs::tests::shell_host_results_are_mapped |
| TC-1029 | `shell-host-internal <file>` with `MarkSensitive` in an initialised workspace. | Returns `Ok(())`; the policy reports the file excluded afterwards. | src/entry.rs::tests::shell_host_mark_sensitive_updates_policy |
| TC-1029 | `shell-host-internal <file>` with `run_isolated_host` returning an error. | Returns `Reported` with code 1. | src/entry.rs::tests::shell_host_error_is_reported_with_code_one |
| TC-1029 | `init` through `run` with `--root <dir>`; then `annotate` and `task` on an untracked file. | `init` creates the workspace (`Ok`); `annotate` and `task` return `Reported` with code 3 (`E_NOT_FOUND`); the error text is printed by `doctask_cli` on standard error. | src/entry.rs::tests::init_and_record_commands_through_run |
| TC-1029 | `Cli::try_parse_from` with no subcommand, and with an unknown option. | Both return clap errors; no host call is made. | src/entry.rs::tests::cli_rejects_missing_and_unknown_arguments |
| TC-938 | Source of the `Doc`, `Annotate` and `Task` arms of `run`/`command`, read in `tests/it_doctask_cli.rs::it_938_entry_arms_are_single_calls`. | Each arm is one call into `doctask_cli`; no SQL token in the arm. | tests/it_doctask_cli.rs::it_938_entry_arms_are_single_calls |

## Change Notes

- `doc` parsing and the `doc` branch are covered by `tests/it_doctask_cli.rs`, `tests/it_doctask_cli_binary.rs` and the unit tests of `src/doctask_cli.rs` (TC-930 to TC-939).
- The `annotate` and `task` branches call `doctask_cli`; their expected outcomes in this table are the ones above.

## Boundary & Failure Cases

- A watch batch with an empty path set captures nothing and runs no full scan.
- A path outside the workspace that is not a file is ignored by the removal branch.
- `Reported` converts to `anyhow::Error` and is found again by `downcast_ref`, so `main_entry` can exit with the code.
- `tag-folder` with `--endpoint` and `--model` but no tracked files returns `tagged 0 file(s)` and makes no call.
- A tag value that the tagger returns twice is stored once.
- A `tag-folder` run over a non-initialised workspace fails in `Engine::open` before any file is read.

## Regression Scope

Every command of `Command`, the clap help text and defaults, the watch loop, Google sign-in and sign-out, `tag-folder`, `annotate`, `task` and the shell host branches. The behaviour of `Engine`, `policy`, `google_oauth`, `tagging` and `shell` is not changed by this module and is covered by their own tests.

## MCP Command Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1030 | `Cli::parse_from(["ladon","--root","R","mcp-serve","--launch","20261125-101010-ab"])`. | `root` is `R`; the command is `McpServe` with the launch id and no global rule file. | src/entry.rs::tests::mcp_serve_arguments_parse |
| TC-1031 | `mcp-serve` with `--global-ignore G` and no `--launch`. | Parsing fails with the clap error for the missing required option. | src/entry.rs::tests::mcp_serve_requires_launch |
| TC-1032 | `run` with `mcp-serve` on a root that has no `.ladon/meta.turso`. | `Err` carrying `Reported { code: 1 }`; no `Engine::open` is attempted. | src/entry.rs::tests::mcp_serve_reports_uninitialized_workspace |

## Remote Gate Command Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1033 | `ignore_check_label` on a tracked copy `shared/a.txt` of excluded content `contracts/a.txt` (rule `/contracts/`), and on an untracked `docs/req.md` that no rule matches. | `EXCLUDED: local only` for the copy; `ALLOWED: subject to explicit archive action` for `docs/req.md`. | src/entry.rs::tests::ignore_check_reports_copy_of_excluded_content_as_local_only |
| TC-1034 | `ignore_check_label` after the root `.LADONignore` has been removed from an initialised workspace. | `Err` whose text names `.LADONignore`; no label is returned. | src/entry.rs::tests::ignore_check_fails_while_root_rule_is_missing |
| TC-1035 | `tag-folder` with `--endpoint` (a remote URL) and `--model`, after the root `.LADONignore` has been removed. | `Err` whose text names `.LADONignore`; the tagger is never called. | src/entry.rs::tests::tag_folder_fails_while_root_rule_is_missing_and_never_calls_ai |
