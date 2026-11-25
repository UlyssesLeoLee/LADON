# Test Design: `src/main.rs`

Source: `src/main.rs`

Source-SHA256: `be36fd70a9fa4dfee00393676b65d8f955f72229fd647651a8d8c9f62086999d`

## Test Strategy

The binary is tested as a process. `tests/it_entry_legacy.rs` runs the built `ladon` binary (`CARGO_BIN_EXE_ladon`) in temporary workspaces and compares the status, stdout and stderr of every legacy command with its recorded transcript. `tests/it_cli_binary_smoke.rs` runs the binary for the version and help output and for a first workspace, checks that `main.rs` holds exactly one call, and checks that the entry point is reachable from the library. The command logic itself is tested in process through `src/entry.rs`; the in-process cases TC-1027 to TC-1029 are listed in `.quality/test-design/src/entry.rs.test.md`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1020 | `init`, `scan`, `capture`, `files`, `history`, `verify` and `diff` in a temporary workspace. | The transcript of status, stdout and stderr equals the recorded transcript after normalisation. | tests/it_entry_legacy.rs::legacy_workspace_lifecycle |
| TC-1021 | `ignore-check`, `grep`, `tag-folder` without AI, `tags`, `annotate`, `task`, `plan` and `parallel` in a temporary workspace. | The transcript equals the recorded transcript after normalisation. | tests/it_entry_legacy.rs::legacy_queries_and_records |
| TC-1022 | `agent-capabilities`, `agent-list`, `sensitive-add`, `ignore-check` after the rule, `file-space` and `tag-folder` without `--endpoint` and `--model`. | The transcript equals the recorded transcript after normalisation. | tests/it_entry_legacy.rs::legacy_agent_sensitivity_and_tagging |
| TC-1023 | `files` and `tag-folder` before `init`; `history`, `verify` and `annotate` of a missing file; `diff`, `restore` (with and without `--approve`) and `archive` of an unknown version; `archive` with an unknown provider; an unknown subcommand. | Each stderr and non-zero status equals the recorded transcript; no file is created outside the workspace. | tests/it_entry_legacy.rs::legacy_error_paths |
| TC-1024 | `google-login` with `LADON_GOOGLE_CLIENT_ID` unset, with and without a settings folder; `google-logout` with a settings folder and without one; `watch` stopped after its start line. | Messages and statuses equal the recorded transcript; no network call is made. | tests/it_entry_legacy.rs::legacy_google_offline_and_watch_start |
| TC-1025 | `--version`, `--help`, and `init`, `scan` and `files` in a temporary workspace through the binary; the source of `src/main.rs` is read. | Version text equals the package version; help lists every legacy command; the statuses are 0; `main.rs` has exactly one `main_entry()` call and no `tokio`, `clap` or other `fn` than `main`. | tests/it_cli_binary_smoke.rs::binary_smoke_version_help_and_workspace |
| TC-1025 | The source of `src/main.rs` checked for its statements. | The file has one `fn main` and one call of `ladon_explorer::entry::main_entry()`. | tests/it_cli_binary_smoke.rs::binary_smoke_main_is_one_call |
| TC-1026 | A function pointer of type `fn() -> anyhow::Result<()>` is assigned from `ladon_explorer::entry::main_entry`. | The test target compiles; the entry point is reachable from the library crate. | tests/it_cli_binary_smoke.rs::binary_smoke_entry_is_reachable |

## Acceptance Traceability

| Acceptance item (WBS 3.2 WP00C) | TC |
| --- | --- |
| Every command's output and exit behaviour equals the characterization record | TC-1020, TC-1021, TC-1022, TC-1023, TC-1024 |
| `main.rs` holds one call and is covered by the binary smoke test | TC-1025, TC-1026 |
| `entry.rs` covers the injected seams and the command logic in process | TC-1027, TC-1028, TC-1029 |
| `Cargo.toml` and both `Cargo.lock` files are unchanged | checked by `git diff --stat`, not by a test |
| The crate and its test targets compile with `entry` declared | TC-1026 |

## Boundary & Failure Cases

- An unknown version or file produces the error text recorded in the transcripts, on stderr with a non-zero status.
- `watch` is started and stopped by the test; its start line is compared, and its events are covered by TC-1027.
- `shell-menu` and `shell-host-internal` run only on Windows. Their Windows behaviour is covered by the manual system test ST-088; the branches are covered in process by TC-1029.

## Regression Scope

Every command of the `ladon` binary, the global `--root` option, the clap help and version output, and the exit status of each path.
