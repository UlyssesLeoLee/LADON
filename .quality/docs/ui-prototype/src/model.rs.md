# Technical Design: `ui-prototype/src/model.rs`

Source: `ui-prototype/src/model.rs`

Source-SHA256: `751e5c6c2b5e043362125cd4e7600be8714c71306fc5e3e262b18ee08ae9f08e`

## Responsibility

`model.rs` holds the pure logic of the LADON window. It validates workspace-relative paths and their components, provides the drive-root and directory-comparison helpers used for root candidates, builds the rows of the This PC page from the drive letters, classifies binary content, builds the rows of the browse list and of the version list, decides when an open folder is listed again and which row keeps the selection, computes line diffs between two text versions, and formats bytes, times, drive status and section lists for display. It performs no file-system, database, Slint, Tokio or kernel call. Apart from the elapsed-time check that `diff_lines` uses for its `approximate` flag, every function is a pure function of its inputs: the time zone is passed as a `chrono::FixedOffset` and the diff time limit as a `std::time::Duration`.

The module supports REQ-009 (version history and diff), NFR-013 (path safety), NFR-004 (sensitive status and availability text), NFR-007 (actor and reason in every version row) and NFR-003 (offline operation).

## Dependencies

- Crates: `chrono` (`DateTime`, `FixedOffset`, `Utc`), `similar` (`TextDiff`, `ChangeTag`) and `std` (`borrow::Cow`, `collections::HashMap`, `time::{Duration, Instant}`).
- Upstream: none inside the crate. Callers pass plain values: directory entries as `RawEntry`, ledger rows as `LedgerRow`, version records as `VersionInput`, and drive letters with their initialised flag as `(char, bool)` pairs.
- Downstream: `app.rs` (feature `kernel`) calls the path, entry, version and diff functions to build the reply of each command, `strip_extended_prefix` for display paths and `same_directory` for the profile-directory check. It builds the `Drives` reply of the `ListDrives` command with `drive_rows`. The window (`main.rs`) maps the row structs carried by `app::Reply`, including `DriveRow`, into Slint models and shows the status of each drive row with `drive_status_text`.

## Public Contract

Types. Every type derives `Debug`, `Clone`, `PartialEq` and `Eq`. The fieldless enums `PathProblem`, `EntryKind` and `LineKind` also derive `Copy`.

