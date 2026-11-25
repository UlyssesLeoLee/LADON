//! Stdio tests of `ladon mcp-serve` (WP13): the handshake, the list and the golden client session
//! (IT-062), business refusals and argument errors as ordinary results (IT-063), the read-only
//! invariant and the disclosure rows (IT-072), and the global rule file inside the MCP process
//! (IT-074). The stub client of `tests/support/mcp_stub.rs` drives the built binary.
#[path = "support/mcp_stub.rs"]
mod mcp_stub;

use clap::Parser;
use ladon_explorer::{
    doctask_cli::{self, DocArgs, DocIo},
    engine::Engine,
    entry::{Cli, Command},
};
use mcp_stub::{envelope, Stub};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
};

/// The launch identifier used by every test workspace.
const LAUNCH: &str = "20261125-101010-ab";

/// A console stand-in: the console state is fixed and the answers come from a queue.
#[derive(Default)]
struct ScriptedIo {
    console: bool,
    answers: VecDeque<String>,
}

impl DocIo for ScriptedIo {
    fn stdin_is_console(&self) -> bool {
        self.console
    }

    fn stdout_is_console(&self) -> bool {
        self.console
    }

    fn read_line(&mut self) -> Option<String> {
        self.answers.pop_front()
    }

    fn out(&mut self, _text: &str) {}

    fn err(&mut self, _text: &str) {}

    fn wait_refresh(&mut self) -> bool {
        false
    }
}

/// Parses `ladon doc <args>` into the doc arguments.
fn doc_args(args: &[&str]) -> DocArgs {
    let mut argv = vec!["ladon", "doc"];
    argv.extend_from_slice(args);
    match Cli::try_parse_from(argv).expect("doc command parses").cmd {
        Command::Doc(doc) => doc,
        _ => panic!("not a doc command"),
    }
}

/// Runs one `doc` command in process; `console` answers its confirmations.
async fn doc(engine: &Engine, args: &[&str], console: &[&str]) -> i32 {
    let mut io = ScriptedIo {
        console: !console.is_empty(),
        answers: console.iter().map(|answer| answer.to_string()).collect(),
    };
    doctask_cli::run_doc(engine, &doc_args(args), &mut io)
        .await
        .expect("the command returns an exit code")
}

/// A workspace with `a.md` captured, the schema migrated and `a.md` registered as the baseline.
async fn baselined_workspace() -> (tempfile::TempDir, Engine) {
    let tmp = tempfile::tempdir().expect("temp dir");
    let engine = Engine::init(tmp.path()).await.expect("workspace");
    fs::write(
        tmp.path().join("a.md"),
        "# 1 Scope\nThe scope text.\n\n# 2 Rules\nThe rule text.\n",
    )
    .expect("write a.md");
    engine
        .capture(&tmp.path().join("a.md"), "human", "test")
        .await
        .expect("capture");
    assert_eq!(doc(&engine, &["migrate"], &[]).await, 0);
    assert_eq!(doc(&engine, &["baseline", "add", "a.md"], &["y"]).await, 0);
    (tmp, engine)
}

/// The file reference of `a.md`, found through the MCP tool.
fn file_ref_of(stub: &mut Stub) -> String {
    let found = envelope(&stub.call("doc_file_find", json!({"query": "a.md"})));
    found["data"]["files"][0]["file_ref"]
        .as_str()
        .expect("a.md is listed")
        .to_string()
}

/// A raw connection to the workspace database. It is dropped before any child process runs.
async fn connect(root: &Path) -> turso::Connection {
    let path: PathBuf = root.join(".ladon").join("meta.turso");
    let db = turso::Builder::new_local(&path.to_string_lossy())
        .build()
        .await
        .expect("database opens");
    db.connect().expect("connection")
}

async fn count(conn: &turso::Connection, sql: &str) -> i64 {
    let mut rows = conn.query(sql, ()).await.expect("query");
    rows.next()
        .await
        .expect("row")
        .expect("one row")
        .get::<i64>(0)
        .expect("integer count")
}

