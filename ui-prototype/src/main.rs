//! LADON Explorer window: the Slint markup in `ui/app.slint` wired to one `app::Worker`.
//!
//! The window sends at most one `app::Command` for each accepted user action and applies the
//! `app::Reply` that answers it on the user-interface thread. State that exists only on that
//! thread lives in the thread-local `UI`.

use std::cell::RefCell;

use ladon_ui_prototype::{app, model};
use slint::{ComponentHandle, ModelRc, VecModel};

slint::slint! {
    import { AppWindow, FileItem, VersionItem, DiffItem } from "ui/app.slint";
    export { AppWindow, FileItem, VersionItem, DiffItem }
}

const BUSY_STATUS: &str = "正在处理，请稍候。";
const NEEDS_INIT_STATUS: &str = "该文件夹还不是工作区。可以点击“初始化”。";
const OPENED_STATUS: &str = "已打开工作区";
const UNCHANGED_STATUS: &str = "内容与最新版本相同，未新增版本";
const NO_VERSIONS_NOTE: &str = "该文件还没有版本。";
/// Status text of an untracked file in `model::browse_rows`.
const UNTRACKED_STATUS: &str = "未记录";

/// State that exists only on the user-interface thread.
#[derive(Default)]
struct UiState {
    worker: Option<app::Worker>,
    /// Rows shown in `folder-items`, in the same order.
    rows: Vec<model::BrowseRow>,
    /// Versions shown in `versions`, in the same order.
    versions: Vec<model::VersionRow>,
    /// Set by `open-history` until its reply arrives. Only that reply switches to the history page.
    history_pending: bool,
    /// Set by `refresh-folder` until its reply arrives. A silent listing does not set `busy`.
    silent_refresh: bool,
}

thread_local! {
    static UI: RefCell<UiState> = RefCell::new(UiState::default());
}

/// Copy of the selected browse row. Holding a copy keeps `UI` unborrowed while the window is updated.
struct Selection {
    index: i32,
    name: String,
    path: String,
    status: String,
    size: String,
    modified: String,
    is_folder: bool,
    tracked: bool,
    can_record: bool,
    sensitive: bool,
}