- `pub enum PathProblem { Absolute, Traversal, Internal, Invalid }`.
- `pub enum EntryKind { File, Directory, Link, Other }`.
- `pub struct RawEntry { pub name: String, pub kind: EntryKind, pub size: u64, pub modified_unix: Option<i64> }`. The caller fills it from `symlink_metadata`, so a link is reported as `Link` and never as the object it points to.
- `pub struct LedgerRow { pub file_id: String, pub version_no: i64, pub deleted: bool, pub sensitive: bool }`.
- `pub struct BrowseRow { pub name: String, pub relative_path: String, pub is_directory: bool, pub is_link: bool, pub size_text: String, pub modified_text: String, pub status_text: String, pub file_id: Option<String>, pub tracked: bool, pub sensitive: bool, pub version_no: Option<i64> }`. `relative_path` is workspace-relative with `/` separators.
- `pub struct VersionInput { pub version_id: String, pub number: i64, pub content_hash: String, pub byte_size: i64, pub sensitive: bool, pub changed_sections: Vec<String>, pub actor: String, pub reason: String, pub created_at: String, pub local: bool }`.
- `pub struct VersionRow { pub version_id: String, pub number: i64, pub label: String, pub actor: String, pub reason: String, pub time_text: String, pub size_text: String, pub sections_text: String, pub hash_short: String, pub sensitive: bool, pub local: bool, pub availability_text: String, pub current: bool }`.
- `pub enum LineKind { Context, Added, Removed, Gap }`.
- `pub struct DiffLine { pub kind: LineKind, pub old_no: Option<usize>, pub new_no: Option<usize>, pub text: String }`. Line numbers are 1-based.
- `pub struct DiffResult { pub lines: Vec<DiffLine>, pub added: usize, pub removed: usize, pub truncated: bool, pub approximate: bool }`.
- `pub struct DriveRow { pub root: String, pub label: String, pub initialised: bool }`. `root` is `X:\` and `label` is `本地磁盘 (X:)`, where `X` is an upper-case ASCII letter. `initialised` is true when the drive is already a workspace.

Functions:

- `pub fn validate_relative(input: &str) -> Result<String, PathProblem>`. The input may use `/` or `\` as separator. The checks run in this order, and the first failing check decides the result:
  1. a NUL character gives `Invalid`;
  2. a leading `/` or `\`, including UNC paths, gives `Absolute`;
  3. an ASCII letter followed by `:` at the start gives `Absolute`;
  4. one trailing `/` or `\` is removed; a result that is empty is the workspace root and gives `Ok("")`;
  5. the remainder is split on `/` and `\`, `check_component` is applied to every component, and the components are joined with `/`.
- `pub fn check_component(component: &str) -> Result<(), PathProblem>`. Checks one path component. The checks run in this order, and the first failing check decides the result:
  1. `..` gives `Traversal`;
  2. `.` gives `Invalid`;
  3. an empty component gives `Invalid`;
  4. `norm`, the component with trailing dots and spaces removed, equals `.ladon` or `.git` ASCII-case-insensitively: `Internal`;
  5. a `:` anywhere in the component gives `Invalid`;
  6. a component made only of dots and spaces gives `Invalid`;
  7. a component that ends with `.` or a space gives `Invalid`;
  8. `stem`, the part of `norm` before its first `.` with trailing spaces removed, equals `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9` or `LPT1` to `LPT9` ASCII-case-insensitively: `Invalid`.
- `pub fn is_binary(bytes: &[u8]) -> bool`. True when a zero byte occurs anywhere in the slice. An empty slice is not binary.
- `pub fn strip_extended_prefix(path: &str) -> Cow<'_, str>`. Removes the extended-length prefix `\\?\`. `\\?\UNC\server\share` becomes `\\server\share`, which is an owned string. Other paths with the prefix give the rest of the path borrowed. Inputs without the prefix are returned borrowed and unchanged.
- `pub fn is_drive_root(path: &str) -> bool`. Applies `strip_extended_prefix`, then returns true for `X:`, `X:\` and `X:/`, where `X` is an ASCII letter.
- `pub fn same_directory(a: &str, b: &str) -> bool`. Applies `strip_extended_prefix` to both inputs and removes one trailing `/` or `\` from each. The results are compared ASCII-case-insensitively. Separators are compared as written.
- `pub fn drive_rows(entries: &[(char, bool)]) -> Vec<DriveRow>`. Builds one row per drive letter:
  - an entry whose character is an ASCII letter gives a row with `root` `X:\`, `label` `本地磁盘 (X:)` and `initialised` equal to the flag of the entry, where `X` is the letter in upper case;
  - an entry whose character is not an ASCII letter is skipped;
  - a letter that repeats, in either case, keeps the row of its first entry, and the later entries are dropped;
  - rows are sorted by letter.
- `pub fn drive_status_text(initialised: bool) -> &'static str`. Returns `已是工作区` when `initialised` is true and `未初始化` when it is false.
- `pub fn format_bytes(n: u64) -> String`. A value below 1024 gives `{n} B`. Larger values use binary multiples. The unit is the first of `KB`, `MB` and `GB` whose value, rounded to one decimal place, is below 1024.0; `GB` is used when none is. The value is shown with one decimal place. The rounding is the one that the display uses, so `1048575` gives `1.0 MB`, `1536` gives `1.5 KB`, `5242880` gives `5.0 MB` and `1099511627776` gives `1024.0 GB`.
- `pub fn format_time(rfc3339: &str, offset: FixedOffset) -> String`. Converts the instant to `offset` and formats it as `YYYY-MM-DD HH:MM`. Input that does not parse as RFC 3339 is returned unchanged.
- `pub fn unix_time_text(secs: Option<i64>, offset: FixedOffset) -> String`. `None`, or a value out of range for `DateTime::<Utc>::from_timestamp(secs, 0)`, gives `—`. Otherwise the output is `YYYY-MM-DD HH:MM` in `offset`.
- `pub fn sections_text(sections: &[String]) -> String`. The checks run in this order:
  1. a list that contains `DIFF_DEFERRED_PREVIOUS_COLD` gives `上一版本内容不在本地，未计算段落`;
  2. otherwise a list that contains `LARGE_BINARY_OR_TEXT` gives `内容过大，未计算段落`;
  3. otherwise an empty list gives `—`;
  4. otherwise the first three keys joined by `、`, followed by ` 等 {n} 项` when there are more than three keys, where `n` is the total number of keys.
