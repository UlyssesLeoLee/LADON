# Technical Design: `ui-prototype/src/main.rs`

Source: `ui-prototype/src/main.rs`

Source-SHA256: `242bdb00099f5829b033ca374c74deac98babd39b24f63e73c88c68c25d1db1c`

## Responsibility

`main.rs` is the entry point of the binary `ladon-gui`, the LADON Explorer desktop window. It creates the window from `ui/app.slint`, starts one `app::Worker`, connects each window callback to at most one `app::Command`, and applies each `app::Reply` on the user-interface thread. It performs no file-system, ledger, diff or rule work.

The window offers browsing, recording of a regular file (the first version of an untracked file, a further version of a tracked file), the version history of one tracked file, and a read-only text diff between two versions of one file. It offers no restore, whole-workspace scan, rule editing, tags, document tasks, archive or cloud function.

## Dependencies

- `ladon_ui_prototype::app`: `Worker` (`spawn`, `send`), `Command`, `Reply`, `diff_block_message`, and the constants `MSG_NO_WORKSPACE`, `MSG_DIFF_APPROXIMATE` and `MSG_DIFF_TRUNCATED`.
- `ladon_ui_prototype::model`: `BrowseRow`, `VersionRow`, `DiffLine` and `LineKind`, which are copied into Slint models.
- `slint` 1.19 (optional, enabled by feature `gui`): `slint::slint!`, `ComponentHandle`, `slint::Weak` with `upgrade_in_event_loop`, `ModelRc`, `VecModel` and `slint::PlatformError`.
- `std`: `cell::RefCell` for the thread-local state and `panic::set_hook`.
- The markup file `ui/app.slint`, resolved by the macro relative to the crate directory `ui-prototype/`. The markup imports `Button`, `LineEdit`, `ScrollView`, `ListView` and `AboutSlint` from `std-widgets.slint` and uses the built-in `MenuBar`, `Menu`, `MenuItem`, `FocusScope` and `TouchArea` elements.
- `main.rs` does not depend on the kernel crate directly. Every workspace operation goes through `app`.

## Build Configuration

- `ui-prototype/Cargo.toml` keeps its own `[workspace]` table, so the Slint dependency tree stays outside the feature resolution of the root workspace.
- The binary target is `[[bin]] name = "ladon-gui" path = "src/main.rs" required-features = ["kernel", "gui"]`.
- Features: `default = ["kernel", "gui"]`, `kernel = ["dep:ladon-explorer", "dep:tokio"]`, `gui = ["dep:slint"]`.
- Dependency declared for the window: `slint = { version = "1.19", optional = true }`.
- Verification lanes that involve this module:
  - model lane: `cargo test --lib --no-default-features` (compiles no window code);
  - kernel lane: `cargo test --no-default-features --features kernel` (compiles no window code);
  - window build: `cargo build --release --bin ladon-gui` (compiles `main.rs` and `ui/app.slint`).

## Public Contract

### Entry point and window

- `fn main() -> Result<(), slint::PlatformError>`:
  1. installs a silent panic hook, `std::panic::set_hook(Box::new(|_| {}))`;
  2. creates `AppWindow` and sets `status` to `app::MSG_NO_WORKSPACE`;
  3. wires the thirteen callbacks listed below;
  4. starts one `app::Worker` whose reply closure calls `slint::Weak::upgrade_in_event_loop` with a closure that applies the reply on the window, and stores the worker in `UiState`;
  5. runs the event loop until the window closes and returns the error from `run`.
- `slint::slint!` imports `ui/app.slint` and exports `AppWindow`, `FileItem`, `VersionItem` and `DiffItem`. Those Slint types are the only window types used in `main.rs`.
- The window title is `LADON Explorer`. The preferred size is 1180 by 760 logical pixels and the minimum size is 960 by 640. The window sets `default-font-family` to `Microsoft YaHei UI`, which text elements and standard widgets inherit.
- The window is light only. It uses no dark palette.

