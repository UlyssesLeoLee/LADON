# Test Design: `ui-prototype/src/lib.rs`

Source: `ui-prototype/src/lib.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Test Strategy

`lib.rs` has no logic and no test function. Its check is the build of the library target in both library build lanes, which confirms that the module declarations match the feature set. Both commands run in the `ui-prototype` directory:

- `cargo build --lib --no-default-features`: the model-only library build;
- `cargo build --no-default-features --features kernel --lib`: the library build with the `app` module.

The other verification lanes of the package are:

- model test lane: `cargo test --lib --no-default-features` (unit tests of `model.rs`, TC-601 to TC-615);
- kernel test lane: `cargo test --no-default-features --features kernel` (integration tests of `tests/it_slice_a.rs`, TC-620 to TC-645);
- window build lane: `cargo build --release --bin ladon-gui` (manual Windows 11 tests ST-GUI-01 to ST-GUI-11, TC-650 to TC-660).

TC-600 covers the two library build lanes. It has no test function.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-600 | Command 1: `cargo build --lib --no-default-features`. Command 2: `cargo build --no-default-features --features kernel --lib`. Both run in `ui-prototype` | Both commands finish with exit status 0. Command 1 compiles the model only; command 2 compiles the model and the app | build lane: cargo build --lib --no-default-features; cargo build --no-default-features --features kernel --lib |

## Boundary & Failure Cases

- Without the `kernel` feature the module `app` is absent, so a path that names `ladon_ui_prototype::app` fails at compile time with an unresolved module. This is a build failure, not a pass.
- The default feature set enables `kernel` and `gui`, so both modules are present. That set is covered by the window build lane and the kernel test lane, not by TC-600.

## Regression Scope

The module declarations of this file define the public surface of the library. `src/main.rs` (window build), `tests/it_slice_a.rs` (kernel test lane) and the model unit tests (model test lane) depend on `pub mod model;` and on `pub mod app;` behind the `kernel` feature. The two library build lanes of TC-600 compile these declarations without the `kernel` feature and with it.
