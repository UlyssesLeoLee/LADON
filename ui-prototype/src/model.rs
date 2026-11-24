//! Pure logic of the LADON Explorer window.
//!
//! The module validates workspace-relative paths, builds the rows of the
//! browse list and the version list, computes line diffs and formats values
//! for display. It depends only on `chrono`, `similar` and `std`.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Utc};
use similar::{ChangeTag, TextDiff};

/// Text shown for a value that is missing or not applicable.
const MISSING: &str = "—";

/// Format of the times shown in the window, in the offset given by the caller.
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M";

/// Units of `format_bytes` for values of 1024 bytes and more, smallest first.
const SIZE_UNITS: [&str; 3] = ["KB", "MB", "GB"];

/// Unchanged lines kept on each side of a change.
const CONTEXT_LINES: usize = 3;

/// Number of leading characters of a content hash that are shown.
const HASH_SHORT_LEN: usize = 12;

/// Engine marker: the content of the older version is not available locally.
const MARKER_DEFERRED_PREVIOUS_COLD: &str = "DIFF_DEFERRED_PREVIOUS_COLD";

/// Engine marker: the content is too large or binary for sections to be computed.
const MARKER_LARGE_BINARY_OR_TEXT: &str = "LARGE_BINARY_OR_TEXT";

/// Reason a relative path is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    Absolute,
    Traversal,
    Internal,
    Invalid,
}

/// Kind of a directory entry, as reported by `symlink_metadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Link,
    Other,
}

/// One directory entry read from the file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified_unix: Option<i64>,
}

/// Ledger record of one tracked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    pub file_id: String,
    pub version_no: i64,
    pub deleted: bool,
    pub sensitive: bool,
}

/// One row of the browse list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseRow {
    pub name: String,
    /// Workspace-relative path with `/` separators.
    pub relative_path: String,
    pub is_directory: bool,
    pub is_link: bool,
    pub size_text: String,
    pub modified_text: String,
    pub status_text: String,
    pub file_id: Option<String>,
    pub tracked: bool,
    pub sensitive: bool,
    pub version_no: Option<i64>,
}

/// Version record before display formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInput {
    pub version_id: String,
    pub number: i64,
    pub content_hash: String,
    pub byte_size: i64,
    pub sensitive: bool,
    pub changed_sections: Vec<String>,
    pub actor: String,
    pub reason: String,
    pub created_at: String,
    pub local: bool,
}

/// One row of the version list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    pub version_id: String,
    pub number: i64,
    pub label: String,
    pub actor: String,
    pub reason: String,
    pub time_text: String,
    pub size_text: String,
    pub sections_text: String,
    pub hash_short: String,
    pub sensitive: bool,
    pub local: bool,
    pub availability_text: String,
    pub current: bool,
}

/// Kind of one line of a text diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    Gap,
}

/// One line of a text diff. Line numbers are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

/// Result of a text diff. Totals are counted before truncation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResult {
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
    pub truncated: bool,
    pub approximate: bool,
}

/// Validates a workspace-relative path and returns it with `/` separators.
///
/// The input may use `/` or `\` as separator. The empty string denotes the
/// workspace root and is returned unchanged. The checks run in a fixed order
/// and the first failing check decides the result.
pub fn validate_relative(input: &str) -> Result<String, PathProblem> {
    if input.contains('\0') {
        return Err(PathProblem::Invalid);
    }
    if input.starts_with('/') || input.starts_with('\\') {
        return Err(PathProblem::Absolute);
    }
    let bytes = input.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(PathProblem::Absolute);
    }
    let body = input.strip_suffix(is_separator).unwrap_or(input);
    if body.is_empty() {
        return Ok(String::new());
    }
    let mut components = Vec::new();
    for component in body.split(is_separator) {
        check_component(component)?;
        components.push(component);
    }
    Ok(components.join("/"))
}

/// Checks one path component. The checks run in a fixed order and the first
/// failing check decides the result:
///
/// 1. `..` gives `Traversal`;
/// 2. `.` gives `Invalid`;
/// 3. an empty component gives `Invalid`;
/// 4. the component with trailing dots and spaces removed equals `.ladon` or
///    `.git`, ASCII-case-insensitively, gives `Internal`;
/// 5. a `:` anywhere in the component gives `Invalid`;
/// 6. a component made only of dots and spaces gives `Invalid`;
/// 7. a component that ends with `.` or a space gives `Invalid`;
/// 8. the stem, which is the part before the first `.` with trailing spaces
///    removed, equals `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9` or `LPT1`
///    to `LPT9`, ASCII-case-insensitively, gives `Invalid`.
pub fn check_component(component: &str) -> Result<(), PathProblem> {
    if component == ".." {
        return Err(PathProblem::Traversal);
    }
    if component == "." {
        return Err(PathProblem::Invalid);
    }
    if component.is_empty() {
        return Err(PathProblem::Invalid);
    }
    let norm = component.trim_end_matches(is_dot_or_space);
    if norm.eq_ignore_ascii_case(".ladon") || norm.eq_ignore_ascii_case(".git") {
        return Err(PathProblem::Internal);
    }
    if component.contains(':') {
        return Err(PathProblem::Invalid);
    }
    if component.chars().all(is_dot_or_space) {
        return Err(PathProblem::Invalid);
    }
    if component.ends_with(is_dot_or_space) {
        return Err(PathProblem::Invalid);
    }
    let stem = norm.split('.').next().unwrap_or(norm).trim_end_matches(' ');
    if is_device_name(stem) {
        return Err(PathProblem::Invalid);
    }
    Ok(())
}

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

fn is_dot_or_space(c: char) -> bool {
    c == '.' || c == ' '
}

fn is_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

/// Returns true when the slice contains a zero byte.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

/// Removes the extended-length prefix `\\?\`. `\\?\UNC\server\share` becomes
/// `\\server\share`, which is an owned string. Other paths with the prefix
/// give the rest of the path borrowed. Inputs without the prefix are returned
/// borrowed and unchanged.
pub fn strip_extended_prefix(path: &str) -> Cow<'_, str> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        Cow::Owned(format!(r"\\{rest}"))
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        Cow::Borrowed(rest)
    } else {
        Cow::Borrowed(path)
    }
}

/// Returns true for `X:`, `X:\` and `X:/`, where `X` is an ASCII letter.
/// The path is first passed through `strip_extended_prefix`.
pub fn is_drive_root(path: &str) -> bool {
    let stripped = strip_extended_prefix(path);
    match stripped.as_bytes() {
        [drive, b':'] => drive.is_ascii_alphabetic(),
        [drive, b':', b'\\' | b'/'] => drive.is_ascii_alphabetic(),
        _ => false,
    }
}

