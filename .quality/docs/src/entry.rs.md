# Technical Design: `src/entry.rs`

Source: `src/entry.rs`

Source-SHA256: `b35770920b262a5d258b2271ae68bc669051c81a34a175ac6c68efe8cab9aa60`

## Responsibility

`entry.rs` is the command entry of the `ladon` program. It holds the `clap` definitions (`Cli`, `Command`), the dispatch of every command and the command logic that is not part of the kernel: `watch` with its batch handling, `google-login`, `google-logout`, `tag-folder`, `annotate`, `task`, `shell-menu`, `shell-host-internal` and the record lookup `resolve_record`. Every side effect that tests must control is reached only through the `Host` trait.

`main.rs` contains one call, `main_entry()`. No command logic is outside this module.

## Dependencies

- `clap` (argument parsing), `tokio` (multi-thread runtime built by `main_entry`), `async-trait` (the `Host` trait), `notify` (the filesystem watcher behind `SystemHost`), `walkdir` (`tag-folder`), `serde_json` (JSON output), `similar` (`diff`), `anyhow` (errors).
- Kernel modules: `engine` (`Engine`, `normalise_relative`), `google_oauth`, `model` (`FileRecord`), `nema`, `policy`, `provider`, `search`, `shell`, `tagging`, `workspace` (`FileSpace`), `agent_api`. `diff` is used through `engine`.

## Public Contract

### Command line

`Cli` has the global option `--root <PATH>` (default `.`) and one subcommand `cmd`. The subcommand `doc` is `Command::Doc(DocArgs)`; its argument tree, options and exit codes are defined in `doctask_cli.rs`, and `entry.rs` only forwards it. The program name is `ladon` and the version is the package version. Subcommands keep their names, arguments, defaults and help text. `shell-host-internal` is hidden from the help output.

`shell-menu` and `shell-host-internal` open no workspace (`shell-host-internal` opens one only for `MarkSensitive`). `init`, `google-login`, `google-logout` and `mcp-serve` open none either. Every other command calls `Engine::open(--root)` before its own work.

`mcp-serve` is `Command::McpServe { launch: String, global_ignore: Option<PathBuf> }`, with `--launch` required and `--global-ignore` optional; `--root` is the global option. Its behaviour is in `doctask_mcp.rs.md`.

### Output and exit behaviour

- Output is written with `println!` and `eprintln!` in the forms recorded by `tests/it_entry_legacy.rs`.
- A returned error is printed by the Rust runtime as `Error: ...` and the status is 1.
- A failure that has already been printed is returned as `Reported { code }`. `main_entry` exits the process with `code` and prints nothing more. The shell host failure prints `{e:#}` on stderr and returns `Reported { code: 1 }`.

### Seams

```rust
pub trait WatchSource {
    fn recv(&mut self) -> Option<notify::Result<notify::Event>>;     // None when the watcher disconnected
    fn try_recv(&mut self) -> Option<notify::Result<notify::Event>>; // None when no event is queued
}

#[async_trait(?Send)]
pub trait Host: Sync {
    fn start_watch(&self, root: &Path) -> anyhow::Result<Box<dyn WatchSource>>;
    fn stop_requested(&self) -> bool { false }
    fn open_browser(&self, url: &str);
    fn settings_dir(&self) -> Option<PathBuf>;
    async fn oauth_login<F: Fn(&str) + Sync>(&self, on_url: &F, timeout: Duration) -> anyhow::Result<String>;
    fn save_refresh_token(&self, settings_dir: &Path, refresh_token: &str) -> anyhow::Result<()>;
    fn delete_refresh_token(&self, settings_dir: &Path) -> anyhow::Result<()>;
    async fn ai_tags(&self, endpoint: &str, model: &str, description: &str, filename: &str, preview: &str) -> anyhow::Result<Vec<String>>;
    fn spawn_shell_host(&self, root: &Path, path: &Path) -> anyhow::Result<i32>;
    fn open_in_explorer(&self, path: &Path) -> anyhow::Result<()>;
    fn run_isolated_host(&self, path: &Path) -> anyhow::Result<ShellResult>;
}

pub struct SystemHost;
```

`SystemHost` implements the seams as follows:

- `start_watch` creates a recommended watcher whose handler sends each result into a channel, watches `root` recursively and returns a source that keeps the watcher alive.
- `stop_requested` is the default `false`.
- `open_browser` runs `rundll32.exe url.dll,FileProtocolHandler <url>` on Windows and does nothing elsewhere. The URL is passed as one argument.
- `settings_dir` is `google_oauth::settings_dir()`.
- `oauth_login` reads the client with `OAuthClient::from_env()`, then calls `google_oauth::login` with the five-minute limit and passes the authorization URL to `on_url`.
- `save_refresh_token` and `delete_refresh_token` call the `google_oauth` functions of the same names.
- `ai_tags` calls `tagging::ai_tags`.
- `spawn_shell_host`, `open_in_explorer` and `run_isolated_host` call the functions of `shell` with the same names.

### Entry points

- `pub fn main_entry() -> anyhow::Result<()>`: starts a thread named `ladon-command` with a 32 MiB stack (`COMMAND_STACK_BYTES`) and waits for it. The thread parses the process arguments, builds the multi-thread runtime, runs `run` with `SystemHost` and exits with the code of a `Reported` error. A panic on the thread is resumed on the caller. The stack size is needed because the `clap` command tree of the `doc` subcommands and the command futures exceed the default 1 MiB main-thread stack of Windows in an unoptimised build.
- `pub async fn run<H: Host>(cli: Cli, host: &H) -> anyhow::Result<()>`: runs one parsed command line.
- `pub struct Reported { pub code: i32 }`: a failure that has already been printed. It implements `Display` and `std::error::Error`.

## Behaviour

### Shell menu

`shell-menu <file>` canonicalises the file, calls `spawn_shell_host(root, file)` and, when the returned status is not 0, `open_in_explorer(file)`. It returns `Ok`.

### Shell host (hidden)

`shell-host-internal <file>` canonicalises the file and calls `run_isolated_host`:

- `ExecutedOrDismissed`: returns `Ok` with no output.
- `MarkSensitive`: opens the workspace, calls `policy::add_sensitive(root, file)` and prints `Sensitive rule added for <file>`.
- `ViewHistory`: prints `Use: ladon --root <workspace> history <file> (GUI pane is planned)`.
- `DocumentTask`: prints `Open LADON Document Task editor (GUI integration planned)`.
- `OpenExplorer`: calls `open_in_explorer(file)`.
- error: prints `{e:#}` on stderr and returns `Reported { code: 1 }`.

### Google sign-in and sign-out

- `google-login`: resolves the settings folder; when none is available the error is `no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR`. It then calls `oauth_login` with a callback that prints `Complete the Google Drive sign-in in the browser. Waiting up to five minutes.` and calls `open_browser(url)`. The URL is never printed. On success it saves the token with `save_refresh_token` and prints `Google Drive sign-in saved`.
- `google-logout`: resolves the settings folder in the same way, calls `delete_refresh_token` and prints `Google Drive sign-in removed`. Calling it when no sign-in is stored is not an error.

### Watch

`watch` calls `start_watch(root)`, prints `watching <root> — Ctrl+C stops` and then loops:

1. If `stop_requested()` is true, it returns `Ok(())` without reading the source.
2. It waits for an event. `None` returns the error `watcher disconnected`; an event that carries a notify error returns that error.
3. It collects the paths of the event, waits 350 ms without blocking the runtime (`tokio::time::sleep`) and collects the paths of every queued event. A notify error among the queued events returns that error.
4. It calls `process_batch` with the collected paths.

`process_batch(engine, actor, paths)` handles one batch of paths. Root rule events and rule files in any ASCII case are handled first, so every other path of the batch is captured under the reloaded rules:

- paths below `.ladon` or `.git` are skipped;
- a root rule event or a `.LADONignore` file reloads the policy and prints `policy reloaded: <path>`, or prints `policy reload failed, check external-transfer protections: <error>` on stderr;
- for a path that is a file, `capture(path, actor, "filesystem stable save")`; a failure prints `capture warning <path>: <error>` on stderr;
- for a path that is not a file and lies inside the workspace, the stored record is marked deleted with reason `watcher removal`; a failure prints `delete warning: <error>` on stderr. A store lookup failure is returned;
- when a path needs a full scan, `scan(actor)` runs once and prints `full scan after watch event: scanned <n>`, or `scan warning: <error>` on stderr.