### User-interface state

- `thread_local! static UI: RefCell<UiState>` holds the state that exists only on the user-interface thread. It is accessed inside window callbacks and inside the closure given to `upgrade_in_event_loop`. The worker thread never touches it.
- `UiState` holds:
  - `worker: Option<app::Worker>`;
  - `rows: Vec<model::BrowseRow>`, the rows shown in `folder-items`, in the same order;
  - `versions: Vec<model::VersionRow>`, the versions shown in `versions`, in the same order;
  - `history_pending: bool`, set by `open-history` and cleared by the next reply.
- The folder being shown is the window property `current-dir`, a workspace-relative path with `/` separators. The workspace root shown by the kernel is `workspace-root`.
- The selected row is the window property `selected` (index into `folder-items`, -1 when none). The selected row's fields are copied into the `selected-*` properties.

### Command gating

- Each callback sends at most one command, and only when `busy` is false and `worker-stopped` is false.
- When `worker-stopped` is true, nothing is sent and `status` keeps its text. The banner shows the stopped state.
- When `busy` is true and `worker-stopped` is false, nothing is sent and `status` becomes `正在处理，请稍候。` with `status-error` false, so the status line uses the muted colour.
- Sending a command sets `busy` to true. Each reply clears `busy` before it is applied, so `busy` is false exactly when the reply to the last command has been applied.
- A `Worker::send` that returns false sets `worker-stopped` to true and clears `busy`. The banner is then shown.
- Buttons that start a command are disabled while `busy` or `worker-stopped` is true. The overlay buttons are also disabled while their overlay is hidden, so they are not focusable then.
- `select-row` sends `LoadHistory` only for a tracked regular file. `request-init` and `cancel-init` send nothing.

### Callbacks

| Window callback | Command or effect |
| --- | --- |
| `open-workspace(path)` | sends `Open { path }` |
| `request-init(path)` | sets `init-path` to `path` and sets `init-overlay`; sends nothing |
| `confirm-init(path)` | clears `init-overlay` and sends `Init { path, confirmed: true }` |
| `cancel-init()` | clears `init-overlay` |
| `enter-folder(relative-path)` | sends `ListFolder { relative_dir }` |
| `go-up()` | sends `ListFolder` for the parent of `current-dir`; sends nothing when `current-dir` is empty |
| `select-row(index)` | sets the selection to the row at `index`, or clears it when the index is out of range; sets the `selected-*` properties and empties `details-version`; for a tracked regular file sends `LoadHistory` |
| `open-history()` | for the selected tracked regular file sends `LoadHistory` and sets `history_pending`; sends nothing when no such row is selected |
| `capture-selected()` | for the selected row when it can be recorded (`selected-can-record`) sends `Capture { relative_path: selected-path }`; this covers untracked and tracked regular files |
| `compare-with-previous(index)` | sends `LoadDiff` with the version at `index + 1` as `older_version_id` and the version at `index` as `newer_version_id`; sends nothing when there is no version at `index + 1` |
| `back-to-browse()` | sets `page` to 0 |
| `open-about()` | sets `about-visible` to true |
| `close-about()` | sets `about-visible` to false |

### Reply effects