/// One row of the This PC page: a drive root, its label, and whether that root is an
/// initialised workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRow {
    pub root: String,
    pub label: String,
    pub initialised: bool,
    /// Total size of the volume in bytes, or None when the volume cannot be queried.
    pub total_bytes: Option<u64>,
    /// Space available to the caller in bytes, or None when the volume cannot be queried.
    pub free_bytes: Option<u64>,
}

/// Builds the rows of the This PC page from drive letters and their initialised flags,
/// sorted by letter. Characters that are not ASCII letters are skipped. Letters are
/// compared without case, and a letter that repeats keeps its first entry.
pub fn drive_rows(entries: &[(char, bool)]) -> Vec<DriveRow> {
    let mut first: BTreeMap<char, bool> = BTreeMap::new();
    for &(letter, initialised) in entries {
        if letter.is_ascii_alphabetic() {
            first.entry(letter.to_ascii_uppercase()).or_insert(initialised);
        }
    }
    first
        .into_iter()
        .map(|(letter, initialised)| DriveRow {
            root: format!("{letter}:\\"),
            label: format!("本地磁盘 ({letter}:)"),
            initialised,
            total_bytes: None,
            free_bytes: None,
        })
        .collect()
}

/// Status text of a drive row on the This PC page.
pub fn drive_status_text(initialised: bool) -> &'static str {
    if initialised {
        "已是工作区"
    } else {
        "未初始化"
    }
}

/// One visible row of the folder tree in the navigation pane. `path` is the absolute
/// display path of the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub expanded: bool,
}

/// One collapsed row per drive, at depth 0, in the order given.
pub fn drive_tree_rows(drives: &[DriveRow]) -> Vec<TreeRow> {
    drives
        .iter()
        .map(|drive| TreeRow {
            path: drive.root.clone(),
            name: drive.label.clone(),
            depth: 0,
            expanded: false,
        })
        .collect()
}

/// Path of a child folder: `parent` and `name` joined by one backslash, which is not added
/// when `parent` already ends with one.
pub fn child_path(parent: &str, name: &str) -> String {
    if parent.ends_with('\\') {
        format!("{parent}{name}")
    } else {
        format!("{parent}\\{name}")
    }
}

/// Index of the first row whose path equals `path` ignoring ASCII case.
pub fn tree_index(rows: &[TreeRow], path: &str) -> Option<usize> {
    rows.iter().position(|row| row.path.eq_ignore_ascii_case(path))
}

/// Expands the row at `index` with one child row per name, directly after it. Returns false
/// and changes nothing when the row does not exist or is already expanded.
pub fn tree_expand(rows: &mut Vec<TreeRow>, index: usize, names: &[String]) -> bool {
    let Some(parent) = rows.get(index) else {
        return false;
    };
    if parent.expanded {
        return false;
    }
    let depth = parent.depth + 1;
    let children: Vec<TreeRow> = names
        .iter()
        .map(|name| TreeRow {
            path: child_path(&parent.path, name),
            name: name.clone(),
            depth,
            expanded: false,
        })
        .collect();
    rows[index].expanded = true;
    rows.splice(index + 1..index + 1, children);
    true
}

/// Collapses the expanded row at `index` by removing the rows below it that are deeper than
/// it. Returns false and changes nothing when the row does not exist or is not expanded.
pub fn tree_collapse(rows: &mut Vec<TreeRow>, index: usize) -> bool {
    let Some(node) = rows.get(index) else {
        return false;
    };
    if !node.expanded {
        return false;
    }
    let depth = node.depth;
    let end = rows[index + 1..]
        .iter()
        .position(|row| row.depth <= depth)
        .map_or(rows.len(), |offset| index + 1 + offset);
    rows.drain(index + 1..end);
    rows[index].expanded = false;
    true
}

/// The drive root of a path that starts with `X:\`, or is exactly `X:`, as `X:\`. Other paths,
/// including UNC paths, give None.
pub fn drive_root_of(path: &str) -> Option<String> {
    match path.as_bytes() {
        [drive, b':'] if drive.is_ascii_alphabetic() => Some(format!("{}:\\", *drive as char)),
        [drive, b':', b'\\', ..] if drive.is_ascii_alphabetic() => {
            Some(format!("{}:\\", *drive as char))
        }
        _ => None,
    }
}

/// The part of `path` below `root`, with `/` separators and no leading separator. The root
/// itself gives the empty string. A path outside `root` gives None. The comparison of the
/// root part ignores ASCII case.
pub fn relative_in_root(root: &str, path: &str) -> Option<String> {
    let head = path.get(..root.len())?;
    if !head.eq_ignore_ascii_case(root) {
        return None;
    }
    let rest = &path[root.len()..];
    Some(rest.replace('\\', "/").trim_start_matches('/').to_string())
}

/// Message for a name that cannot be used for a new file, folder or rename.
pub const MSG_NAME_INVALID: &str =
    "名称无效：不能为空，不能包含 \\ / : * ? \" < > |，不能以点或空格结尾，不能是系统保留名。";

const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Name of the root rule file. It is never created, renamed or replaced from the GUI.
pub const RULE_FILE_NAME: &str = ".LADONignore";

/// Message for a name that is the root rule file (matched case-insensitively).
pub const MSG_RULE_FILE_NAME: &str = "名称无效：不能使用规则文件名 .LADONignore。";

/// Checks a name for a new file, folder or rename. The rules are those of Windows file names.
pub fn check_new_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.chars().count() > 255 {
        return Err(MSG_NAME_INVALID);
    }
    if name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
        return Err(MSG_NAME_INVALID);
    }
    if name.eq_ignore_ascii_case(RULE_FILE_NAME) {
        return Err(MSG_RULE_FILE_NAME);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(MSG_NAME_INVALID);
    }
    // Windows ignores trailing spaces before the extension, so "CON .txt" is the reserved name CON.
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    if RESERVED_DEVICE_NAMES.iter().any(|reserved| reserved.eq_ignore_ascii_case(stem)) {
        return Err(MSG_NAME_INVALID);
    }
    Ok(())
}

/// Capacity line of a drive row: available and total space, or an unknown-capacity text.
pub fn capacity_text(total: Option<u64>, free: Option<u64>) -> String {
    match (total, free) {
        (Some(total), Some(free)) => format!("可用 {} / 共 {}", format_bytes(free), format_bytes(total)),
        _ => "容量未知".to_string(),
    }
}


/// Compares two directory paths after removing the extended-length prefix and
/// one trailing separator from each. The comparison ignores ASCII case, and
/// separators are compared as written.
pub fn same_directory(a: &str, b: &str) -> bool {
    let a = strip_extended_prefix(a);
    let b = strip_extended_prefix(b);
    trim_one_separator(&a).eq_ignore_ascii_case(trim_one_separator(&b))
}

fn trim_one_separator(path: &str) -> &str {
    path.strip_suffix(is_separator).unwrap_or(path)
}

