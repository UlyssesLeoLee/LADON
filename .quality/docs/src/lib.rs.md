# Technical Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `5019b4afbbb718bbe2e27e81017f12919deb3625feba52bc000a852737e72d6b`

## Responsibility

`lib.rs` declares the modules of the kernel crate `ladon-explorer`. It contains no logic.

## Dependencies

Each module listed under Public Contract is a file under `src/`. No module depends on `lib.rs` for behaviour.

## Public Contract

The public modules are `archive`, `crypto`, `diff`, `engine`, `entry`, `google_oauth`, `model`, `nema`, `policy`, `provider`, `search`, `store`, `tagging`, `workspace`, `shell`, `auth`, `agent_api`, `storage_rules`, `cloud_journal`, `ui_layout` and `emergence`.

`entry` is the command entry of the `ladon` program. Its contract is in `entry.rs.md`. `google_oauth` is the module that signs in to Google Drive; its contract is in `google_oauth.rs.md`.

## Change Notes

- `entry` is declared. It holds the command logic of the `ladon` binary.
- `google_oauth` is declared and is public so that the command-line program and the window can call it.
