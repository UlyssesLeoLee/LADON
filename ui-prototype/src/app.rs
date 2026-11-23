//! Command handler and worker thread of the LADON Explorer window.
//!
//! This is the only module of the window that calls the kernel. It requires the
//! `kernel` feature. Every command yields exactly one reply.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{FixedOffset, Local};
use ladon_explorer::engine::Engine;

use crate::model;

/// Largest version content loaded for a text diff, in bytes.
pub const DIFF_LIMIT_BYTES: u64 = 2 * 1024 * 1024;
/// Most diff lines kept in one reply.
pub const DIFF_MAX_LINES: usize = 4000;
/// Time budget of one text diff.
pub const DIFF_TIMEOUT: Duration = Duration::from_secs(2);
/// Most folder rows kept in one reply.
pub const LIST_LIMIT: usize = 5000;

pub const MSG_NO_WORKSPACE: &str = "请先打开或初始化一个工作区。";
pub const MSG_INVALID_PATH: &str =
    "该路径无效，或包含链接、重解析点，或位于工作区之外，已拒绝访问。";
pub const MSG_NOT_FOUND: &str = "找不到该文件、文件夹或版本。";
pub const MSG_NOT_CONFIRMED: &str = "需要先确认才能初始化该文件夹。";
pub const MSG_ROOT_REFUSED: &str =
    "不能把用户主目录本身作为工作区，请选择其中的子文件夹。";
pub const MSG_RULE_FILE_REFUSED: &str = "规则文件 .LADONignore 不作为历史记录对象。";
pub const MSG_INTERNAL_REFUSED: &str = "内部目录 .ladon 和 .git 不作为历史记录对象。";
pub const MSG_RULE_PARSE_FAILED: &str =
    "规则文件 .LADONignore 无法解析，已停止记录。请先修正该文件。";
pub const MSG_OPEN_FAILED: &str =
    "无法打开该工作区。请检查 .ladon 目录和 .LADONignore 规则文件是否完整。";
pub const MSG_INIT_FAILED: &str = "无法初始化该文件夹。请检查该文件夹的写入权限。";
pub const MSG_LIST_FAILED: &str = "无法读取该文件夹。";
pub const MSG_CAPTURE_FAILED: &str = "无法记录该文件，请稍后重试。";
pub const MSG_HISTORY_FAILED: &str = "无法读取该文件的历史。";
pub const MSG_DIFF_FAILED: &str = "无法读取这两个版本的内容。";
pub const MSG_INTERNAL: &str = "内部错误，操作未能完成。请重试；若再次出现，请重新打开应用。";
pub const MSG_DIFF_BINARY: &str = "二进制文件，不显示文本差异。";
pub const MSG_DIFF_NOT_UTF8: &str = "文件不是 UTF-8 文本，不显示文本差异。";
pub const MSG_DIFF_TOO_LARGE: &str = "文件超过 2 MiB，不显示文本差异。";
pub const MSG_DIFF_COLD: &str = "该版本的内容不在本地，暂不能比较。";
pub const MSG_DIFF_APPROXIMATE: &str = "差异计算超过 2 秒，结果可能不是最小改动。";
pub const MSG_DIFF_TRUNCATED: &str = "差异超过 4000 行，仅显示前 4000 行。";

/// Name of the rule file in the workspace root.
const RULE_FILE: &str = ".LADONignore";
/// Metadata database of a workspace, relative to its root.
const META_DB: &str = ".ladon/meta.turso";
/// Entries that the kernel owns under a workspace root, checked in this order.
const OWNED_ENTRIES: [&str; 4] = [".ladon", META_DB, ".ladon/cas", RULE_FILE];
/// FILE_ATTRIBUTE_REPARSE_POINT from the Windows file attribute set.
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

/// One request from the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Open { path: String },
    Init { path: String, confirmed: bool },
    ListFolder { relative_dir: String },
    Capture { relative_path: String },
    LoadHistory { relative_path: String },
    LoadDiff { older_version_id: String, newer_version_id: String },
    ListDrives,
}