/// Formats a byte count for display: `512 B`, `1.5 KB`, `5.0 MB`, `3.0 GB`.
///
/// Values below 1024 show the number of bytes. Larger values use binary
/// multiples. The unit is the first of KB, MB and GB whose value, rounded to
/// one decimal place, is below 1024.0; GB is used when none is. The value is
/// shown with one decimal place.
pub fn format_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while unit + 1 < SIZE_UNITS.len() && rounds_to_1024_or_more(value) {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", SIZE_UNITS[unit])
}

/// True when `value`, rounded to one decimal place, is 1024.0 or more.
fn rounds_to_1024_or_more(value: f64) -> bool {
    let rounded: f64 = format!("{value:.1}").parse().unwrap_or(value);
    rounded >= 1024.0
}

/// Converts an RFC 3339 instant to `offset` and formats it as
/// `YYYY-MM-DD HH:MM`. Input that does not parse is returned unchanged.
pub fn format_time(rfc3339: &str, offset: FixedOffset) -> String {
    match DateTime::<FixedOffset>::parse_from_rfc3339(rfc3339) {
        Ok(instant) => instant.with_timezone(&offset).format(TIME_FORMAT).to_string(),
        Err(_) => rfc3339.to_string(),
    }
}

/// Formats a Unix timestamp in seconds as `YYYY-MM-DD HH:MM` in `offset`.
/// A missing value, or a value outside the range of `DateTime::<Utc>`, gives
/// `—`.
pub fn unix_time_text(secs: Option<i64>, offset: FixedOffset) -> String {
    match secs.and_then(|value| DateTime::<Utc>::from_timestamp(value, 0)) {
        Some(instant) => instant.with_timezone(&offset).format(TIME_FORMAT).to_string(),
        None => MISSING.to_string(),
    }
}

/// Describes the changed sections of a version.
///
/// A list that contains the deferred-content marker gives
/// `上一版本内容不在本地，未计算段落`. Otherwise a list that contains the
/// large-content marker gives `内容过大，未计算段落`. Otherwise an empty list
/// gives `—`. Any other list gives its first three keys joined by `、`,
/// followed by ` 等 {n} 项` when it has more than three keys, where `n` is the
/// total number of keys.
pub fn sections_text(sections: &[String]) -> String {
    if sections.iter().any(|key| key == MARKER_DEFERRED_PREVIOUS_COLD) {
        return "上一版本内容不在本地，未计算段落".to_string();
    }
    if sections.iter().any(|key| key == MARKER_LARGE_BINARY_OR_TEXT) {
        return "内容过大，未计算段落".to_string();
    }
    if sections.is_empty() {
        return MISSING.to_string();
    }
    let mut text = sections
        .iter()
        .take(3)
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join("、");
    if sections.len() > 3 {
        text.push_str(&format!(" 等 {} 项", sections.len()));
    }
    text
}

/// Builds the browse rows of one folder.
///
/// There is one row per input entry. Directories come first, then the names
/// compared ASCII-case-insensitively, then the exact names. The ledger is
/// keyed by workspace-relative path; a file is tracked only when its ledger
/// row exists and is not deleted.
pub fn browse_rows(
    entries: &[RawEntry],
    relative_dir: &str,
    ledger: &HashMap<String, LedgerRow>,
    offset: FixedOffset,
) -> Vec<BrowseRow> {
    let mut ordered: Vec<&RawEntry> = entries.iter().collect();
    ordered.sort_by(|a, b| {
        (a.kind != EntryKind::Directory)
            .cmp(&(b.kind != EntryKind::Directory))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    ordered
        .into_iter()
        .map(|entry| browse_row(entry, relative_dir, ledger, offset))
        .collect()
}

fn browse_row(
    entry: &RawEntry,
    relative_dir: &str,
    ledger: &HashMap<String, LedgerRow>,
    offset: FixedOffset,
) -> BrowseRow {
    let relative_path = if relative_dir.is_empty() {
        entry.name.clone()
    } else {
        format!("{relative_dir}/{}", entry.name)
    };
    let mut row = BrowseRow {
        name: entry.name.clone(),
        relative_path: relative_path.clone(),
        is_directory: entry.kind == EntryKind::Directory,
        is_link: entry.kind == EntryKind::Link,
        size_text: MISSING.to_string(),
        modified_text: unix_time_text(entry.modified_unix, offset),
        status_text: MISSING.to_string(),
        file_id: None,
        tracked: false,
        sensitive: false,
        version_no: None,
    };
    match entry.kind {
        EntryKind::Directory => {}
        EntryKind::Link => row.status_text = "链接（不跟随）".to_string(),
        EntryKind::Other => row.status_text = "其他".to_string(),
        EntryKind::File => {
            row.size_text = format_bytes(entry.size);
            row.status_text = "未记录".to_string();
            if let Some(ledger_row) = ledger.get(&relative_path) {
                if !ledger_row.deleted {
                    row.status_text = if ledger_row.sensitive {
                        format!("敏感 v{}", ledger_row.version_no)
                    } else {
                        format!("已记录 v{}", ledger_row.version_no)
                    };
                    row.file_id = Some(ledger_row.file_id.clone());
                    row.tracked = true;
                    row.sensitive = ledger_row.sensitive;
                    row.version_no = Some(ledger_row.version_no);
                }
            }
        }
    }
    row
}

/// Builds the version rows, largest number first.
///
/// Only the first row is marked current. A negative byte size shows `—`.
/// The hash is shown as its first 12 characters.
pub fn version_rows(versions: &[VersionInput], offset: FixedOffset) -> Vec<VersionRow> {
    let mut ordered: Vec<&VersionInput> = versions.iter().collect();
    ordered.sort_by(|a, b| b.number.cmp(&a.number));
    ordered
        .into_iter()
        .enumerate()
        .map(|(index, version)| VersionRow {
            version_id: version.version_id.clone(),
            number: version.number,
            label: format!("v{}", version.number),
            actor: version.actor.clone(),
            reason: version.reason.clone(),
            time_text: format_time(&version.created_at, offset),
            size_text: match u64::try_from(version.byte_size) {
                Ok(bytes) => format_bytes(bytes),
                Err(_) => MISSING.to_string(),
            },
            sections_text: sections_text(&version.changed_sections),
            hash_short: version.content_hash.chars().take(HASH_SHORT_LEN).collect(),
            sensitive: version.sensitive,
            local: version.local,
            availability_text: availability_text(version.local, version.sensitive).to_string(),
            current: index == 0,
        })
        .collect()
}

fn availability_text(local: bool, sensitive: bool) -> &'static str {
    if !local {
        "内容不在本地，暂不可比较"
    } else if sensitive {
        "本地可读 · 敏感，不外传"
    } else {
        "本地可读"
    }
}

/// Computes a line diff of two texts.
///
/// The diff keeps three unchanged lines around each change, and one `Gap`
/// line separates groups that are not adjacent. Lines are kept in document
/// order up to `max_lines`, and `Gap` lines count toward that limit. The
/// totals `added` and `removed` count every change, including the ones that
/// are not kept. `approximate` is true when the diff call took at least
/// `timeout`.
pub fn diff_lines(old: &str, new: &str, max_lines: usize, timeout: Duration) -> DiffResult {
    let started = Instant::now();
    let diff = TextDiff::configure().timeout(timeout).diff_lines(old, new);
    let approximate = started.elapsed() >= timeout;
    let groups = diff.grouped_ops(CONTEXT_LINES);

    let mut sink = LineSink {
        lines: Vec::new(),
        limit: max_lines,
        truncated: false,
    };
    let mut added: usize = 0;
    let mut removed: usize = 0;
    for (group_index, group) in groups.iter().enumerate() {
        if group_index > 0 {
            sink.push(DiffLine {
                kind: LineKind::Gap,
                old_no: None,
                new_no: None,
                text: String::new(),
            });
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let text = strip_line_ending(change.value()).to_string();
                let line = match change.tag() {
                    ChangeTag::Equal => DiffLine {
                        kind: LineKind::Context,
                        old_no: one_based(change.old_index()),
                        new_no: one_based(change.new_index()),
                        text,
                    },
                    ChangeTag::Delete => {
                        removed += 1;
                        DiffLine {
                            kind: LineKind::Removed,
                            old_no: one_based(change.old_index()),
                            new_no: None,
                            text,
                        }
                    }
                    ChangeTag::Insert => {
                        added += 1;
                        DiffLine {
                            kind: LineKind::Added,
                            old_no: None,
                            new_no: one_based(change.new_index()),
                            text,
                        }
                    }
                };
                sink.push(line);
            }
        }
    }

    DiffResult {
        lines: sink.lines,
        added,
        removed,
        truncated: sink.truncated,
        approximate,
    }
}

