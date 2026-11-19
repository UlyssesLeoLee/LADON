# Technical Design: `src/agent_api.rs`

Source: `src/agent_api.rs`

Source-SHA256: `20b7d5f52db5aa6b29c7e20bcb4158e7126996f0122be8d96dc28192ee18a48a`

## Responsibility

`AgentApi` is the local-first, agent-facing read service over one `Engine`. It lists folders, reads files within a byte limit, answers whether remote transfer of a path is allowed, returns a file's version history with ledger-chain validity, and runs bounded regex search. It deliberately opens no network listener and trusts no caller-supplied tenant identity; authentication belongs to whatever adapter wraps it. `capabilities()` publishes the static capability table with the implementation status of each agent feature.

## Dependencies

- `engine::Engine` (workspace root, path resolution, file lookup), `store::Store` (through the engine: `find_path`, `versions`, `verify_chain`), `policy::LadonPolicy` (external transfer rules), `search::grep` (regex search), `model::VersionRecord`.
- `serde::Serialize` for the response types `ResourceInfo`, `HistoryView`, `ApiCapability`; `anyhow` for errors; `std::fs`.
- Callers: the `agent-list` and `agent-capabilities` CLI commands and IPC or MCP adapters that wrap this service.

## Public Contract

- `capabilities() -> Vec<ApiCapability>` returns 21 entries; each has a name, a status string (`implemented`, `cli-implemented` or `design-only`) and whether approval is required.
- `list_folder(relative, limit) -> Vec<ResourceInfo>`: `limit` must be 1 to 1000 (error `limit must be 1..=1000`). The folder must exist, must not itself be a symbolic link (`symlink folder denied`), must resolve inside the workspace root (`folder escapes workspace`, which also applies to an absolute path outside the root) and must be a directory (`not a directory`). Entries that are symbolic links or junctions are skipped. Entries whose workspace-relative path starts with `.ladon` or `.git`, or equals `.LADONignore`, are hidden. Each result carries the name, kind (`file` or `directory`), size in bytes, forward-slash relative path and the ledger file id when the path is tracked. Reading stops once `limit` entries are collected, and the result is sorted by name using bytewise string order (so `B.md` precedes `a.md`). Entry type and size come from `DirEntry::metadata`, which does not follow symbolic links.
- `read_file_local(relative, max_bytes) -> Vec<u8>`: `max_bytes` must be 1 to 8 MiB (`read limit must be 1..8MiB`); the path is resolved through `Engine::resolve`, which rejects paths outside the workspace; a file larger than `max_bytes` is rejected (`file exceeds caller limit`).
- `allow_remote_read(relative) -> Result<()>` loads the current `.LADONignore` rules and fails with `LADON policy denies external data transfer` for excluded paths.
- `history(relative) -> HistoryView` returns the file id, all versions newest first and whether the hash chain verifies; an untracked path yields `file not found in ledger`.
- `grep_local(pattern, limit)` delegates to `search::grep` over the workspace root.

## Change Notes

`list_folder` obtains each entry's metadata with `DirEntry::metadata()`, so the symbolic-link check, the directory flag and the byte size all come from one non-following query on the directory entry itself. The response shape, filtering rules, limits and error messages are those specified under Public Contract.