/// Why a text diff is not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffBlock {
    Binary,
    NotUtf8,
    TooLarge,
    Cold,
}

/// Category of a failed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    NoWorkspace,
    InvalidPath,
    NotFound,
    NotConfirmed,
    Refused,
    Kernel,
    Internal,
}

/// Exactly one reply per command.
#[derive(Debug)]
pub enum Reply {
    Opened { root: String },
    NeedsInit { path: String },
    Folder { relative_dir: String, rows: Vec<model::BrowseRow>, truncated: bool },
    Captured { relative_path: String, version_no: i64, changed: bool },
    History {
        relative_path: String,
        versions: Vec<model::VersionRow>,
        chain_valid: bool,
        event_count: usize,
    },
    Diff { older_version_id: String, newer_version_id: String, diff: model::DiffResult },
    DiffUnavailable { older_version_id: String, newer_version_id: String, reason: DiffBlock },
    Drives { drives: Vec<model::DriveRow> },
    Failed { kind: FailKind, message: String, detail: Option<String> },
}

/// Workspace state owned by the worker thread: the open engine, and the rule file
/// modification time that the engine's rules were loaded with.
pub struct State {
    engine: Option<Engine>,
    rule_mtime: Option<SystemTime>,
}

impl State {
    /// A state without an open workspace.
    pub fn new() -> State {
        State { engine: None, rule_mtime: None }
    }
}

/// Handle to the worker thread. Commands sent through it run one after another.
pub struct Worker {
    sender: Sender<Command>,
}

impl Worker {
    /// Starts the worker thread. `on_reply` receives every reply on that thread.
    pub fn spawn<F: Fn(Reply) + Send + 'static>(on_reply: F) -> Worker {
        let (sender, receiver) = mpsc::channel::<Command>();
        thread::spawn(move || run_worker(receiver, on_reply));
        Worker { sender }
    }

    /// Queues a command. Returns false when the worker thread has stopped.
    pub fn send(&self, command: Command) -> bool {
        self.sender.send(command).is_ok()
    }
}

fn run_worker<F: Fn(Reply)>(receiver: Receiver<Command>, on_reply: F) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok();
    let mut state = State::new();
    while let Ok(command) = receiver.recv() {
        let reply = match runtime.as_ref() {
            Some(runtime) => guard(|| runtime.block_on(handle(&mut state, command))),
            None => failed(FailKind::Internal, MSG_INTERNAL),
        };
        on_reply(reply);
    }
}

/// Runs one command and returns its reply. It does not catch panics; `guard` does.
pub async fn handle(state: &mut State, command: Command) -> Reply {
    match command {
        Command::Open { path } => open_workspace(state, &path).await,
        Command::Init { path, confirmed } => init_workspace(state, &path, confirmed).await,
        Command::ListFolder { relative_dir } => list_folder(state, &relative_dir).await,
        Command::Capture { relative_path } => capture_file(state, &relative_path).await,
        Command::LoadHistory { relative_path } => load_history(state, &relative_path).await,
        Command::LoadDiff { older_version_id, newer_version_id } => {
            load_diff(state, &older_version_id, &newer_version_id).await
        }
        Command::ListDrives => list_drives(),
    }
}

/// Runs `f`; a panic becomes an internal failure reply without detail.
pub fn guard<F: FnOnce() -> Reply>(f: F) -> Reply {
    panic::catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| failed(FailKind::Internal, MSG_INTERNAL))
}

/// Message shown in place of a text diff that is not available.
pub fn diff_block_message(reason: DiffBlock) -> &'static str {
    match reason {
        DiffBlock::Binary => MSG_DIFF_BINARY,
        DiffBlock::NotUtf8 => MSG_DIFF_NOT_UTF8,
        DiffBlock::TooLarge => MSG_DIFF_TOO_LARGE,
        DiffBlock::Cold => MSG_DIFF_COLD,
    }
}

