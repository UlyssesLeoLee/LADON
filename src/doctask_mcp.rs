//! MCP tool host of `ladon mcp-serve`: the thirteen document-task tools of design 57 section 10.2
//! that WP13 delivers, their argument rules, the result envelopes, the disclosure record written
//! before content is returned, the panic guard and the stdio serve loop. Every call runs as the
//! agent principal through the document-task services, which apply the transfer gate.

#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use crate::doctask_flow::DocFlow;
use crate::doctask_model::{
    parse_section_selector, sha256_hex, DocError, ErrorCode, ProposalState,
};
use crate::doctask_outline::{self, Outline};
use crate::doctask_service::{
    Caller, DocTaskService, EngineGate, TaskSpec, TaskView, TransferGate,
};
use crate::doctask_store::DocStore;
use crate::engine::Engine;
use crate::mcp::{Server, ServerInfo, ToolCall, ToolHost};
use crate::model::VersionRecord;
use crate::store::StoreBusy;
use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use turso::Value as SqlValue;

/// Number of tools this module delivers; `doc_plan_submit` is added by WP14.
pub const TOOL_COUNT: usize = 13;

/// Names of the delivered tools, in the order of `tools/list`.
pub const TOOL_NAMES: [&str; TOOL_COUNT] = [
    "doc_file_find",
    "doc_clause_list",
    "doc_clause_read",
    "doc_mentions_find",
    "doc_task_list",
    "doc_task_get",
    "doc_trace_get",
    "doc_schedule_get",
    "doc_task_create",
    "doc_annotation_add",
    "doc_task_transition",
    "doc_evidence_add",
    "doc_link_add",
];

/// Tools that carry `annotations.readOnlyHint = true`.
pub const READ_ONLY_TOOLS: [&str; 8] = [
    "doc_file_find",
    "doc_clause_list",
    "doc_clause_read",
    "doc_mentions_find",
    "doc_task_list",
    "doc_task_get",
    "doc_trace_get",
    "doc_schedule_get",
];

/// Largest serialized result text in UTF-8 bytes.
pub const RESULT_LIMIT_BYTES: usize = 32 * 1024;

/// Bytes left for list items once the envelope overhead is reserved.
const LIST_BUDGET: usize = RESULT_LIMIT_BYTES - 1024;

/// Longest accepted launch id in bytes.
pub const LAUNCH_ID_MAX: usize = 64;

/// Candidate files scanned by `doc_mentions_find`.
pub const MENTION_FILE_LIMIT: usize = 2000;

/// Largest file in bytes scanned by `doc_mentions_find`.
pub const MENTION_SIZE_LIMIT: i64 = 1_048_576;

/// Characters of clause text returned by `doc_task_get`.
pub const CLAUSE_TEXT_LIMIT: usize = 3000;

/// Largest evidence summary with its reference, in characters.
const EVIDENCE_TEXT_LIMIT: usize = 1000;

/// Longest transition reason other than a work summary, in characters.
const REASON_LIMIT: usize = 500;

/// Longest work summary, in characters.
const SUMMARY_LIMIT: usize = 4000;

const STATUS_VALUES: [&str; 7] = [
    "open",
    "analysis",
    "approval",
    "working",
    "verification",
    "done",
    "cancelled",
];

const TRANSITION_TARGETS: [&str; 3] = ["analysis", "verification", "working"];

const EVIDENCE_KINDS: [&str; 4] = ["test_run", "report", "note", "version"];

const STORE_BUSY_MESSAGE: &str = "数据库正被占用";
const STORE_BUSY_HINT: &str = "约 2 秒后重试";
const INTERNAL_MESSAGE: &str = "内部错误";
const INTERNAL_HINT: &str = "保留现场并报告问题";

/// Kind of one argument, with the bounds of the field.
#[derive(Clone, Copy)]
enum Kind {
    /// A string whose length in characters lies in the closed range.
    Text { min: usize, max: usize },
    /// An integer in the closed range.
    Int { min: i64, max: i64 },
    /// A boolean.
    Bool,
    /// One of the listed strings.
    Choice(&'static [&'static str]),
    /// An array of listed strings, at most `max_items` long.
    ChoiceList {
        values: &'static [&'static str],
        max_items: usize,
    },
    /// An array of text items, at most `max_items` long, each at most `item_max` characters.
    TextList { max_items: usize, item_max: usize },
}

/// One argument of a tool.
#[derive(Clone)]
struct Field {
    name: &'static str,
    kind: Kind,
    required: bool,
    description: &'static str,
}

fn field(name: &'static str, kind: Kind, required: bool, description: &'static str) -> Field {
    Field {
        name,
        kind,
        required,
        description,
    }
}

fn text(
    name: &'static str,
    min: usize,
    max: usize,
    required: bool,
    description: &'static str,
) -> Field {
    field(name, Kind::Text { min, max }, required, description)
}

fn int(name: &'static str, min: i64, max: i64, required: bool, description: &'static str) -> Field {
    field(name, Kind::Int { min, max }, required, description)
}

fn boolean(name: &'static str, description: &'static str) -> Field {
    field(name, Kind::Bool, false, description)
}

/// The arguments of a tool, in schema order. Empty for an unknown name.
fn fields_of(tool: &str) -> Vec<Field> {
    let version = "版本号，缺省为最新版本";
    match tool {
        "doc_file_find" => vec![
            text("query", 1, 100, false, "路径中的不区分大小写子串。"),
            boolean(
                "has_tasks",
                "true 只列有未完成任务的文件，false 只列没有的文件。",
            ),
            int("limit", 1, 50, false, "返回的最多文件数，默认 20。"),
        ],
        "doc_clause_list" => vec![
            text("file_ref", 1, 200, true, "文件编号或路径。"),
            int("version", 1, i64::MAX, false, version),
        ],
        "doc_clause_read" => vec![
            text("file_ref", 1, 200, true, "文件编号或路径。"),
            text("section", 1, 200, true, "条款键、条款编号或 ROOT。"),
            int("version", 1, i64::MAX, false, version),
            int("offset", 0, i64::MAX, false, "以字符计的起始偏移，默认 0。"),
            int(
                "max_chars",
                200,
                6000,
                false,
                "本次返回的最多字符数，默认 3000。",
            ),
        ],
        "doc_mentions_find" => vec![
            text("token", 2, 80, true, "要检索的字面子串，区分大小写。"),
            int("limit", 1, 30, false, "返回的最多命中数，默认 10。"),
        ],
        "doc_task_list" => vec![
            field(
                "status",
                Kind::ChoiceList {
                    values: &STATUS_VALUES,
                    max_items: 7,
                },
                false,
                "任务状态的筛选，多个取并集。",
            ),
            text("file_ref", 1, 200, false, "文件编号或路径的筛选。"),
            text("owner", 1, 200, false, "负责人的筛选。"),
            boolean("proposed_only", "true 只列尚未采纳的提案。"),
            int("limit", 1, 50, false, "本页的最多任务数，默认 20。"),
            text("cursor", 1, 20, false, "上一次返回的 next_cursor。"),
        ],
        "doc_task_get" => vec![
            text("task", 1, 20, true, "任务号，如 T-12。"),
            int("events", 0, 30, false, "返回的最近事件数，默认 10。"),
        ],
        "doc_trace_get" => vec![
            text(
                "task",
                1,
                20,
                false,
                "任务号；与 file_ref、section 二选一。",
            ),
            text(
                "file_ref",
                1,
                200,
                false,
                "文件编号或路径，需与 section 同时给出。",
            ),
            text("section", 1, 200, false, "条款键，需与 file_ref 同时给出。"),
        ],
        "doc_schedule_get" => Vec::new(),
        "doc_task_create" => vec![
            text("file_ref", 1, 200, true, "文件编号或路径。"),
            text("section", 1, 200, true, "条款键、条款编号或 ROOT。"),
            text("title", 1, 200, true, "任务标题。"),
            text("description", 0, 4000, false, "任务描述。"),
            int("priority", 1, 3, false, "优先级，1 最高，默认 2。"),
            field(
                "depends_on",
                Kind::TextList {
                    max_items: 10,
                    item_max: 20,
                },
                false,
                "前置任务号列表。",
            ),
            text("rationale", 0, 1000, false, "创建理由。"),
            text(
                "hermes_session",
                0,
                64,
                false,
                "Hermes 会话标识，仅作记录。",
            ),
        ],
        "doc_annotation_add" => vec![
            text("file_ref", 1, 200, true, "文件编号或路径。"),
            text("section", 1, 200, true, "条款键、条款编号或 ROOT。"),
            text("text", 1, 4000, true, "批注正文。"),
            text("rationale", 0, 1000, false, "批注理由。"),
            text("quote", 0, 500, false, "被批注的原文摘录。"),
            text(
                "hermes_session",
                0,
                64,
                false,
                "Hermes 会话标识，仅作记录。",
            ),
        ],
        "doc_task_transition" => vec![
            text("task", 1, 20, true, "任务号。"),
            field(
                "to",
                Kind::Choice(&TRANSITION_TARGETS),
                true,
                "目标状态：analysis、verification 或 working。",
            ),
            text(
                "reason",
                0,
                SUMMARY_LIMIT,
                false,
                "理由；目标为 verification 时为工作摘要，必填。",
            ),
            text(
                "hermes_session",
                0,
                64,
                false,
                "Hermes 会话标识，仅作记录。",
            ),
        ],
        "doc_evidence_add" => vec![
            text("task", 1, 20, true, "任务号。"),
            field(
                "kind",
                Kind::Choice(&EVIDENCE_KINDS),
                true,
                "证据种类：test_run、report、note 或 version。",
            ),
            text("result", 1, 40, false, "证据结果，默认 info。"),
            text("summary", 1, 1000, true, "证据摘要。"),
            text("ref", 0, 500, false, "参考位置，附于摘要之后。"),
            text("version", 1, 64, false, "关联的版本标识。"),
            text(
                "hermes_session",
                0,
                64,
                false,
                "Hermes 会话标识，仅作记录。",
            ),
        ],
        "doc_link_add" => vec![
            text("task", 1, 20, true, "任务号。"),
            text("kind", 1, 64, true, "关联种类。"),
            text(
                "file_ref",
                1,
                200,
                false,
                "关联的文件编号或路径，与 commit、url 三选一。",
            ),
            text("section", 1, 200, false, "关联文件中的条款键。"),
            text(
                "commit",
                1,
                64,
                false,
                "关联的提交号，与 file_ref、url 三选一。",
            ),
            text(
                "url",
                1,
                2000,
                false,
                "关联的网址，与 file_ref、commit 三选一。",
            ),
            text("label", 0, 120, false, "关联的显示名称。"),
            text(
                "hermes_session",
                0,
                64,
                false,
                "Hermes 会话标识，仅作记录。",
            ),
        ],
        _ => Vec::new(),
    }
}

