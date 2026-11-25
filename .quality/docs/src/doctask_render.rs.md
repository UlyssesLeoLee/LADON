# Technical Design: `src/doctask_render.rs`

Source: `src/doctask_render.rs`

Source-SHA256: `cd2be14308cb79ee023eec64a0c83de6cf368a9262e07c518d1608ce636c54f5`

## Responsibility

`doctask_render` is the pure text layer of the document-task module. It owns the Chinese and English message catalogues, the display width of text containing CJK characters, the table layout, the three console screens (outline, trace and approval), and the rendering of `DocError`. It takes plain data as input and returns plain text. It performs no I/O, reads no environment variable, holds no database connection and does not decide whether colour is used: the caller passes the `Mode`, which `detect_mode` derives from the console state and `NO_COLOR`. Persistence, the service layer and the command or console layers belong to later modules.

## Dependencies

- `crate::doctask_model` for `DocError`, `ErrorCode`, the enumerations used as screen fields (`TaskState`, `PlanStatus`, `AnchorState`, `Visibility`, `LinkKind`, `EvidenceKind`, `EvidenceResult`, `EvidenceSource`, `Risk`, `ActorKind`) and `sanitize_display`. Every user-controlled or agent-controlled string passes through `sanitize_display` before it reaches the output.
- The standard library only. The module adds no crate and calls no external function.
- The module does not import `crate::store`, `crate::engine` or any module that does.
- Callers: `doctask_cli` (errors and outline screen), `doctask_mcp` (errors), and `console` (all screens and the error rendering).

## Public Contract

### Languages and modes

`Lang` has the variants `Zh` and `En`. `Lang::ALL` lists both in that order, `as_str()` returns `zh` or `en`, and `parse(&str) -> Option<Lang>` accepts exactly those two strings. Any other input, including upper-case forms, returns `None`.

`Mode` has the variants `Plain` and `Color`. `Plain` output contains no escape character. `Color` output contains only SGR sequences of the form `ESC [ digits ; digits m` emitted by the module's own painter; every escape character produced by user data is replaced earlier by `sanitize_display`.

`detect_mode(stdout_is_console: bool, no_color: bool) -> Mode` returns `Color` when the standard output is a console and `no_color` is false, and `Plain` otherwise.

### Catalogue

`catalog(lang) -> &'static [(&'static str, &'static str)]` returns the key and text pairs of one language. The two catalogues have identical key sets, no key occurs twice in one catalogue, every text is non-empty, and the set of `{name}` named slots of a key is the same in both languages. The key families are:

| Family | Keys | Use |
|---|---|---|
| `common.*` | none, error label, hint label, detail label, next-step line, list separator | shared labels |
| `task.<TaskState>` | the seven task states | status in the trace and approval screens |
| `plan.<PlanStatus>` | the seven plan statuses | downstream task lines |
| `anchor.<AnchorState>` | the eight anchor states | anchor lines |
| `visibility.<Visibility>` | `shared`, `local_only` | annotation lines |
| `link.<LinkKind>` | the nine link kinds | link lines |
| `evidence.kind.<EvidenceKind>`, `evidence.result.<EvidenceResult>`, `evidence.source.<EvidenceSource>` | seven, three and two values | evidence lines |
| `risk.<Risk>` | `low`, `medium`, `high` | approval screen |
| `actor.<ActorKind>` | `human`, `agent`, `system`, `unknown` | submitter line |
| `err.<ErrorCode>.message`, `err.<ErrorCode>.hint` | the 27 codes of `ErrorCode::ALL` | English error text and Chinese fallback |
| `outline.*`, `trace.*`, `approval.*` | labels and line templates of the three screens | screen text |

Each enumeration key uses the stored string of the variant, so the catalogue is checked against the model by listing each `ALL` array. `text(lang, key) -> &str` returns the catalogue text, or the key itself when it is absent (the callers use only keys that the tests prove are present). `fill(template, values) -> String` replaces each `{name}` that has a value in `values`, in a single left-to-right pass; substituted text is not scanned again, and a named slot without a value stays literal.

### Width and cells

`char_width(c) -> usize` returns 2 for the East Asian wide ranges `U+1100` to `U+115F`, `U+2E80` to `U+A4CF`, `U+AC00` to `U+D7A3`, `U+F900` to `U+FAFF`, `U+FE30` to `U+FE6F`, `U+FF00` to `U+FF60` and `U+FFE0` to `U+FFE6`, and 1 otherwise. `display_width(text)` is the sum of `char_width` over the characters. `pad_to(text, width)` appends spaces until the display width reaches `width`; a text already at or beyond `width` is returned unchanged.

`cell(text) -> String` applies `sanitize_display` and then replaces each newline and tab with a space, so that the result occupies one line of a table.

`short_hash(hash) -> String` returns the first twelve characters of the sanitised hash, or the whole sanitised value when it is shorter.

### Painter

The painter is private to the module. `Style` has `Heading` (bold), `Error` (bold red) and `Dim`. `paint(mode, style, text)` returns the text unchanged in `Plain` mode and wraps it in the matching SGR sequence and a reset in `Color` mode.

### Table

