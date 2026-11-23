# Technical Design: `ui-prototype/src/app.rs`

Source: `ui-prototype/src/app.rs`

Source-SHA256: `73888b2cc4f20fd66530bacd0bbf55f5e5b9605b06bf0c1aa28aaca10b538452`

## Responsibility

`app.rs` is the only module of the desktop window that calls the LADON kernel (`ladon_explorer`). It requires the `kernel` feature. It translates one `Command` from the window into kernel operations on the single open workspace of the process and returns exactly one `Reply`. `ListDrives` and `OpenDrive` need no open workspace: `ListDrives` reads the drive letters of the machine, and `OpenDrive` opens a drive root, initialising it first when it holds no `.ladon` database. It enforces the path rules of the window, the command preconditions, and the mapping of every failure to a fixed Simplified Chinese message.

All commands run on one dedicated operating-system thread with a current-thread Tokio runtime. The Turso database and the content store are therefore accessed by one thread only. The module performs browsing, drive listing, recording, version history and text diff. It never calls restore, scan, archive or any storage provider, and it never reads `LADON_ARCHIVE_KEY_HEX` or any token. It renders nothing.

## Dependencies

- Kernel (`ladon_explorer`):
  - `engine::Engine`: async associated functions `open` and `init`; fields `root`, `cas` and `store`; methods `reload_policy` (synchronous), `capture` (async) and `load_version` (async, provider `None`).
  - `archive::CasStore`: `has` and `evict` (synchronous).
  - `store::Store`: `list_files`, `find_path`, `versions`, `version`, `events` and `verify_chain` (async reads).
  - Used by the tests only: `Store::new`, `Store::mark_deleted`, `CasStore::new` and the `cas` field's `evict`.
- Window model (`crate::model`): `check_component`, `validate_relative`, `PathProblem`, `RawEntry`, `EntryKind`, `LedgerRow`, `browse_rows`, `BrowseRow`, `VersionInput`, `version_rows`, `VersionRow`, `diff_lines`, `DiffResult`, `strip_extended_prefix`, `drive_rows`, `DriveRow` and `same_directory`. The types `BrowseRow`, `VersionRow`, `DiffResult` and `DriveRow` implement `Debug`, which `Reply` requires.
- `chrono::{FixedOffset, Local}`: the offset argument of `browse_rows` and `version_rows` is the local offset read when the command runs.
- Standard library and Tokio: `runtime::Builder::new_current_thread` with `enable_all`, `std::sync::mpsc`, `std::thread`, `std::panic::catch_unwind`, `std::fs`, `std::time`, `std::path::{Component, Prefix}`, and `std::os::windows::fs::MetadataExt` for the hidden, system and reparse-point attributes.
- Consumers: `main.rs` creates one `Worker`, sends `Command` values and applies each `Reply` on the user-interface thread. The window's start page, This PC, shows the rows of the `Drives` reply. `tests/it_slice_a.rs` calls `handle` and `Worker`.

## Public Contract

### Constants

- `DIFF_LIMIT_BYTES: u64 = 2 * 1024 * 1024`: the largest version content loaded for a text diff, in bytes.
- `DIFF_MAX_LINES: usize = 4000`: the most diff lines kept in one reply.
- `DIFF_TIMEOUT: Duration = Duration::from_secs(2)`: the time budget of one text diff.
- `LIST_LIMIT: usize = 5000`: the most folder rows kept in one reply.

### Types

- `pub enum Command` (derives `Debug, Clone, PartialEq, Eq`):
  - `Open { path: String }`: opens an initialised workspace at an absolute directory path.
  - `Init { path: String, confirmed: bool }`: initialises an absolute directory path as a workspace.
  - `OpenDrive { path: String }`: opens a drive root as the workspace. An initialised root opens as `Open` does. A root that holds no `.ladon/meta.turso` is initialised first, without a confirmation, and then opened.
  - `ListFolder { relative_dir: String }`: lists one folder. The empty string is the workspace root.
  - `Capture { relative_path: String }`: records the current content of one file as actor `human` and reason `gui-capture`.
  - `LoadHistory { relative_path: String }`: loads the versions and chain status of one file.
  - `LoadDiff { older_version_id: String, newer_version_id: String }`: computes a line diff between two versions of one file.
  - `ListDrives`: lists the drive roots of the machine. It needs no workspace and never fails.
  - `ListSubfolders { path: String }`: lists the folders directly under one absolute directory path. It needs no workspace and writes nothing.