- `Opened { root }`: sets `workspace-open` to true, `workspace-root` to the display path, `current-dir` to the empty string and `truncated` to false; clears the history, the diff panel, the folder items, the selection and `details-version`; sets `page` to 0; sets `status` to `已打开工作区` with `status-error` false; then sends `ListFolder { relative_dir: "" }`.
- `NeedsInit { path }`: sets `status` to `该文件夹还不是工作区。可以点击“初始化”。` with `status-error` false. The open workspace, if any, is kept.
- `Folder { relative_dir, rows, truncated }`: sets `current-dir` and `truncated`; clears the selection and `details-version`; replaces `folder-items` and `UiState.rows`; sets `status` to `共 {n} 项` where `n` is the number of rows, with `status-error` false.
- `Captured { version_no, changed, .. }`: sets `status` to `已记录为 v{n}` when `changed` is true, otherwise to `内容与最新版本相同，未新增版本`, both with `status-error` false, so the status line uses the muted colour; then sends `ListFolder` for `current-dir`. The listing that follows sets `status` to `共 {n} 项`, so the capture text is replaced in the same round trip.
- `History { relative_path, versions, chain_valid, event_count }`:
  - when `history_pending` is set (the reply answers `open-history`):
    - sets `history-available` to true and `history-title` to `版本历史：{relative_path}`;
    - sets `chain-text` to `事件 {n} 条 · 事件链校验：通过`, or `事件 {n} 条 · 事件链校验：失败` when the chain is not valid;
    - sets `history-note` to `该文件还没有版本。` when `versions` is empty, otherwise to the empty string;
    - replaces `versions`; `can-compare` is true for every card except the last card in the list, which is the oldest version and has no previous version;
    - clears the diff panel;
    - sets `page` to 1;
  - otherwise (the reply answers a select-row request): sets `details-version` only when `selected-path` equals `relative_path`, to `当前版本：v{n} · 事件 {m} 条 · 事件链校验：通过` (or `失败`), with `n` from the first version and `m` the event count; when `versions` is empty it is set to the empty string. No other property changes.
- `Diff { older_version_id, newer_version_id, diff }`:
  - sets `diff-title` to `差异 v{old} → v{new}`;
  - sets `diff-totals` to `+{added} −{removed}`;
  - sets `diff-note` to `MSG_DIFF_APPROXIMATE` when `approximate` is true, else to `MSG_DIFF_TRUNCATED` when `truncated` is true, else to the empty string;
  - replaces `diff-lines`.
- `DiffUnavailable { older_version_id, newer_version_id, reason }`: sets `diff-title`, clears `diff-totals` and `diff-lines`, and sets `diff-note` to `diff_block_message(reason)`.
- `Failed { message, .. }`: sets `status` to `message` with `status-error` true. The `kind` and `detail` fields are not displayed.
- Version numbers in a diff title come from `VersionRow.number` in `UiState.versions`. An identifier that is not in `UiState.versions` gives `v?`.
- Every reply clears `busy` before it is applied.

### Recording rules

- A browse row can be recorded when it is not a folder, not a link, and either tracked or untracked with the status `未记录`. Other entries, whose status text is `其他`, cannot be recorded. This value is `FileItem.can-record` and `selected-can-record`.
- The record buttons follow the selected row:
  - `记录此文件` when the row can be recorded and is untracked;
  - `记录新版本` when the row can be recorded and is tracked.
  Both are enabled when `selected-can-record` is true and the window is neither `busy` nor `worker-stopped`.
- `查看历史` shows for a tracked row that can be recorded.
- Folder rows show `进入文件夹` and no record button. Link rows and other entries show neither `进入文件夹` nor a record button.

### Window properties

- Input and output: `workspace-path` (bound to the path field), `workspace-root`, `workspace-open`, `busy`, `worker-stopped`, `status`, `status-error`, `page` (0 browse, 1 history), `current-dir`, `folder-items: [FileItem]`, `truncated`, `selected` (-1 when none), `selected-name`, `selected-path`, `selected-status`, `selected-size`, `selected-modified`, `selected-is-folder`, `selected-tracked`, `selected-sensitive`, `selected-can-record`, `details-version`, `history-title`, `chain-text`, `history-note`, `versions: [VersionItem]`, `history-available`, `diff-title`, `diff-totals`, `diff-note`, `diff-lines: [DiffItem]`, `init-overlay`, `init-path`, `about-visible`.
- Every property listed above is an `in-out` property of `AppWindow`.
- `workspace-root`, `truncated`, `selected-sensitive` and the `sensitive` and `local` fields of `VersionItem` are set but have no visible text, because the string list defines none for them. Sensitivity is shown only inside the availability and status texts that the kernel model provides.

