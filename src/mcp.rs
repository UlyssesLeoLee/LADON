//! MCP protocol layer for `ladon mcp-serve`: JSON-RPC 2.0 over newline-delimited streams,
//! the MCP lifecycle methods, protocol version negotiation, the `ToolHost` seam and bounded
//! line reading. The module performs no I/O other than through the reader and writer it is
//! given, and it writes nothing to stdout or stderr.

#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

use std::io::{self, BufRead, Write};

use serde_json::{json, Map, Value};

/// Protocol versions echoed by `initialize`, newest first.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Version returned for every requested version that is not in `PROTOCOL_VERSIONS`.
pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Largest accepted line in bytes, excluding its newline.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

const OVERSIZED_MESSAGE: &str = "Request line exceeds 4 MiB";

/// Identity of the server, reported in the `serverInfo` member of the `initialize` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// The tools the server offers. The protocol layer neither inspects the descriptors nor
/// validates the arguments; both belong to the host.
pub trait ToolHost {
    /// Descriptors returned verbatim as the `tools` array of `tools/list`.
    fn tools(&self) -> Vec<Value>;

    /// Runs the named tool. Returns `None` when no listed tool has that name; otherwise the
    /// complete `CallToolResult` object, returned verbatim as the `result` member.
    fn call(&mut self, name: &str, arguments: Value) -> Option<Value>;
}

/// A classified message from the client.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request, which must be answered. `id` is a string, a number or `null`.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// A message without an `id` member. It is never answered.
    Notification { method: String, params: Value },
    /// A client reply to a request that the server never sent. It is never answered.
    Response,
}

/// A message that must be answered with an error reply.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    pub id: Value,
    pub code: i64,
    pub message: &'static str,
}

/// Outcome of reading one line with `read_bounded_line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRead {
    Line,
    Oversized,
    Eof,
}

/// The protocol server. It answers each request with `host`.
pub struct Server<H: ToolHost> {
    info: ServerInfo,
    host: H,
}

/// Returns the requested version when it is listed in `PROTOCOL_VERSIONS`, and
/// `LATEST_PROTOCOL_VERSION` otherwise.
pub fn negotiate_version(requested: &str) -> &'static str {
    PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|version| *version == requested)
        .unwrap_or(LATEST_PROTOCOL_VERSION)
}

/// Reads one line into `buf` (newline excluded) and reports whether it fits in `max` bytes.
pub fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max: usize,
    buf: &mut Vec<u8>,
) -> io::Result<LineRead> {
    let mut oversized = false;
    let mut saw_bytes = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(if oversized {
                LineRead::Oversized
            } else if saw_bytes {
                LineRead::Line
            } else {
                LineRead::Eof
            });
        }
        saw_bytes = true;
        let newline = available.iter().position(|&byte| byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if !oversized {
            if buf.len() + take > max {
                oversized = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&available[..take]);
            }
        }
        let consumed = if newline.is_some() { take + 1 } else { take };
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if oversized {
                LineRead::Oversized
            } else {
                LineRead::Line
            });
        }
    }
}

/// Classifies one line of client input.
pub fn parse_line(line: &[u8]) -> Result<Incoming, Rejection> {
    let text = std::str::from_utf8(line).map_err(|_| parse_error())?;
    let value: Value = serde_json::from_str(text).map_err(|_| parse_error())?;
    let Value::Object(object) = value else {
        return Err(invalid_request(Value::Null));
    };
    let id = echo_id(object.get("id"));
    let method = match object.get("method") {
        Some(Value::String(method)) => method.clone(),
        Some(_) => return Err(invalid_request(id)),
        None => {
            if object.contains_key("id")
                && (object.contains_key("result") || object.contains_key("error"))
            {
                return Ok(Incoming::Response);
            }
            return Err(invalid_request(id));
        }
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    if !object.contains_key("id") {
        return Ok(Incoming::Notification { method, params });
    }
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid_request(id));
    }
    if !object.get("id").is_some_and(is_valid_id) {
        return Err(invalid_request(id));
    }
    Ok(Incoming::Request { id, method, params })
}

fn parse_error() -> Rejection {
    Rejection {
        id: Value::Null,
        code: PARSE_ERROR,
        message: "Parse error",
    }
}

fn invalid_request(id: Value) -> Rejection {
    Rejection {
        id,
        code: INVALID_REQUEST,
        message: "Invalid Request",
    }
}

fn is_valid_id(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Null)
}

