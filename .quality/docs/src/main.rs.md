# Technical Design: `src/main.rs`

Source: `src/main.rs`

Source-SHA256: `82164e22b0ac071d5cdd9b8391d7503023cb76fbbb2ee1ce2c911e6c8607f6b1`

## Responsibility

`main.rs` is the binary entry point of the command-line program `ladon`. It contains one call, `ladon_explorer::entry::main_entry()`, and returns its result. Parsing, dispatch and every command are implemented in `src/entry.rs`, which is the library module behind this file.

## Dependencies

`ladon_explorer::entry` (the command entry), and `anyhow` for the `Result` type returned from `main`.

## Public Contract

The binary is named `ladon`. `main` returns `anyhow::Result<()>`: an error returned by the entry is printed by the Rust runtime as `Error: ...` and the process exits with status 1. A failure that the entry has already printed exits with the code carried by `Reported`, as defined in `entry.rs.md`.

The command set, its arguments, its output and its exit statuses are defined in `entry.rs.md` and are not repeated here.

## Change Notes

- `main.rs` contains no command logic and no runtime set-up. The runtime is built by `entry::main_entry`.
- The `clap` definitions and the command dispatch are in `src/entry.rs`.
