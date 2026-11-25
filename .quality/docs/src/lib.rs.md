# Technical Design: `src/lib.rs`

Source: `src/lib.rs`

Source-SHA256: `9e7cc01271e6eb3e0627815e290346f315e20b8292dca31a35f098cbd55d8c57`

## Responsibility

`lib.rs` declares the modules of the kernel crate `ladon-explorer`. It contains no logic.

## Dependencies

Each module listed under Public Contract is a file under `src/`. No module depends on `lib.rs` for behaviour.

## Public Contract

The public modules are `archive`, `crypto`, `diff`, `engine`, `entry`, `google_oauth`, `model`, `nema`, `policy`, `provider`, `search`, `store`, `tagging`, `workspace`, `shell`, `auth`, `agent_api`, `storage_rules`, `cloud_journal`, `ui_layout`, `emergence`, `doctask_model`, `doctask_outline` and `doctask_store`.

`doctask_model` is the pure model of the document-task module; its contract is in `doctask_model.rs.md`. `doctask_outline` is the pure clause outline, anchor classification and byte-level section splice of the same module; its contract is in `doctask_outline.rs.md`. `doctask_store` is the persistence layer of the same module, with immediate transactions, counters, the governance chain and the legacy backfill; its contract is in `doctask_store.rs.md`.

`doctask_service` is the service core of the document-task module; its contract is in `doctask_service.rs.md`. `doctask_flow` is the workflow layer of the same module (transitions, approvals, proposals, dependencies, links, evidence, quotas, inbox, schedule and verification); its contract is in `doctask_flow.rs.md`.

`entry` is the command entry of the `ladon` program. Its contract is in `entry.rs.md`. `google_oauth` is the module that signs in to Google Drive; its contract is in `google_oauth.rs.md`.

## Change Notes

- `entry` is declared. It holds the command logic of the `ladon` binary.
- `google_oauth` is declared and is public so that the command-line program and the window can call it.
- `doctask_model` is declared. It has no dependency on `store` or `engine`, so it compiles and tests without the database layer.
- `doctask_outline` is declared. It depends only on `doctask_model`, so it compiles and tests without the database layer.
- `doctask_store` is declared. It is the only document-task module that writes the new tables; it depends on `store` and `doctask_model`.
- `doctask_service` is declared. It depends on `engine`, `store`, `doctask_store`, `doctask_outline` and `doctask_model`; the command entry, the console and the MCP layer call it.
- `doctask_reconcile` is declared. It depends on `engine`, `doctask_store`, `doctask_outline`, `doctask_model` and `doctask_service`; the decision flow, the command entry and the console call it.
- `doctask_flow` is declared. It depends on `engine`, `doctask_service`, `doctask_store`, `doctask_outline` and `doctask_model`; the command entry, the console and the MCP layer call it.
- `doctask_render` is declared. It is pure: it depends only on `doctask_model`, so it compiles and tests without the database layer; the command entry and the console call it for every screen and error text.
- `doctask_cli` is declared. It depends on `engine`, `doctask_service`, `doctask_flow`, `doctask_reconcile`, `doctask_store`, `doctask_outline`, `doctask_render`, `doctask_report` and `doctask_model`; the command entry calls it for `doc`, `annotate` and `task`. Its contract is in `doctask_cli.rs.md`.
- `doctask_report` is declared. It is pure: it depends only on `doctask_model` and `nema`, and builds the Markdown and JSON audit reports and the `.NEMA` export; the command entry calls it and writes the returned text.