`Table { headers: Vec<String>, rows: Vec<Vec<String>> }` renders with `render(mode) -> String`:
1. Every header and cell passes through `cell`. A row shorter than the header list is padded with empty cells; a longer row is truncated.
2. The width of a column is the largest display width among its header and cells.
3. Each line joins its cells with two spaces. Every column except the last is padded to its width; trailing spaces of a line are removed, so no line ends with a space.
4. The second line is a rule of hyphens, one run per column, with the same gutter.
5. In `Color` mode the header line is painted `Heading` and the rule is painted `Dim`. The padding is computed on the unpainted text.
6. Lines are joined with `\n`; the output has no trailing newline. A table without headers renders as the empty string.

### Errors

`render_error(error, lang, mode) -> String` writes up to three lines:
1. `{Error} {CODE}: {message}`, where the label is `common.error_label` in the language and painted `Error`, and the code is the stable code and is not translated.
2. `{Hint}: {hint}`.
3. `{Detail}: {detail}`, only when the detail is non-empty.

In `Zh` the message and hint are `message_zh` and `hint_zh` of the error, and the catalogue Chinese text is used when a field is empty. In `En` the message and hint are the catalogue texts of `err.<CODE>.message` and `err.<CODE>.hint`. Every text is sanitised; the detail is sanitised as well, because it may name a path or a value.

### Screens

Each screen returns lines joined by `\n` with no trailing newline. Titles and section headings are painted `Heading` in `Color` mode; the next-step and prompt lines are painted `Dim`. Every string taken from the input passes through `cell` or `sanitize_display`.

**Outline** `OutlineScreen { file, baseline: Option<BaselineView>, current_version, baseline_matches, rows: Vec<OutlineRow>, next_step }`. `BaselineView { version, approved_at, approved_by }`; `OutlineRow { ordinal, clause, title, first_line, last_line, notes, tasks }`.
- Header line with a baseline: `{file}   {approved} v{version}{detail}  {current} v{current_version}  {matches|differs}`; `detail` is the `outline.baseline_detail` template filled with the approval time and the approver.
- Header line without a baseline: `{file}   {current} v{current_version}  {no_baseline}`.
- A `Table` with the headers `#`, `条款` or `Clause`, `标题` or `Title`, `行范围` or `Lines`, `批注` or `Notes`, `任务` or `Tasks`. Lines read `first-last`.
- An empty row list renders the `outline.empty` text in place of the table.
- The last line is `common.next` filled with `next_step`.

**Trace** `TraceScreen { file, clause, clause_title, anchor, baseline_bound, annotations, tasks, links, changes, evidence, rollback, events, next_step }`.
- Title: `trace.title` filled with file, clause and clause title. Next line: `trace.anchor` filled with the anchor label and the baseline text (`trace.baseline_bound` or `trace.baseline_unbound`).
- Sections in this order, each with a heading and a blank line before the next section: annotations (`AnnotationLine { handle, text, visibility }`, line `{handle} [{visibility}] {text}`); downstream tasks (`TaskLine { handle, title, status, plan, depends_on }`, line `{handle} [{status}] {title}` followed by the plan and the dependencies when present); links (`LinkLine { kind, label, target }`, line `[{kind}] {label} -> {target}`); Agent changes and commits (`ChangeLine { plan, version_id, event_id, approver }`, one line with the plan, version, ledger event and approver); test evidence (`EvidenceLine { kind, result, source, summary }`, line `[{kind}] {result} {source}: {summary}`); rollback chain (`RollbackLine { versions, restore_version }`: the version sequence joined with ` -> `, then the restore command `ladon restore {version} --approve` for the version before the applied change); recent events (`EventLine { seq, kind, actor, at }`, line `#{seq} {kind} {actor} {at}`).
- Each section item is indented by two spaces. An empty section shows the `common.none` text as its only item.
- The last line is the next-step line.

**Approval** `ApprovalScreen { task, task_title, clause, target_file, target_section, submitted_by, submitted_kind, session, summary, risk, impact, options, diff, expected_hash, result_hash, baseline, other_plans, dependencies, anchor }`, with `BaselineCheck` of `Matches`, `Differs` and `NoBaseline`.
- Title `approval.title`; clause; target file and section; submitter with its kind label; session; summary; risk label; impact.
- Options, when present: a heading and `  n. option` items; at most `MAX_PLAN_OPTIONS` (five) items are shown.
- The unified diff: a heading, then each diff line sanitised and kept without indentation so that the `+` and `-` markers are at column one. An empty diff shows the none text.
- `expected_hash` and `result_hash`, each on its own line with `short_hash` of the value.
- The baseline check on its own line: `approval.baseline_matches`, `approval.baseline_differs` or `approval.baseline_none`.
- Other approved plans on the file, dependencies and anchor state, each on its own line.
- The prompt line `approval.prompt` filled with the task handle, painted `Dim`.

Chinese and English differ only in catalogue text. The layout, the order of lines and the ASCII tokens (`expected_hash`, `result_hash`, `ladon restore`, handles, codes) are identical.

## Change Notes

The module is new. It is the only module that produces screen text; the command and console layers call it and do not format screens themselves. The width ranges are those of the detailed design, section 11.3. The English catalogue carries the error text for every code, so an English session never shows a Chinese message that the model supplied.

`lib.rs` gains one `pub mod doctask_render;` line. No other module changes. The crate gains no dependency.