/// Collects diff lines up to a limit and records whether any were dropped.
struct LineSink {
    lines: Vec<DiffLine>,
    limit: usize,
    truncated: bool,
}

impl LineSink {
    fn push(&mut self, line: DiffLine) {
        if self.lines.len() < self.limit {
            self.lines.push(line);
        } else {
            self.truncated = true;
        }
    }
}

fn one_based(index: Option<usize>) -> Option<usize> {
    index.map(|value| value + 1)
}

/// Removes one trailing `\n` or `\r\n`. A lone `\r` is kept in the text.
fn strip_line_ending(text: &str) -> &str {
    if let Some(stripped) = text.strip_suffix("\r\n") {
        stripped
    } else if let Some(stripped) = text.strip_suffix('\n') {
        stripped
    } else {
        text
    }
}

/// Whether the open folder may be listed again in the background. The refresh waits while a
/// command is in flight, and it needs a workspace that is open and a worker that runs.
pub fn should_refresh(busy: bool, worker_stopped: bool, workspace_open: bool) -> bool {
    !busy && !worker_stopped && workspace_open
}

/// Index of the row whose relative path is `selected_path`, so that a refreshed listing keeps the
/// selection. An empty path and a path with no matching row give `None`.
pub fn selection_after_refresh(rows: &[BrowseRow], selected_path: &str) -> Option<usize> {
    if selected_path.is_empty() {
        return None;
    }
    rows.iter().position(|row| row.relative_path == selected_path)
}