/// The one-sentence description of a tool.
fn description_of(tool: &str) -> &'static str {
    match tool {
        "doc_file_find" => "列出已跟踪且允许外传的文档文件及其版本、基线与未完成任务数。",
        "doc_clause_list" => "列出文件某版本的条款目录，含批注数与任务数。",
        "doc_clause_read" => "分段读取文件某版本中一个条款的文本。",
        "doc_mentions_find" => "在允许外传的文本文件中做字面子串检索并返回所在条款。",
        "doc_task_list" => "按状态、文件、负责人筛选可见的文档任务。",
        "doc_task_get" => "读取一个任务的卡片、最近事件、最近决定与条款文本。",
        "doc_trace_get" => "读取任务或条款的关联与证据。",
        "doc_schedule_get" => "读取可见任务的调度批次、冲突与阻塞。",
        "doc_task_create" => "创建待所有者采纳的文档任务提案。",
        "doc_annotation_add" => "为文件条款添加共享批注。",
        "doc_task_transition" => "推进任务的分析、工作与验证阶段。",
        "doc_evidence_add" => "为任务添加证据，证据不会使任务完成。",
        "doc_link_add" => "为任务添加文件、提交或网址关联。",
        _ => "",
    }
}

/// Whether a tool is one of the delivered tools.
fn is_known_tool(tool: &str) -> bool {
    TOOL_NAMES.contains(&tool)
}

/// The JSON Schema of one argument.
fn property(field: &Field) -> Value {
    let mut property = Map::new();
    match field.kind {
        Kind::Text { min, max } => {
            property.insert("type".into(), json!("string"));
            if min > 0 {
                property.insert("minLength".into(), json!(min));
            }
            property.insert("maxLength".into(), json!(max));
        }
        Kind::Int { min, max } => {
            property.insert("type".into(), json!("integer"));
            property.insert("minimum".into(), json!(min));
            property.insert("maximum".into(), json!(max));
        }
        Kind::Bool => {
            property.insert("type".into(), json!("boolean"));
        }
        Kind::Choice(values) => {
            property.insert("type".into(), json!("string"));
            property.insert("enum".into(), json!(values));
        }
        Kind::ChoiceList { values, max_items } => {
            property.insert("type".into(), json!("array"));
            property.insert("items".into(), json!({"type": "string", "enum": values}));
            property.insert("maxItems".into(), json!(max_items));
        }
        Kind::TextList {
            max_items,
            item_max,
        } => {
            property.insert("type".into(), json!("array"));
            property.insert(
                "items".into(),
                json!({"type": "string", "minLength": 1, "maxLength": item_max}),
            );
            property.insert("maxItems".into(), json!(max_items));
        }
    }
    property.insert("description".into(), json!(field.description));
    Value::Object(property)
}

/// The `tools/list` descriptors of the delivered tools, in `TOOL_NAMES` order.
pub fn tool_descriptors() -> Vec<Value> {
    TOOL_NAMES.iter().map(|name| descriptor(name)).collect()
}

fn descriptor(name: &str) -> Value {
    let fields = fields_of(name);
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &fields {
        properties.insert(field.name.to_string(), property(field));
        if field.required {
            required.push(json!(field.name));
        }
    }
    let mut tool = Map::new();
    tool.insert("name".into(), json!(name));
    tool.insert("description".into(), json!(description_of(name)));
    tool.insert(
        "inputSchema".into(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": properties,
            "required": required,
        }),
    );
    if READ_ONLY_TOOLS.contains(&name) {
        tool.insert("annotations".into(), json!({"readOnlyHint": true}));
    }
    Value::Object(tool)
}

fn invalid_arg(message: impl Into<String>, hint: impl Into<String>) -> DocError {
    DocError::new(ErrorCode::InvalidArg, message, hint)
}

fn invalid_property(name: &str, detail: &str) -> DocError {
    invalid_arg(
        format!("参数不合法：{name}，{detail}"),
        "按工具模式修正参数后重试",
    )
}

/// Whether a character is refused in text arguments: every control character except line feed
/// and tab.
fn is_refused_control(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

fn check_field(field: &Field, value: &Value) -> Result<(), DocError> {
    match field.kind {
        Kind::Text { min, max } => {
            let Some(text) = value.as_str() else {
                return Err(invalid_property(field.name, "应为字符串"));
            };
            let length = text.chars().count();
            if length < min || length > max {
                return Err(invalid_property(field.name, "长度超出范围"));
            }
            if text.chars().any(is_refused_control) {
                return Err(invalid_property(field.name, "含有控制字符"));
            }
            Ok(())
        }
        Kind::Int { min, max } => {
            let Some(number) = value.as_i64() else {
                return Err(invalid_property(field.name, "应为整数"));
            };
            if number < min || number > max {
                return Err(invalid_property(field.name, "数值超出范围"));
            }
            Ok(())
        }
        Kind::Bool => {
            if value.as_bool().is_none() {
                return Err(invalid_property(field.name, "应为布尔值"));
            }
            Ok(())
        }
        Kind::Choice(values) => match value.as_str() {
            Some(text) if values.contains(&text) => Ok(()),
            _ => Err(invalid_property(field.name, "取值不在枚举之中")),
        },
        Kind::ChoiceList { values, max_items } => {
            let Some(items) = value.as_array() else {
                return Err(invalid_property(field.name, "应为数组"));
            };
            if items.len() > max_items {
                return Err(invalid_property(field.name, "元素过多"));
            }
            for item in items {
                match item.as_str() {
                    Some(text) if values.contains(&text) => {}
                    _ => return Err(invalid_property(field.name, "元素不在枚举之中")),
                }
            }
            Ok(())
        }
        Kind::TextList {
            max_items,
            item_max,
        } => {
            let Some(items) = value.as_array() else {
                return Err(invalid_property(field.name, "应为数组"));
            };
            if items.len() > max_items {
                return Err(invalid_property(field.name, "元素过多"));
            }
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(invalid_property(field.name, "元素应为字符串"));
                };
                let length = text.chars().count();
                if length == 0 || length > item_max || text.chars().any(is_refused_control) {
                    return Err(invalid_property(field.name, "元素不合法"));
                }
            }
            Ok(())
        }
    }
}