/// The identifier to echo in a rejection: the member when it is a valid id, `null` otherwise.
fn echo_id(value: Option<&Value>) -> Value {
    match value {
        Some(id) if is_valid_id(id) => id.clone(),
        _ => Value::Null,
    }
}

/// The error reply for a line longer than `MAX_LINE_BYTES`.
pub fn oversized_line_reply() -> Value {
    error_reply(Value::Null, INVALID_REQUEST, OVERSIZED_MESSAGE)
}

fn result_reply(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_reply<W: Write>(output: &mut W, reply: &Value) -> io::Result<()> {
    let mut text = serde_json::to_string(reply)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    text.push('\n');
    output.write_all(text.as_bytes())?;
    output.flush()
}

impl<H: ToolHost> Server<H> {
    pub fn new(info: ServerInfo, host: H) -> Self {
        Self { info, host }
    }

    /// Returns the reply for one line, or `None` when the line gets no reply.
    pub fn handle_line(&mut self, line: &[u8]) -> Option<Value> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        match parse_line(line) {
            Err(rejection) => Some(error_reply(rejection.id, rejection.code, rejection.message)),
            Ok(Incoming::Notification { .. } | Incoming::Response) => None,
            Ok(Incoming::Request { id, method, params }) => {
                Some(self.dispatch(id, &method, &params))
            }
        }
    }

    /// Reads lines from `input` and writes one reply per answered line to `output`, until
    /// `input` ends.
    pub fn serve<R: BufRead, W: Write>(&mut self, mut input: R, mut output: W) -> io::Result<()> {
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let reply = match read_bounded_line(&mut input, MAX_LINE_BYTES, &mut buf)? {
                LineRead::Eof => return Ok(()),
                LineRead::Oversized => Some(oversized_line_reply()),
                LineRead::Line => self.handle_line(&buf),
            };
            if let Some(reply) = reply {
                write_reply(&mut output, &reply)?;
            }
        }
    }

    fn dispatch(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => self.initialize(id, params),
            "ping" => result_reply(id, json!({})),
            "tools/list" => result_reply(id, json!({ "tools": self.host.tools() })),
            "tools/call" => self.call_tool(id, params),
            _ => error_reply(id, METHOD_NOT_FOUND, "Method not found"),
        }
    }

    fn initialize(&self, id: Value, params: &Value) -> Value {
        match params.get("protocolVersion") {
            None | Some(Value::Null) => {
                error_reply(id, INVALID_PARAMS, "protocolVersion is required")
            }
            Some(Value::String(requested)) => result_reply(
                id,
                json!({
                    "protocolVersion": negotiate_version(requested),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": self.info.name, "version": self.info.version },
                }),
            ),
            Some(_) => error_reply(id, INVALID_PARAMS, "protocolVersion must be a string"),
        }
    }

    fn call_tool(&mut self, id: Value, params: &Value) -> Value {
        let name = match params.get("name") {
            None | Some(Value::Null) => {
                return error_reply(id, INVALID_PARAMS, "name is required");
            }
            Some(Value::String(name)) => name,
            Some(_) => return error_reply(id, INVALID_PARAMS, "name must be a string"),
        };
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(arguments) => arguments.clone(),
        };
        match self.host.call(name, arguments) {
            Some(result) => result_reply(id, result),
            None => error_reply(id, INVALID_PARAMS, "Unknown tool"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor, Read};

    struct RecordingHost {
        tools: Vec<Value>,
        reply: Option<Value>,
        calls: Vec<(String, Value)>,
    }

    impl RecordingHost {
        fn new(reply: Option<Value>) -> Self {
            Self {
                tools: vec![json!({"name": "probe_echo", "inputSchema": {"type": "object"}})],
                reply,
                calls: Vec::new(),
            }
        }
    }

    impl ToolHost for RecordingHost {
        fn tools(&self) -> Vec<Value> {
            self.tools.clone()
        }

        fn call(&mut self, name: &str, arguments: Value) -> Option<Value> {
            self.calls.push((name.to_owned(), arguments));
            if name == "probe_echo" {
                self.reply.clone()
            } else {
                None
            }
        }
    }

    fn server(reply: Option<Value>) -> Server<RecordingHost> {
        Server::new(
            ServerInfo {
                name: "ladon",
                version: "9.9.9",
            },
            RecordingHost::new(reply),
        )
    }

    fn reply_of(server: &mut Server<RecordingHost>, line: &str) -> Option<Value> {
        server.handle_line(line.as_bytes())
    }

    fn error_parts(reply: &Value) -> (i64, String) {
        let error = reply.get("error").expect("an error reply");
        (
            error["code"].as_i64().expect("an integer code"),
            error["message"]
                .as_str()
                .expect("a string message")
                .to_owned(),
        )
    }

    /// Runs `serve` over `input` and returns each output line as a parsed JSON object.
    fn run_serve(server: &mut Server<RecordingHost>, input: &[u8]) -> Vec<Value> {
        let mut output = Vec::new();
        server
            .serve(Cursor::new(input), &mut output)
            .expect("serve succeeds");
        let text = String::from_utf8(output).expect("UTF-8 output");
        assert!(text.is_empty() || text.ends_with('\n'));
        text.split_terminator('\n')
            .map(|line| {
                let value: Value =
                    serde_json::from_str(line).expect("each output line is one JSON object");
                assert!(value.is_object());
                value
            })
            .collect()
    }

    /// Reads every line of `input` with `read_bounded_line` until `Eof`.
    fn read_all(input: &[u8], max: usize) -> Vec<(LineRead, Vec<u8>)> {
        let mut reader = Cursor::new(input);
        let mut out = Vec::new();
        loop {
            let mut buf = Vec::new();
            let kind = read_bounded_line(&mut reader, max, &mut buf).expect("read succeeds");
            out.push((kind, buf));
            if kind == LineRead::Eof {
                return out;
            }
        }
    }

    /// A reader that reports `Interrupted` once before it delivers its data.
    struct Flaky {
        interrupted: bool,
        data: Cursor<Vec<u8>>,
    }

    impl Read for Flaky {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            self.data.read(out)
        }
    }

    fn classify_all(text: &str) -> Vec<Incoming> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                parse_line(line.as_bytes()).unwrap_or_else(|rejection| {
                    panic!("fixture line rejected: {line}: {rejection:?}")
                })
            })
            .collect()
    }

    const HERMES_MCP_TEST: &str =
        include_str!("../tests/fixtures/mcp_client/hermes_mcp_test.jsonl");
    const SDK_SESSION: &str = include_str!("../tests/fixtures/mcp_client/sdk_session.jsonl");
    const LADON_REPLIES: &str = include_str!("../tests/fixtures/mcp_client/ladon_replies.jsonl");
    const PROBE_REPLIES: &str = include_str!("../tests/fixtures/mcp_client/probe_replies.jsonl");

    #[test]
    fn ut_155_handshake_version_negotiation() {
        tc940_supported_versions_echoed();
        tc941_unknown_or_missing_version();
        tc942_initialize_result_shape();
    }

    #[test]
    fn ut_156_notifications_ping_bad_lines() {
        tc943_notifications_and_client_replies_silent();
        tc944_ping_list_and_call_routing();
        tc945_unknown_method_echoes_id();
        tc946_bad_json_and_invalid_requests();
        tc947_invalid_utf8_line();
        tc948_bounded_reading_and_eof();
        tc949_fixture_lines_parse_and_replay();
    }

    fn tc940_supported_versions_echoed() {
        let mut server = server(None);
        for version in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
            assert_eq!(negotiate_version(version), version);
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}"}}}}"#
            );
            let reply = reply_of(&mut server, &line).expect("initialize is answered");
            assert_eq!(reply["result"]["protocolVersion"], version);
            assert_eq!(reply["id"], 1);
        }
    }

    fn tc941_unknown_or_missing_version() {
        assert_eq!(negotiate_version("2026-07-28"), LATEST_PROTOCOL_VERSION);
        let mut server = server(None);
        for version in ["2026-07-28", "", "2025-11-26", "2025-06-18 ", "2024-11-05 "] {
            let line = json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialize",
                "params": {"protocolVersion": version},
            })
            .to_string();
            let reply = reply_of(&mut server, &line).expect("initialize is answered");
            assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
        }
        for line in [
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":null}}"#,
        ] {
            let reply = reply_of(&mut server, line).expect("initialize is answered");
            assert_eq!(reply["id"], 3);
            assert_eq!(
                error_parts(&reply),
                (INVALID_PARAMS, "protocolVersion is required".to_owned())
            );
        }
        let reply = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":5}}"#,
        )
        .expect("initialize is answered");
        assert_eq!(reply["id"], 4);
        assert_eq!(
            error_parts(&reply),
            (
                INVALID_PARAMS,
                "protocolVersion must be a string".to_owned()
            )
        );
    }

    fn tc942_initialize_result_shape() {
        let mut server = server(None);
        let reply = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"mcp","version":"0.1.0"}}}"#,
        )
        .expect("initialize is answered");
        assert_eq!(
            reply["result"],
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "ladon", "version": "9.9.9"},
            })
        );
    }

    fn tc943_notifications_and_client_replies_silent() {
        let mut server = server(None);
        for line in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/unknown"}"#,
            r#"{"method":"notifications/initialized","jsonrpc":"1.0"}"#,
            "",
            "   \r",
            r#"{"jsonrpc":"2.0","id":7,"result":{}}"#,
        ] {
            assert_eq!(reply_of(&mut server, line), None, "line: {line:?}");
        }
        assert!(server.host.calls.is_empty());
    }

    fn tc944_ping_list_and_call_routing() {
        let result = json!({"content": [{"type": "text", "text": "x"}], "isError": false});
        let mut server = server(Some(result.clone()));

        let ping = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":4,"method":"ping","params":{"_meta":{}}}"#,
        )
        .expect("ping is answered");
        assert_eq!(ping, json!({"jsonrpc": "2.0", "id": 4, "result": {}}));
        let ping = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":"p-1","method":"ping"}"#,
        )
        .expect("ping is answered");
        assert_eq!(ping, json!({"jsonrpc": "2.0", "id": "p-1", "result": {}}));

        let list = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .expect("tools/list is answered");
        assert_eq!(
            list,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {"tools": [{"name": "probe_echo", "inputSchema": {"type": "object"}}]},
            })
        );

        let call = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"probe_echo","arguments":{}}}"#,
        )
        .expect("tools/call is answered");
        assert_eq!(call, json!({"jsonrpc": "2.0", "id": 3, "result": result}));
        reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"probe_echo"}}"#,
        )
        .expect("tools/call is answered");
        reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"probe_echo","arguments":null}}"#,
        )
        .expect("tools/call is answered");
        reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"probe_echo","arguments":[1]}}"#,
        )
        .expect("tools/call is answered");
        let recorded = &server.host.calls;
        assert_eq!(recorded.len(), 4);
        assert_eq!(recorded[0], ("probe_echo".to_owned(), json!({})));
        assert_eq!(recorded[1], ("probe_echo".to_owned(), json!({})));
        assert_eq!(recorded[2], ("probe_echo".to_owned(), json!({})));
        assert_eq!(recorded[3], ("probe_echo".to_owned(), json!([1])));

        let missing = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"missing_tool"}}"#,
        )
        .expect("tools/call is answered");
        assert_eq!(missing["id"], 7);
        assert_eq!(
            error_parts(&missing),
            (INVALID_PARAMS, "Unknown tool".to_owned())
        );
        let no_params = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call"}"#,
        )
        .expect("tools/call is answered");
        assert_eq!(
            error_parts(&no_params),
            (INVALID_PARAMS, "name is required".to_owned())
        );
        let bad_name = reply_of(
            &mut server,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":3}}"#,
        )
        .expect("tools/call is answered");
        assert_eq!(
            error_parts(&bad_name),
            (INVALID_PARAMS, "name must be a string".to_owned())
        );
    }

    fn tc945_unknown_method_echoes_id() {
        let mut server = server(None);
        let cases = [
            (
                r#"{"jsonrpc":"2.0","id":9,"method":"resources/list"}"#,
                json!(9),
            ),
            (
                r#"{"jsonrpc":"2.0","id":"req-10","method":"prompts/list"}"#,
                json!("req-10"),
            ),
            (
                r#"{"jsonrpc":"2.0","id":null,"method":"resources/list"}"#,
                Value::Null,
            ),
            (
                r#"{"jsonrpc":"2.0","id":0,"method":"resources/list"}"#,
                json!(0),
            ),
            (
                r#"{"jsonrpc":"2.0","id":"","method":"resources/list"}"#,
                json!(""),
            ),
        ];
        for (line, id) in cases {
            let reply = reply_of(&mut server, line).expect("request is answered");
            assert_eq!(reply["id"], id, "line: {line}");
            assert_eq!(
                error_parts(&reply),
                (METHOD_NOT_FOUND, "Method not found".to_owned())
            );
        }
    }

    fn tc946_bad_json_and_invalid_requests() {
        let mut server = server(None);
        let nested = format!("{}{}", "[".repeat(200), "]".repeat(200));
        let cases: Vec<(String, i64, Value)> = vec![
            (
                r#"{"jsonrpc":"2.0","id":1,"#.to_owned(),
                PARSE_ERROR,
                Value::Null,
            ),
            ("not json".to_owned(), PARSE_ERROR, Value::Null),
            (nested, PARSE_ERROR, Value::Null),
            ("[]".to_owned(), INVALID_REQUEST, Value::Null),
            ("42".to_owned(), INVALID_REQUEST, Value::Null),
            (
                r#"{"jsonrpc":"2.0","id":5}"#.to_owned(),
                INVALID_REQUEST,
                json!(5),
            ),
            (
                r#"{"jsonrpc":"2.0","id":6,"method":7}"#.to_owned(),
                INVALID_REQUEST,
                json!(6),
            ),
            (
                r#"{"id":8,"method":"ping"}"#.to_owned(),
                INVALID_REQUEST,
                json!(8),
            ),
            (
                r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#.to_owned(),
                INVALID_REQUEST,
                Value::Null,
            ),
            (
                r#"{"jsonrpc":"2.0","id":[1],"method":"ping"}"#.to_owned(),
                INVALID_REQUEST,
                Value::Null,
            ),
        ];
        for (line, code, id) in cases {
            let reply =
                reply_of(&mut server, &line).unwrap_or_else(|| panic!("no reply for {line}"));
            assert_eq!(reply["id"], id, "line: {line}");
            assert_eq!(error_parts(&reply).0, code, "line: {line}");
        }

        let replies = run_serve(
            &mut server,
            b"not json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n[]\n",
        );
        assert_eq!(replies.len(), 3);
        assert_eq!(error_parts(&replies[0]).0, PARSE_ERROR);
        assert_eq!(replies[1], json!({"jsonrpc": "2.0", "id": 1, "result": {}}));
        assert_eq!(error_parts(&replies[2]).0, INVALID_REQUEST);
        assert_eq!(replies[2]["id"], Value::Null);
    }

    fn tc947_invalid_utf8_line() {
        let mut server = server(None);
        let bad_method: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\"}";
        let bad_pair: &[u8] = b"\xc3\x28";
        for line in [bad_method, bad_pair] {
            let reply = server.handle_line(line).expect("invalid UTF-8 is answered");
            assert_eq!(reply["id"], Value::Null);
            assert_eq!(error_parts(&reply), (PARSE_ERROR, "Parse error".to_owned()));
        }

        let mut input = b"\xff\n".to_vec();
        input.extend_from_slice(
            "{\"jsonrpc\":\"2.0\",\"id\":\"中文\",\"method\":\"ping\"}\n".as_bytes(),
        );
        let mut output = Vec::new();
        server
            .serve(Cursor::new(input), &mut output)
            .expect("serve succeeds");
        let text = String::from_utf8(output).expect("UTF-8 output");
        let lines: Vec<&str> = text.split_terminator('\n').collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).expect("JSON"),
            json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})
        );
        assert_eq!(
            serde_json::from_str::<Value>(lines[1]).expect("JSON"),
            json!({"jsonrpc": "2.0", "id": "中文", "result": {}})
        );
        assert!(
            lines[1].contains("中文"),
            "non-ASCII text is written as UTF-8"
        );
    }

    fn tc948_bounded_reading_and_eof() {
        assert_eq!(
            read_all(b"12345678\n", 8),
            vec![
                (LineRead::Line, b"12345678".to_vec()),
                (LineRead::Eof, Vec::new())
            ]
        );
        assert_eq!(
            read_all(b"123456789\nok\n", 8),
            vec![
                (LineRead::Oversized, Vec::new()),
                (LineRead::Line, b"ok".to_vec()),
                (LineRead::Eof, Vec::new()),
            ]
        );
        assert_eq!(
            read_all(b"abc", 8),
            vec![
                (LineRead::Line, b"abc".to_vec()),
                (LineRead::Eof, Vec::new())
            ]
        );
        assert_eq!(read_all(b"", 8), vec![(LineRead::Eof, Vec::new())]);
        assert_eq!(
            read_all(b"123456789", 8),
            vec![
                (LineRead::Oversized, Vec::new()),
                (LineRead::Eof, Vec::new())
            ]
        );
        assert_eq!(
            read_all(b"\n", 8),
            vec![(LineRead::Line, Vec::new()), (LineRead::Eof, Vec::new())]
        );

        let mut reader = BufReader::new(Flaky {
            interrupted: false,
            data: Cursor::new(b"ok\n".to_vec()),
        });
        let mut buf = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, 8, &mut buf).expect("read succeeds"),
            LineRead::Line
        );
        assert_eq!(buf, b"ok".to_vec());

        let mut server = server(None);
        let prefix = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"pad":""#;
        let suffix = r#""}}"#;
        let pad = MAX_LINE_BYTES - prefix.len() - suffix.len();
        let exact = format!("{prefix}{}{suffix}", "a".repeat(pad));
        assert_eq!(exact.len(), MAX_LINE_BYTES);
        let oversized = "a".repeat(MAX_LINE_BYTES + 1);

        let mut input = Vec::new();
        input.extend_from_slice(exact.as_bytes());
        input.push(b'\n');
        input.extend_from_slice(oversized.as_bytes());
        input.push(b'\n');
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        let replies = run_serve(&mut server, &input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], json!({"jsonrpc": "2.0", "id": 1, "result": {}}));
        assert_eq!(replies[1]["id"], Value::Null);
        assert_eq!(
            error_parts(&replies[1]),
            (INVALID_REQUEST, OVERSIZED_MESSAGE.to_owned())
        );
        assert_eq!(replies[2], json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
        assert_eq!(oversized_line_reply(), replies[1]);

        let mut output = Vec::new();
        server
            .serve(Cursor::new(Vec::new()), &mut output)
            .expect("serve returns Ok at end of input");
        assert!(output.is_empty());
    }

    fn tc949_fixture_lines_parse_and_replay() {
        let hermes = classify_all(HERMES_MCP_TEST);
        assert_eq!(hermes.len(), 3);
        assert!(
            matches!(&hermes[0], Incoming::Request { method, .. } if method.as_str() == "initialize")
        );
        assert!(
            matches!(&hermes[1], Incoming::Notification { method, .. } if method.as_str() == "notifications/initialized")
        );
        assert!(
            matches!(&hermes[2], Incoming::Request { method, .. } if method.as_str() == "tools/list")
        );

        let sdk = classify_all(SDK_SESSION);
        assert_eq!(sdk.len(), 5);
        assert!(
            matches!(&sdk[1], Incoming::Notification { method, .. } if method.as_str() == "notifications/initialized")
        );
        assert!(
            matches!(&sdk[3], Incoming::Request { method, .. } if method.as_str() == "tools/call")
        );
        assert!(matches!(&sdk[4], Incoming::Request { method, .. } if method.as_str() == "ping"));

        let ladon = classify_all(LADON_REPLIES);
        assert_eq!(ladon.len(), 14);
        assert!(ladon.iter().all(|line| matches!(line, Incoming::Response)));
        let probe = classify_all(PROBE_REPLIES);
        assert_eq!(probe.len(), 4);
        assert!(probe.iter().all(|line| matches!(line, Incoming::Response)));

        let probe_lines: Vec<Value> = PROBE_REPLIES
            .lines()
            .map(|line| serde_json::from_str(line).expect("JSON"))
            .collect();
        let tools = probe_lines[1]["result"]["tools"]
            .as_array()
            .expect("tools array")
            .clone();
        let echo_result = probe_lines[2]["result"].clone();
        let new_host = || RecordingHost {
            tools: tools.clone(),
            reply: Some(echo_result.clone()),
            calls: Vec::new(),
        };

        let mut hermes_server = Server::new(
            ServerInfo {
                name: "ladon",
                version: "0.0.3",
            },
            new_host(),
        );
        let hermes_replies = run_serve(&mut hermes_server, HERMES_MCP_TEST.as_bytes());
        assert_eq!(hermes_replies.len(), 2);
        assert_eq!(hermes_replies[0]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(hermes_replies[1]["result"], json!({"tools": tools}));

        let mut sdk_server = Server::new(
            ServerInfo {
                name: "ladon",
                version: "0.0.3",
            },
            new_host(),
        );
        let sdk_replies = run_serve(&mut sdk_server, SDK_SESSION.as_bytes());
        assert_eq!(sdk_replies.len(), 4);
        let ids: Vec<Value> = sdk_replies
            .iter()
            .map(|reply| reply["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!(1), json!(2), json!(3), json!(4)]);
        assert_eq!(sdk_replies[0]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(sdk_replies[1]["result"], json!({"tools": tools}));
        assert_eq!(sdk_replies[2]["result"], echo_result);
        assert_eq!(sdk_replies[3]["result"], json!({}));
        assert_eq!(
            sdk_server.host.calls,
            vec![("probe_echo".to_owned(), json!({}))]
        );
    }
}