- `pub fn browse_rows(entries: &[RawEntry], relative_dir: &str, ledger: &HashMap<String, LedgerRow>, offset: FixedOffset) -> Vec<BrowseRow>`:
  - there is one row per input entry; `relative_path` is `relative_dir/name`, or `name` when `relative_dir` is empty; the ledger key is that relative path;
  - order: directories first, then names compared ASCII-case-insensitively, then by exact name; entries that compare equal keep their input order;
  - a directory has size `—` and status `—`; a link has size `—` and status `链接（不跟随）`; any other entry has size `—` and status `其他`; these entries have `tracked = false`, `file_id = None`, `version_no = None` and `sensitive = false`;
  - a file has size `format_bytes(size)`. When its ledger row exists and `deleted` is false, its status is `已记录 v{n}`, or `敏感 v{n}` when the row is sensitive; the row has `tracked = true`, `file_id = Some(file_id)`, `version_no = Some(n)`, and `sensitive` copied from the ledger row. Any other file has status `未记录`, `tracked = false`, `file_id = None`, `version_no = None` and `sensitive = false`;
  - `modified_text` is `unix_time_text(modified_unix, offset)` for every entry.
- `pub fn version_rows(versions: &[VersionInput], offset: FixedOffset) -> Vec<VersionRow>`:
  - rows are sorted by `number`, largest first; only the first row has `current = true`;
  - `label` is `v{number}`; `time_text` is `format_time(created_at, offset)`; `hash_short` is the first 12 characters of `content_hash`, or the whole value when it is shorter;
  - `size_text` is `format_bytes(byte_size)`, or `—` when `byte_size` is negative; `sections_text` is `sections_text(changed_sections)`;
  - `version_id`, `number`, `actor`, `reason`, `sensitive` and `local` are copied from the input;
  - `availability_text` is `内容不在本地，暂不可比较` when the version is not local, `本地可读 · 敏感，不外传` when it is local and sensitive, and `本地可读` otherwise.
- `pub fn diff_lines(old: &str, new: &str, max_lines: usize, timeout: Duration) -> DiffResult`:
  - the diff is `similar::TextDiff::configure().timeout(timeout).diff_lines(old, new)` followed by `grouped_ops(3)`: three unchanged lines surround each change, and one `Gap` line separates groups that are not adjacent;
  - a `Context` line carries both line numbers; a `Removed` line carries `old_no` only; an `Added` line carries `new_no` only; a `Gap` line carries neither number and has empty text;
  - the lines are the tokens that `similar` 2.7.0 produces for line diffs (`tokenize_lines`): a token ends at `\n`, at `\r\n`, or at a lone `\r`. A lone `\r` stays in its token's text. The displayed line text drops one trailing `\n` or `\r\n` and keeps a lone `\r`;
  - lines are emitted in document order. At most `max_lines` lines are kept, and `Gap` lines count toward that limit. `truncated` is true when at least one line was dropped;
  - `added` and `removed` count every change, including dropped ones;
  - `approximate` is true when the elapsed time of the diff call is at least `timeout`;
  - identical inputs and two empty inputs produce no lines.