/// Checks the arguments of a delivered tool against its field table and returns the supplied
/// values. Defaults are applied by the handlers, not here.
pub fn validate_arguments(tool: &str, arguments: &Value) -> Result<Map<String, Value>, DocError> {
    let fields = fields_of(tool);
    let object = match arguments {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => return Err(invalid_arg("参数应为对象", "按工具模式修正参数后重试")),
    };
    for key in object.keys() {
        if !fields.iter().any(|field| field.name == key) {
            return Err(invalid_arg(
                format!("参数不合法：未知属性 {key}"),
                "按工具模式修正参数后重试",
            ));
        }
    }
    for field in &fields {
        match object.get(field.name) {
            None if field.required => {
                return Err(invalid_arg(
                    format!("参数不合法：缺少 {}", field.name),
                    "按工具模式修正参数后重试",
                ));
            }
            None => {}
            Some(value) => check_field(field, value)?,
        }
    }
    Ok(object)
}

/// The launch identifier rule: 1 to `LAUNCH_ID_MAX` ASCII letters, digits or hyphens.
pub fn valid_launch_id(launch_id: &str) -> bool {
    !launch_id.is_empty()
        && launch_id.len() <= LAUNCH_ID_MAX
        && launch_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Removes control characters other than line feed and tab from every string of a value.
fn sanitize(value: &mut Value) {
    match value {
        Value::String(text) => {
            if text.chars().any(is_refused_control) {
                *text = text.chars().filter(|c| !is_refused_control(*c)).collect();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(sanitize),
        Value::Object(map) => map.values_mut().for_each(sanitize),
        _ => {}
    }
}

/// Keeps the longest prefix of `items` whose serialized size stays within `budget` bytes, and
/// reports whether any item was dropped.
pub fn fit_list(items: Vec<Value>, budget: usize) -> (Vec<Value>, bool) {
    let total = items.len();
    let mut used = 0usize;
    let mut kept = Vec::new();
    for item in items {
        let size = serde_json::to_string(&item)
            .map(|text| text.len())
            .unwrap_or(0);
        if used + size > budget {
            break;
        }
        used += size;
        kept.push(item);
    }
    let truncated = kept.len() < total;
    (kept, truncated)
}

/// The success envelope: an ordinary result whose text is the compact JSON of the reply.
pub fn envelope_ok(data: Value, summary_zh: &str, next_zh: &str) -> Value {
    let body = json!({"ok": true, "data": data, "summary_zh": summary_zh, "next_zh": next_zh});
    text_result(&body, false)
}

/// The refusal envelope of a business, argument or transient failure. It is an ordinary result:
/// no `isError` member and no `error` key.
pub fn envelope_refused(code: ErrorCode, message: &str, hint: &str) -> Value {
    let body = json!({"ok": false, "code": code.as_str(), "message": message, "hint": hint});
    text_result(&body, false)
}

/// The failure envelope of an internal fault, with `isError` set.
pub fn internal_failure() -> Value {
    let body = json!({
        "ok": false,
        "code": ErrorCode::Internal.as_str(),
        "message": INTERNAL_MESSAGE,
        "hint": INTERNAL_HINT,
    });
    text_result(&body, true)
}

fn text_result(body: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string(body).unwrap_or_else(|_| String::from("{\"ok\":false}"));
    let mut result = json!({"content": [{"type": "text", "text": text}]});
    if is_error {
        result["isError"] = json!(true);
    }
    result
}

/// The text of the message written for a panic: the location only, never the payload.
pub fn panic_report(location: Option<(&str, u32)>) -> String {
    match location {
        Some((file, line)) => format!("mcp-serve panic at {file}:{line} E_INTERNAL"),
        None => "mcp-serve panic E_INTERNAL".to_string(),
    }
}

/// Writes one line to standard error. Failures are ignored: diagnostics never fail a call.
fn write_stderr(line: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
}

/// Wraps one tool call so that a panic completes it with `E_INTERNAL` instead of unwinding
/// through the server loop. The inner future need not be `Send`.
pub struct CatchPanic<'a> {
    inner: ToolCall<'a>,
}

impl<'a> CatchPanic<'a> {
    pub fn new(inner: ToolCall<'a>) -> CatchPanic<'a> {
        CatchPanic { inner }
    }
}

impl Future for CatchPanic<'_> {
    type Output = Option<Value>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Value>> {
        let inner = &mut self.inner;
        match catch_unwind(AssertUnwindSafe(|| inner.as_mut().poll(cx))) {
            Ok(Poll::Ready(output)) => Poll::Ready(output),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Some(internal_failure())),
        }
    }
}

/// The result of a handler: the data of the reply and its two sentences.
struct Payload {
    data: Value,
    summary: String,
    next: String,
}

fn done(data: Value, summary: impl Into<String>, next: impl Into<String>) -> Payload {
    Payload {
        data,
        summary: summary.into(),
        next: next.into(),
    }
}

/// An envelope with the code it reports, for the log line.
struct Outcome {
    envelope: Value,
    code: &'static str,
}

fn refusal_outcome(err: &anyhow::Error) -> Outcome {
    if let Some(doc) = err.downcast_ref::<DocError>() {
        return Outcome {
            envelope: envelope_refused(doc.code, &doc.message_zh, &doc.hint_zh),
            code: doc.code.as_str(),
        };
    }
    if err.downcast_ref::<StoreBusy>().is_some() {
        return Outcome {
            envelope: envelope_refused(ErrorCode::StoreBusy, STORE_BUSY_MESSAGE, STORE_BUSY_HINT),
            code: ErrorCode::StoreBusy.as_str(),
        };
    }
    Outcome {
        envelope: internal_failure(),
        code: ErrorCode::Internal.as_str(),
    }
}

fn outcome_of(result: Result<Payload>) -> Outcome {
    match result {
        Ok(payload) => Outcome {
            envelope: envelope_ok(payload.data, &payload.summary, &payload.next),
            code: "ok",
        },
        Err(err) => refusal_outcome(&err),
    }
}

/// Maps a failed engine open. A lock failure is `E_STORE_BUSY`; an unreadable global rule file
/// is `E_POLICY_DENIED`; anything else is internal.
fn open_failure(err: &anyhow::Error, global_ignore: Option<&Path>) -> Outcome {
    if err.downcast_ref::<StoreBusy>().is_some() {
        return refusal_outcome(err);
    }
    if let Some(path) = global_ignore {
        if std::fs::File::open(path).is_err() {
            return Outcome {
                envelope: envelope_refused(
                    ErrorCode::PolicyDenied,
                    "外传门禁不可用",
                    "检查全局规则文件与规则集",
                ),
                code: ErrorCode::PolicyDenied.as_str(),
            };
        }
    }
    refusal_outcome(err)
}

/// Sanitizes the envelope, enforces the size limit, writes the diagnostic line and returns the
/// envelope to send.
fn finish(name: &str, started: Instant, outcome: Outcome) -> Value {
    let mut outcome = outcome;
    sanitize(&mut outcome.envelope);
    let oversized = outcome.envelope["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.len() > RESULT_LIMIT_BYTES);
    if oversized {
        outcome = Outcome {
            envelope: internal_failure(),
            code: ErrorCode::Internal.as_str(),
        };
    }
    write_stderr(&format!(
        "mcp-serve tool={} ms={} code={}",
        name,
        started.elapsed().as_millis(),
        outcome.code
    ));
    outcome.envelope
}

/// Per-call state of a read or write handler.
struct Ctx<'a> {
    engine: &'a Engine,
    launch: &'a str,
    disclosed: &'a mut HashSet<(String, String)>,
}

fn caller(launch: &str) -> Caller {
    Caller::agent(launch)
}

async fn svc<'a>(engine: &'a Engine, launch: &str) -> Result<DocTaskService<'a>> {
    DocTaskService::attach(engine, caller(launch)).await
}

async fn flow<'a>(engine: &'a Engine, launch: &str) -> Result<DocFlow<'a>> {
    DocFlow::attach(engine, caller(launch)).await
}

fn not_found() -> anyhow::Error {
    DocError::new(ErrorCode::NotFound, "没有找到该对象", "检查编号是否正确").into()
}

fn req_str<'a>(args: &'a Map<String, Value>, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