/// The row counts that the read-only invariant compares.
#[derive(Debug, PartialEq, Eq)]
struct Counts {
    events: i64,
    anchors: i64,
    meta: i64,
    disclosures: i64,
}

async fn snapshot(root: &Path) -> Counts {
    let conn = connect(root).await;
    Counts {
        events: count(&conn, "SELECT COUNT(*) FROM doc_task_events").await,
        anchors: count(&conn, "SELECT COUNT(*) FROM doc_anchors").await,
        meta: count(&conn, "SELECT COUNT(*) FROM doc_task_meta").await,
        disclosures: count(&conn, "SELECT COUNT(*) FROM doc_agent_disclosures").await,
    }
}

/// Registers the session row that the console writes before it starts Hermes.
async fn register_session(root: &Path) {
    let conn = connect(root).await;
    let sql = format!(
        "INSERT INTO doc_agent_sessions (launch_id, hermes_slug, hermes_exe, started_by, started_at, policy_fp) \
         VALUES ('{LAUNCH}', 'test', 'test', 'test', '2026-11-25T00:00:00Z', '')"
    );
    conn.execute(&sql, ()).await.expect("session row");
}

async fn initialized(stub: &mut Stub) {
    let init = stub.request("initialize", json!({"protocolVersion": "2025-11-25"}));
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test]
async fn it_062_handshake_list_ping_and_eof() {
    let (tmp, _engine) = baselined_workspace().await;
    let mut stub = Stub::spawn(tmp.path(), LAUNCH, None);
    let init = stub.request(
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "mcp", "version": "0.1.0"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(init["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "ladon");
    stub.notify("notifications/initialized", json!({}));
    let list = stub.request("tools/list", json!({}));
    let tools = list["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(tools.len(), 13);
    assert!(tools.iter().all(|tool| tool.get("outputSchema").is_none()));
    assert_eq!(stub.request("ping", json!({}))["result"], json!({}));
    assert_eq!(
        stub.request("resources/list", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        stub.request(
            "tools/call",
            json!({"name": "doc_unknown", "arguments": {}})
        )["error"]["code"],
        -32602
    );
    let found = envelope(&stub.call("doc_file_find", json!({"query": "中文"})));
    assert_eq!(found["ok"], true);
    assert_eq!(found["data"]["files"], json!([]));
    let (status, _) = stub.close();
    assert!(
        status.is_some_and(|status| status.success()),
        "the server exits within two seconds of end of input"
    );
}

#[tokio::test]
async fn it_062_golden_client_session_replays() {
    let (tmp, _engine) = baselined_workspace().await;
    let mut stub = Stub::spawn(tmp.path(), LAUNCH, None);
    let golden = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_client/sdk_session.jsonl"),
    )
    .expect("golden session is readable");
    for line in golden.lines().filter(|line| !line.trim().is_empty()) {
        let message: Value = serde_json::from_str(line).expect("golden line is JSON");
        stub.send_line(line);
        if let Some(id) = message.get("id") {
            let reply = stub.reply();
            assert_eq!(&reply["id"], id);
            assert!(reply.get("result").is_some() || reply.get("error").is_some());
        }
    }
    let (status, _) = stub.close();
    assert!(status.is_some_and(|status| status.success()));
}

#[tokio::test]
async fn it_063_refusals_are_ordinary_results() {
    let (tmp, _engine) = baselined_workspace().await;
    let mut stub = Stub::spawn(tmp.path(), LAUNCH, None);
    initialized(&mut stub).await;
    let file_ref = file_ref_of(&mut stub);
    let bad_type = stub.call("doc_task_get", json!({"task": 7}));
    assert!(bad_type.get("isError").is_none());
    assert_eq!(envelope(&bad_type)["code"], "E_INVALID_ARG");
    let unknown_property =
        envelope(&stub.call("doc_task_get", json!({"task": "T-1", "extra": true})));
    assert_eq!(unknown_property["code"], "E_INVALID_ARG");
    let missing = envelope(&stub.call("doc_task_get", json!({"task": "T-99"})));
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["code"], "E_NOT_FOUND");
    let created = envelope(&stub.call(
        "doc_task_create",
        json!({"file_ref": file_ref, "section": "1", "title": "Scope review"}),
    ));
    assert_eq!(created["ok"], true);
    assert_eq!(created["data"]["existing"], false);
    let again = envelope(&stub.call(
        "doc_task_create",
        json!({"file_ref": file_ref, "section": "1", "title": "Scope review"}),
    ));
    assert_eq!(again["data"]["existing"], true);
    let version = envelope(&stub.call(
        "doc_clause_list",
        json!({"file_ref": file_ref, "version": 99}),
    ));
    assert_eq!(version["code"], "E_NOT_FOUND");
    let denied = envelope(&stub.call(
        "doc_task_transition",
        json!({"task": created["data"]["handle"], "to": "verification", "reason": "done"}),
    ));
    assert_eq!(denied["code"], "E_TRANSITION_DENIED");
    let (status, _) = stub.close();
    assert!(status.is_some_and(|status| status.success()));
}

#[tokio::test]
async fn it_072_reads_leave_the_chain_and_write_disclosures() {
    let (tmp, _engine) = baselined_workspace().await;
    register_session(tmp.path()).await;
    let before = snapshot(tmp.path()).await;
    let mut stub = Stub::spawn(tmp.path(), LAUNCH, None);
    initialized(&mut stub).await;
    let file_ref = file_ref_of(&mut stub);
    let reads = [
        ("doc_file_find", json!({})),
        ("doc_task_list", json!({})),
        ("doc_schedule_get", json!({})),
        ("doc_clause_list", json!({"file_ref": file_ref})),
        (
            "doc_clause_read",
            json!({"file_ref": file_ref, "section": "1"}),
        ),
        ("doc_mentions_find", json!({"token": "scope"})),
    ];
    for (tool, arguments) in reads {
        assert_eq!(envelope(&stub.call(tool, arguments))["ok"], true, "{tool}");
    }
    let (status, stderr) = stub.close();
    assert!(status.is_some_and(|status| status.success()));
    assert!(!stderr.contains("panic"), "{stderr}");
    let after = snapshot(tmp.path()).await;
    assert_eq!(after.events, before.events);
    assert_eq!(after.anchors, before.anchors);
    assert_eq!(after.meta, before.meta);
    assert!(after.disclosures > before.disclosures);
    let conn = connect(tmp.path()).await;
    let ours = count(
        &conn,
        &format!("SELECT COUNT(*) FROM doc_agent_disclosures WHERE launch_id = '{LAUNCH}'"),
    )
    .await;
    assert_eq!(ours, after.disclosures - before.disclosures);
}

#[tokio::test]
async fn it_074_global_rule_file_applies_inside_the_mcp_process() {
    let (tmp, _engine) = baselined_workspace().await;
    let global = tmp.path().join("global.LADONignore");
    fs::write(&global, "a.md\n").expect("write the global rule file");
    let mut hidden = Stub::spawn(tmp.path(), LAUNCH, Some(global.as_path()));
    initialized(&mut hidden).await;
    let refused = envelope(&hidden.call("doc_file_find", json!({"query": "a.md"})));
    assert_eq!(refused["ok"], true);
    assert_eq!(refused["data"]["files"], json!([]));
    let (status, _) = hidden.close();
    assert!(status.is_some_and(|status| status.success()));

    let mut plain = Stub::spawn(tmp.path(), LAUNCH, None);
    initialized(&mut plain).await;
    let listed = envelope(&plain.call("doc_file_find", json!({"query": "a.md"})));
    assert_eq!(listed["data"]["files"][0]["path"], "a.md");
    let _ = plain.close();

    let missing = tmp.path().join("missing.LADONignore");
    let mut broken = Stub::spawn(tmp.path(), LAUNCH, Some(missing.as_path()));
    initialized(&mut broken).await;
    let failed = envelope(&broken.call("doc_file_find", json!({})));
    assert_eq!(failed["code"], "E_POLICY_DENIED");
    let _ = broken.close();
}