### Tag folder

`tag-folder` refreshes the rules with `Engine::refresh_policy_if_stale` first (a root or global rule file that cannot be loaded fails the command with its error), then walks the workspace (depth 1, or 32 with `--recursive`, without following links and without `.ladon` and `.git`) and, for each tracked file:

- reads at most 16384 bytes and derives heuristic tags;
- when both `--endpoint` and `--model` are given, asks `ai_tags` for more tags with the description `<description>; dimensions: <presets>`, unless `Engine::remote_read_allowed` refuses the file: it is flagged sensitive, its path is excluded by the rules in force, its current content is excluded, or the rules in force are not usable. A refused file is sent only to a local (loopback) endpoint; otherwise the command prints `skipped remote AI tagging for excluded file: <path>` on stderr and sends nothing;
- adds `rule-described` when `--description` is not empty, sorts the tags and removes duplicates;
- stores the tags with source `ai+rules` when both endpoint and model are given, otherwise `heuristic`, and counts the file.

The command prints `tagged <count> file(s)`.

### Ignore check

`ignore-check <file>` refreshes the rules like `tag-folder` (a rule file that cannot be loaded fails the command with its error). It then prints `ALLOWED: subject to explicit archive action` when `Engine::remote_path_allowed` allows the path and `EXCLUDED: local only` otherwise. The label is produced by `ignore_check_label`, so the command reports the same decision that the archive and remote read paths use: a tracked copy of excluded content is reported as `EXCLUDED: local only`.
### Doc, annotate and task

- `doc <…>`: the workspace is opened with `Engine::open(--root)`, then `doctask_cli::run_doc(engine, args, &SystemIo)` runs the command. It returns an exit code; a non-zero code is returned as `Reported { code }`, so `main_entry` exits with that code and prints nothing more. Its output and its errors are described in `doctask_cli.rs.md`.
- `annotate <file> [--section S] <text> [--actor A]`: `doctask_cli::annotate_legacy(engine, actor, file, section, text, &SystemIo)` is the whole branch. It prints `annotation id: <id>` on standard output. A non-zero code is returned as `Reported`.
- `task <file> [--section S] <title> [--description D] [--actor A]`: `doctask_cli::task_legacy(...)` is the whole branch. It prints `task id: <id>` on standard output and `任务编号 T-<n>，查看: ladon doc show T-<n>` on standard error. A non-zero code is returned as `Reported`.

Neither branch contains SQL, store access or any other logic. `--actor` defaults to `human`. The legacy commands keep their positional arguments and the default section `ROOT`.

### MCP server

- `mcp-serve --launch <id> [--global-ignore <path>]`: `run` calls `doctask_mcp::serve(root, launch, global_ignore)` before any `Engine::open`, because the MCP host opens the engine on its first tool call. The returned code is `0` at end of input; a non-zero code is returned as `Reported { code }`. Standard output carries only protocol lines, and standard error carries the diagnostics described in `doctask_mcp.rs.md`.
- The branch holds no logic of its own; the argument `--global-ignore` is passed through unchanged.

## Change Notes

- The external calls of the command logic are reached through `Host`. `SystemHost` is the only implementation that calls the system.
- `Cli` and `Command` are public so that tests can parse arguments in process and `run` accepts a parsed command line.
- A shell host failure is returned as `Reported` instead of ending the process inside the command logic.
- The command logic of the former `main.rs` moved here without a change of output, exit status or argument definitions.
- The `doc` command tree, its confirmation flow and its exit codes are in `doctask_cli`; `entry` holds one branch for `doc` and one branch each for `annotate` and `task`, all calling `doctask_cli`.
- `annotate` and `task` write through the document-task service. Their output lines are unchanged; `task` adds one standard error line with the short handle.
- `mcp-serve` is parsed in `Cli` like the other subcommands. It is the only subcommand that runs before the engine is opened, so an uninitialized workspace is reported by the MCP host as a single stderr line with code `1`, not as an `Engine::open` failure.