- `pub enum DiffBlock { Binary, NotUtf8, TooLarge, Cold }` (derives `Debug, Clone, Copy, PartialEq, Eq`).
- `pub enum FailKind { NoWorkspace, InvalidPath, NotFound, NotConfirmed, Refused, Kernel, Internal }` (derives `Debug, Clone, Copy, PartialEq, Eq`).
- `pub enum Reply` (derives `Debug`):
  - `Opened { root: String }`: the display form of the canonical root. On receipt the window sets `workspace-open` to true, `workspace-root` to that display path, `current-dir` to the empty string and `truncated` to false. It clears the history, the diff panel, the folder items, the selection and `details-version`, and sets `page` to 0. Status `已打开工作区`, with `status-error` false. Then it sends `ListFolder` with the empty string.
  - `NeedsInit { path: String }`: the display form of the canonical candidate.
  - `Folder { relative_dir: String, rows: Vec<model::BrowseRow>, truncated: bool }`.
  - `Captured { relative_path: String, version_no: i64, changed: bool }`.
  - `History { relative_path: String, versions: Vec<model::VersionRow>, chain_valid: bool, event_count: usize }`.
  - `Diff { older_version_id: String, newer_version_id: String, diff: model::DiffResult }`.
  - `DiffUnavailable { older_version_id: String, newer_version_id: String, reason: DiffBlock }`.
  - `Failed { kind: FailKind, message: String, detail: Option<String> }`. `detail` is `None` in every reply produced by this module.
  - `Drives { drives: Vec<model::DriveRow> }`: the reply to `ListDrives`, built with `model::drive_rows` from the drive letters that are present.
  - `Subfolders { path: String, names: Vec<String> }`: the reply to `ListSubfolders`. `path` is the request path as given. `names` holds the folder names, sorted by their lowercased text.
- `pub struct State` holds the open engine, or none, and the rule-file modification time. Its fields are private. `State::new() -> State` returns a state without a workspace.
- `pub struct Worker` holds the sending side of the command channel. Its field is private.

### Functions

- `pub async fn handle(state: &mut State, command: Command) -> Reply`: the complete command contract. It does not catch panics.
- `pub fn guard<F: FnOnce() -> Reply>(f: F) -> Reply`: runs `f` under `std::panic::catch_unwind(AssertUnwindSafe(f))`. A panic gives `Failed { kind: Internal, message: MSG_INTERNAL, detail: None }`.
- `Worker::spawn<F: Fn(Reply) + Send + 'static>(on_reply: F) -> Worker`: starts one operating-system thread. The thread builds a current-thread Tokio runtime with `enable_all()` and creates one `State`. When the runtime cannot be built, every command the thread receives gets `Failed { kind: Internal, message: MSG_INTERNAL, detail: None }`. Otherwise the thread loops: it receives a command, runs `guard(|| runtime.block_on(handle(&mut state, command)))` and calls `on_reply` with the reply. The loop ends when every sender is dropped.
- `Worker::send(&self, command: Command) -> bool`: returns `false` once the thread has stopped.
- `pub fn diff_block_message(reason: DiffBlock) -> &'static str`: `Binary` gives `MSG_DIFF_BINARY`, `NotUtf8` gives `MSG_DIFF_NOT_UTF8`, `TooLarge` gives `MSG_DIFF_TOO_LARGE` and `Cold` gives `MSG_DIFF_COLD`.

### Messages

All messages are `pub const` `&str` values with these exact texts (22 constants):