- `pub fn should_refresh(busy: bool, worker_stopped: bool, workspace_open: bool) -> bool`: true exactly when `busy` is false, `worker_stopped` is false and `workspace_open` is true.
- `pub fn selection_after_refresh(rows: &[BrowseRow], selected_path: &str) -> Option<usize>`: the index of the first row whose `relative_path` equals `selected_path`; `None` when `selected_path` is empty or no row matches.
- `pub fn listing_changed(old: &[BrowseRow], new: &[BrowseRow]) -> bool`: false only when both slices have the same length and every pair of rows at the same index is equal in all fields; true otherwise.

- `pub struct TreeRow { pub path: String, pub name: String, pub depth: usize, pub expanded: bool }` derives `Debug, Clone, PartialEq, Eq`. It is one visible row of the folder tree in the navigation pane. `path` is the absolute display path of the node.
- `pub fn drive_tree_rows(drives: &[DriveRow]) -> Vec<TreeRow>`: one row per drive, in the order given, with `path` equal to `root`, `name` equal to `label`, `depth` 0 and `expanded` false.
- `pub fn child_path(parent: &str, name: &str) -> String`: `parent` followed by `name`, with a `\` between them unless `parent` already ends with `\`.
- `pub fn tree_index(rows: &[TreeRow], path: &str) -> Option<usize>`: the index of the first row whose `path` equals `path` ignoring ASCII case.
- `pub fn tree_expand(rows: &mut Vec<TreeRow>, index: usize, names: &[String]) -> bool`: when the row at `index` exists and is not expanded, marks it expanded and inserts one row per name directly after it, with `depth` one more than the row, `path` from `child_path` and `expanded` false, and returns true. Otherwise it changes nothing and returns false.
- `pub fn tree_collapse(rows: &mut Vec<TreeRow>, index: usize) -> bool`: when the row at `index` exists and is expanded, removes the following rows whose `depth` is greater than its depth, marks it not expanded, and returns true. Otherwise it changes nothing and returns false.
- `pub fn drive_root_of(path: &str) -> Option<String>`: for a path that starts with an ASCII letter, a colon and a backslash, or is exactly `X:`, returns `X:\`. Any other path gives None.
- `pub fn relative_in_root(root: &str, path: &str) -> Option<String>`: when `path` equals `root` or starts with it (ASCII case-insensitive), returns the rest of `path` with `\` replaced by `/` and a leading `/` removed; the root itself gives the empty string. Any other input gives None.

- `pub fn check_new_name(name: &str) -> Result<(), &'static str>`: accepts a name for a new file, folder or rename. The name must not be empty, must not be longer than 255 characters, must not contain a control character or any of `\ / : * ? " < > |`, must not end with a dot or a space, and must not be a Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`, with or without an extension; compared without case). The errors are `MSG_NAME_INVALID` (`名称无效：不能为空，不能包含 \ / : * ? " < > |，不能以点或空格结尾，不能是系统保留名。`) for every case.
- `pub fn capacity_text(total: Option<u64>, free: Option<u64>) -> String`: `可用 {free} / 共 {total}` using `format_bytes`; `容量未知` when either value is `None`.
- `DriveRow` gains `total_bytes: Option<u64>` and `free_bytes: Option<u64>`. `drive_rows` sets both to `None`.

## Change Notes

The module writes no data and has no side effects. `validate_relative` and `check_component` refuse the `..`, `.ladon` and `.git` components, so the handler reports these paths before the engine is called. The `.ladon` and `.git` comparison in `check_component` ignores ASCII case and trailing dots and spaces. `app.rs` (`read_entries`) omits the entries named `.ladon` or `.git` with an ASCII-case-insensitive comparison that does not remove trailing dots and spaces. `drive_rows` takes the drive letters and their flags from the caller and does not probe the machine, so the caller decides which letters exist and which drives are already workspaces. Symbolic links and reparse points are not visible to this module. The handler decides them with `symlink_metadata`, and `RawEntry` carries only the kind that call reports.