fn failed(kind: FailKind, message: &str) -> Reply {
    Reply::Failed { kind, message: message.to_string(), detail: None }
}

fn invalid_path() -> Reply {
    failed(FailKind::InvalidPath, MSG_INVALID_PATH)
}

fn workspace(state: &State) -> Result<&Engine, Reply> {
    state
        .engine
        .as_ref()
        .ok_or_else(|| failed(FailKind::NoWorkspace, MSG_NO_WORKSPACE))
}

/// Validates a workspace-relative path with `model::validate_relative`.
fn checked_relative(input: &str) -> Result<String, Reply> {
    model::validate_relative(input).map_err(|problem| match problem {
        model::PathProblem::Internal => failed(FailKind::Refused, MSG_INTERNAL_REFUSED),
        model::PathProblem::Absolute
        | model::PathProblem::Traversal
        | model::PathProblem::Invalid => invalid_path(),
    })
}

/// Components of a validated relative path; the empty path has none.
fn components_of(relative: &str) -> Vec<String> {
    if relative.is_empty() {
        Vec::new()
    } else {
        relative.split('/').map(str::to_owned).collect()
    }
}

/// Splits a validated relative path into its parent and its last component.
fn split_parent(relative: &str) -> (&str, &str) {
    relative.rsplit_once('/').unwrap_or(("", relative))
}

/// Path text for replies, without the verbatim prefix.
fn display_path(path: &Path) -> String {
    model::strip_extended_prefix(&path.to_string_lossy()).to_string()
}

/// Workspace-relative key with `/` separators for a path under `root`.
fn relative_key(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

/// Local UTC offset at the time of the command.
fn local_offset() -> FixedOffset {
    *Local::now().offset()
}

/// Modification time of the rule file, or None when it cannot be read.
fn rule_file_mtime(root: &Path) -> Option<SystemTime> {
    fs::metadata(root.join(RULE_FILE))
        .and_then(|meta| meta.modified())
        .ok()
}

#[cfg(windows)]
fn has_reparse_point(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
fn has_reparse_point(_meta: &Metadata) -> bool {
    false
}

/// True for symbolic links and for entries with the reparse-point attribute.
fn is_link_or_reparse(meta: &Metadata) -> bool {
    meta.file_type().is_symlink() || has_reparse_point(meta)
}

/// Seconds since the Unix epoch of the modification time, when representable.
fn unix_seconds(meta: &Metadata) -> Option<i64> {
    let since_epoch = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_secs()).ok()
}

/// True when `canonical` is the user profile directory. The value of USERPROFILE is
/// compared in its canonical form; the value as set is compared when it cannot be
/// canonicalised. An unset USERPROFILE matches nothing.
fn is_profile_dir(canonical: &str) -> bool {
    let Some(profile) = std::env::var_os("USERPROFILE") else {
        return false;
    };
    let profile_text = match fs::canonicalize(Path::new(&profile)) {
        Ok(resolved) => resolved.to_string_lossy().into_owned(),
        Err(_) => profile.to_string_lossy().into_owned(),
    };
    model::same_directory(canonical, &profile_text)
}

/// A workspace root that passed the root candidate checks.
struct Candidate {
    root: PathBuf,
    initialised: bool,
}

/// Checks every component after the drive or root prefix with `model::check_component`.
///
/// The text after the prefix is split in the same way as a relative path: one leading
/// separator and one trailing separator are removed, and the rest is split on `\` and
/// `/`. Empty components are therefore checked too and give InvalidPath.
fn check_root_components(input: &str, path: &Path) -> Result<(), Reply> {
    let prefix_len = match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            if matches!(prefix.kind(), Prefix::DeviceNS(_)) {
                return Err(invalid_path());
            }
            match prefix.as_os_str().to_str() {
                Some(text) => text.len(),
                None => return Err(invalid_path()),
            }
        }
        _ => 0,
    };
    let rest = input.get(prefix_len..).ok_or_else(invalid_path)?;
    let is_separator = |c: char| c == '\\' || c == '/';
    let body = rest.strip_prefix(is_separator).unwrap_or(rest);
    let body = body.strip_suffix(is_separator).unwrap_or(body);
    if body.is_empty() {
        return Ok(());
    }
    for component in body.split(is_separator) {
        match model::check_component(component) {
            Ok(()) => {}
            Err(model::PathProblem::Internal) => {
                return Err(failed(FailKind::Refused, MSG_INTERNAL_REFUSED));
            }
            Err(_) => return Err(invalid_path()),
        }
    }
    Ok(())
}