impl Selection {
    fn of(index: i32, row: &model::BrowseRow) -> Self {
        Self {
            index,
            name: row.name.clone(),
            path: row.relative_path.clone(),
            status: row.status_text.clone(),
            size: row.size_text.clone(),
            modified: row.modified_text.clone(),
            is_folder: row.is_directory,
            tracked: row.tracked,
            can_record: can_record(row),
            sensitive: row.sensitive,
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    // Panic payloads never reach the console. The worker reports internal errors as replies.
    std::panic::set_hook(Box::new(|_| {}));

    let window = AppWindow::new()?;
    window.set_status(app::MSG_NO_WORKSPACE.into());
    wire_callbacks(&window);

    let weak = window.as_weak();
    let worker = app::Worker::spawn(move |reply| {
        // Runs on the worker thread and only forwards the reply. A reply that arrives
        // after the window has closed is dropped.
        let _ = weak.upgrade_in_event_loop(move |ui| apply_reply(&ui, reply));
    });
    UI.with(|state| state.borrow_mut().worker = Some(worker));

    window.run()
}

/// Runs `action` with the window when the window still exists.
fn with_window(weak: &slint::Weak<AppWindow>, action: impl FnOnce(&AppWindow)) {
    if let Some(ui) = weak.upgrade() {
        action(&ui);
    }
}

fn wire_callbacks(window: &AppWindow) {
    let weak = window.as_weak();

    window.on_open_workspace({
        let weak = weak.clone();
        move |path| {
            with_window(&weak, |ui| {
                send_command(ui, app::Command::Open { path: path.to_string() });
            });
        }
    });

    window.on_request_init({
        let weak = weak.clone();
        move |path| {
            with_window(&weak, |ui| {
                ui.set_init_path(path);
                ui.set_init_overlay(true);
            });
        }
    });

    window.on_confirm_init({
        let weak = weak.clone();
        move |path| {
            with_window(&weak, |ui| {
                ui.set_init_overlay(false);
                send_command(ui, app::Command::Init { path: path.to_string(), confirmed: true });
            });
        }
    });

    window.on_cancel_init({
        let weak = weak.clone();
        move || with_window(&weak, |ui| ui.set_init_overlay(false))
    });

    window.on_enter_folder({
        let weak = weak.clone();
        move |path| {
            with_window(&weak, |ui| {
                send_command(ui, app::Command::ListFolder { relative_dir: path.to_string() });
            });
        }
    });

    window.on_go_up({
        let weak = weak.clone();
        move || with_window(&weak, go_up)
    });

    window.on_select_row({
        let weak = weak.clone();
        move |index| with_window(&weak, |ui| select_row(ui, index))
    });

    window.on_open_history({
        let weak = weak.clone();
        move || with_window(&weak, open_history)
    });

    window.on_capture_selected({
        let weak = weak.clone();
        move || with_window(&weak, capture_selected)
    });

    window.on_compare_with_previous({
        let weak = weak.clone();
        move |index| with_window(&weak, |ui| compare_with_previous(ui, index))
    });

    window.on_back_to_browse({
        let weak = weak.clone();
        move || with_window(&weak, |ui| ui.set_page(0))
    });

    window.on_open_about({
        let weak = weak.clone();
        move || with_window(&weak, |ui| ui.set_about_visible(true))
    });

    window.on_refresh_folder({
        let weak = weak.clone();
        move || with_window(&weak, refresh_folder)
    });

    window.on_close_about({
        let weak = weak.clone();
        move || with_window(&weak, |ui| ui.set_about_visible(false))
    });
}

/// Sends one command when the window is idle and the worker runs. Returns whether it was sent.
///
/// When the worker has stopped, nothing changes because the banner already shows that state.
/// When `busy` is set, the status shows the busy text and nothing is sent.
fn send_command(ui: &AppWindow, command: app::Command) -> bool {
    if ui.get_worker_stopped() {
        return false;
    }
    if ui.get_busy() || silent_in_flight() {
        set_status(ui, BUSY_STATUS, false);
        return false;
    }
    let sent = UI.with(|state| {
        state.borrow().worker.as_ref().is_some_and(|worker| worker.send(command))
    });
    if sent {
        ui.set_busy(true);
    } else {
        ui.set_busy(false);
        ui.set_worker_stopped(true);
    }
    sent
}

fn set_status(ui: &AppWindow, text: &str, error: bool) {
    ui.set_status(text.into());
    ui.set_status_error(error);
}

/// Applies the reply to one command on the user-interface thread.
fn apply_reply(ui: &AppWindow, reply: app::Reply) {
    ui.set_busy(false);
    let history_pending = UI.with(|state| std::mem::take(&mut state.borrow_mut().history_pending));
    let silent = UI.with(|state| std::mem::take(&mut state.borrow_mut().silent_refresh));

    match reply {
        app::Reply::Opened { root } => {
            ui.set_workspace_root(root.into());
            ui.set_workspace_open(true);
            ui.set_page(0);
            clear_history(ui);
            clear_folder(ui);
            set_status(ui, OPENED_STATUS, false);
            send_command(ui, app::Command::ListFolder { relative_dir: String::new() });
        }
        app::Reply::NeedsInit { .. } => set_status(ui, NEEDS_INIT_STATUS, false),
        app::Reply::Folder { relative_dir, rows, truncated } => {
            if silent && ui.get_current_dir().as_str() == relative_dir.as_str() {
                apply_silent_listing(ui, rows, truncated);
            } else {
                ui.set_current_dir(relative_dir.into());
                ui.set_truncated(truncated);
                clear_selection(ui);
                set_status(ui, &format!("共 {} 项", rows.len()), false);
                let items: Vec<FileItem> = rows.iter().map(file_item).collect();
                ui.set_folder_items(ModelRc::new(VecModel::from(items)));
                UI.with(|state| state.borrow_mut().rows = rows);
            }
        }
        app::Reply::Captured { version_no, changed, .. } => {
            if changed {
                set_status(ui, &format!("已记录为 v{version_no}"), false);
            } else {
                set_status(ui, UNCHANGED_STATUS, false);
            }
            let folder = ui.get_current_dir().to_string();
            send_command(ui, app::Command::ListFolder { relative_dir: folder });
        }
        app::Reply::History { relative_path, versions, chain_valid, event_count } => {
            if history_pending {
                show_history_page(ui, relative_path, versions, chain_valid, event_count);
            } else {
                show_current_version(ui, &relative_path, &versions, chain_valid, event_count);
            }
        }
        app::Reply::Diff { older_version_id, newer_version_id, diff } => {
            ui.set_diff_title(pair_title(&older_version_id, &newer_version_id).into());
            ui.set_diff_totals(format!("+{} −{}", diff.added, diff.removed).into());
            let note = if diff.approximate {
                app::MSG_DIFF_APPROXIMATE
            } else if diff.truncated {
                app::MSG_DIFF_TRUNCATED
            } else {
                ""
            };
            ui.set_diff_note(note.into());
            let lines: Vec<DiffItem> = diff.lines.iter().map(diff_item).collect();
            ui.set_diff_lines(ModelRc::new(VecModel::from(lines)));
        }
        app::Reply::DiffUnavailable { older_version_id, newer_version_id, reason } => {
            ui.set_diff_title(pair_title(&older_version_id, &newer_version_id).into());
            ui.set_diff_totals("".into());
            ui.set_diff_note(app::diff_block_message(reason).into());
            ui.set_diff_lines(ModelRc::new(VecModel::from(Vec::<DiffItem>::new())));
        }
        app::Reply::Failed { message, .. } => set_status(ui, &message, true),
    }
}

/// Lists the open folder again without a message, when no command is in flight.
fn refresh_folder(ui: &AppWindow) {
    let busy = ui.get_busy() || silent_in_flight();
    if !model::should_refresh(busy, ui.get_worker_stopped(), ui.get_workspace_open()) {
        return;
    }
    let command = app::Command::ListFolder { relative_dir: ui.get_current_dir().to_string() };
    UI.with(|state| state.borrow_mut().silent_refresh = true);
    let sent = UI.with(|state| state.borrow().worker.as_ref().is_some_and(|worker| worker.send(command)));
    if !sent {
        UI.with(|state| state.borrow_mut().silent_refresh = false);
        ui.set_worker_stopped(true);
    }
}

/// Whether a silent refresh is in flight. Commands sent meanwhile are refused.
fn silent_in_flight() -> bool {
    UI.with(|state| state.borrow().silent_refresh)
}

/// Applies the listing of a silent refresh. A listing equal to the one shown changes nothing.
/// Otherwise the rows are replaced, the status keeps any text other than a listing count, and the
/// selection stays on the same path when that row still exists.
fn apply_silent_listing(ui: &AppWindow, rows: Vec<model::BrowseRow>, truncated: bool) {
    let changed = UI.with(|state| model::listing_changed(&state.borrow().rows, &rows));
    if !changed {
        return;
    }
    let details = ui.get_details_version();
    let selection = model::selection_after_refresh(&rows, ui.get_selected_path().as_str())
        .map(|position| Selection::of(i32::try_from(position).unwrap_or(-1), &rows[position]));
    ui.set_truncated(truncated);
    let items: Vec<FileItem> = rows.iter().map(file_item).collect();
    ui.set_folder_items(ModelRc::new(VecModel::from(items)));
    let status = ui.get_status().to_string();
    if status.starts_with("共 ") || status == app::MSG_LIST_FAILED {
        set_status(ui, &format!("共 {} 项", rows.len()), false);
    }
    match selection {
        Some(selection) => {
            apply_selection(ui, &selection);
            ui.set_details_version(details);
        }
        None => clear_selection(ui),
    }
    UI.with(|state| state.borrow_mut().rows = rows);
}

/// Fills the history page for a pending `open-history` request and switches to the page.
fn show_history_page(
    ui: &AppWindow,
    relative_path: String,
    versions: Vec<model::VersionRow>,
    chain_valid: bool,
    event_count: usize,
) {
    let count = versions.len();
    let items: Vec<VersionItem> = versions
        .iter()
        .enumerate()
        .map(|(position, row)| version_item(row, position + 1 < count))
        .collect();
    let note = if versions.is_empty() { NO_VERSIONS_NOTE } else { "" };

    ui.set_history_available(true);
    ui.set_history_title(format!("版本历史：{relative_path}").into());
    ui.set_chain_text(format!("事件 {event_count} 条 · {}", chain_check(chain_valid)).into());
    ui.set_history_note(note.into());
    ui.set_versions(ModelRc::new(VecModel::from(items)));
    clear_diff(ui);
    UI.with(|state| state.borrow_mut().versions = versions);
    ui.set_page(1);
}

/// Fills the details line of the selected row from a history reply to a select-row request.
/// The line is set only when the reply path equals the selected path.
fn show_current_version(
    ui: &AppWindow,
    relative_path: &str,
    versions: &[model::VersionRow],
    chain_valid: bool,
    event_count: usize,
) {
    let selected_here = ui.get_selected() >= 0 && ui.get_selected_path().as_str() == relative_path;
    if !selected_here {
        return;
    }
    let details = match versions.first() {
        Some(current) => format!(
            "当前版本：v{} · 事件 {event_count} 条 · {}",
            current.number,
            chain_check(chain_valid)
        ),
        None => String::new(),
    };
    ui.set_details_version(details.into());
}

fn chain_check(chain_valid: bool) -> &'static str {
    if chain_valid {
        "事件链校验：通过"
    } else {
        "事件链校验：失败"
    }
}

/// Clears the folder listing, the selection and the breadcrumb until the next listing arrives.
fn clear_folder(ui: &AppWindow) {
    clear_selection(ui);
    ui.set_current_dir("".into());
    ui.set_truncated(false);
    ui.set_folder_items(ModelRc::new(VecModel::from(Vec::<FileItem>::new())));
    UI.with(|state| state.borrow_mut().rows.clear());
}

fn clear_selection(ui: &AppWindow) {
    ui.set_selected(-1);
    ui.set_selected_name("".into());
    ui.set_selected_path("".into());
    ui.set_selected_status("".into());
    ui.set_selected_size("".into());
    ui.set_selected_modified("".into());
    ui.set_selected_is_folder(false);
    ui.set_selected_tracked(false);
    ui.set_selected_sensitive(false);
    ui.set_selected_can_record(false);
    ui.set_details_version("".into());
}

fn clear_history(ui: &AppWindow) {
    ui.set_history_available(false);
    ui.set_history_title("".into());
    ui.set_chain_text("".into());
    ui.set_history_note("".into());
    ui.set_versions(ModelRc::new(VecModel::from(Vec::<VersionItem>::new())));
    UI.with(|state| state.borrow_mut().versions.clear());
    clear_diff(ui);
}

fn clear_diff(ui: &AppWindow) {
    ui.set_diff_title("".into());
    ui.set_diff_totals("".into());
    ui.set_diff_note("".into());
    ui.set_diff_lines(ModelRc::new(VecModel::from(Vec::<DiffItem>::new())));
}

/// Selects one browse row. Only a tracked file asks the worker for its history.
fn select_row(ui: &AppWindow, index: i32) {
    let selection = UI.with(|state| {
        let state = state.borrow();
        let position = usize::try_from(index).ok().filter(|position| *position < state.rows.len())?;
        Some(Selection::of(index, &state.rows[position]))
    });
    match selection {
        Some(selection) => show_selection(ui, selection),
        None => clear_selection(ui),
    }
}

fn show_selection(ui: &AppWindow, selection: Selection) {
    apply_selection(ui, &selection);
    ui.set_details_version("".into());
    if selection.tracked && !selection.is_folder {
        send_command(ui, app::Command::LoadHistory { relative_path: selection.path });
    }
}

/// Sets the `selected-*` properties from a selection. It sends no command and keeps the details card.
fn apply_selection(ui: &AppWindow, selection: &Selection) {
    ui.set_selected(selection.index);
    ui.set_selected_name(selection.name.as_str().into());
    ui.set_selected_path(selection.path.as_str().into());
    ui.set_selected_status(selection.status.as_str().into());
    ui.set_selected_size(selection.size.as_str().into());
    ui.set_selected_modified(selection.modified.as_str().into());
    ui.set_selected_is_folder(selection.is_folder);
    ui.set_selected_tracked(selection.tracked);
    ui.set_selected_sensitive(selection.sensitive);
    ui.set_selected_can_record(selection.can_record);
}

fn go_up(ui: &AppWindow) {
    let current = ui.get_current_dir().to_string();
    if current.is_empty() {
        return;
    }
    let parent = current
        .rsplit_once('/')
        .map_or_else(String::new, |(parent, _)| parent.to_string());
    send_command(ui, app::Command::ListFolder { relative_dir: parent });
}

fn open_history(ui: &AppWindow) {
    if ui.get_selected() < 0 || ui.get_selected_is_folder() || !ui.get_selected_tracked() {
        return;
    }
    UI.with(|state| state.borrow_mut().history_pending = true);
    let command = app::Command::LoadHistory { relative_path: ui.get_selected_path().to_string() };
    if !send_command(ui, command) {
        UI.with(|state| state.borrow_mut().history_pending = false);
    }
}

/// Records the selected regular file: the first version when it is untracked, a further version
/// when it is tracked. The kernel records a version only when the content changed.
fn capture_selected(ui: &AppWindow) {
    if ui.get_selected() < 0 || !ui.get_selected_can_record() {
        return;
    }
    send_command(ui, app::Command::Capture { relative_path: ui.get_selected_path().to_string() });
}

/// Compares the version at `index` with the version listed after it, which is the one before it in time.
fn compare_with_previous(ui: &AppWindow, index: i32) {
    let pair = UI.with(|state| {
        let state = state.borrow();
        let position = usize::try_from(index).ok()?;
        let newer = state.versions.get(position)?;
        let older = state.versions.get(position + 1)?;
        Some((older.version_id.clone(), newer.version_id.clone()))
    });
    if let Some((older_version_id, newer_version_id)) = pair {
        send_command(ui, app::Command::LoadDiff { older_version_id, newer_version_id });
    }
}

/// Title of a diff, such as `差异 v1 → v2`, looked up from the versions on the history page.
fn pair_title(older_version_id: &str, newer_version_id: &str) -> String {
    UI.with(|state| {
        let state = state.borrow();
        let number = |version_id: &str| {
            state
                .versions
                .iter()
                .find(|row| row.version_id == version_id)
                .map_or_else(|| String::from("v?"), |row| format!("v{}", row.number))
        };
        format!("差异 {} → {}", number(older_version_id), number(newer_version_id))
    })
}

/// A browse row is recordable when it is a regular file: not a folder, not a link, and either tracked
/// or untracked with the status `未记录`. Other entries have another status and are not recordable.
fn can_record(row: &model::BrowseRow) -> bool {
    !row.is_directory && !row.is_link && (row.tracked || row.status_text == UNTRACKED_STATUS)
}

fn file_item(row: &model::BrowseRow) -> FileItem {
    FileItem {
        name: row.name.as_str().into(),
        relative_path: row.relative_path.as_str().into(),
        is_folder: row.is_directory,
        is_link: row.is_link,
        tracked: row.tracked,
        can_record: can_record(row),
        size_text: row.size_text.as_str().into(),
        modified_text: row.modified_text.as_str().into(),
        status_text: row.status_text.as_str().into(),
    }
}

fn version_item(row: &model::VersionRow, can_compare: bool) -> VersionItem {
    VersionItem {
        label: row.label.as_str().into(),
        time_text: row.time_text.as_str().into(),
        actor: row.actor.as_str().into(),
        reason: row.reason.as_str().into(),
        size_text: row.size_text.as_str().into(),
        sections_text: row.sections_text.as_str().into(),
        hash_short: row.hash_short.as_str().into(),
        availability_text: row.availability_text.as_str().into(),
        sensitive: row.sensitive,
        local: row.local,
        current: row.current,
        can_compare,
    }
}

fn diff_item(line: &model::DiffLine) -> DiffItem {
    let kind = match line.kind {
        model::LineKind::Context => 0,
        model::LineKind::Added => 1,
        model::LineKind::Removed => 2,
        model::LineKind::Gap => 3,
    };
    DiffItem {
        kind,
        old_no: line.old_no.map_or_else(String::new, |number| number.to_string()).into(),
        new_no: line.new_no.map_or_else(String::new, |number| number.to_string()).into(),
        text: line.text.as_str().into(),
    }
}