fn opt_str<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn opt_int(args: &Map<String, Value>, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

fn int_or(args: &Map<String, Value>, key: &str, default: i64) -> i64 {
    opt_int(args, key).unwrap_or(default)
}

fn opt_bool(args: &Map<String, Value>, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn str_list(args: &Map<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn ext_of(path: &str) -> &str {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

/// Whether the bytes are text: valid UTF-8 without NUL.
fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// The version selected by number, or the latest one. A refused version is the same not-found
/// as a missing one.
async fn select_version(
    engine: &Engine,
    file_id: &str,
    number: Option<i64>,
) -> Result<VersionRecord> {
    let versions = engine.store.versions(file_id).await?;
    let chosen = match number {
        Some(wanted) => versions
            .into_iter()
            .find(|version| version.number == wanted),
        None => versions.into_iter().max_by_key(|version| version.number),
    };
    let Some(version) = chosen else {
        return Err(not_found());
    };
    if !EngineGate::new(engine).version_allowed(&version).await {
        return Err(not_found());
    }
    Ok(version)
}

/// The bytes of a version, which must be text.
async fn load_text(engine: &Engine, version: &VersionRecord) -> Result<Vec<u8>> {
    let bytes = engine.load_version(&version.id, None).await?;
    if !is_text(&bytes) {
        return Err(DocError::new(
            ErrorCode::UnsupportedFile,
            "只能读取文本文件",
            "二进制文件不能作为条款读取",
        )
        .into());
    }
    Ok(bytes)
}

/// Appends the disclosure row of one version, once per process. It is written before the
/// content of the version is returned.
async fn disclose(cx: &mut Ctx<'_>, file_id: &str, version: &VersionRecord) -> Result<()> {
    let key = (file_id.to_string(), version.id.clone());
    if cx.disclosed.contains(&key) {
        return Ok(());
    }
    DocStore::new(cx.engine.store.clone())
        .insert_row(
            "doc_agent_disclosures",
            &[
                ("launch_id", SqlValue::Text(cx.launch.to_string())),
                ("file_id", SqlValue::Text(file_id.to_string())),
                ("version_id", SqlValue::Text(version.id.clone())),
                ("content_hash", SqlValue::Text(version.content_hash.clone())),
                ("first_at", SqlValue::Text(Utc::now().to_rfc3339())),
            ],
        )
        .await?;
    cx.disclosed.insert(key);
    Ok(())
}

/// The key of the clause that holds a 1-based line, or `ROOT`.
fn section_of(outline: &Outline, line: usize) -> String {
    outline
        .clauses()
        .find(|clause| clause.first_line <= line && line <= clause.last_line)
        .map(|clause| clause.key.clone())
        .unwrap_or_else(|| "ROOT".to_string())
}

async fn open_task_counts(svc: &DocTaskService<'_>) -> Result<HashMap<String, i64>> {
    let mut counts = HashMap::new();
    for task in svc.list_tasks().await?.items {
        if task.status != "done" && task.status != "cancelled" {
            *counts.entry(task.path.clone()).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn task_json(task: &TaskView) -> Value {
    json!({
        "task": task.handle,
        "id": task.id,
        "title": task.title,
        "status": task.status,
        "priority": task.priority,
        "owner": task.owner,
        "file_ref": task.file_ref,
        "section": task.section,
        "anchor_state": task.anchor_state,
        "proposal_state": task.proposal_state,
        "blocked_by": task.blocked_by,
        "updated_at": task.updated_at,
    })
}

fn step_json(step: &crate::doctask_flow::Step) -> Value {
    json!({
        "handle": step.handle,
        "from": step.from,
        "to": step.to,
        "events": step.events,
    })
}

async fn file_find(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let query = opt_str(args, "query").map(str::to_lowercase);
    let has_tasks = args.get("has_tasks").and_then(Value::as_bool);
    let limit = int_or(args, "limit", 20).max(1) as usize;
    let open = open_task_counts(&service).await?;
    let baselines = service.list_baselines(None).await?.items;
    let gate = EngineGate::new(cx.engine);
    let mut files = cx.engine.store.list_files().await?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut rows = Vec::new();
    let mut truncated = false;
    for file in files {
        if file.deleted || !gate.file_allowed(&file).await {
            continue;
        }
        if let Some(needle) = &query {
            if !file.relative_path.to_lowercase().contains(needle) {
                continue;
            }
        }
        let count = open.get(&file.relative_path).copied().unwrap_or(0);
        if let Some(wanted) = has_tasks {
            if wanted != (count > 0) {
                continue;
            }
        }
        if rows.len() == limit {
            truncated = true;
            break;
        }
        let baseline = baselines
            .iter()
            .filter(|baseline| baseline.file_id == file.id && baseline.revoked_at.is_none())
            .map(|baseline| baseline.version_no)
            .max();
        rows.push(json!({
            "file_ref": file.id,
            "path": file.relative_path,
            "version_no": file.version_no,
            "baseline_version_no": baseline,
            "open_tasks": count,
        }));
    }
    let (rows, cut) = fit_list(rows, LIST_BUDGET);
    let count = rows.len();
    Ok(done(
        json!({"files": rows, "truncated": truncated || cut}),
        format!("{count} 个文件"),
        "用 doc_clause_list 查看文件的条款",
    ))
}

async fn clause_list(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let file = service.resolve_file(req_str(args, "file_ref")).await?;
    let version = select_version(cx.engine, &file.id, opt_int(args, "version")).await?;
    let bytes = load_text(cx.engine, &version).await?;
    disclose(cx, &file.id, &version).await?;
    let outline = doctask_outline::outline(&bytes, ext_of(&file.relative_path));
    let mut notes: HashMap<String, i64> = HashMap::new();
    for note in service
        .list_annotations(Some(file.id.as_str()))
        .await?
        .items
    {
        *notes.entry(note.section.clone()).or_insert(0) += 1;
    }
    let mut tasks: HashMap<String, i64> = HashMap::new();
    for task in service.list_tasks().await?.items {
        if task.path == file.relative_path {
            *tasks.entry(task.section.clone()).or_insert(0) += 1;
        }
    }
    let rows: Vec<Value> = outline
        .clauses()
        .map(|clause| {
            json!({
                "ordinal": clause.ordinal,
                "number": clause.number.clone().unwrap_or_default(),
                "title": clause.title,
                "level": clause.level,
                "first_line": clause.first_line,
                "last_line": clause.last_line,
                "notes": notes.get(&clause.key).copied().unwrap_or(0),
                "tasks": tasks.get(&clause.key).copied().unwrap_or(0),
            })
        })
        .collect();
    let (rows, cut) = fit_list(rows, LIST_BUDGET);
    let count = rows.len();
    Ok(done(
        json!({
            "file_ref": file.id,
            "version_no": version.number,
            "rows": rows,
            "truncated": cut,
        }),
        format!("{count} 个条款"),
        "用 doc_clause_read 读取条款文本",
    ))
}

async fn clause_read(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let file = service.resolve_file(req_str(args, "file_ref")).await?;
    let version = select_version(cx.engine, &file.id, opt_int(args, "version")).await?;
    let bytes = load_text(cx.engine, &version).await?;
    let outline = doctask_outline::outline(&bytes, ext_of(&file.relative_path));
    let selector = parse_section_selector(Some(req_str(args, "section")))?;
    let clause = doctask_outline::resolve(&outline, &selector)?;
    let slice = doctask_outline::clause_bytes(&bytes, clause).ok_or_else(|| {
        DocError::new(
            ErrorCode::SectionNotFound,
            "条款范围不在版本内容之中",
            "用 doc_clause_list 查看条款范围",
        )
    })?;
    let hash = sha256_hex(slice);
    let chars: Vec<char> = String::from_utf8_lossy(slice).chars().collect();
    let offset = int_or(args, "offset", 0).max(0) as usize;
    let window = int_or(args, "max_chars", 3000).max(1) as usize;
    let start = offset.min(chars.len());
    let end = start.saturating_add(window).min(chars.len());
    let text: String = chars[start..end].iter().collect();
    let truncated = end < chars.len();
    disclose(cx, &file.id, &version).await?;
    Ok(done(
        json!({
            "text": text,
            "version_no": version.number,
            "hash": hash,
            "truncated": truncated,
            "next_offset": if truncated { json!(end) } else { Value::Null },
        }),
        "已读取条款文本",
        "用 next_offset 继续读取剩余文本",
    ))
}

async fn mentions_find(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let token = req_str(args, "token");
    let limit = int_or(args, "limit", 10).max(1) as usize;
    let gate = EngineGate::new(cx.engine);
    let mut files = cx.engine.store.list_files().await?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut matches = Vec::new();
    let mut hits: Vec<(String, VersionRecord)> = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;
    'files: for file in files {
        if file.deleted || !matches!(ext_of(&file.relative_path), "md" | "nema" | "txt") {
            continue;
        }
        if !gate.file_allowed(&file).await {
            continue;
        }
        if scanned == MENTION_FILE_LIMIT {
            truncated = true;
            break;
        }
        let Some(version) = cx.engine.store.latest_version(&file.id).await? else {
            continue;
        };
        if version.byte_size > MENTION_SIZE_LIMIT || !gate.version_allowed(&version).await {
            continue;
        }
        scanned += 1;
        let bytes = cx.engine.load_version(&version.id, None).await?;
        if !is_text(&bytes) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let outline = doctask_outline::outline(&bytes, ext_of(&file.relative_path));
        let mut found = false;
        for (index, line) in text.lines().enumerate() {
            if !line.contains(token) {
                continue;
            }
            if matches.len() == limit {
                truncated = true;
                break 'files;
            }
            let number = index + 1;
            matches.push(json!({
                "file_ref": file.id,
                "path": file.relative_path,
                "section": section_of(&outline, number),
                "line": number,
            }));
            found = true;
        }
        if found {
            hits.push((file.id.clone(), version));
        }
    }
    for (file_id, version) in &hits {
        disclose(cx, file_id, version).await?;
    }
    let (matches, cut) = fit_list(matches, LIST_BUDGET);
    let count = matches.len();
    Ok(done(
        json!({"matches": matches, "truncated": truncated || cut}),
        format!("{count} 处命中"),
        "用 doc_clause_read 读取命中所在的条款",
    ))
}

async fn task_list(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let listing = service.list_tasks().await?;
    let statuses = str_list(args, "status");
    let file_ref = opt_str(args, "file_ref");
    let owner = opt_str(args, "owner");
    let proposed_only = opt_bool(args, "proposed_only");
    let limit = int_or(args, "limit", 20).max(1) as usize;
    let offset = match opt_str(args, "cursor") {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| invalid_arg("游标不合法", "使用上一次返回的 next_cursor"))?,
    };
    let filtered: Vec<&TaskView> = listing
        .items
        .iter()
        .filter(|task| statuses.is_empty() || statuses.contains(&task.status))
        .filter(|task| file_ref.is_none_or(|f| task.file_ref == f || task.path == f))
        .filter(|task| owner.is_none_or(|o| task.owner == o))
        .filter(|task| !proposed_only || task.proposal_state == ProposalState::Proposed.as_str())
        .collect();
    let total = filtered.len();
    let page: Vec<Value> = filtered
        .iter()
        .skip(offset)
        .take(limit)
        .map(|task| task_json(task))
        .collect();
    let (page, cut) = fit_list(page, LIST_BUDGET);
    let next = offset + page.len();
    let next_cursor = if next < total {
        json!(next.to_string())
    } else {
        Value::Null
    };
    let count = page.len();
    Ok(done(
        json!({
            "tasks": page,
            "hidden": listing.hidden,
            "next_cursor": next_cursor,
            "truncated": cut,
        }),
        format!("{count} 个任务"),
        "用 doc_task_get 查看任务卡片",
    ))
}

/// The latest approval row of a task, or null.
async fn last_decision(engine: &Engine, task_id: &str) -> Result<Value> {
    let approvals = DocStore::new(engine.store.clone()).approvals().await?;
    Ok(approvals
        .iter()
        .rev()
        .find(|approval| approval.task_id == task_id)
        .map(|approval| {
            json!({
                "decision": approval.decision.as_str(),
                "comment": approval.comment,
                "by": approval.approver_kind.as_str(),
                "at": approval.created_at,
            })
        })
        .unwrap_or(Value::Null))
}

/// The text of the clause of a task in its base version, cut to `CLAUSE_TEXT_LIMIT` characters,
/// with the truncation flag. Null when the version is refused or the section does not resolve.
async fn base_clause(cx: &mut Ctx<'_>, task: &TaskView) -> Result<(Value, bool)> {
    let Some(version) = cx.engine.store.version(&task.base_version_id).await? else {
        return Ok((Value::Null, false));
    };
    if !EngineGate::new(cx.engine).version_allowed(&version).await {
        return Ok((Value::Null, false));
    }
    let bytes = cx.engine.load_version(&version.id, None).await?;
    if !is_text(&bytes) {
        return Ok((Value::Null, false));
    }
    let outline = doctask_outline::outline(&bytes, ext_of(&task.path));
    let Ok(selector) = parse_section_selector(Some(&task.section)) else {
        return Ok((Value::Null, false));
    };
    let Ok(clause) = doctask_outline::resolve(&outline, &selector) else {
        return Ok((Value::Null, false));
    };
    let Some(slice) = doctask_outline::clause_bytes(&bytes, clause) else {
        return Ok((Value::Null, false));
    };
    let chars: Vec<char> = String::from_utf8_lossy(slice).chars().collect();
    let truncated = chars.len() > CLAUSE_TEXT_LIMIT;
    let shown: String = chars.iter().take(CLAUSE_TEXT_LIMIT).collect();
    disclose(cx, &task.file_ref, &version).await?;
    Ok((json!(shown), truncated))
}

async fn task_get(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let card = service.show_task(req_str(args, "task")).await?;
    let wanted = int_or(args, "events", 10).max(0) as usize;
    let skip = card.events.len().saturating_sub(wanted);
    let events: Vec<Value> = card.events[skip..]
        .iter()
        .map(|event| {
            json!({
                "seq": event.seq,
                "kind": event.kind,
                "from_status": event.from_status,
                "to_status": event.to_status,
                "actor": event.actor,
                "actor_kind": event.actor_kind,
                "timestamp": event.timestamp,
            })
        })
        .collect();
    let decision = last_decision(cx.engine, &card.task.id).await?;
    let (clause_text, clause_truncated) = base_clause(cx, &card.task).await?;
    let mut detail = task_json(&card.task);
    detail["description"] = json!(card.task.description);
    let annotations: Vec<Value> = service
        .list_annotations(Some(card.task.file_ref.as_str()))
        .await?
        .items
        .into_iter()
        .filter(|note| note.section == card.task.section)
        .map(|note| {
            json!({
                "id": note.id,
                "handle": note.handle,
                "text": note.text,
                "author": note.author,
                "author_kind": note.author_kind,
                "quote": note.quote,
                "created_at": note.created_at,
            })
        })
        .collect();
    let (annotations, cut) = fit_list(annotations, LIST_BUDGET / 2);
    Ok(done(
        json!({
            "task": detail,
            "events": events,
            "last_decision": decision,
            "clause_text": clause_text,
            "clause_truncated": clause_truncated,
            "annotations": annotations,
            "annotations_truncated": cut,
        }),
        format!("任务 {}", card.task.handle),
        "按 last_decision 与 events 判断下一步",
    ))
}

async fn trace_get(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let listing = service.list_tasks().await?;
    let visible: HashSet<String> = listing
        .items
        .iter()
        .map(|task| task.handle.clone())
        .collect();
    let mut hidden = listing.hidden;
    let targets: Vec<TaskView> = match opt_str(args, "task") {
        Some(handle) => vec![service.show_task(handle).await?.task],
        None => {
            let (Some(file_ref), Some(section)) =
                (opt_str(args, "file_ref"), opt_str(args, "section"))
            else {
                return Err(invalid_arg(
                    "必须给出 task，或同时给出 file_ref 与 section",
                    "按工具模式修正参数后重试",
                )
                .into());
            };
            let file = service.resolve_file(file_ref).await?;
            listing
                .items
                .iter()
                .filter(|task| task.path == file.relative_path && task.section == section)
                .cloned()
                .collect()
        }
    };
    let ds = DocStore::new(cx.engine.store.clone());
    let all_links = ds.links().await?;
    let all_evidence = ds.evidence().await?;
    let gate = EngineGate::new(cx.engine);
    let mut traces = Vec::new();
    for target in &targets {
        let mut links = Vec::new();
        for link in all_links
            .iter()
            .filter(|link| link.task_id == target.id && link.removed_at.is_none())
        {
            if link.target_kind == "task" && !visible.contains(&link.target_ref) {
                hidden += 1;
                continue;
            }
            if !link.target_file_id.is_empty() {
                let allowed = match cx.engine.store.file_by_id(&link.target_file_id).await? {
                    Some(file) => gate.file_allowed(&file).await,
                    None => false,
                };
                if !allowed {
                    hidden += 1;
                    continue;
                }
            }
            links.push(json!({
                "link_kind": link.link_kind,
                "target_kind": link.target_kind,
                "target_ref": link.target_ref,
                "target_section": link.target_section,
                "label": link.label,
                "created_at": link.created_at,
            }));
        }
        let evidence: Vec<Value> = all_evidence
            .iter()
            .filter(|row| row.task_id == target.id)
            .map(|row| {
                json!({
                    "kind": row.kind.as_str(),
                    "result": row.result.as_str(),
                    "source": row.source.as_str(),
                    "actor_kind": row.actor_kind.as_str(),
                    "created_at": row.created_at,
                })
            })
            .collect();
        traces.push(json!({
            "task": target.handle,
            "status": target.status,
            "links": links,
            "evidence": evidence,
        }));
    }
    let (traces, cut) = fit_list(traces, LIST_BUDGET);
    let count = traces.len();
    Ok(done(
        json!({"traces": traces, "hidden": hidden, "truncated": cut}),
        format!("{count} 个任务的追踪"),
        "用 doc_task_get 查看任务卡片",
    ))
}

async fn schedule_get(cx: &mut Ctx<'_>) -> Result<Payload> {
    let flow = flow(cx.engine, cx.launch).await?;
    let view = flow.schedule().await?;
    let schedule = &view.schedule;
    Ok(done(
        json!({
            "batches": schedule.batches,
            "conflicts": schedule
                .conflicts
                .iter()
                .map(|conflict| json!({"file": conflict.file, "tasks": conflict.tasks}))
                .collect::<Vec<Value>>(),
            "blocked": schedule
                .blocked
                .iter()
                .map(|blocked| json!({"task": blocked.task, "reason": blocked.reason}))
                .collect::<Vec<Value>>(),
            "hidden": view.hidden,
        }),
        "已读取调度",
        "按批次顺序处理可执行的任务",
    ))
}

async fn task_create(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let flow = flow(cx.engine, cx.launch).await?;
    flow.admit_proposal().await?;
    let service = svc(cx.engine, cx.launch).await?;
    let spec = TaskSpec {
        title: req_str(args, "title").to_string(),
        description: opt_str(args, "description").unwrap_or("").to_string(),
        owner: String::new(),
        priority: int_or(args, "priority", 2),
        private: false,
    };
    let created = service
        .create_task(req_str(args, "file_ref"), req_str(args, "section"), &spec)
        .await?;
    for prerequisite in str_list(args, "depends_on") {
        flow.add_dependency(&created.handle, &prerequisite).await?;
    }
    Ok(done(
        json!({"id": created.id, "handle": created.handle, "existing": created.existing}),
        if created.existing {
            "任务已存在"
        } else {
            "任务提案已创建"
        },
        "请所有者退出 Hermes，在 LADON 控制台采纳该提案后再开始分析",
    ))
}

async fn annotation_add(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let service = svc(cx.engine, cx.launch).await?;
    let added = service
        .add_annotation(
            req_str(args, "file_ref"),
            req_str(args, "section"),
            req_str(args, "text"),
            opt_str(args, "rationale").unwrap_or(""),
            opt_str(args, "quote").unwrap_or(""),
            false,
        )
        .await?;
    Ok(done(
        json!({"id": added.id, "handle": added.handle, "existing": added.existing}),
        if added.existing {
            "批注已存在"
        } else {
            "批注已添加"
        },
        "所有者可在控制台查看该批注",
    ))
}

/// The operation that a transition request maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    StartAnalysis,
    Rework,
    ReportWork,
    ReturnToWork,
}

fn transition_denied(status: &str) -> DocError {
    let moves = match status {
        "open" => "analysis",
        "working" => "analysis（重做方案）、verification（提交工作摘要）",
        "verification" => "working（返工）",
        "analysis" => "提交方案（doc_plan_submit）",
        "approval" => "无，等待所有者在控制台批准",
        _ => "无",
    };
    DocError::new(
        ErrorCode::TransitionDenied,
        "当前状态不允许该迁移",
        format!("当前状态 {status} 可做的迁移：{moves}"),
    )
}

fn require_reason(has_reason: bool, plan: Plan) -> Result<Plan, DocError> {
    if has_reason {
        Ok(plan)
    } else {
        Err(invalid_arg("该迁移需要理由", "在 reason 中给出理由后重试"))
    }
}

/// The operation for a move from `status` to `to`. Anything not listed is denied.
fn transition_plan(status: &str, to: &str, has_reason: bool) -> Result<Plan, DocError> {
    match (status, to) {
        ("open", "analysis") => Ok(Plan::StartAnalysis),
        ("working", "analysis") => require_reason(has_reason, Plan::Rework),
        ("working", "verification") => require_reason(has_reason, Plan::ReportWork),
        ("verification", "working") => require_reason(has_reason, Plan::ReturnToWork),
        _ => Err(transition_denied(status)),
    }
}

/// The reason limit: a work summary may be 4000 characters, any other reason 500.
fn check_transition_reason(to: &str, reason: &str) -> Result<(), DocError> {
    let limit = if to == "verification" {
        SUMMARY_LIMIT
    } else {
        REASON_LIMIT
    };
    if reason.chars().count() > limit {
        return Err(invalid_arg(
            format!("理由超过 {limit} 字符"),
            "缩短理由后重试",
        ));
    }
    Ok(())
}

async fn task_transition(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let to = req_str(args, "to");
    let reason = opt_str(args, "reason").unwrap_or("");
    check_transition_reason(to, reason)?;
    let service = svc(cx.engine, cx.launch).await?;
    let card = service.show_task(req_str(args, "task")).await?;
    let plan = transition_plan(&card.task.status, to, !reason.trim().is_empty())?;
    let flow = flow(cx.engine, cx.launch).await?;
    let handle = card.task.handle.clone();
    let step = match plan {
        Plan::StartAnalysis => flow.start_analysis(&handle).await?,
        Plan::Rework => flow.rework(&handle, reason).await?,
        Plan::ReportWork => flow.report_work(&handle, reason).await?,
        Plan::ReturnToWork => flow.return_to_work(&handle, reason).await?,
    };
    Ok(done(
        step_json(&step),
        format!("{} 迁移到 {}", step.handle, step.to),
        "用 doc_task_get 查看任务的最新状态",
    ))
}

/// The summary written to the evidence row: the summary, and the reference on its own line.
fn evidence_summary(summary: &str, reference: Option<&str>) -> Result<String, DocError> {
    let text = match reference {
        Some(reference) => format!("{summary}\n参考: {reference}"),
        None => summary.to_string(),
    };
    if text.chars().count() > EVIDENCE_TEXT_LIMIT {
        return Err(invalid_arg(
            "证据摘要与参考合计超过 1000 字符",
            "缩短 summary 或 ref 后重试",
        ));
    }
    Ok(text)
}

async fn evidence_add(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let summary = evidence_summary(req_str(args, "summary"), opt_str(args, "ref"))?;
    let flow = flow(cx.engine, cx.launch).await?;
    let step = flow
        .add_evidence(
            req_str(args, "task"),
            req_str(args, "kind"),
            opt_str(args, "result").unwrap_or("info"),
            &summary,
            opt_str(args, "version"),
        )
        .await?;
    Ok(done(
        step_json(&step),
        format!("已为 {} 添加证据", step.handle),
        "证据不会使任务完成，完成由所有者验收",
    ))
}

/// The one target of a link: the kind and the value of exactly one of `file_ref`, `commit`
/// and `url`.
fn link_target<'a>(
    file_ref: Option<&'a str>,
    commit: Option<&'a str>,
    url: Option<&'a str>,
) -> Result<(&'static str, &'a str), DocError> {
    let mut chosen = Vec::new();
    if let Some(value) = file_ref {
        chosen.push(("file", value));
    }
    if let Some(value) = commit {
        chosen.push(("commit", value));
    }
    if let Some(value) = url {
        chosen.push(("url", value));
    }
    match chosen.as_slice() {
        [single] => Ok(*single),
        _ => Err(invalid_arg(
            "关联必须且只能给出 file_ref、commit、url 之一",
            "只保留一个目标后重试",
        )),
    }
}

async fn link_add(cx: &mut Ctx<'_>, args: &Map<String, Value>) -> Result<Payload> {
    let (target_kind, target) = link_target(
        opt_str(args, "file_ref"),
        opt_str(args, "commit"),
        opt_str(args, "url"),
    )?;
    let flow = flow(cx.engine, cx.launch).await?;
    let step = flow
        .add_link(
            req_str(args, "task"),
            req_str(args, "kind"),
            target_kind,
            target,
            opt_str(args, "section"),
            opt_str(args, "label").unwrap_or(""),
        )
        .await?;
    Ok(done(
        step_json(&step),
        format!("已为 {} 添加关联", step.handle),
        "用 doc_trace_get 查看关联",
    ))
}

async fn execute(cx: &mut Ctx<'_>, tool: &str, args: &Map<String, Value>) -> Result<Payload> {
    match tool {
        "doc_file_find" => file_find(cx, args).await,
        "doc_clause_list" => clause_list(cx, args).await,
        "doc_clause_read" => clause_read(cx, args).await,
        "doc_mentions_find" => mentions_find(cx, args).await,
        "doc_task_list" => task_list(cx, args).await,
        "doc_task_get" => task_get(cx, args).await,
        "doc_trace_get" => trace_get(cx, args).await,
        "doc_schedule_get" => schedule_get(cx).await,
        "doc_task_create" => task_create(cx, args).await,
        "doc_annotation_add" => annotation_add(cx, args).await,
        "doc_task_transition" => task_transition(cx, args).await,
        "doc_evidence_add" => evidence_add(cx, args).await,
        "doc_link_add" => link_add(cx, args).await,
        _ => Err(invalid_arg("未知工具", "按 tools/list 的名称调用").into()),
    }
}

/// The tool host of one `mcp-serve` process.
pub struct DocToolHost {
    root: PathBuf,
    launch_id: String,
    global_ignore: Option<PathBuf>,
    engine: Option<Engine>,
    disclosed: HashSet<(String, String)>,
}

impl DocToolHost {
    /// A host for one workspace and launch. It opens nothing.
    pub fn new(root: PathBuf, launch_id: String, global_ignore: Option<PathBuf>) -> DocToolHost {
        DocToolHost {
            root,
            launch_id,
            global_ignore,
            engine: None,
            disclosed: HashSet::new(),
        }
    }

    async fn invoke(&mut self, name: &str, arguments: Value) -> Value {
        let started = Instant::now();
        let args = match validate_arguments(name, &arguments) {
            Ok(args) => args,
            Err(err) => {
                return finish(
                    name,
                    started,
                    Outcome {
                        envelope: envelope_refused(err.code, &err.message_zh, &err.hint_zh),
                        code: err.code.as_str(),
                    },
                )
            }
        };
        let DocToolHost {
            root,
            launch_id,
            global_ignore,
            engine,
            disclosed,
        } = self;
        if engine.is_none() {
            match Engine::open(root.as_path()).await {
                Ok(opened) => *engine = Some(opened),
                Err(err) => {
                    let outcome = open_failure(&err, global_ignore.as_deref());
                    return finish(name, started, outcome);
                }
            }
        }
        let Some(engine_ref) = engine.as_ref() else {
            return finish(
                name,
                started,
                refusal_outcome(&anyhow::anyhow!("engine missing")),
            );
        };
        let mut cx = Ctx {
            engine: engine_ref,
            launch: launch_id.as_str(),
            disclosed,
        };
        let outcome = outcome_of(execute(&mut cx, name, &args).await);
        finish(name, started, outcome)
    }
}

impl ToolHost for DocToolHost {
    fn tools(&self) -> Vec<Value> {
        tool_descriptors()
    }

    fn call<'a>(&'a mut self, name: &'a str, arguments: Value) -> ToolCall<'a> {
        if !is_known_tool(name) {
            return Box::pin(async { None::<Value> });
        }
        Box::pin(CatchPanic::new(Box::pin(async move {
            Some(self.invoke(name, arguments).await)
        })))
    }
}

/// Runs the MCP server of one workspace on standard input and output, and returns the process
/// exit code. A missing workspace or an invalid launch id is reported on standard error with
/// code 1 before any protocol line is read.
pub async fn serve(root: &Path, launch_id: &str, global_ignore: Option<&Path>) -> Result<i32> {
    if !valid_launch_id(launch_id) {
        write_stderr("mcp-serve: invalid launch id");
        return Ok(1);
    }
    if !root.join(".ladon").join("meta.turso").exists() {
        write_stderr("mcp-serve: workspace not initialized");
        return Ok(1);
    }
    if let Some(path) = global_ignore {
        std::env::set_var("LADON_GLOBAL_IGNORE", path);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map(|place| (place.file(), place.line()));
        write_stderr(&panic_report(location));
    }));
    let host = DocToolHost::new(
        root.to_path_buf(),
        launch_id.to_string(),
        global_ignore.map(Path::to_path_buf),
    );
    let mut server = Server::new(
        ServerInfo {
            name: "ladon",
            version: env!("CARGO_PKG_VERSION"),
        },
        host,
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let result = server.serve(stdin.lock(), stdout.lock()).await;
    std::panic::set_hook(previous);
    result?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::task::Waker;

    /// Serializes the tests that replace the panic hook.
    static HOOK_LOCK: Mutex<()> = Mutex::new(());
    /// Records the report lines of the test panic hook.
    static REPORTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future must be ready on the first poll"),
        }
    }

    fn text_of(result: &Value) -> Value {
        let text = result["content"][0]["text"].as_str().unwrap_or("{}");
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    #[test]
    fn ut_157_tool_list_and_panic_guard() {
        tc950_tool_list_snapshot();
        tc951_catch_panic_reports_internal_and_continues();
        tc952_panic_hook_writes_location_only();
    }

    fn tc950_tool_list_snapshot() {
        assert_eq!(TOOL_COUNT, 13);
        let tools = tool_descriptors();
        assert_eq!(tools.len(), TOOL_COUNT);
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(names, TOOL_NAMES.to_vec());
        let allowed = [
            "type",
            "description",
            "enum",
            "minLength",
            "maxLength",
            "minimum",
            "maximum",
            "items",
            "maxItems",
        ];
        let mut read_only = 0;
        for tool in &tools {
            let name = tool["name"].as_str().unwrap_or("");
            let hinted = tool["annotations"]["readOnlyHint"] == json!(true);
            assert_eq!(hinted, READ_ONLY_TOOLS.contains(&name), "{name}");
            if hinted {
                read_only += 1;
            }
            assert!(tool.get("outputSchema").is_none());
            assert!(format!("mcp__ladon__{name}").len() <= 64);
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object");
            assert_eq!(schema["additionalProperties"], false);
            let rendered = schema.to_string();
            for banned in ["$ref", "oneOf", "anyOf", "nullable", "allOf"] {
                assert!(!rendered.contains(banned), "{name} has {banned}");
            }
            let properties = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for required in schema["required"].as_array().cloned().unwrap_or_default() {
                let key = required.as_str().unwrap_or("");
                assert!(properties.contains_key(key), "{name}.{key}");
            }
            for property in properties.values() {
                let object = property.as_object().cloned().unwrap_or_default();
                for key in object.keys() {
                    assert!(allowed.contains(&key.as_str()), "{name} key {key}");
                }
                let description = object["description"].as_str().unwrap_or("");
                assert!(!description.contains('\n'));
            }
        }
        assert_eq!(read_only, READ_ONLY_TOOLS.len());
        assert_eq!(READ_ONLY_TOOLS.len(), 8);
    }

    /// Panics with the marker payload. The result type keeps the panic out of an async block.
    fn panic_with_marker() -> Option<Value> {
        panic!("marker-951")
    }

    fn tc951_catch_panic_reports_internal_and_continues() {
        let _guard = HOOK_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let panicking: ToolCall<'_> = Box::pin(async { panic_with_marker() });
        let first = block_on_ready(CatchPanic::new(panicking));
        let value = first.unwrap_or(Value::Null);
        assert_eq!(value["isError"], true);
        assert_eq!(text_of(&value)["code"], "E_INTERNAL");
        assert!(text_of(&value).get("error").is_none());
        assert!(!value.to_string().contains("marker-951"));
        let holder = std::rc::Rc::new(5u8);
        let rc_future: ToolCall<'_> = Box::pin(async move {
            let _kept = holder;
            None::<Value>
        });
        assert_eq!(block_on_ready(CatchPanic::new(rc_future)), None);
        let next = block_on_ready(CatchPanic::new(Box::pin(async { Some(json!({"ok": 1})) })));
        assert_eq!(next, Some(json!({"ok": 1})));
        std::panic::set_hook(previous);
    }

    fn tc952_panic_hook_writes_location_only() {
        assert_eq!(
            panic_report(Some(("src/x.rs", 12))),
            "mcp-serve panic at src/x.rs:12 E_INTERNAL"
        );
        assert_eq!(panic_report(None), "mcp-serve panic E_INTERNAL");
        let _guard = HOOK_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| {
            let location = info.location().map(|place| (place.file(), place.line()));
            if let Ok(mut reports) = REPORTS.lock() {
                reports.push(panic_report(location));
            }
        }));
        let caught = catch_unwind(|| -> () { panic!("marker-952") });
        std::panic::set_hook(previous);
        assert!(caught.is_err());
        let reports = REPORTS.lock().map(|r| r.clone()).unwrap_or_default();
        let last = reports.last().cloned().unwrap_or_default();
        assert!(last.starts_with("mcp-serve panic at "), "{last}");
        assert!(
            last.replace('\\', "/").contains("doctask_mcp.rs:"),
            "{last}"
        );
        assert!(last.ends_with(" E_INTERNAL"));
        assert!(!last.contains("marker-952"));
    }

    #[test]
    fn tc953_argument_rules() {
        let valid = [
            ("doc_file_find", json!({})),
            (
                "doc_task_list",
                json!({"status": ["open", "working"], "limit": 50, "cursor": "0"}),
            ),
            (
                "doc_clause_read",
                json!({"file_ref": "f1", "section": "ROOT", "max_chars": 200}),
            ),
            (
                "doc_task_create",
                json!({
                    "file_ref": "f1",
                    "section": "ROOT",
                    "title": "t".repeat(200),
                    "description": "d".repeat(4000),
                }),
            ),
            (
                "doc_annotation_add",
                json!({"file_ref": "f", "section": "ROOT", "text": "a\nb\tc"}),
            ),
            ("doc_schedule_get", Value::Null),
        ];
        for (tool, arguments) in &valid {
            assert!(validate_arguments(tool, arguments).is_ok(), "{tool}");
        }
        let kept = validate_arguments("doc_file_find", &json!({"limit": 5})).unwrap_or_default();
        assert_eq!(kept.len(), 1);
        let chinese = "中".repeat(200);
        assert!(validate_arguments(
            "doc_task_create",
            &json!({"file_ref": "f", "section": "ROOT", "title": chinese})
        )
        .is_ok());
        let invalid: Vec<(&str, Value)> = vec![
            ("doc_file_find", json!({"foo": 1})),
            ("doc_task_get", json!({})),
            ("doc_task_get", json!({"task": 7})),
            ("doc_task_list", json!({"proposed_only": "yes"})),
            (
                "doc_clause_read",
                json!({"file_ref": "f", "section": "ROOT", "max_chars": 2.0}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t".repeat(201)}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": ""}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t", "description": "d".repeat(4001)}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t", "rationale": "r".repeat(1001)}),
            ),
            ("doc_file_find", json!({"limit": 0})),
            ("doc_file_find", json!({"limit": 51})),
            (
                "doc_clause_read",
                json!({"file_ref": "f", "section": "ROOT", "max_chars": 199}),
            ),
            (
                "doc_clause_read",
                json!({"file_ref": "f", "section": "ROOT", "max_chars": 6001}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t", "priority": 0}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t", "priority": 4}),
            ),
            ("doc_task_list", json!({"limit": 0})),
            ("doc_task_list", json!({"status": ["done", "bogus"]})),
            ("doc_task_transition", json!({"task": "T-1", "to": "done"})),
            (
                "doc_evidence_add",
                json!({"task": "T-1", "kind": "approve", "summary": "s"}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "t", "depends_on": (0..11).map(|i| format!("T-{i}")).collect::<Vec<_>>()}),
            ),
            (
                "doc_task_list",
                json!({"status": ["open","analysis","approval","working","verification","done","cancelled","open"]}),
            ),
            (
                "doc_annotation_add",
                json!({"file_ref": "f", "section": "ROOT", "text": "a\u{7}b"}),
            ),
            (
                "doc_task_create",
                json!({"file_ref": "f", "section": "ROOT", "title": "a\u{0}"}),
            ),
            ("doc_file_find", json!([])),
            ("doc_file_find", json!("x")),
        ];
        for (tool, arguments) in &invalid {
            let refused = validate_arguments(tool, arguments);
            assert_eq!(
                refused.err().map(|e| e.code),
                Some(ErrorCode::InvalidArg),
                "{tool} {arguments}"
            );
        }
        assert!(validate_arguments(
            "doc_task_create",
            &json!({"file_ref": "f", "section": "ROOT", "title": "中".repeat(201)})
        )
        .is_err());
    }

    #[test]
    fn tc954_envelope_shapes() {
        let ok = envelope_ok(json!({"a": 1}), "s", "n");
        assert_eq!(
            text_of(&ok),
            json!({"ok": true, "data": {"a": 1}, "summary_zh": "s", "next_zh": "n"})
        );
        assert!(ok.get("isError").is_none() && ok.get("error").is_none());
        let busy = envelope_refused(ErrorCode::StoreBusy, "m", "h");
        assert_eq!(
            text_of(&busy),
            json!({"ok": false, "code": "E_STORE_BUSY", "message": "m", "hint": "h"})
        );
        assert!(busy.get("isError").is_none() && busy.get("error").is_none());
        assert!(text_of(&busy).get("error").is_none());
        for code in [
            ErrorCode::InvalidArg,
            ErrorCode::SchemaOutdated,
            ErrorCode::NotFound,
        ] {
            let refused = envelope_refused(code, "m", "h");
            assert!(refused.get("isError").is_none());
            assert_eq!(text_of(&refused)["code"], code.as_str());
        }
        let internal = internal_failure();
        assert_eq!(internal["isError"], true);
        assert_eq!(text_of(&internal)["code"], "E_INTERNAL");
        assert!(!ok.to_string().contains('\n'));
    }

    #[test]
    fn tc955_sanitize_and_fit_list() {
        let mut value = json!({"text": "a\u{1}b\nc\td"});
        sanitize(&mut value);
        assert_eq!(value["text"], "ab\nc\td");
        let items: Vec<Value> = (0..3).map(|_| json!("x".repeat(98))).collect();
        let (kept, cut) = fit_list(items.clone(), 250);
        assert_eq!((kept.len(), cut), (2, true));
        let (kept, cut) = fit_list(items, 300);
        assert_eq!((kept.len(), cut), (3, false));
        let (kept, cut) = fit_list(Vec::new(), 300);
        assert_eq!((kept.len(), cut), (0, false));
        let (kept, cut) = fit_list(vec![json!("y".repeat(498))], 300);
        assert_eq!((kept.len(), cut), (0, true));
    }

    #[test]
    fn tc956_cross_field_rules_and_transition_plan() {
        assert_eq!(
            link_target(None, None, None).err().map(|e| e.code),
            Some(ErrorCode::InvalidArg)
        );
        assert_eq!(
            link_target(Some("f"), None, Some("u"))
                .err()
                .map(|e| e.code),
            Some(ErrorCode::InvalidArg)
        );
        assert_eq!(
            link_target(None, Some("abc"), None).ok(),
            Some(("commit", "abc"))
        );
        assert!(evidence_summary(&"s".repeat(990), Some(&"r".repeat(20))).is_err());
        let joined = evidence_summary(&"s".repeat(990), Some(&"r".repeat(5)));
        assert!(joined
            .map(|text| text.ends_with("参考: rrrrr"))
            .unwrap_or(false));
        assert!(check_transition_reason("verification", &"x".repeat(1000)).is_ok());
        assert!(check_transition_reason("working", &"x".repeat(1000)).is_err());
        assert_eq!(
            transition_plan("open", "analysis", false).ok(),
            Some(Plan::StartAnalysis)
        );
        assert_eq!(
            transition_plan("working", "analysis", true).ok(),
            Some(Plan::Rework)
        );
        assert_eq!(
            transition_plan("working", "analysis", false)
                .err()
                .map(|e| e.code),
            Some(ErrorCode::InvalidArg)
        );
        assert_eq!(
            transition_plan("working", "verification", true).ok(),
            Some(Plan::ReportWork)
        );
        assert_eq!(
            transition_plan("verification", "working", true).ok(),
            Some(Plan::ReturnToWork)
        );
        for (status, to) in [
            ("open", "working"),
            ("analysis", "verification"),
            ("approval", "analysis"),
            ("done", "open"),
        ] {
            let denied = transition_plan(status, to, true).err();
            assert_eq!(denied.map(|e| e.code), Some(ErrorCode::TransitionDenied));
        }
        let hint = transition_plan("open", "working", true)
            .err()
            .map(|e| e.hint_zh)
            .unwrap_or_default();
        assert!(hint.contains("analysis"));
    }

    #[test]
    fn tc957_serve_argument_checks() {
        assert!(valid_launch_id("20261125-101010-ab"));
        assert!(!valid_launch_id(""));
        assert!(valid_launch_id(&"a".repeat(64)));
        assert!(!valid_launch_id(&"a".repeat(65)));
        assert!(!valid_launch_id("a b"));
        assert!(!valid_launch_id("a/b"));
        assert!(!valid_launch_id("é"));
        let dir = tempfile::tempdir().expect("temporary directory");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let missing = runtime.block_on(serve(dir.path(), "20261125-101010-ab", None));
        assert_eq!(missing.ok(), Some(1));
        let invalid = runtime.block_on(serve(dir.path(), "bad id", None));
        assert_eq!(invalid.ok(), Some(1));
    }
}