/// Root candidate checks used by Open and Init. `io_message` is the message for an
/// unexpected I/O failure of the command.
fn root_candidate(input: &str, io_message: &'static str) -> Result<Candidate, Reply> {
    if input.contains('\0') || !Path::new(input).is_absolute() {
        return Err(invalid_path());
    }
    check_root_components(input, Path::new(input))?;
    // Rebuilding the path from its components drops a trailing separator, so the
    // entry named by the path is the one that is checked.
    let path: PathBuf = Path::new(input).components().collect();
    let mut chain: Vec<&Path> = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .filter(|ancestor| !matches!(ancestor.components().next_back(), Some(Component::Prefix(_))))
        .collect();
    chain.reverse();
    let mut last = None;
    for step in chain {
        match fs::symlink_metadata(step) {
            Ok(meta) => {
                if is_link_or_reparse(&meta) {
                    return Err(invalid_path());
                }
                last = Some(meta);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(failed(FailKind::NotFound, MSG_NOT_FOUND));
            }
            Err(_) => return Err(failed(FailKind::Kernel, io_message)),
        }
    }
    if !last.is_some_and(|meta| meta.is_dir()) {
        return Err(invalid_path());
    }
    let root = fs::canonicalize(&path).map_err(|_| failed(FailKind::Kernel, io_message))?;
    let shown = root.to_string_lossy().into_owned();
    if is_profile_dir(&shown) {
        return Err(failed(FailKind::Refused, MSG_ROOT_REFUSED));
    }
    let initialised = owned_entries_state(&root, io_message)?;
    Ok(Candidate { root, initialised })
}

/// Checks the entries that the kernel owns under a candidate root. None may be a
/// symbolic link or reparse point. Returns whether the metadata database exists.
fn owned_entries_state(root: &Path, io_message: &'static str) -> Result<bool, Reply> {
    let mut initialised = false;
    for relative in OWNED_ENTRIES {
        match fs::symlink_metadata(root.join(relative)) {
            Ok(meta) => {
                if is_link_or_reparse(&meta) {
                    return Err(invalid_path());
                }
                if relative == META_DB {
                    initialised = true;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(failed(FailKind::Kernel, io_message)),
        }
    }
    Ok(initialised)
}

/// Walks `components` below `root`. Each component must exist, must not be a link or
/// reparse point, and must be a directory.
fn walk(root: &Path, components: &[String], io_message: &'static str) -> Result<PathBuf, Reply> {
    let mut current = root.to_path_buf();
    for name in components {
        current.push(name);
        let meta = match fs::symlink_metadata(&current) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(failed(FailKind::NotFound, MSG_NOT_FOUND));
            }
            Err(_) => return Err(failed(FailKind::Kernel, io_message)),
        };
        if is_link_or_reparse(&meta) || !meta.is_dir() {
            return Err(invalid_path());
        }
    }
    Ok(current)
}

/// Classifies the entries of one directory without following links.
fn read_entries(dir: &Path) -> Option<Vec<model::RawEntry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir).ok()? {
        let item = item.ok()?;
        let name = item.file_name().into_string().ok()?;
        if name.eq_ignore_ascii_case(".ladon") || name.eq_ignore_ascii_case(".git") {
            continue;
        }
        let meta = fs::symlink_metadata(item.path()).ok()?;
        let (kind, size, modified_unix) = if is_link_or_reparse(&meta) {
            (model::EntryKind::Link, 0, None)
        } else if meta.is_dir() {
            (model::EntryKind::Directory, 0, None)
        } else if meta.is_file() {
            (model::EntryKind::File, meta.len(), unix_seconds(&meta))
        } else {
            (model::EntryKind::Other, 0, None)
        };
        entries.push(model::RawEntry { name, kind, size, modified_unix });
    }
    Some(entries)
}