- `MSG_NO_WORKSPACE` = `请先打开或初始化一个工作区。`
- `MSG_INVALID_PATH` = `该路径无效，或包含链接、重解析点，或位于工作区之外，已拒绝访问。`
- `MSG_NOT_FOUND` = `找不到该文件、文件夹或版本。`
- `MSG_NOT_CONFIRMED` = `需要先确认才能初始化该文件夹。`
- `MSG_ROOT_REFUSED` = `不能把用户主目录本身作为工作区，请选择其中的子文件夹。`
- `MSG_RULE_FILE_REFUSED` = `规则文件 .LADONignore 不作为历史记录对象。`
- `MSG_INTERNAL_REFUSED` = `内部目录 .ladon 和 .git 不作为历史记录对象。`
- `MSG_RULE_PARSE_FAILED` = `规则文件 .LADONignore 无法解析，已停止记录。请先修正该文件。`
- `MSG_OPEN_FAILED` = `无法打开该工作区。请检查 .ladon 目录和 .LADONignore 规则文件是否完整。`
- `MSG_INIT_FAILED` = `无法初始化该文件夹。请检查该文件夹的写入权限。`
- `MSG_DRIVE_INIT_FAILED` = `无法初始化该驱动器。请检查写入权限；访问系统盘等受保护位置时，请以管理员身份运行 LADON。`
- `MSG_LIST_FAILED` = `无法读取该文件夹。`
- `MSG_CAPTURE_FAILED` = `无法记录该文件，请稍后重试。`
- `MSG_HISTORY_FAILED` = `无法读取该文件的历史。`
- `MSG_DIFF_FAILED` = `无法读取这两个版本的内容。`
- `MSG_INTERNAL` = `内部错误，操作未能完成。请重试；若再次出现，请重新打开应用。`
- `MSG_DIFF_BINARY` = `二进制文件，不显示文本差异。`
- `MSG_DIFF_NOT_UTF8` = `文件不是 UTF-8 文本，不显示文本差异。`
- `MSG_DIFF_TOO_LARGE` = `文件超过 2 MiB，不显示文本差异。`
- `MSG_DIFF_COLD` = `该版本的内容不在本地，暂不能比较。`
- `MSG_DIFF_APPROXIMATE` = `差异计算超过 2 秒，结果可能不是最小改动。`
- `MSG_DIFF_TRUNCATED` = `差异超过 4000 行，仅显示前 4000 行。`

### Path rules

Relative paths, for every command except `ListDrives`, `Open`, `Init` and `OpenDrive`:

- The input goes through `model::validate_relative`. `Absolute`, `Traversal` and `Invalid` give `Failed { kind: InvalidPath, message: MSG_INVALID_PATH }`. `Internal` gives `Failed { kind: Refused, message: MSG_INTERNAL_REFUSED }`.
- The validated path, with `/` separators, is the value used for ledger lookups and for the walk.

Walk, used by `ListFolder` for the folder and by `Capture` for the parent folder of the file:

- Starting at the canonical workspace root, each component is read with `symlink_metadata`. A missing component gives `NotFound` with `MSG_NOT_FOUND`.
- A component that is a symbolic link or has the Windows reparse-point attribute (`FILE_ATTRIBUTE_REPARSE_POINT`, `0x400`) gives `InvalidPath`.
- Every component must be a directory, otherwise `InvalidPath`. For `ListFolder` the final component is the folder itself, so a file there gives `InvalidPath`.
- Any other I/O error gives `Kernel` with the command's message (`MSG_LIST_FAILED` or `MSG_CAPTURE_FAILED`).
- For `ListFolder`, the folder is canonicalised (an error gives `Kernel` with `MSG_LIST_FAILED`); the canonical path must start with the canonical root, otherwise `InvalidPath`.

Root candidates, used by `Open`, `Init` and `OpenDrive` in this order:

