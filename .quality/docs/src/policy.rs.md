# Technical Design: `src/policy.rs`

Source: `src/policy.rs`

Source-SHA256: `e0eecd499ed2901d0c88b9340c3de348088b6a2a88598ee76a4ace3a00b9c21a`

## Responsibility

`LadonPolicy` decides which workspace paths may leave the machine. It builds one gitignore-style matcher from the `.LADONignore` rule files of a workspace (plus an optional global rule file) and answers whether a path is excluded from external data transfer. Local history capture is not restricted by it; only transfers to external providers, remote AI endpoints and similar sinks are. `add_sensitive` appends an exact, root-anchored rule for an existing file or directory to the root `.LADONignore`. A rule affects later transfers only; it does not recall data that was already uploaded.

## Dependencies

- `ignore` crate (`GitignoreBuilder`, `Gitignore`) for rule parsing and matching, `walkdir` to find rule files without following links, `anyhow` for errors, `std::fs` and `std::env`.
- The optional environment variable `LADON_GLOBAL_IGNORE` names one extra rule file.
- Callers: `engine::Engine` (load at init and open, `reload_policy`, `excluded` during capture, `allow_external` before archiving), `agent_api::AgentApi` (`allow_remote_read`), and the `ladon` CLI (`sensitive-add`, `ignore-check`, folder tagging, the Shell host).

## Public Contract

- `LadonPolicy::load(root) -> Result<LadonPolicy>`: canonicalises the root (error `workspace not found` when it does not exist). Rules come from the file named by `LADON_GLOBAL_IGNORE` when that variable names an existing file, and from every file called `.LADONignore` found under the root by a walk that skips `.ladon` and `.git` and does not follow links. A rule file that cannot be parsed makes `load` fail with that error.
- `excluded(path) -> Result<bool>`: a relative path is joined to the canonical root; an absolute path is used as given and must lie under the canonical root, otherwise the error is `outside workspace`. On Windows the canonical root has the extended-length form, so absolute arguments must be built from the canonicalised root. A path with a `.ladon` or `.git` component is always excluded, a file named `.LADONignore` is always excluded, and otherwise the matcher decides, considering the path's parent directories and whether the path is a directory on disk.
- `allow_external(path) -> Result<()>`: succeeds unless the path is excluded, in which case the error is `LADON policy denies external data transfer: {path}`; it also propagates the `outside workspace` error.
- `add_sensitive(root, target) -> Result<PathBuf>`: canonicalises the root and the target (a relative target is taken relative to the root). Errors: `cannot exclude resource outside workspace` when the target lies outside the root, `invalid ignore target` for the root itself, anything under `.ladon/` and `.LADONignore`, and the file-system error when the target does not exist. The rule is `/` followed by the root-relative forward-slash path, with the characters `*`, `?`, `[`, `]`, `!`, `#` and `\` escaped by a backslash and a trailing `/` for directories. Unless that exact rule line is already present, a comment line `# LADON sensitive: exclude remote transfers` and the rule are appended to the root `.LADONignore` (created when missing) by writing a temporary file and renaming it. The result is the path of the root `.LADONignore`.

## Change Notes

Unit tests that pass absolute paths to `excluded` derive them from the canonicalised workspace root, which is the form the policy compares against, so the tests behave the same on Windows and on Unix. The contract for absolute paths is the one given under Public Contract. The remaining behaviour of the module is as specified in this document.
