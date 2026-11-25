# Test Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `992b2939c944acab655b30ec7d32f2f744338c1ab4748a89c4079693978b55d4`

## Test Strategy

`lib.rs` has no executable statements. Its modules are compiled by every test target of the crate, so a build of the test targets is the check. The `entry` module is also exercised by `tests/it_entry_legacy.rs`, `tests/it_cli_binary_smoke.rs` and the test module in `src/entry.rs`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1026 | Build of all test targets with `cargo test --locked --no-run`. | The crate compiles with the `entry` module declared and `main_entry` is reachable from the library. | tests/it_cli_binary_smoke.rs::binary_smoke_entry_is_reachable |
| TC-830 | Build of the library with the `doctask_model` declaration present, and the model's own status and boundary tests. | The crate compiles with `doctask_model` declared, and the module is reachable as `ladon_explorer::doctask_model`. | src/doctask_model.rs::tests::tc830_status_strings_match_legacy_seven |
| TC-860 | Build of the library with the `doctask_store` declaration present, and the store's rollback and chain tests. | The crate compiles with `doctask_store` declared, and the module is reachable as `ladon_explorer::doctask_store`. | tests/it_doctask_store.rs::it_049_second_statement_failure_rolls_back_first |
| TC-840 | Build of the library with the `doctask_outline` declaration present, and the outline module's key-equality test. | The crate compiles with `doctask_outline` declared, and the module is reachable as `ladon_explorer::doctask_outline`. | src/doctask_outline.rs::tests::ut_147_outline_keys_match_section_hashes |
| TC-880 | Build of the library with the `doctask_flow` declaration present, and the flow integration tests. | The crate compiles with `doctask_flow` declared, and the module is reachable as `ladon_explorer::doctask_flow`. | tests/it_doctask_flow.rs::tc880_every_legal_edge_writes_its_events |
| TC-900 | Build of the library with the `doctask_reconcile` declaration present, and the reconciliation integration tests. | The crate compiles with `doctask_reconcile` declared, and the module is reachable as `ladon_explorer::doctask_reconcile`. | tests/it_doctask_reconcile.rs::tc900_renamed_heading_relocates_and_gates_approval |
| TC-870 | Build of the library with the `doctask_service` declaration present, and the service integration tests. | The crate compiles with `doctask_service` declared, and the module is reachable as `ladon_explorer::doctask_service`. | tests/it_doctask_service.rs::tc870_create_requires_baseline_and_pins_base_version |
| TC-910 | Build of the library with the `doctask_render` declaration present, and the render module's width and table tests. | The crate compiles with `doctask_render` declared, and the module is reachable as `ladon_explorer::doctask_render`. | src/doctask_render.rs::tests::tc910_display_width_and_padding |

## Boundary & Failure Cases

- None; the file has no logic.

## Regression Scope

Every module of the crate.
