//! Stub MCP client for the stdio tests of WP13. It starts the built `ladon mcp-serve` with the
//! environment reduced to a Windows baseline, writes newline-delimited JSON-RPC and reads one
//! reply line per request. It forwards what the tests send and prints nothing.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Environment names the child keeps. Every other variable is removed before the start.
const BASELINE_ENV: [&str; 4] = ["SystemRoot", "Path", "TEMP", "TMP"];

/// A running `ladon mcp-serve` process and its pipes.
pub struct Stub {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Stub {
    /// Starts the server for one workspace and launch id, with an optional global rule file.
    pub fn spawn(root: &Path, launch: &str, global_ignore: Option<&Path>) -> Stub {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ladon"));
        command.env_clear();
        for name in BASELINE_ENV {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .arg("--root")
            .arg(root)
            .arg("mcp-serve")
            .arg("--launch")
            .arg(launch);
        if let Some(path) = global_ignore {
            command.arg("--global-ignore").arg(path);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ladon mcp-serve starts");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped standard output"));
        Stub {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    /// Sends one request and returns its reply object.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.reply()
    }

    /// Sends one notification. No reply follows.
    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Sends one line exactly as given, followed by a newline.
    pub fn send_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("standard input is open");
        stdin.write_all(line.as_bytes()).expect("write line");
        stdin.write_all(b"\n").expect("write newline");
        stdin.flush().expect("flush");
    }

    fn send(&mut self, message: &Value) {
        let text = serde_json::to_string(message).expect("message serializes");
        self.send_line(&text);
    }

    /// Reads one reply line and parses it as one JSON object.
    pub fn reply(&mut self) -> Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read reply");
        assert!(read > 0, "server closed standard output");
        serde_json::from_str(line.trim_end()).expect("reply is one JSON object")
    }

    /// Calls a tool and returns the `result` member of its reply.
    pub fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        reply["result"].clone()
    }

    /// Ends standard input and waits up to two seconds. Returns the status when the process
    /// exited in time, and its standard error.
    pub fn close(mut self) -> (Option<ExitStatus>, String) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match self.child.try_wait().expect("wait for the server") {
                Some(status) => break Some(status),
                None if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    break None;
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        (status, stderr)
    }
}

/// The envelope JSON carried by the text of a tool result.
pub fn envelope(result: &Value) -> Value {
    let text = result["content"][0]["text"].as_str().unwrap_or("{}");
    serde_json::from_str(text).expect("the result text is an envelope")
}