async fn open_workspace(state: &mut State, path: &str) -> Reply {
    match root_candidate(path, MSG_OPEN_FAILED) {
        Ok(candidate) => open_candidate(state, candidate).await,
        Err(reply) => reply,
    }
}

/// Opens an accepted root, or reports that it needs initialisation.
async fn open_candidate(state: &mut State, candidate: Candidate) -> Reply {
    if !candidate.initialised {
        return Reply::NeedsInit { path: display_path(&candidate.root) };
    }
    let rule_mtime = rule_file_mtime(&candidate.root);
    match Engine::open(&candidate.root).await {
        Ok(engine) => {
            let root = display_path(&engine.root);
            state.engine = Some(engine);
            state.rule_mtime = rule_mtime;
            Reply::Opened { root }
        }
        Err(_) => failed(FailKind::Kernel, MSG_OPEN_FAILED),
    }
}

async fn init_workspace(state: &mut State, path: &str, confirmed: bool) -> Reply {
    if !confirmed {
        return failed(FailKind::NotConfirmed, MSG_NOT_CONFIRMED);
    }
    let candidate = match root_candidate(path, MSG_INIT_FAILED) {
        Ok(candidate) => candidate,
        Err(reply) => return reply,
    };
    if candidate.initialised {
        return open_candidate(state, candidate).await;
    }
    let rule_mtime = rule_file_mtime(&candidate.root);
    match Engine::init(&candidate.root).await {
        Ok(engine) => {
            let root = display_path(&engine.root);
            state.engine = Some(engine);
            state.rule_mtime = rule_mtime;
            Reply::Opened { root }
        }
        Err(_) => failed(FailKind::Kernel, MSG_INIT_FAILED),
    }
}

/// Lists the drive letters present on this machine, with whether each drive root holds
/// an initialised workspace. Writes nothing and never fails; a drive that is not
/// present is left out.
fn list_drives() -> Reply {
    let mut pairs = Vec::new();
    for letter in 'A'..='Z' {
        let root_text = format!("{letter}:\\");
        let root = Path::new(&root_text);
        if root.is_dir() {
            pairs.push((letter, drive_initialised(root)));
        }
    }
    Reply::Drives { drives: model::drive_rows(&pairs) }
}

/// True when `root` holds the metadata database as a regular file that is neither a
/// link nor a reparse point. Only that entry is checked.
fn drive_initialised(root: &Path) -> bool {
    fs::symlink_metadata(root.join(META_DB))
        .is_ok_and(|meta| meta.is_file() && !is_link_or_reparse(&meta))
}

async fn list_folder(state: &State, relative_dir: &str) -> Reply {
    let engine = match workspace(state) {
        Ok(engine) => engine,
        Err(reply) => return reply,
    };
    let relative = match checked_relative(relative_dir) {
        Ok(relative) => relative,
        Err(reply) => return reply,
    };
    let dir = match walk(&engine.root, &components_of(&relative), MSG_LIST_FAILED) {
        Ok(dir) => dir,
        Err(reply) => return reply,
    };
    let canonical = match fs::canonicalize(&dir) {
        Ok(canonical) if canonical.starts_with(&engine.root) => canonical,
        Ok(_) => return invalid_path(),
        Err(_) => return failed(FailKind::Kernel, MSG_LIST_FAILED),
    };
    let recorded = match relative_key(&engine.root, &canonical) {
        Some(recorded) => recorded,
        None => return invalid_path(),
    };
    let Some(entries) = read_entries(&canonical) else {
        return failed(FailKind::Kernel, MSG_LIST_FAILED);
    };
    let files = match engine.store.list_files().await {
        Ok(files) => files,
        Err(_) => return failed(FailKind::Kernel, MSG_LIST_FAILED),
    };
    let ledger: HashMap<String, model::LedgerRow> = files
        .into_iter()
        .map(|file| {
            let row = model::LedgerRow {
                file_id: file.id,
                version_no: file.version_no,
                deleted: file.deleted,
                sensitive: file.sensitive,
            };
            (file.relative_path, row)
        })
        .collect();
    let mut rows = model::browse_rows(&entries, &recorded, &ledger, local_offset());
    let truncated = rows.len() > LIST_LIMIT;
    rows.truncate(LIST_LIMIT);
    Reply::Folder { relative_dir: recorded, rows, truncated }
}