### Palette and structs (`ui/app.slint`)

- `export global Palette` (light only): background `#f4f5f0`, surface `#ffffff`, border `#dde2d6`, accent `#4f6b55`, text `#1f2a22`, muted `#58634f`, selected row `#e6eee2`, added background `#e3f1e5`, added text `#1d5a2c`, removed background `#f7e4e0`, removed text `#8a2b1f`, danger `#a33a2b`, overlay `#1f2a2280`.
- `export struct FileItem { name, relative-path, is-folder, is-link, tracked, can-record, size-text, modified-text, status-text }`.
- `export struct VersionItem { label, time-text, actor, reason, size-text, sections-text, hash-short, availability-text, sensitive, local, current, can-compare }`.
- `export struct DiffItem { kind: int, old-no: string, new-no: string, text: string }`, where `kind` is 0 for context, 1 for added, 2 for removed and 3 for a gap.

### Colour use

Every text colour pair used in the window reaches at least 4.5:1: muted text on background 5.79, on surface 6.34, on selected row 5.34, on added background 5.43, on removed background 5.17; accent on surface 5.89 and on removed background 4.80; danger on surface 6.57; added and removed text on their backgrounds at least 6.71.

The window places text in these colour pairs only:

- `Palette.text` on the surface, on the selected row, and in context lines;
- `Palette.muted` on the surface (secondary labels, the untracked status, link names, the gap line) and on the added and removed backgrounds (diff line numbers);
- `Palette.accent` on the surface (tracked status of unselected rows, `当前版本`, `处理中…`), and as the focus ring;
- `Palette.danger` on the surface (status errors, the stopped banner, the diff note);
- `Palette.added-text` on `Palette.added-bg` and `Palette.removed-text` on `Palette.removed-bg`.

Every `Text` element sets its `color` explicitly.

### Window layout (`ui/app.slint`)

- Menu bar: `帮助` with the item `关于 LADON Explorer`, which calls `open-about()`.
- Top bar on the surface colour: the label `工作区`, a path field bound to `workspace-path`, and the buttons `打开` (calls `open-workspace`) and `初始化` (calls `request-init`).
- Left navigation, 180 px wide on the surface colour: `浏览` (page 0) and `版本历史` (page 1, enabled when `history-available` is true).
- Browse page:
  - a breadcrumb card with the button `上一级` and the text `工作区 /` at the root, `工作区 / {current-dir}` below the root, or `未打开工作区` when no workspace is open;
  - a header row with the labels `名称`, `大小`, `修改时间` and `状态`;
  - the folder rows in a virtualised `ListView`, which instantiates only the visible rows. Each row shows the name, size, modified time and status. The selected row has the selected background. A tracked row shows its status in the accent colour when it is not selected;
  - the details card, 300 px wide: the name, or `未选择` when nothing is selected; then `状态：{status}`, `大小：{size}` and `修改时间：{modified}` for the selected row; then `details-version`; then the buttons `进入文件夹`, `记录此文件`, `记录新版本` and `查看历史` as described under Recording rules.
- History page:
  - a card, 380 px wide, with the button `返回浏览`, `history-title`, `chain-text`, `history-note` when it is not empty, and the version cards in a `ScrollView`, in the order of `versions`;
  - each version card shows the label, the marker `当前版本` on the current card, the time, `actor · reason`, `size · sections`, the short hash, the availability text, and the button `与上一版比较`, enabled when `can-compare` is true, `busy` is false and `worker-stopped` is false;
  - a diff panel with `diff-title` (or `选择“与上一版比较”查看差异。` when empty), `diff-totals`, `diff-note` in the danger colour, and the lines in a `ListView`. Added lines use the added colours with the prefix `+ `, removed lines use the removed colours with the prefix `− `, context lines have two spaces as a prefix, and a gap line shows `…`. The first column shows the line number in the first compared version and the second column the line number in the second compared version; the text is in the third column.
