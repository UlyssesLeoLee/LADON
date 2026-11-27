# Technical Design: `src/policy.rs`

Source: `src/policy.rs`

Source-SHA256: `119315605162fc528e8490d39e7af4294d49f41e1ea5d72c470d2c19c97119b7`

## Responsibility

`LadonPolicy` decides which workspace paths may leave the machine. It builds one gitignore-style matcher from the `.LADONignore` rule files of a workspace (plus an optional global rule file) and answers whether a path is excluded from external data transfer. Local history capture is not restricted by it; only transfers to external providers, remote AI endpoints and similar sinks are. `add_sensitive` appends an exact, root-anchored rule for an existing file or directory to the root `.LADONignore`. A rule affects later transfers only; it does not recall data that was already uploaded.

## Dependencies

- `ignore` crate (`GitignoreBuilder`, `Gitignore`) for rule parsing and matching, `walkdir` to find rule files without following links, `anyhow` for errors, `std::fs`, `std::env` and `std::time::SystemTime`, and `crypto::digest` for the content digest of the rule-file stamp.
- The optional environment variable `LADON_GLOBAL_IGNORE` names one extra rule file.
- Callers: `engine::Engine` (load at init and open, `reload_policy`, `excluded` during capture, `allow_external` before archiving), `agent_api::AgentApi` (`allow_remote_read`), and the `ladon` CLI (`sensitive-add`, `ignore-check`, folder tagging, the Shell host).

## Public Contract

- `LadonPolicy::load(root) -> Result<LadonPolicy>`: canonicalises the root (error `workspace not found` when it does not exist). Rules come from the file named by `LADON_GLOBAL_IGNORE` when that variable names an existing file, and from every file called `.LADONignore` in any ASCII case found under the root by a walk that skips `.ladon` and `.git` and does not follow links. A rule file that cannot be parsed makes `load` fail with that error.
- `excluded(path) -> Result<bool>`: a relative path is joined to the canonical root; an absolute path is used as given and must lie under the canonical root, otherwise the error is `outside workspace`. On Windows the canonical root has the extended-length form, so absolute arguments must be built from the canonicalised root. A path that contains a `..` component fails with `outside workspace`. A path with a `.ladon` or `.git` component is always excluded, a file named `.LADONignore` (any ASCII case) is always excluded, and otherwise the matcher decides, considering the path's parent directories and whether the path is a directory on disk.
- `allow_external(path) -> Result<()>`: succeeds unless the path is excluded, in which case the error is `LADON policy denies external data transfer: {path}`; it also propagates the `outside workspace` error.
- `add_sensitive(root, target) -> Result<PathBuf>`: canonicalises the root and the target (a relative target is taken relative to the root). Errors: `cannot exclude resource outside workspace` when the target lies outside the root, `invalid ignore target` for the root itself, anything under `.ladon/` and `.LADONignore`, and the file-system error when the target does not exist. The rule is `/` followed by the root-relative forward-slash path, with the characters `*`, `?`, `[`, `]`, `!` and `#` escaped by a backslash, path separators (including Windows backslashes) normalised to `/`, and a trailing `/` for directories. Unless that exact rule line is already present, a comment line `# LADON sensitive: exclude remote transfers` and the rule are appended to the root `.LADONignore` (created when missing) by writing a temporary file and renaming it. The result is the path of the root `.LADONignore`.

- `LadonPolicy::check_root(root) -> Result<()>`: canonicalises the root with the same error as `load`, parses the file named by `LADON_GLOBAL_IGNORE` when that variable names an existing file, and parses the root `.LADONignore` when it exists. No other file is read, so its cost does not depend on the size of the tree. A parse error is returned as `load` returns it.
- `PendingPolicy`: a rule set loaded on a background thread. `PendingPolicy::start(root) -> PendingPolicy` spawns one thread that runs `LadonPolicy::load(root)` and keeps its result. A load that fails keeps the error message. When the thread cannot be started, the stored result is the message `rule scan could not start`. `start` returns at once.
- `PendingPolicy::with<R>(&self, f: impl FnOnce(&LadonPolicy) -> Result<R>) -> Result<R>`: blocks until the load has finished. A failed load returns an error whose message is the stored message. A successful load runs `f` on the rule set. Every call sees the same stored result.
- `PendingPolicy::ready(result: Result<LadonPolicy>) -> PendingPolicy`: holds the result of a load that has already finished, so `with` and `is_ready` answer at once. An error is stored as its message `{err:#}`, and `with` returns it as `start` does.
- `PendingPolicy::is_ready(&self) -> bool`: true once the load has finished, whether it succeeded or failed. A poisoned internal lock also reports true, so the caller installs the policy, and `with` then fails with `rule scan lock poisoned`, which keeps transfers refused.
- `global_rule_path() -> Option<PathBuf>` (crate-internal): the path named by `LADON_GLOBAL_IGNORE`, when the variable is set. `load`, `check_root` and `RuleStamp::read` take the global rule file from it.
- `RuleStamp` (crate-internal) with `RuleStamp::read(root) -> RuleStamp`: records, for each root rule file (any ASCII case, sorted by path) and for the global rule file, its path, its modification time, its byte length and the SHA-256 digest of its content (`crypto::digest`). A file that is missing or cannot be read has no time, length or digest. Two stamps are equal when none of these differ. Nested rule files are not part of the stamp, because they are not read on every call; `Engine` discovers them through a background traversal.
- The rules of a workspace are workspace-wide. The `ignore` crate compiles the globs of each rule file relative to the workspace root, not relative to the directory that holds the file, so a `.LADONignore` in a subdirectory can match any path in the workspace. For this reason `load` still reads every rule file under the root, and `Engine` runs that read on a `PendingPolicy` rather than before returning from `open` or `init`.

## Change Notes

Unit tests that pass absolute paths to `excluded` derive them from the canonicalised workspace root, which is the form the policy compares against, so the tests behave the same on Windows and on Unix. The contract for absolute paths is the one given under Public Contract. The remaining behaviour of the module is as specified in this document.

`add_global_rule_file` reads the global path through `global_rule_path`; its behaviour is unchanged. `RuleStamp` and the readiness functions exist for the engine's rule freshness check (`Engine::refresh_policy_if_stale`) and are not part of the public transfer decision.
