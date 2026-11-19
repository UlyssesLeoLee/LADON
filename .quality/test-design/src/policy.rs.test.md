# Test Design: `src/policy.rs`

Source: `src/policy.rs`

Source-SHA256: `e0eecd499ed2901d0c88b9340c3de348088b6a2a88598ee76a4ace3a00b9c21a`

## Test Strategy

Unit tests inside `src/policy.rs` cover the basic matching, the always-excluded rule file, the outside-workspace error and `add_sensitive` with a relative path. Integration tests in `tests/it_policy.rs` cover the remaining branches of `add_sensitive`, `excluded`, `allow_external` and `load` with real files in a `tempfile::tempdir`, with no mocks. Tests that pass absolute paths build them from the canonicalised temporary directory, because the policy root is canonical (extended-length form on Windows). The global rule file is read from a process-wide environment variable, so its test lives alone in `tests/it_policy_global.rs`; each integration test file runs in its own process, so the variable cannot race with other tests. All other tests assume `LADON_GLOBAL_IGNORE` is unset.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-501 | Rule file with `*.pem` and `private/`; absolute paths under the canonical root for x.pem, private/x.md and public.txt | The first two are excluded and public.txt is not | src/policy.rs::ut_policy_01_ignore_and_allow |
| TC-502 | Empty rule file; the absolute canonical path of .LADONignore | The rule file itself is excluded | src/policy.rs::ut_policy_02_rule_is_ignored |
| TC-503 | An absolute path that is not below the workspace | Error from excluded | src/policy.rs::ut_policy_03_outside_workspace |
| TC-504 | add_sensitive for a relative file name containing a space, then reload the policy | The relative path is excluded after the reload | src/policy.rs::ut_policy_04_add_sensitive |
| TC-505 | add_sensitive twice for doc.md in a workspace without a rule file, with another doc.md in a subdirectory | The rule file is created, has the comment line and exactly one /doc.md rule, the returned path is the rule file, and the subdirectory file is not excluded | tests/it_policy.rs::tc505_add_sensitive_writes_anchored_rule_once |
| TC-506 | add_sensitive for a directory named private | The rule is /private/ and files below the directory are excluded while unrelated files are not | tests/it_policy.rs::tc506_add_sensitive_directory_rule |
| TC-507 | add_sensitive for a file whose name contains brackets, a hash and an exclamation mark | The written rule escapes each special character, that exact file is excluded and a similar plain name is not | tests/it_policy.rs::tc507_add_sensitive_escapes_glob_characters |
| TC-508 | add_sensitive for the root, for .LADONignore, for a file under .ladon, for a file outside the workspace and for a missing file | Errors invalid ignore target for the first three, cannot exclude resource outside workspace for the outside file, and an error for the missing file; the rule file is not changed | tests/it_policy.rs::tc508_add_sensitive_rejects_invalid_targets |
| TC-509 | excluded for .git/config, .ladon/meta.turso and nested/.git/x with no rule file | All three are excluded | tests/it_policy.rs::tc509_internal_directories_always_excluded |
| TC-510 | allow_external for a normal file and for a .pem file under the default style rules | Ok for the normal file; error containing denies external data transfer for the .pem file | tests/it_policy.rs::tc510_allow_external_follows_rules |
| TC-511 | A rule file containing an unterminated character class | load returns an error | tests/it_policy.rs::tc511_invalid_rule_file_fails_load |
| TC-512 | load on a directory that does not exist | Error containing workspace not found | tests/it_policy.rs::tc512_load_requires_existing_workspace |
| TC-513 | LADON_GLOBAL_IGNORE names a rule file with a secret extension; then a non-existent file; then the variable is removed | The extension is excluded while the variable names the rule file, a non-existent file is ignored without error, and nothing is excluded once the variable is removed | tests/it_policy_global.rs::tc513_global_ignore_file_applies |

## Boundary & Failure Cases

- Absolute paths outside the canonical root fail with the outside workspace error, and relative paths are always interpreted below the root (TC-503, TC-504).
- A second `add_sensitive` for the same target does not duplicate the rule (TC-505).
- The rule is anchored at the root, so a same-named file in a subdirectory is not excluded (TC-505).
- Rule characters in file names are escaped, so only the exact file matches (TC-507).
- Targets that would exclude the rule file, the internal directory or the whole workspace are refused (TC-508).
- Paths below `.git` and `.ladon` are excluded even without any rule (TC-509).
- A malformed rule file makes loading fail instead of silently dropping rules (TC-511).

## Regression Scope

`engine.rs` (capture sensitivity, archive transfer checks, `reload_policy`), `agent_api.rs` (`allow_remote_read`) and the CLI `sensitive-add` and `ignore-check` commands depend on this module; `tests/it_engine.rs` TC-308 and TC-314, `tests/it_history.rs` it_02 and `tests/it_agent_folder.rs` TC-407 must stay green.