- Status bar on the surface colour: `status` in the danger colour when `status-error` is true and in the muted colour otherwise; the text `处理中…` in the accent colour while `busy` is true; the banner `后台服务已停止，请关闭并重新打开应用。` in the danger colour while `worker-stopped` is true.
- Init overlay, shown when `init-overlay` is true: a card with the text `将以下文件夹初始化为工作区：{init-path}。这会在其中创建 .ladon 目录和 .LADONignore 规则文件。之后可以记录该文件夹中文件的历史。不会上传任何文件。` and the buttons `确认初始化` (calls `confirm-init(init-path)`) and `取消` (calls `cancel-init()`).
- About overlay, shown when `about-visible` is true: a card with `LADON Explorer`, the `AboutSlint` widget and the button `关闭` (calls `close-about()`).
- Overlays cover the whole window with the background `#1f2a2280` and block pointer input to the content behind them. Showing the init overlay moves focus to `确认初始化`; showing the about overlay moves focus to `关闭`; hiding either overlay moves focus to the path field.
- Font sizes: the details card title (the name, or `未选择`) and `history-title` use 16 px, the about dialog title uses 18 px, and all other text uses the window default size.

### Keyboard and focus

- Initial focus is the path field, set with `forward-focus`.
- Tab order follows the tree order: the path field, `打开`, `初始化`; `浏览`, then `版本历史` when enabled; on the browse page `上一级` when enabled, the list, then the details buttons that are shown; on the history page `返回浏览`, then the `与上一版比较` buttons that are enabled. Disabled elements are skipped.
- The folder list is a zero-size `FocusScope` next to the `ListView`. It takes focus by Tab, when a row is clicked, and when the browse page is shown again from the history page; the initial focus stays on the path field. Its focus ring is drawn around the list.
  - Down selects the next row, or the first row when none is selected. The list scrolls the selected row into view with `ensure-row-visible`.
  - Up selects the previous row, or the first row when none is selected. The list scrolls the selected row into view.
  - Enter enters the selected folder, or opens the history of the selected tracked file.
  - Backspace goes up one folder when `current-dir` is not empty.
- Navigation entries are zero-size `FocusScope`s that take focus when enabled and activate with Enter or Space.
- Escape closes the init overlay when it is shown, otherwise the about overlay. The window-level `FocusScope` `keys` is enabled (`enabled: true`), because a disabled FocusScope cannot take focus and would never receive the Escape key that its focused descendants reject; `keys` receives the key events that its focused descendants reject and handles Escape for the overlays.

## Known Limitations

- Capture reads the file by path after its containment check; a junction placed over a directory between the check and the read is not detected. Owner: kernel. Planned change: read through a verified file handle.
- The kernel compares `.ladon` and `.git` case-sensitively in `resolve` and `scan`; the window refuses them case-insensitively. A folder named `.GIT` can still be recorded through the command line.
- Recording and initialising cannot be cancelled. Closing the window during them leaves at most an unreferenced content object or a partly created `.ladon` directory; no ledger entry is written without its version record.
- Only Simplified Chinese messages exist (NFR-015 is not met in this release).
- Files that carry the Windows reparse-point attribute, such as cloud placeholders, are listed as links and cannot be recorded.
- The window never reads an archive key or token and never archives, restores or scans.

## Change Notes

The window is a client of `app::Worker`. It never calls the kernel directly and keeps no workspace state apart from `UiState` and the window properties. Each reply produces a visible result: a status line, a folder listing, a history page, a details line, a diff panel or an error message. Restore, scan, rule editing and tags are not offered by the window.

The status line after a listing is always `共 {n} 项`, because the listing that follows `Opened` or `Captured` is a `Folder` reply. The texts `已打开工作区` and `已记录为 v{n}` are therefore visible only until that listing is applied.

The window derives `can-record` from the browse row fields that the model provides: `is_directory`, `is_link`, `tracked` and the status text `未记录`. The model has no field that marks other entries, so an entry with any other status is not recordable.
