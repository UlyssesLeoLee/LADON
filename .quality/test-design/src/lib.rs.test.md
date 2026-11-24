# Test Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `83c60ca06de0588a6626d895adc3732be5214e150f4a712a9482a7a347a9330e`

## Test Strategy

`lib.rs` has no executable statements. Its modules are compiled by every test target of the crate, so a build of the test targets is the check. The Google module is also covered by `tests/it_google_oauth.rs`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-812 | Build of all test targets with `cargo test --no-run`. | The crate compiles with the `google_oauth` module declared. | tests/it_google_oauth.rs (compiled as a test target) |

## Boundary & Failure Cases

- None; the file has no logic.

## Regression Scope

Every module of the crate.
