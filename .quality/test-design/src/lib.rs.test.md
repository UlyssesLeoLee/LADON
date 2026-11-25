# Test Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `266aa4370d27125b44a1b94afa2498316c70e87c5b177671ca14d9240138f815`

## Test Strategy

`lib.rs` has no executable statements. Its modules are compiled by every test target of the crate, so a build of the test targets is the check. The `entry` module is also exercised by `tests/it_entry_legacy.rs`, `tests/it_cli_binary_smoke.rs` and the test module in `src/entry.rs`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1026 | Build of all test targets with `cargo test --locked --no-run`. | The crate compiles with the `entry` module declared and `main_entry` is reachable from the library. | tests/it_cli_binary_smoke.rs::binary_smoke_entry_is_reachable |
| TC-830 | Build of the library with the `doctask_model` declaration present, and the model's own status and boundary tests. | The crate compiles with `doctask_model` declared, and the module is reachable as `ladon_explorer::doctask_model`. | src/doctask_model.rs::tests::tc830_status_strings_match_legacy_seven |
| TC-860 | Build of the library with the `doctask_store` declaration present, and the store's rollback and chain tests. | The crate compiles with `doctask_store` declared, and the module is reachable as `ladon_explorer::doctask_store`. | tests/it_doctask_store.rs::it_049_second_statement_failure_rolls_back_first |
| TC-840 | Build of the library with the `doctask_outline` declaration present, and the outline module's key-equality test. | The crate compiles with `doctask_outline` declared, and the module is reachable as `ladon_explorer::doctask_outline`. | src/doctask_outline.rs::tests::ut_147_outline_keys_match_section_hashes |

## Boundary & Failure Cases

- None; the file has no logic.

## Regression Scope

Every module of the crate.
