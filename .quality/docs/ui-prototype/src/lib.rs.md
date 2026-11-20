# Technical Design: `ui-prototype/src/lib.rs`

Source: `ui-prototype/src/lib.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`lib.rs` is the library root of the `ladon-ui-prototype` package. It declares the public modules and contains no logic. The `model` module is compiled in every feature set. The `app` module is compiled only with the `kernel` feature, because it depends on the LADON kernel crate and on Tokio. The window binary `ladon-gui` and the integration test `tests/it_slice_a.rs` reach the library through these modules.

## Dependencies

- Upstream: none. The file declares modules and uses no items from them.
- Downstream: `model` depends on `chrono`, `similar` and `std`. `app` depends on `model`, on the kernel crate `ladon-explorer` and on `tokio`; the `kernel` feature enables the last two. The window binary (`src/main.rs`, features `kernel` and `gui`) and the integration test (`tests/it_slice_a.rs`, feature `kernel`) consume these modules.
- Features in `ui-prototype/Cargo.toml`: `default = ["kernel", "gui"]`, `kernel = ["dep:ladon-explorer", "dep:tokio"]`, `gui = ["dep:slint"]`. The library code does not reference the `gui` feature.
- The library target is `path = "src/lib.rs"` in `ui-prototype/Cargo.toml`.

## Public Contract

The crate root declares exactly these modules:

- `pub mod model;` in every feature set;
- `pub mod app;` only when the `kernel` feature is enabled.

No other item is declared in `lib.rs`. The contract of each module is in its own design document.

The root builds in two library build lanes:

- `cargo build --lib --no-default-features`: the model only;
- `cargo build --no-default-features --features kernel --lib`: the model and the app.

## Change Notes

`app` sits behind `kernel`, so the model lane builds without the kernel crate and without Tokio. A consumer that uses `app` must enable `kernel`; the default feature set enables it.
