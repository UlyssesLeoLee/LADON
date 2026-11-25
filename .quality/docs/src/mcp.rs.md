# Technical Design: `src/mcp.rs`

Source: `src/mcp.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`mcp` is the protocol layer of the `ladon mcp-serve` server. It reads newline-delimited JSON-RPC 2.0 messages from a buffered reader, answers the MCP lifecycle methods (`initialize`, the notifications, `ping`, `tools/list`, `tools/call`), delegates tool discovery and tool execution to a `ToolHost`, and writes one compact JSON object per reply to a writer, flushing after each reply. The module knows no tool name, opens no database, reads no environment variable and does not read the clock. The tool table, argument validation against the tool schemas, the caller identity, the external-transfer gate, the panic guard and the stderr log belong to `doctask_mcp` (WP13). The module writes only through the writer it is given and does not log.

## Dependencies

- `serde_json` (`Value`, `Map`, `json!`, `to_string`, `from_slice`) and `std::io`. The module adds no crate and imports no `crate::` module.
- Module-level `#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]`. The attribute takes effect only when clippy runs; the stdout discipline is also checked by the reply-line tests of this document.
- Callers: `doctask_mcp` (WP13) implements `ToolHost` and calls `Server::serve`. The module is declared in `lib.rs` as `pub mod mcp`.

## Public Contract

### Constants

| Name | Value | Meaning |
|---|---|---|
| `PROTOCOL_VERSIONS` | `["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]` | Versions echoed by `initialize`, in this order. |
| `LATEST_PROTOCOL_VERSION` | `"2025-11-25"` | Reply to every other requested version. |
| `MAX_LINE_BYTES` | `4 * 1024 * 1024` | Largest accepted line, excluding its newline. |
| `PARSE_ERROR` | `-32700` | Bad JSON or invalid UTF-8. |
| `INVALID_REQUEST` | `-32600` | Well-formed JSON that is not a valid request, and an oversized line. |
| `METHOD_NOT_FOUND` | `-32601` | Unknown method on a request. |
| `INVALID_PARAMS` | `-32602` | Bad `params` for a known method, and an unknown tool name. |

### Types

- `ServerInfo { pub name: &'static str, pub version: &'static str }`, derives `Debug, Clone, Copy, PartialEq, Eq`. Supplies the `serverInfo` member of the `initialize` result. `doctask_mcp` passes `ladon` and `env!("CARGO_PKG_VERSION")`.
- `trait ToolHost`:
  - `fn tools(&self) -> Vec<Value>` returns the descriptors that become the `tools` array of `tools/list`, verbatim. The protocol layer does not inspect them and adds no cursor.
  - `fn call(&mut self, name: &str, arguments: Value) -> Option<Value>` returns `None` when `name` is not a listed tool; the protocol then replies `-32602` with `Unknown tool`. Otherwise it returns the complete `CallToolResult` object, which becomes the `result` member verbatim. `arguments` is the `arguments` member of `params`; it is an empty object when the member is absent or `null`, and any other JSON value is passed unchanged. Argument validation therefore belongs to the host.
- `enum Incoming`:
  - `Request { id: Value, method: String, params: Value }`: `id` is a string, a number or `null`; `params` is `Value::Null` when absent.
  - `Notification { method: String, params: Value }`: a message with a string `method` and no `id` member.
  - `Response`: an object with an `id` member and a `result` or `error` member and no `method`. The server never issues requests, so a client reply is recognised and ignored.
- `struct Rejection { pub id: Value, pub code: i64, pub message: &'static str }`: a message that must be answered with an error reply.
- `enum LineRead { Line, Oversized, Eof }`.
- `struct Server<H: ToolHost>` holds a `ServerInfo` and the host.

### Functions

- `negotiate_version(requested: &str) -> &'static str` returns the element of `PROTOCOL_VERSIONS` equal to `requested`, or `LATEST_PROTOCOL_VERSION` when there is none. The comparison is exact.
- `parse_line(line: &[u8]) -> Result<Incoming, Rejection>` classifies one line, in the order of the parsing rules below.
- `read_bounded_line<R: BufRead>(reader: &mut R, max: usize, buf: &mut Vec<u8>) -> io::Result<LineRead>` reads one line.
- `oversized_line_reply() -> Value` returns the error reply for an oversized line.
- `Server::new(info: ServerInfo, host: H) -> Self`.
- `Server::handle_line(&mut self, line: &[u8]) -> Option<Value>` returns the reply for one line, or `None` when no line is written.
- `Server::serve<R: BufRead, W: Write>(&mut self, input: R, output: W) -> io::Result<()>` runs the loop until end of input. Only an I/O error ends it early; protocol errors are replies and do not stop the loop.

## Line Reading

`read_bounded_line` appends the bytes of the next line to `buf`, without the terminating `\n`, and consumes the newline. The caller clears `buf` before each call.

