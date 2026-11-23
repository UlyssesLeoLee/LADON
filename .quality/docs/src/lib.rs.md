# Technical Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `f4a2863a4354a2bc723789854fa79e8b66d3f666fc48d24e5239ac9748bf43c6`

## Responsibility

`lib.rs` declares the modules of the kernel crate `ladon-explorer`. It contains no logic.

## Public Contract

The public modules are `archive`, `crypto`, `diff`, `engine`, `google_oauth`, `model`, `nema`, `policy`, `provider`, `search`, `store`, `tagging`, `workspace`, `shell`, `auth`, `agent_api`, `storage_rules`, `cloud_journal`, `ui_layout` and `emergence`.

`google_oauth` is the module that signs in to Google Drive. Its contract is in `google_oauth.rs.md`.

## Dependencies

Each module listed under Public Contract is a file under `src/`. No module depends on `lib.rs` for behaviour.

## Change Notes

`google_oauth` is declared. The module is public so that the command-line program and the window can call it.