async fn capture_file(state: &mut State, relative_path: &str) -> Reply {
    let State { engine, rule_mtime } = state;
    let Some(engine) = engine.as_ref() else {
        return failed(FailKind::NoWorkspace, MSG_NO_WORKSPACE);
    };
    if relative_path.is_empty() {
        return invalid_path();
    }
    let relative = match checked_relative(relative_path) {
        Ok(relative) => relative,
        Err(reply) => return reply,
    };
    let (parent, name) = split_parent(&relative);
    if name.eq_ignore_ascii_case(RULE_FILE) {
        return failed(FailKind::Refused, MSG_RULE_FILE_REFUSED);
    }
    let directory = match walk(&engine.root, &components_of(parent), MSG_CAPTURE_FAILED) {
        Ok(directory) => directory,
        Err(reply) => return reply,
    };
    let file = directory.join(name);
    let meta = match fs::symlink_metadata(&file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return failed(FailKind::NotFound, MSG_NOT_FOUND);
        }
        Err(_) => return failed(FailKind::Kernel, MSG_CAPTURE_FAILED),
    };
    if is_link_or_reparse(&meta) || !meta.is_file() {
        return invalid_path();
    }
    let canonical = match fs::canonicalize(&file) {
        Ok(canonical) if canonical.starts_with(&engine.root) => canonical,
        Ok(_) => return invalid_path(),
        Err(_) => return failed(FailKind::Kernel, MSG_CAPTURE_FAILED),
    };
    let recorded = match relative_key(&engine.root, &canonical) {
        Some(recorded) => recorded,
        None => return invalid_path(),
    };
    let mtime = rule_file_mtime(&engine.root);
    if mtime != *rule_mtime {
        if engine.reload_policy().is_err() {
            return failed(FailKind::Refused, MSG_RULE_PARSE_FAILED);
        }
        *rule_mtime = mtime;
    }
    let outcome = match engine.capture(&canonical, "human", "gui-capture").await {
        Ok(outcome) => outcome,
        Err(_) => return failed(FailKind::Kernel, MSG_CAPTURE_FAILED),
    };
    let version_no = match engine.store.find_path(&recorded).await {
        Ok(Some(file)) => file.version_no,
        _ => return failed(FailKind::Kernel, MSG_CAPTURE_FAILED),
    };
    Reply::Captured {
        relative_path: recorded,
        version_no,
        changed: outcome.status != "unchanged",
    }
}