/// Whether a refreshed listing differs from the listing shown. Every field and the order of the
/// rows are compared.
pub fn listing_changed(old: &[BrowseRow], new: &[BrowseRow]) -> bool {
    old != new
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    fn listed(relative_path: &str) -> BrowseRow {
        BrowseRow {
            name: relative_path.to_string(),
            relative_path: relative_path.to_string(),
            is_directory: false,
            is_link: false,
            size_text: "—".to_string(),
            modified_text: "—".to_string(),
            status_text: "未记录".to_string(),
            file_id: None,
            tracked: false,
            sensitive: false,
            version_no: None,
        }
    }

    #[test]
    fn ut_616_should_refresh() {
        assert!(!should_refresh(false, false, false));
        assert!(should_refresh(false, false, true));
        assert!(!should_refresh(false, true, false));
        assert!(!should_refresh(false, true, true));
        assert!(!should_refresh(true, false, false));
        assert!(!should_refresh(true, false, true));
        assert!(!should_refresh(true, true, false));
        assert!(!should_refresh(true, true, true));
    }

    #[test]
    fn ut_617_selection_after_refresh() {
        let rows = vec![listed("a.txt"), listed("docs"), listed("zeta.md")];
        assert_eq!(selection_after_refresh(&rows, "docs"), Some(1));
        assert_eq!(selection_after_refresh(&rows, "zeta.md"), Some(2));
        assert_eq!(selection_after_refresh(&rows, "Docs"), None);
        assert_eq!(selection_after_refresh(&rows, "missing.md"), None);
        assert_eq!(selection_after_refresh(&rows, ""), None);
    }

    #[test]
    fn ut_618_listing_changed() {
        let base = vec![listed("a.txt"), listed("docs"), listed("zeta.md")];
        let mut status_changed = base.clone();
        status_changed[1].status_text = "已记录 v1".to_string();
        let mut one_added = base.clone();
        one_added.push(listed("new.txt"));
        let reversed: Vec<BrowseRow> = base.iter().rev().cloned().collect();
        assert!(!listing_changed(&base, &base.clone()));
        assert!(listing_changed(&base, &status_changed));
        assert!(listing_changed(&base, &one_added));
        assert!(listing_changed(&base, &reversed));
        assert!(!listing_changed(&[], &[]));
        assert!(listing_changed(&[], &base[..1]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).expect("valid offset")
    }

    fn plus8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).expect("valid offset")
    }

    fn minus5() -> FixedOffset {
        FixedOffset::east_opt(-5 * 3600).expect("valid offset")
    }

    fn entry(name: &str, kind: EntryKind, size: u64) -> RawEntry {
        RawEntry {
            name: name.to_string(),
            kind,
            size,
            modified_unix: None,
        }
    }

    fn ledger_row(file_id: &str, version_no: i64, deleted: bool, sensitive: bool) -> LedgerRow {
        LedgerRow {
            file_id: file_id.to_string(),
            version_no,
            deleted,
            sensitive,
        }
    }

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn line(kind: LineKind, old_no: Option<usize>, new_no: Option<usize>, text: &str) -> DiffLine {
        DiffLine {
            kind,
            old_no,
            new_no,
            text: text.to_string(),
        }
    }

    fn find<'a>(rows: &'a [BrowseRow], name: &str) -> &'a BrowseRow {
        rows.iter()
            .find(|row| row.name == name)
            .expect("row present")
    }

    /// Lines 1 to 20, with line 2 and line 18 replaced.
    fn twenty_line_pair() -> (String, String) {
        let old: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        let new: String = (1..=20)
            .map(|i| match i {
                2 => "changed 2\n".to_string(),
                18 => "changed 18\n".to_string(),
                _ => format!("line{i}\n"),
            })
            .collect();
        (old, new)
    }

    #[test]
    fn ut_601_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1023), "1023.0 KB");
        assert_eq!(format_bytes(1024 * 1024 - 1), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 1024), "1024.0 GB");
    }

    #[test]
    fn ut_602_is_binary_zero_byte() {
        assert!(!is_binary(b""));
        assert!(!is_binary("中文 text".as_bytes()));
        assert!(is_binary(&[0u8]));
        let mut zero_at_7999 = vec![b'a'; 8000];
        zero_at_7999[7999] = 0;
        assert!(is_binary(&zero_at_7999));
        let mut zero_at_8000 = vec![b'a'; 8001];
        zero_at_8000[8000] = 0;
        assert!(is_binary(&zero_at_8000));
        assert!(!is_binary(&vec![1u8; 9000]));
        let mut zero_at_8999 = vec![1u8; 9000];
        zero_at_8999[8999] = 0;
        assert!(is_binary(&zero_at_8999));
    }

    #[test]
    fn ut_603_validate_accepts() {
        assert_eq!(validate_relative(""), Ok(String::new()));
        assert_eq!(validate_relative("docs/a.md"), Ok("docs/a.md".to_string()));
        assert_eq!(
            validate_relative("docs\\中文 文件.md"),
            Ok("docs/中文 文件.md".to_string())
        );
        assert_eq!(validate_relative("docs/"), Ok("docs".to_string()));
        assert_eq!(validate_relative("docs\\"), Ok("docs".to_string()));
        assert_eq!(validate_relative("a\\b/c"), Ok("a/b/c".to_string()));
        assert_eq!(validate_relative(".LADONignore"), Ok(".LADONignore".to_string()));
        assert_eq!(validate_relative("docs/.gitignore"), Ok("docs/.gitignore".to_string()));
        assert_eq!(validate_relative("COM0"), Ok("COM0".to_string()));
        assert_eq!(validate_relative("COM10"), Ok("COM10".to_string()));
        assert_eq!(validate_relative("COM10 .txt"), Ok("COM10 .txt".to_string()));
        assert_eq!(validate_relative("CONSOLE"), Ok("CONSOLE".to_string()));
    }

    #[test]
    fn ut_604_validate_prefixes_and_roots() {
        assert_eq!(validate_relative("/\0"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("/etc/x"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("\\windows"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("\\\\server\\share"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("C:"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("c:x"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("C:/x"), Err(PathProblem::Absolute));
        assert_eq!(validate_relative("1:"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("docs//"), Err(PathProblem::Invalid));
    }

    #[test]
    fn ut_605_validate_components() {
        assert_eq!(check_component(".."), Err(PathProblem::Traversal));
        assert_eq!(check_component(".ladon"), Err(PathProblem::Internal));
        assert_eq!(check_component("file.txt"), Ok(()));
        assert_eq!(check_component(""), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("a//b"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("a/./b"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("a/../b"), Err(PathProblem::Traversal));
        assert_eq!(validate_relative("../x"), Err(PathProblem::Traversal));
        assert_eq!(validate_relative(".GIT "), Err(PathProblem::Internal));
        assert_eq!(validate_relative(".ladon."), Err(PathProblem::Internal));
        assert_eq!(
            validate_relative("docs/.ladon/meta.turso"),
            Err(PathProblem::Internal)
        );
        assert_eq!(validate_relative(".ladon:x"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative(" "), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("..."), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("a."), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("a "), Err(PathProblem::Invalid));
        assert_eq!(validate_relative(".git/.."), Err(PathProblem::Internal));
        assert_eq!(validate_relative("../.git"), Err(PathProblem::Traversal));
    }

    #[test]
    fn ut_606_validate_device_names() {
        assert_eq!(validate_relative("CON"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("con.txt"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("NUL.tar.gz"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("x/LPT9.log"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("aux/x"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("Com1"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("prn"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("CON .txt"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("Com1 .log"), Err(PathProblem::Invalid));
        assert_eq!(validate_relative("aux . x"), Err(PathProblem::Invalid));
    }

    #[test]
    fn ut_607_strip_prefix_and_drive_root() {
        assert_eq!(strip_extended_prefix(r"\\?\C:\Users"), r"C:\Users");
        assert_eq!(
            strip_extended_prefix(r"\\?\UNC\server\share"),
            r"\\server\share"
        );
        assert_eq!(strip_extended_prefix(r"\\server\share"), r"\\server\share");
        assert_eq!(strip_extended_prefix(r"C:\Windows"), r"C:\Windows");
        assert!(matches!(strip_extended_prefix(r"\\?\C:\Users"), Cow::Borrowed(_)));
        assert!(matches!(strip_extended_prefix(r"\\?\UNC\server\share"), Cow::Owned(_)));
        assert!(matches!(strip_extended_prefix(r"\\server\share"), Cow::Borrowed(_)));
        assert!(matches!(strip_extended_prefix(r"C:\Windows"), Cow::Borrowed(_)));
        assert!(is_drive_root("C:"));
        assert!(is_drive_root(r"C:\"));
        assert!(is_drive_root("c:/"));
        assert!(is_drive_root(r"\\?\D:\"));
        assert!(!is_drive_root(r"C:\Windows"));
        assert!(!is_drive_root(r"\\?\UNC\server\share"));
        assert!(!is_drive_root("1:\\"));
        assert!(!is_drive_root("CC:"));
    }

    #[test]
    fn ut_608_same_directory() {
        assert!(same_directory(r"C:\Users\Me\", r"c:\users\me"));
        assert!(same_directory(r"\\?\C:\Users\Me", r"C:\Users\Me"));
        assert!(!same_directory(r"C:\Users\Me\\", r"C:\Users\Me"));
        assert!(!same_directory(r"C:\Users\Me", r"C:\Users\Other"));
        assert!(!same_directory(r"\\?\UNC\server\share", r"C:\Users\Me"));
    }

    #[test]
    fn ut_609_time_text() {
        assert_eq!(
            format_time("2026-11-19T10:00:00Z", plus8()),
            "2026-11-19 18:00"
        );
        assert_eq!(
            format_time("2026-11-19T10:00:00Z", minus5()),
            "2026-11-19 05:00"
        );
        assert_eq!(format_time("not a time", plus8()), "not a time");
        assert_eq!(format_time("", utc()), "");
        assert_eq!(
            format_time("2026-13-01T00:00:00Z", utc()),
            "2026-13-01T00:00:00Z"
        );
        assert_eq!(unix_time_text(Some(0), utc()), "1970-01-01 00:00");
        assert_eq!(
            unix_time_text(Some(1_700_000_000), plus8()),
            "2023-11-15 06:13"
        );
        assert_eq!(unix_time_text(None, utc()), "—");
        assert_eq!(unix_time_text(Some(i64::MAX), utc()), "—");
        assert_eq!(unix_time_text(Some(100_000_000_000_000), utc()), "—");
    }

    #[test]
    fn ut_610_sections_text() {
        assert_eq!(sections_text(&[]), "—");
        assert_eq!(sections_text(&owned(&["A"])), "A");
        assert_eq!(sections_text(&owned(&["A", "B", "C"])), "A、B、C");
        assert_eq!(
            sections_text(&owned(&["A", "B", "C", "D", "E"])),
            "A、B、C 等 5 项"
        );
        assert_eq!(
            sections_text(&owned(&["DIFF_DEFERRED_PREVIOUS_COLD"])),
            "上一版本内容不在本地，未计算段落"
        );
        assert_eq!(
            sections_text(&owned(&["LARGE_BINARY_OR_TEXT"])),
            "内容过大，未计算段落"
        );
        assert_eq!(
            sections_text(&owned(&["A", "DIFF_DEFERRED_PREVIOUS_COLD"])),
            "上一版本内容不在本地，未计算段落"
        );
        assert_eq!(
            sections_text(&owned(&["A", "LARGE_BINARY_OR_TEXT"])),
            "内容过大，未计算段落"
        );
        assert_eq!(
            sections_text(&owned(&["LARGE_BINARY_OR_TEXT", "DIFF_DEFERRED_PREVIOUS_COLD"])),
            "上一版本内容不在本地，未计算段落"
        );
    }

    #[test]
    fn ut_611_browse_rows_order_and_paths() {
        let entries = vec![
            entry("zeta.md", EntryKind::File, 1),
            entry("Zed", EntryKind::Directory, 0),
            entry("beta.txt", EntryKind::File, 1),
            entry("Alpha", EntryKind::Directory, 0),
            entry("link", EntryKind::Link, 0),
            entry("Same.md", EntryKind::File, 1),
            entry("same.md", EntryKind::File, 1),
            entry("Gamma.md", EntryKind::File, 1),
        ];
        let ledger: HashMap<String, LedgerRow> = HashMap::new();
        let root_rows = browse_rows(&entries, "", &ledger, utc());
        let names: Vec<String> = root_rows.iter().map(|row| row.name.clone()).collect();
        assert_eq!(
            names,
            owned(&[
                "Alpha", "Zed", "beta.txt", "Gamma.md", "link", "Same.md", "same.md", "zeta.md",
            ])
        );
        assert_eq!(find(&root_rows, "link").relative_path, "link");
        let docs_rows = browse_rows(&entries, "docs", &ledger, utc());
        assert_eq!(find(&docs_rows, "Alpha").relative_path, "docs/Alpha");
        assert_eq!(find(&docs_rows, "zeta.md").relative_path, "docs/zeta.md");
    }

    #[test]
    fn ut_612_browse_rows_status() {
        let entries = vec![
            RawEntry {
                name: "zeta.md".to_string(),
                kind: EntryKind::File,
                size: 1536,
                modified_unix: Some(0),
            },
            RawEntry {
                name: "beta.txt".to_string(),
                kind: EntryKind::File,
                size: 0,
                modified_unix: None,
            },
            RawEntry {
                name: "Gamma.md".to_string(),
                kind: EntryKind::File,
                size: 5 * 1024 * 1024,
                modified_unix: None,
            },
            RawEntry {
                name: "untracked.txt".to_string(),
                kind: EntryKind::File,
                size: 0,
                modified_unix: None,
            },
            RawEntry {
                name: "link".to_string(),
                kind: EntryKind::Link,
                size: 10,
                modified_unix: None,
            },
            RawEntry {
                name: "Alpha".to_string(),
                kind: EntryKind::Directory,
                size: 4096,
                modified_unix: None,
            },
            RawEntry {
                name: "other.bin".to_string(),
                kind: EntryKind::Other,
                size: 7,
                modified_unix: None,
            },
        ];
        let mut ledger: HashMap<String, LedgerRow> = HashMap::new();
        ledger.insert("docs/zeta.md".to_string(), ledger_row("file-zeta", 3, false, false));
        ledger.insert("docs/beta.txt".to_string(), ledger_row("file-beta", 2, true, false));
        ledger.insert("docs/Gamma.md".to_string(), ledger_row("file-gamma", 1, false, true));
        ledger.insert("docs/link".to_string(), ledger_row("file-link", 1, false, false));
        ledger.insert("docs/Alpha".to_string(), ledger_row("file-alpha", 1, false, false));
        let rows = browse_rows(&entries, "docs", &ledger, utc());

        let zeta = find(&rows, "zeta.md");
        assert_eq!(zeta.status_text, "已记录 v3");
        assert!(zeta.tracked);
        assert_eq!(zeta.file_id, Some("file-zeta".to_string()));
        assert_eq!(zeta.version_no, Some(3));
        assert!(!zeta.sensitive);
        assert_eq!(zeta.size_text, "1.5 KB");
        assert_eq!(zeta.modified_text, "1970-01-01 00:00");

        let beta = find(&rows, "beta.txt");
        assert_eq!(beta.status_text, "未记录");
        assert!(!beta.tracked);
        assert_eq!(beta.file_id, None);
        assert_eq!(beta.version_no, None);
        assert_eq!(beta.modified_text, "—");

        let gamma = find(&rows, "Gamma.md");
        assert_eq!(gamma.status_text, "敏感 v1");
        assert!(gamma.tracked);
        assert!(gamma.sensitive);
        assert_eq!(gamma.file_id, Some("file-gamma".to_string()));
        assert_eq!(gamma.size_text, "5.0 MB");

        let untracked = find(&rows, "untracked.txt");
        assert_eq!(untracked.status_text, "未记录");
        assert_eq!(untracked.size_text, "0 B");
        assert!(!untracked.tracked);
        assert_eq!(untracked.file_id, None);

        let link = find(&rows, "link");
        assert!(link.is_link);
        assert!(!link.is_directory);
        assert_eq!(link.status_text, "链接（不跟随）");
        assert_eq!(link.size_text, "—");
        assert!(!link.tracked);
        assert_eq!(link.file_id, None);

        let alpha = find(&rows, "Alpha");
        assert!(alpha.is_directory);
        assert_eq!(alpha.status_text, "—");
        assert_eq!(alpha.size_text, "—");
        assert!(!alpha.tracked);
        assert_eq!(alpha.file_id, None);

        let other = find(&rows, "other.bin");
        assert_eq!(other.status_text, "其他");
        assert_eq!(other.size_text, "—");
        assert!(!other.tracked);
    }

    #[test]
    fn ut_613_version_rows() {
        let versions = vec![
            VersionInput {
                version_id: "v-1".to_string(),
                number: 1,
                content_hash: "0123456789abcdef0123".to_string(),
                byte_size: 1536,
                sensitive: false,
                changed_sections: Vec::new(),
                actor: "alice".to_string(),
                reason: "manual capture".to_string(),
                created_at: "2026-11-19T10:00:00Z".to_string(),
                local: true,
            },
            VersionInput {
                version_id: "v-3".to_string(),
                number: 3,
                content_hash: "0123456789abcdef".to_string(),
                byte_size: -1,
                sensitive: true,
                changed_sections: owned(&["A", "B", "C", "D"]),
                actor: "bob".to_string(),
                reason: "gui-capture".to_string(),
                created_at: "2026-11-19T10:00:00Z".to_string(),
                local: true,
            },
            VersionInput {
                version_id: "v-2".to_string(),
                number: 2,
                content_hash: "abc".to_string(),
                byte_size: 5 * 1024 * 1024,
                sensitive: true,
                changed_sections: Vec::new(),
                actor: "carol".to_string(),
                reason: "gui-capture".to_string(),
                created_at: "2026-11-18T23:30:00+00:00".to_string(),
                local: false,
            },
        ];
        let rows = version_rows(&versions, plus8());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].number, 3);
        assert_eq!(rows[1].number, 2);
        assert_eq!(rows[2].number, 1);
        assert!(rows[0].current);
        assert!(!rows[1].current);
        assert!(!rows[2].current);

        assert_eq!(rows[0].label, "v3");
        assert_eq!(rows[0].hash_short, "0123456789ab");
        assert_eq!(rows[0].size_text, "—");
        assert_eq!(rows[0].sections_text, "A、B、C 等 4 项");
        assert_eq!(rows[0].time_text, "2026-11-19 18:00");
        assert_eq!(rows[0].availability_text, "本地可读 · 敏感，不外传");
        assert!(rows[0].sensitive);
        assert!(rows[0].local);
        assert_eq!(rows[0].actor, "bob");
        assert_eq!(rows[0].reason, "gui-capture");
        assert_eq!(rows[0].version_id, "v-3");

        assert_eq!(rows[1].label, "v2");
        assert_eq!(rows[1].hash_short, "abc");
        assert_eq!(rows[1].size_text, "5.0 MB");
        assert_eq!(rows[1].sections_text, "—");
        assert_eq!(rows[1].time_text, "2026-11-19 07:30");
        assert_eq!(rows[1].availability_text, "内容不在本地，暂不可比较");
        assert!(!rows[1].local);

        assert_eq!(rows[2].label, "v1");
        assert_eq!(rows[2].hash_short, "0123456789ab");
        assert_eq!(rows[2].size_text, "1.5 KB");
        assert_eq!(rows[2].time_text, "2026-11-19 18:00");
        assert_eq!(rows[2].availability_text, "本地可读");
        assert!(!rows[2].sensitive);
        assert_eq!(rows[2].actor, "alice");
        assert_eq!(rows[2].reason, "manual capture");

        assert!(version_rows(&[], utc()).is_empty());
    }

    #[test]
    fn ut_614_diff_lines_content() {
        let timeout = Duration::from_secs(2);

        let same = diff_lines("a\nb\n", "a\nb\n", 100, timeout);
        assert!(same.lines.is_empty());
        assert_eq!((same.added, same.removed), (0, 0));
        assert!(!same.truncated);
        assert!(!same.approximate);
        assert!(diff_lines("", "", 100, timeout).lines.is_empty());

        let crlf = diff_lines("one\r\ntwo\r\n", "one\r\n2wo\r\n", 100, timeout);
        assert_eq!(
            crlf.lines,
            vec![
                line(LineKind::Context, Some(1), Some(1), "one"),
                line(LineKind::Removed, Some(2), None, "two"),
                line(LineKind::Added, None, Some(2), "2wo"),
            ]
        );
        assert_eq!((crlf.added, crlf.removed), (1, 1));

        let lone_cr = diff_lines("a\rb\n", "a\rc\n", 100, timeout);
        assert_eq!(
            lone_cr.lines,
            vec![
                line(LineKind::Context, Some(1), Some(1), "a\r"),
                line(LineKind::Removed, Some(2), None, "b"),
                line(LineKind::Added, None, Some(2), "c"),
            ]
        );
        assert_eq!((lone_cr.added, lone_cr.removed), (1, 1));

        let lone_cr_only = diff_lines("x\r", "y\r", 100, timeout);
        assert_eq!(
            lone_cr_only.lines,
            vec![
                line(LineKind::Removed, Some(1), None, "x\r"),
                line(LineKind::Added, None, Some(1), "y\r"),
            ]
        );
        assert_eq!((lone_cr_only.added, lone_cr_only.removed), (1, 1));

        let (old, new) = twenty_line_pair();
        let gap = diff_lines(&old, &new, 100, timeout);
        assert_eq!(
            gap.lines,
            vec![
                line(LineKind::Context, Some(1), Some(1), "line1"),
                line(LineKind::Removed, Some(2), None, "line2"),
                line(LineKind::Added, None, Some(2), "changed 2"),
                line(LineKind::Context, Some(3), Some(3), "line3"),
                line(LineKind::Context, Some(4), Some(4), "line4"),
                line(LineKind::Context, Some(5), Some(5), "line5"),
                line(LineKind::Gap, None, None, ""),
                line(LineKind::Context, Some(15), Some(15), "line15"),
                line(LineKind::Context, Some(16), Some(16), "line16"),
                line(LineKind::Context, Some(17), Some(17), "line17"),
                line(LineKind::Removed, Some(18), None, "line18"),
                line(LineKind::Added, None, Some(18), "changed 18"),
                line(LineKind::Context, Some(19), Some(19), "line19"),
                line(LineKind::Context, Some(20), Some(20), "line20"),
            ]
        );
        assert_eq!((gap.added, gap.removed), (2, 2));
        assert!(!gap.truncated);
    }

    #[test]
    fn ut_615_diff_lines_limits() {
        let timeout = Duration::from_secs(2);

        let old: String = (1..=100).map(|i| format!("old{i}\n")).collect();
        let new: String = (1..=100).map(|i| format!("new{i}\n")).collect();
        let capped = diff_lines(&old, &new, 10, timeout);
        assert_eq!(capped.lines.len(), 10);
        assert_eq!(capped.lines[0], line(LineKind::Removed, Some(1), None, "old1"));
        assert_eq!(capped.lines[9], line(LineKind::Removed, Some(10), None, "old10"));
        assert!(capped.truncated);
        assert_eq!((capped.added, capped.removed), (100, 100));

        let none = diff_lines("a\n", "b\n", 0, timeout);
        assert!(none.lines.is_empty());
        assert!(none.truncated);
        assert_eq!((none.added, none.removed), (1, 1));

        let one = diff_lines("a\n", "b\n", 1, timeout);
        assert_eq!(one.lines, vec![line(LineKind::Removed, Some(1), None, "a")]);
        assert!(one.truncated);

        let exact = diff_lines("a\n", "b\n", 2, timeout);
        assert_eq!(
            exact.lines,
            vec![
                line(LineKind::Removed, Some(1), None, "a"),
                line(LineKind::Added, None, Some(1), "b"),
            ]
        );
        assert!(!exact.truncated);
        assert!(!exact.approximate);

        let (old, new) = twenty_line_pair();
        let gap_capped = diff_lines(&old, &new, 7, timeout);
        assert_eq!(gap_capped.lines.len(), 7);
        assert_eq!(gap_capped.lines[6].kind, LineKind::Gap);
        assert!(gap_capped.truncated);
        assert_eq!((gap_capped.added, gap_capped.removed), (2, 2));

        let zero_timeout = diff_lines("a\n", "b\n", 100, Duration::ZERO);
        assert!(zero_timeout.approximate);
        assert_eq!((zero_timeout.added, zero_timeout.removed), (1, 1));
    }

    #[test]
    fn ut_619_drive_rows_and_status() {
        let rows = drive_rows(&[('d', false), ('C', true), ('1', true), ('C', false)]);
        assert_eq!(
            rows,
            vec![
                DriveRow {
                    root: r"C:\".to_string(),
                    label: "本地磁盘 (C:)".to_string(),
                    initialised: true,
                    total_bytes: None,
                    free_bytes: None,
                },
                DriveRow {
                    root: r"D:\".to_string(),
                    label: "本地磁盘 (D:)".to_string(),
                    initialised: false,
                    total_bytes: None,
                    free_bytes: None,
                },
            ]
        );
        assert_eq!(drive_status_text(true), "已是工作区");
        assert_eq!(drive_status_text(false), "未初始化");
    }

    #[test]
    fn ut_654_tree_rows_and_child_paths() {
        let drives = vec![
            DriveRow { root: "C:\\".into(), label: "本地磁盘 (C:)".into(), initialised: true, total_bytes: None, free_bytes: None },
            DriveRow { root: "D:\\".into(), label: "本地磁盘 (D:)".into(), initialised: false, total_bytes: None, free_bytes: None },
        ];
        let rows = drive_tree_rows(&drives);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            TreeRow { path: "C:\\".into(), name: "本地磁盘 (C:)".into(), depth: 0, expanded: false }
        );
        assert_eq!(rows[1].path, "D:\\");
        assert_eq!(child_path("C:\\", "docs"), "C:\\docs");
        assert_eq!(child_path("C:\\docs", "a.txt"), "C:\\docs\\a.txt");
    }

    #[test]
    fn ut_655_tree_expand_and_collapse() {
        let mut rows = drive_tree_rows(&[
            DriveRow { root: "C:\\".into(), label: "C".into(), initialised: true, total_bytes: None, free_bytes: None },
            DriveRow { root: "D:\\".into(), label: "D".into(), initialised: false, total_bytes: None, free_bytes: None },
        ]);
        let names = vec!["a".to_string(), "b".to_string()];
        assert!(tree_expand(&mut rows, 0, &names));
        let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
        assert_eq!(paths, ["C:\\", "C:\\a", "C:\\b", "D:\\"]);
        assert_eq!(rows[1].depth, 1);
        assert!(rows[0].expanded);
        assert!(!tree_expand(&mut rows, 0, &names), "an expanded row is not expanded again");
        assert_eq!(tree_index(&rows, "c:\\"), Some(0));
        assert_eq!(tree_index(&rows, "C:\\A"), Some(1));
        assert_eq!(tree_index(&rows, "C:\\x"), None);
        assert!(tree_collapse(&mut rows, 0));
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].expanded);
        assert!(!tree_collapse(&mut rows, 1), "a row that is not expanded does not collapse");
    }

    #[test]
    fn ut_656_drive_root_and_relative_path() {
        assert_eq!(drive_root_of("C:\\docs\\x").as_deref(), Some("C:\\"));
        assert_eq!(drive_root_of("C:\\").as_deref(), Some("C:\\"));
        assert_eq!(drive_root_of("C:").as_deref(), Some("C:\\"));
        assert_eq!(drive_root_of("\\\\server\\share"), None);
        assert_eq!(relative_in_root("C:\\", "C:\\docs\\x").as_deref(), Some("docs/x"));
        assert_eq!(relative_in_root("C:\\", "C:\\").as_deref(), Some(""));
        assert_eq!(relative_in_root("D:\\", "C:\\x"), None);
    }
}

#[cfg(test)]
mod name_and_capacity_tests {
    use super::*;

    #[test]
    fn ut_657_check_new_name() {
        assert!(check_new_name("新建文件夹").is_ok());
        assert!(check_new_name("a.txt").is_ok());
        for bad in [
            "", "a/b", "a:b", "CON", "con.txt", "NUL", "COM1", "LPT9", "trail.", "trail ",
            "CON .txt", "Com1 .log",
        ] {
            assert_eq!(check_new_name(bad), Err(MSG_NAME_INVALID), "{bad}");
        }
        assert!(check_new_name("CONSOLE .txt").is_ok());
        assert!(check_new_name("COM10 .txt").is_ok());
        assert_eq!(check_new_name(&"a".repeat(256)), Err(MSG_NAME_INVALID));
    }

    #[test]
    fn ut_658_capacity_text() {
        assert_eq!(capacity_text(Some(10 * 1024 * 1024 * 1024), Some(4 * 1024 * 1024 * 1024)), "可用 4.0 GB / 共 10.0 GB");
        assert_eq!(capacity_text(None, Some(1)), "容量未知");
        assert_eq!(capacity_text(Some(1), None), "容量未知");
    }

    #[test]
    fn ut_659_drive_rows_have_no_capacity() {
        let rows = drive_rows(&[('C', true), ('D', false)]);
        assert!(rows.iter().all(|row| row.total_bytes.is_none() && row.free_bytes.is_none()));
    }

    #[test]
    fn ut_660_check_new_name_refuses_rule_file() {
        for name in [".LADONignore", ".ladonignore", ".LadonIgnore"] {
            let result = check_new_name(name);
            assert!(result.is_err(), "rule file name must be refused: {name}");
            let message = result.unwrap_err();
            assert!(message.contains(".LADONignore"), "message must name the rule file: {message}");
        }
    }
}