1. The path must be absolute and must not contain a NUL character; otherwise `InvalidPath`.
2. A device-namespace prefix (`\\.\`) gives `InvalidPath`. A verbatim prefix (`\\?\`) and UNC paths are accepted; canonicalisation resolves them, and every later check still applies. After the drive or root prefix, one leading `\` or `/` and one trailing `\` or `/` are removed. The rest is split on `\` and `/`. Each component is checked with `model::check_component`: `Internal` gives `Refused` with `MSG_INTERNAL_REFUSED`, and any other problem gives `InvalidPath`. An empty component gives `InvalidPath`.
3. The path is rebuilt from its components, which drops a trailing separator, so that the entry checked last is the entry the path names. Its ancestors are then read with `symlink_metadata`, from the top down, skipping the empty path and prefix-only ancestors (a drive, or a UNC share on its own). A symbolic link or reparse point gives `InvalidPath`. A missing ancestor or path gives `NotFound` with `MSG_NOT_FOUND`. Any other I/O error gives `Kernel` with the command's message (`MSG_OPEN_FAILED` for `Open`, `MSG_INIT_FAILED` for `Init`). The path itself must be a directory, otherwise `InvalidPath`. For a drive root such as `C:\`, the only step is the root itself.
4. The path is canonicalised; an error gives `Kernel` with the command's message.
5. A canonical path that matches the profile directory gives `Refused` with `MSG_ROOT_REFUSED`. A drive root is accepted. The profile directory is compared with `model::same_directory`. When `USERPROFILE` cannot be canonicalised, the value as set is used. When `USERPROFILE` is unset, no path matches the profile rule.
6. The entries `.ladon`, `.ladon/meta.turso`, `.ladon/cas` and `.LADONignore` under the candidate are read with `symlink_metadata`, in this order. A symbolic link or reparse point gives `InvalidPath`. A missing entry is skipped. Any other I/O error gives `Kernel` with the command's message. `initialised` is true when `.ladon/meta.turso` exists.
7. The display path is the canonical path with the verbatim prefix removed by `model::strip_extended_prefix`.

Capture, file containment:

- The file must exist (`NotFound` with `MSG_NOT_FOUND`). It must not be a symbolic link or reparse point, and it must be a regular file (`InvalidPath`). Its canonical path must start with the canonical root (`InvalidPath`).
- Tracked and untracked regular files are accepted alike.

### Command rules

Precondition: every command except `ListDrives`, `Open`, `Init` and `OpenDrive` needs an open workspace. Without one the reply is `Failed { kind: NoWorkspace, message: MSG_NO_WORKSPACE }`, and no path is examined.

- `Open { path }`: root candidate checks with `MSG_OPEN_FAILED` as the I/O message. When `initialised` is false, the reply is `NeedsInit { path }` with the display path, and the state is unchanged. Otherwise the rule-file modification time (absent when the file cannot be read) is read first, then `Engine::open` runs. An error gives `Kernel` with `MSG_OPEN_FAILED`, and the state is unchanged. A success stores the engine and the modification time and replies `Opened { root }`, whose window effects are listed at `Reply`.
- `Init { path, confirmed }`: `confirmed == false` gives `NotConfirmed` with `MSG_NOT_CONFIRMED`, and no path is examined. Root candidate checks follow with `MSG_INIT_FAILED` as the I/O message. When `initialised` is true, the command behaves as `Open`. Otherwise the rule-file modification time is read first, then `Engine::init` runs. On a drive root it creates `.ladon` and `.LADONignore` in the drive root. An error gives `Kernel` with `MSG_INIT_FAILED`, and the state is unchanged. A success stores the engine and the modification time and replies `Opened { root }`, whose window effects are listed at `Reply`.
- `OpenDrive { path }`: root candidate checks with `MSG_OPEN_FAILED` as the I/O message. When `initialised` is true, the reply is the reply of `Open` for the same candidate. Otherwise the rule-file modification time is read first, then `Engine::init` runs without a confirmation. An error gives `Kernel` with `MSG_DRIVE_INIT_FAILED`, and the state is unchanged. A success stores the engine and the modification time and replies `Opened { root }`.
- `ListFolder { relative_dir }`:
  - The path passes the relative-path rules and the walk, then the canonical folder is read.
  - Each entry's metadata is read from its directory entry, without following links. A symbolic link or reparse point is `Link`; a directory is `Directory`; a regular file is `File`; anything else is `Other`.
  - Only `File` entries carry a size and a modification time (whole seconds since the Unix epoch). Other entries carry size `0` and no modification time.
  - Entries named `.ladon` or `.git` (ASCII case-insensitive) are omitted at every level. Entries with the Windows hidden or system attribute are omitted at every level, as Explorer omits them by default.
  - A failed read of the folder itself gives `Kernel` with `MSG_LIST_FAILED`. An entry whose name is not valid Unicode, or whose metadata cannot be read, is omitted without a message and does not fail the listing.
  - The ledger is built from `store.list_files()`, keyed by relative path, with deleted rows included. A failure gives `Kernel` with `MSG_LIST_FAILED`.
  - `model::browse_rows` receives the canonical relative folder and the local offset. The first `LIST_LIMIT` rows are kept, and `truncated` is true when more rows existed.
  - Reply: `Folder { relative_dir, rows, truncated }`, where `relative_dir` is the canonical relative folder.
- `Capture { relative_path }`:
  - An empty path gives `InvalidPath`.
  - The path passes the relative-path rules. When its last component equals `.LADONignore` (ASCII case-insensitive), the reply is `Refused` with `MSG_RULE_FILE_REFUSED`.
  - The parent is walked (I/O failures give `MSG_CAPTURE_FAILED`), then the file is checked by the containment rules. Its canonical path gives the recorded relative path with `/` separators. A canonicalisation error of the file path gives `Kernel` with `MSG_CAPTURE_FAILED`.
  - The rule file's modification time (absent when it cannot be read) is compared with the stored value. When they differ, `engine.reload_policy()` runs. An error gives `Refused` with `MSG_RULE_PARSE_FAILED`, captures nothing and keeps the stored value. A success stores the value read.
  - `engine.capture(&canonical, "human", "gui-capture")` runs; an error gives `Kernel` with `MSG_CAPTURE_FAILED`.
  - `version_no` is read from `store.find_path` with the recorded path after the call. A read failure of `store.find_path` after the capture, or a missing row, gives `Kernel` with `MSG_CAPTURE_FAILED`.
  - Reply: `Captured { relative_path: recorded, version_no, changed }`. `changed` is `false` exactly when the kernel status is `unchanged`.
- `LoadHistory { relative_path }`:
  - The path passes the relative-path rules. An empty path, or a last component equal to `.LADONignore` (ASCII case-insensitive), gives `NotFound`.
  - `store.find_path`: no row, or a deleted row, gives `NotFound`. A read error gives `Kernel` with `MSG_HISTORY_FAILED`.
  - `store.versions(file_id)` is mapped to `VersionInput` with `local = cas.has(content_hash)` and passed to `version_rows`.
  - `chain_valid = store.verify_chain(file_id)` and `event_count = store.events(file_id).len()`. Any read error gives `Kernel` with `MSG_HISTORY_FAILED`.
  - Reply: `History { relative_path, versions, chain_valid, event_count }`.
- `LoadDiff { older_version_id, newer_version_id }`:
  - `store.version` is read for the older identifier, then for the newer one. A read error gives `Kernel` with `MSG_DIFF_FAILED`; an unknown identifier gives `NotFound`. Versions of different files give `NotFound`.
  - For the older version, then the newer one: when `cas.has(content_hash)` is false, the reply is `DiffUnavailable { reason: Cold }`; when `byte_size` is above `DIFF_LIMIT_BYTES`, the reply is `DiffUnavailable { reason: TooLarge }`. Content is read only after both versions pass these checks.
  - The contents are loaded with `engine.load_version(id, None)`, the older one first. A read error gives `Kernel` with `MSG_DIFF_FAILED`. After both contents are loaded, a content longer than `DIFF_LIMIT_BYTES` gives `DiffUnavailable { reason: TooLarge }`.
  - A zero byte in either content gives `DiffUnavailable { reason: Binary }`. Invalid UTF-8 in either content gives `DiffUnavailable { reason: NotUtf8 }`.
  - Otherwise `model::diff_lines(older, newer, DIFF_MAX_LINES, DIFF_TIMEOUT)` runs and the reply is `Diff { older_version_id, newer_version_id, diff }`. The `truncated` and `approximate` flags are those of the `DiffResult`.
- `ListSubfolders { path }`: the path must be absolute and pass the checks of steps 1 to 3 of the root candidate rules, without steps 4 to 7: no profile refusal and no check of the owned entries. Each failure gives the reply that the root candidate rules give, with `MSG_LIST_FAILED` as the I/O message. The directory is read with `fs::read_dir`; a failure gives `Kernel` with `MSG_LIST_FAILED`. An entry is kept when its name is valid Unicode, is not `.ladon` or `.git` (ASCII case-insensitive), its metadata from the directory entry (no link followed) shows a directory that is neither a symbolic link nor a reparse point, and it has neither the hidden nor the system attribute on Windows. Nothing is written.
- `ListDrives`: probes the drive letters `A` to `Z` in order. A letter is present when `X:\` is a directory (`Path::is_dir`). For each present letter, `initialised` is true when `X:\.ladon\meta.turso`, read with `symlink_metadata`, is a regular file that is neither a symbolic link nor a reparse point. The `(letter, initialised)` pairs of the present letters are passed to `model::drive_rows`, and the reply is `Drives { drives }`. The command needs no workspace, never fails and writes nothing.

Cross-cutting rules:

- The handler never calls restore, scan, archive or any storage provider function. Content is read only through `load_version` with no provider, and only after `cas.has` confirmed that the content is local, so no archive key or token is read.
- `detail` is `None` in every failure reply.

### Worker invariants

- Exactly one reply per command, in submission order, including refusals and internal errors (except when the thread has stopped, see `send`).
- Commands run one after another on the worker thread.
- After a guarded panic the worker keeps running, and the next command is processed normally.
- When the runtime cannot be built, the worker keeps running and answers every command with the internal failure reply.
- The reply callback runs outside `guard`. A panic in the callback ends the thread, and later `send` calls return `false`.

## Known Limitations

- Capture reads the file by path after its containment check; a junction placed over a directory between the check and the read is not detected. Owner: kernel. Planned change: read through a verified file handle.
- The kernel compares `.ladon` and `.git` case-sensitively in `resolve` and `scan`; the window refuses them case-insensitively. A folder named `.GIT` can still be recorded through the command line.
- Recording and initialising cannot be cancelled. Closing the window during them leaves at most an unreferenced content object or a partly created `.ladon` directory; no ledger entry is written without its version record.
- Only Simplified Chinese messages exist (NFR-015 is not met in this release).
- Files that carry the Windows reparse-point attribute, such as on-demand cloud-synced files, are listed as links and cannot be recorded.
- The window never reads an archive key or token and never archives, restores or scans.
- Entries that are hidden or system items, entries whose name is not valid Unicode, and entries whose metadata cannot be read are not listed. The window has no switch to show them, and Explorer's switch for hidden and protected system items is not implemented.
- The window opens workspaces only at drive roots, through `OpenDrive`. A folder that is not a drive root cannot be initialised from the window. `Open` and `Init` remain in the command set and are exercised by the tests.

- Folder tree: the window requests `ListSubfolders` when a tree node is expanded. The tree is not refreshed by itself; a folder created or removed after its parent was expanded appears after the parent is collapsed and expanded again.
- The window opens and initialises workspaces only at drive roots, through `OpenDrive`. Folders below a drive root are listed through `ListFolder`, which needs the drive root to be the open workspace.

## Change Notes

- The root candidate rules refuse a NUL character and a device-namespace prefix as `InvalidPath` before any file system call. Verbatim and UNC prefixes are accepted.
- Rebuilding the path from its components before the link checks makes a trailing separator irrelevant, so a junction named with a trailing separator is checked as the entry it names.
- The profile-directory refusal runs before the owned-entry checks, so no entry inside that directory is read.
- A ledger read that finds no row after a successful capture is a failed read of the version number and gives `Kernel` with `MSG_CAPTURE_FAILED`.
- The module holds no state beyond the open engine and the rule-file modification time. A text diff that reaches `DIFF_TIMEOUT` is reported through the `approximate` flag of `DiffResult`.