async fn load_history(state: &State, relative_path: &str) -> Reply {
    let engine = match workspace(state) {
        Ok(engine) => engine,
        Err(reply) => return reply,
    };
    let relative = match checked_relative(relative_path) {
        Ok(relative) => relative,
        Err(reply) => return reply,
    };
    let (_, name) = split_parent(&relative);
    if relative.is_empty() || name.eq_ignore_ascii_case(RULE_FILE) {
        return failed(FailKind::NotFound, MSG_NOT_FOUND);
    }
    let file = match engine.store.find_path(&relative).await {
        Ok(Some(file)) if !file.deleted => file,
        Ok(_) => return failed(FailKind::NotFound, MSG_NOT_FOUND),
        Err(_) => return failed(FailKind::Kernel, MSG_HISTORY_FAILED),
    };
    let versions = match engine.store.versions(&file.id).await {
        Ok(versions) => versions,
        Err(_) => return failed(FailKind::Kernel, MSG_HISTORY_FAILED),
    };
    let inputs: Vec<model::VersionInput> = versions
        .into_iter()
        .map(|version| {
            let local = engine.cas.has(&version.content_hash);
            model::VersionInput {
                version_id: version.id,
                number: version.number,
                content_hash: version.content_hash,
                byte_size: version.byte_size,
                sensitive: version.sensitive,
                changed_sections: version.changed_sections,
                actor: version.actor,
                reason: version.reason,
                created_at: version.created_at,
                local,
            }
        })
        .collect();
    let rows = model::version_rows(&inputs, local_offset());
    let chain_valid = match engine.store.verify_chain(&file.id).await {
        Ok(valid) => valid,
        Err(_) => return failed(FailKind::Kernel, MSG_HISTORY_FAILED),
    };
    let event_count = match engine.store.events(&file.id).await {
        Ok(events) => events.len(),
        Err(_) => return failed(FailKind::Kernel, MSG_HISTORY_FAILED),
    };
    Reply::History { relative_path: relative, versions: rows, chain_valid, event_count }
}

async fn load_diff(state: &State, older_id: &str, newer_id: &str) -> Reply {
    let engine = match workspace(state) {
        Ok(engine) => engine,
        Err(reply) => return reply,
    };
    let older = match engine.store.version(older_id).await {
        Ok(Some(version)) => version,
        Ok(None) => return failed(FailKind::NotFound, MSG_NOT_FOUND),
        Err(_) => return failed(FailKind::Kernel, MSG_DIFF_FAILED),
    };
    let newer = match engine.store.version(newer_id).await {
        Ok(Some(version)) => version,
        Ok(None) => return failed(FailKind::NotFound, MSG_NOT_FOUND),
        Err(_) => return failed(FailKind::Kernel, MSG_DIFF_FAILED),
    };
    if older.file_id != newer.file_id {
        return failed(FailKind::NotFound, MSG_NOT_FOUND);
    }
    let unavailable = |reason: DiffBlock| Reply::DiffUnavailable {
        older_version_id: older_id.to_string(),
        newer_version_id: newer_id.to_string(),
        reason,
    };
    // Both versions pass these checks before any content is read.
    for version in [&older, &newer] {
        if !engine.cas.has(&version.content_hash) {
            return unavailable(DiffBlock::Cold);
        }
        if version.byte_size > DIFF_LIMIT_BYTES as i64 {
            return unavailable(DiffBlock::TooLarge);
        }
    }
    let older_bytes = match engine.load_version(older_id, None).await {
        Ok(bytes) => bytes,
        Err(_) => return failed(FailKind::Kernel, MSG_DIFF_FAILED),
    };
    let newer_bytes = match engine.load_version(newer_id, None).await {
        Ok(bytes) => bytes,
        Err(_) => return failed(FailKind::Kernel, MSG_DIFF_FAILED),
    };
    if older_bytes.len() as u64 > DIFF_LIMIT_BYTES || newer_bytes.len() as u64 > DIFF_LIMIT_BYTES {
        return unavailable(DiffBlock::TooLarge);
    }
    if older_bytes.contains(&0) || newer_bytes.contains(&0) {
        return unavailable(DiffBlock::Binary);
    }
    let (Ok(old_text), Ok(new_text)) = (
        std::str::from_utf8(&older_bytes),
        std::str::from_utf8(&newer_bytes),
    ) else {
        return unavailable(DiffBlock::NotUtf8);
    };
    let diff = model::diff_lines(old_text, new_text, DIFF_MAX_LINES, DIFF_TIMEOUT);
    Reply::Diff {
        older_version_id: older_id.to_string(),
        newer_version_id: newer_id.to_string(),
        diff,
    }
}
