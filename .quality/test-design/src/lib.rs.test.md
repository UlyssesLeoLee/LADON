# Test Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `f4a2863a4354a2bc723789854fa79e8b66d3f666fc48d24e5239ac9748bf43c6`

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