- A line of at most `max` bytes is returned as `Line`. A line of exactly `max` bytes is accepted.
- A longer line returns `Oversized`. `buf` is left empty. The rest of that line is consumed and discarded up to its newline or end of input, so the next call starts at the following line. Memory use stays at `max` bytes plus one reader buffer.
- At end of input, an unterminated last line with bytes returns `Line`, an unterminated oversized line returns `Oversized`, and nothing left returns `Eof`.
- `Interrupted` errors from the reader are retried; other errors are returned.

`serve` calls the function with `MAX_LINE_BYTES`.

## Message Classification (`parse_line`)

The checks run in this order and the first match decides:

1. The bytes are not valid UTF-8: `Rejection { id: null, -32700, "Parse error" }`.
2. `serde_json` cannot parse the text, including nesting beyond its recursion limit: `Rejection { id: null, -32700, "Parse error" }`.
3. The top-level value is not an object: `Rejection { id: null, -32600, "Invalid Request" }`. Batch arrays are not accepted.
4. The `method` member is absent. If an `id` member and a `result` or `error` member are present, the result is `Response`. Otherwise `Rejection { id: echo, -32600, "Invalid Request" }`.
5. The `method` member is not a string: `Rejection { id: echo, -32600, "Invalid Request" }`.
6. The `id` member is absent: `Notification`. The `jsonrpc` member is not examined for notifications.
7. The `jsonrpc` member is not the string `"2.0"`: `Rejection { id: echo, -32600, "Invalid Request" }`.
8. The `id` member is not a string, a number or `null`: `Rejection { id: null, -32600, "Invalid Request" }`.
9. Otherwise `Request`.

`echo` is the `id` member when it is a string, a number or `null`, and `null` otherwise. Rejections therefore echo the identifier whenever it can be read, as JSON-RPC requires for a detectable id.

## Reply Handling (`handle_line`)

1. A line made only of ASCII whitespace, including an empty line, is ignored and produces no reply.
2. `Rejection` produces `error_reply(id, code, message)`.
3. `Notification` and `Response` produce no reply.
4. `Request` is dispatched as below.

### Dispatch

- `initialize`: a missing or `null` `protocolVersion` member gives `-32602` `protocolVersion is required`. A non-string value gives `-32602` `protocolVersion must be a string`. A string gives the result `{"protocolVersion": negotiate_version(v), "capabilities": {"tools": {}}, "serverInfo": {"name": <name>, "version": <version>}}`. The capabilities object holds `tools` only.
- `ping`: the result is `{}`, whatever `params` holds.
- `tools/list`: the result is `{"tools": host.tools()}`. There is no `nextCursor` member.
- `tools/call`: a missing or `null` `name` gives `-32602` `name is required`; a non-string value gives `-32602` `name must be a string`. Otherwise `host.call(name, arguments)` is run. `Some(result)` becomes the result; `None` gives `-32602` `Unknown tool`.
- Any other method gives `-32601` `Method not found`.

There is no lifecycle gate: requests are answered in any order, and a repeated `initialize` is answered again. The `id` of every reply is the request `id` value, so a number stays a number and a string stays a string.

### Reply shape

A result reply is `{"jsonrpc":"2.0","id":<id>,"result":<result>}`. An error reply is `{"jsonrpc":"2.0","id":<id>,"error":{"code":<code>,"message":"<message>"}}`. `serde_json::to_string` produces the text, which is written followed by one `\n` and then flushed. Strings are escaped by `serde_json`, so a reply never contains a raw newline, and non-ASCII text is written as UTF-8.

## Serve Loop

`serve` repeats the following until `read_bounded_line` returns `Eof`, which returns `Ok(())`:

- `Oversized`: writes `oversized_line_reply()`, which is `Rejection { id: null, -32600, "Request line exceeds 4 MiB" }` in the reply shape.
- `Line`: writes the reply of `handle_line(buf)` when there is one.

A write or flush error returns `Err`. Nothing else is written, logged or printed.

## Error Handling

- Errors that concern one line are replies; the loop continues after each of them.
- The module never panics on input. JSON nesting, invalid UTF-8, oversized input and bad types are handled by the checks above.
- The host may panic; the module does not catch it. The guard belongs to `doctask_mcp` (WP13).

## Regression Scope

`doctask_mcp` depends on these exact values: the `ToolHost` method signatures, the `ServerInfo` fields, the supported version list and fallback, the capabilities object `{"tools":{}}`, the reply shapes, the messages `Unknown tool`, `protocolVersion is required`, `Method not found` and `Parse error`, the 4 MiB limit, and the split of duties in `tools/call`: the protocol layer checks only `name` and rejects unknown names, while the host validates `arguments`. Changing any of them must fail TC-940, TC-942, TC-944, TC-946 or TC-949 before a caller is changed.

## Change Notes

`mcp` is declared in `lib.rs` as a public module. It has no dependency on the document-task modules, so `doctask_mcp` is the only consumer of `ToolHost` and `Server`. The fixtures in `tests/fixtures/mcp_client` are the golden bytes recorded for the Hermes client; the module parses them without change, and they are read by `include_str!` in the unit tests.
