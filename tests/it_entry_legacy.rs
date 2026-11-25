//! Characterization of the legacy `ladon` commands (TC-1020 to TC-1024).
//!
//! Each test runs the built binary in a temporary workspace and compares a transcript of
//! every command (status, stdout and stderr) with the recorded expectation. The transcripts
//! are normalised: the workspace path becomes `<WS>`, identifiers become `<UUID>` or
//! `<SHA256>`, and timestamps become `<TS>`.
use regex::Regex;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Variables that select the Google sign-in folder or client; removed from every child process.
const GOOGLE_ENV: [&str; 3] = ["LADON_GOOGLE_CLIENT_ID", "LADON_GOOGLE_CLIENT_SECRET", "LADON_SETTINGS_DIR"];

/// Collects the transcript of the commands run against one workspace.
struct Transcript {
    root: PathBuf,
    text: String,
}

impl Transcript {
    fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), text: String::new() }
    }

    /// Runs `ladon --root <workspace> <args>` and returns the raw stdout.
    fn run(&mut self, args: &[&str]) -> String {
        self.run_with_env(args, &[], &[])
    }

    /// Runs the command with extra variables and with `remove` variables unset.
    fn run_with_env(&mut self, args: &[&str], set: &[(&str, &str)], remove: &[&str]) -> String {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ladon"));
        command.current_dir(&self.root).arg("--root").arg(&self.root).args(args);
        for name in GOOGLE_ENV.iter().chain(remove.iter()) {
            command.env_remove(name);
        }
        for (name, value) in set {
            command.env(name, value);
        }
        let output = command.output().expect("ladon binary runs");
        let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
        let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
        let status = output.status.code().map_or("signal".to_string(), |code| code.to_string());
        self.record(&format!("ladon {}", args.join(" ")), &status, &stdout, &stderr);
        stdout
    }

    fn record(&mut self, label: &str, status: &str, stdout: &str, stderr: &str) {
        let label = normalise(label, &self.root);
        let stdout = normalise(stdout, &self.root);
        let stderr = normalise(stderr, &self.root);
        self.text.push_str(&format!("$ {label}\nstatus: {status}\nstdout:\n{}stderr:\n{}", section(&stdout), section(&stderr)));
    }

    /// Starts `watch`, records its start line and stops it.
    fn watch_start(&mut self) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ladon"))
            .current_dir(&self.root)
            .arg("--root")
            .arg(&self.root)
            .arg("watch")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("ladon binary runs");
        let mut start_line = String::new();
        BufReader::new(child.stdout.take().expect("stdout piped")).read_line(&mut start_line).expect("start line");
        let _ = child.kill();
        child.wait().expect("child waits");
        let mut stderr = String::new();
        child.stderr.take().expect("stderr piped").read_to_string(&mut stderr).expect("stderr reads");
        self.record("ladon watch (stopped after the start line)", "stopped by the test", &start_line, &stderr);
    }
}

fn section(text: &str) -> String {
    if text.is_empty() {
        "(empty)\n".to_string()
    } else if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

/// Replaces the workspace path, identifiers and timestamps with placeholders.
fn normalise(text: &str, root: &Path) -> String {
    let raw = root.to_string_lossy().into_owned();
    let canonical = fs::canonicalize(root).map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|_| raw.clone());
    let bare = canonical.trim_start_matches(r"\\?\").to_string();
    let mut forms = Vec::new();
    for base in [raw, canonical, bare] {
        forms.push(base.replace('\\', "\\\\"));
        forms.push(base.replace('\\', "/"));
        forms.push(base);
    }
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    let mut text = text.replace("\r\n", "\n");
    for form in forms {
        text = text.replace(&form, "<WS>");
    }
    // clap prints the executable name with its platform suffix; JSON escapes a separator as two characters.
    let text = text.replace("ladon.exe", "ladon").replace("\\\\", "/").replace('\\', "/");
    let uuid = Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").expect("pattern");
    let hash = Regex::new(r"\b[0-9a-f]{64}\b").expect("pattern");
    let stamp = Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?").expect("pattern");
    let text = uuid.replace_all(&text, "<UUID>");
    let text = hash.replace_all(&text, "<SHA256>");
    stamp.replace_all(&text, "<TS>").into_owned()
}

/// Returns the id of the version with the given number from a `history` JSON document.
fn version_id(history: &str, number: i64) -> String {
    let value: Value = serde_json::from_str(history).expect("history is JSON");
    value["versions"]
        .as_array()
        .expect("versions array")
        .iter()
        .find(|version| version["number"] == number)
        .and_then(|version| version["id"].as_str())
        .expect("version id")
        .to_string()
}

/// Compares a transcript with its recorded expectation.
fn check(name: &str, actual: &str, expected: &str) {
    assert_eq!(actual, expected.replace("\r\n", "\n"), "transcript {name}");
}

/// Writes the fixture documents of a workspace.
fn write_documents(root: &Path) {
    fs::create_dir_all(root.join("docs")).expect("docs folder");
    fs::write(root.join("docs/req.md"), "# REQ-001\nApproved\n").expect("requirement");
    fs::write(root.join("notes.txt"), "meeting notes\n").expect("notes");
}

const LIFECYCLE: &str = r##"$ ladon init
status: 0
stdout:
Initialized: <WS>
stderr:
(empty)
$ ladon scan
status: 0
stdout:
scanned 2, captured 2 version(s)
stderr:
(empty)
$ ladon capture docs/req.md --actor tester --reason manual check
status: 0
stdout:
{
  "file_id": "<UUID>",
  "version_id": "<UUID>",
  "event_id": "<UUID>",
  "status": "captured",
  "sensitive": false,
  "changed_sections": [
    "H1:REQ-001#1"
  ]
}
stderr:
(empty)
$ ladon files
status: 0
stdout:
[
  {
    "id": "<UUID>",
    "relative_path": "docs/req.md",
    "current_hash": "<SHA256>",
    "version_no": 2,
    "sensitive": false,
    "deleted": false
  },
  {
    "id": "<UUID>",
    "relative_path": "notes.txt",
    "current_hash": "<SHA256>",
    "version_no": 1,
    "sensitive": false,
    "deleted": false
  }
]
stderr:
(empty)
$ ladon history docs/req.md
status: 0
stdout:
{
  "chain_valid": true,
  "events": [
    {
      "actor": "human",
      "current_hash": "<SHA256>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "kind": "created",
      "ledger_hash": "<SHA256>",
      "ledger_prev_hash": "",
      "new_path": "docs/req.md",
      "old_path": "docs/req.md",
      "previous_hash": "",
      "reason": "scan",
      "timestamp": "<TS>",
      "version_id": "<UUID>"
    },
    {
      "actor": "tester",
      "current_hash": "<SHA256>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "kind": "modified",
      "ledger_hash": "<SHA256>",
      "ledger_prev_hash": "<SHA256>",
      "new_path": "docs/req.md",
      "old_path": "docs/req.md",
      "previous_hash": "<SHA256>",
      "reason": "manual check",
      "timestamp": "<TS>",
      "version_id": "<UUID>"
    }
  ],
  "file": {
    "current_hash": "<SHA256>",
    "deleted": false,
    "id": "<UUID>",
    "relative_path": "docs/req.md",
    "sensitive": false,
    "version_no": 2
  },
  "versions": [
    {
      "actor": "tester",
      "byte_size": 32,
      "changed_sections": [
        "H1:REQ-001#1"
      ],
      "content_hash": "<SHA256>",
      "created_at": "<TS>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "number": 2,
      "reason": "manual check",
      "sensitive": false
    },
    {
      "actor": "human",
      "byte_size": 19,
      "changed_sections": [
        "H1:REQ-001#1"
      ],
      "content_hash": "<SHA256>",
      "created_at": "<TS>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "number": 1,
      "reason": "scan",
      "sensitive": false
    }
  ]
}
stderr:
(empty)
$ ladon verify docs/req.md
status: 0
stdout:
event chain VALID
stderr:
(empty)
$ ladon diff <UUID> <UUID>
status: 0
stdout:
 # REQ-001
-Approved
+Approved with changes
stderr:
(empty)
"##;
const QUERIES: &str = r##"$ ladon init
status: 0
stdout:
Initialized: <WS>
stderr:
(empty)
$ ladon scan
status: 0
stdout:
scanned 2, captured 2 version(s)
stderr:
(empty)
$ ladon ignore-check docs/req.md
status: 0
stdout:
ALLOWED: subject to explicit archive action
stderr:
(empty)
$ ladon grep REQ --limit 10
status: 0
stdout:
[
  {
    "path": "docs/req.md",
    "line": 1,
    "snippet": "# REQ-001"
  },
  {
    "path": "graph.nema",
    "line": 2,
    "snippet": "REQ Requirement"
  },
  {
    "path": "graph.nema",
    "line": 5,
    "snippet": "REQ>E1>DES Traces"
  },
  {
    "path": "graph.nema",
    "line": 6,
    "snippet": "REQ>E2>TEST Traces"
  }
]
stderr:
(empty)
$ ladon tag-folder --recursive --description project notes
status: 0
stdout:
tagged 2 file(s)
stderr:
(empty)
$ ladon tags docs/req.md
status: 0
stdout:
[
  "rule-described"
]
stderr:
(empty)
$ ladon annotate docs/req.md --section ROOT reviewed by tester
status: 0
stdout:
annotation id: <UUID>
stderr:
(empty)
$ ladon task docs/req.md --section ROOT Write summary --description due Friday
status: 0
stdout:
task id: <UUID>
stderr:
(empty)
$ ladon history docs/req.md
status: 0
stdout:
{
  "chain_valid": true,
  "events": [
    {
      "actor": "human",
      "current_hash": "<SHA256>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "kind": "created",
      "ledger_hash": "<SHA256>",
      "ledger_prev_hash": "",
      "new_path": "docs/req.md",
      "old_path": "docs/req.md",
      "previous_hash": "",
      "reason": "scan",
      "timestamp": "<TS>",
      "version_id": "<UUID>"
    }
  ],
  "file": {
    "current_hash": "<SHA256>",
    "deleted": false,
    "id": "<UUID>",
    "relative_path": "docs/req.md",
    "sensitive": false,
    "version_no": 1
  },
  "versions": [
    {
      "actor": "human",
      "byte_size": 19,
      "changed_sections": [
        "H1:REQ-001#1"
      ],
      "content_hash": "<SHA256>",
      "created_at": "<TS>",
      "file_id": "<UUID>",
      "id": "<UUID>",
      "number": 1,
      "reason": "scan",
      "sensitive": false
    }
  ]
}
stderr:
(empty)
$ ladon plan <UUID>
status: 0
stdout:
RESTORE PLAN: <UUID> version 1 (<SHA256>) -> docs/req.md
Target exists: true
Action: require --approve, create new version and preserve current history.
Historical bytes: read from verified local CAS/cloud archive.
stderr:
(empty)
$ ladon parallel graph.nema
status: 0
stdout:
[
  [
    "REQ"
  ],
  [
    "DES",
    "TEST"
  ]
]
stderr:
(empty)
"##;
const AGENT_AND_TAGS: &str = r##"$ ladon init
status: 0
stdout:
Initialized: <WS>
stderr:
(empty)
$ ladon scan
status: 0
stdout:
scanned 2, captured 2 version(s)
stderr:
(empty)
$ ladon agent-capabilities
status: 0
stdout:
[
  {
    "name": "folders.list",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.read",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.grep",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.ref.resolve",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.history",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.diff",
    "status": "cli-implemented",
    "requires_approval": false
  },
  {
    "name": "files.tags.list",
    "status": "cli-implemented",
    "requires_approval": false
  },
  {
    "name": "files.tags.batch",
    "status": "cli-implemented",
    "requires_approval": true
  },
  {
    "name": "files.ignore.check",
    "status": "implemented",
    "requires_approval": false
  },
  {
    "name": "files.ignore.add",
    "status": "cli-implemented",
    "requires_approval": true
  },
  {
    "name": "docs.annotations",
    "status": "cli-implemented",
    "requires_approval": true
  },
  {
    "name": "docs.tasks",
    "status": "cli-implemented",
    "requires_approval": true
  },
  {
    "name": "files.restore.plan",
    "status": "cli-implemented",
    "requires_approval": false
  },
  {
    "name": "files.restore.commit",
    "status": "cli-implemented",
    "requires_approval": true
  },
  {
    "name": "events.subscribe",
    "status": "design-only",
    "requires_approval": false
  },
  {
    "name": "files.write.plan",
    "status": "design-only",
    "requires_approval": false
  },
  {
    "name": "files.write.commit",
    "status": "design-only",
    "requires_approval": true
  },
  {
    "name": "filespaces.allocate",
    "status": "design-only",
    "requires_approval": true
  },
  {
    "name": "worktrees.archive",
    "status": "design-only",
    "requires_approval": true
  },
  {
    "name": "storage.migrate",
    "status": "design-only",
    "requires_approval": true
  },
  {
    "name": "chat.send",
    "status": "design-only",
    "requires_approval": true
  }
]
stderr:
(empty)
$ ladon agent-list docs --limit 5
status: 0
stdout:
[
  {
    "name": "req.md",
    "kind": "file",
    "bytes": 19,
    "relative_path": "docs/req.md",
    "tracked_file_ref": "<UUID>"
  }
]
stderr:
(empty)
$ ladon sensitive-add notes.txt
status: 0
stdout:
Updated sensitivity rules: <WS>/.LADONignore
stderr:
(empty)
$ ladon ignore-check notes.txt
status: 0
stdout:
EXCLUDED: local only
stderr:
(empty)
$ ladon file-space
status: 0
stdout:
{
  "id": "<UUID>",
  "scope": [
    {
      "id": "<UUID>",
      "path": "docs/req.md",
      "version": 1,
      "content_hash": "<SHA256>"
    },
    {
      "id": "<UUID>",
      "path": "notes.txt",
      "version": 1,
      "content_hash": "<SHA256>"
    }
  ]
}
stderr:
(empty)
$ ladon tag-folder
status: 0
stdout:
tagged 0 file(s)
stderr:
(empty)
"##;
const ERRORS: &str = r##"$ ladon files
status: 1
stdout:
(empty)
stderr:
Error: workspace not initialized; run `ladon init`
$ ladon tag-folder
status: 1
stdout:
(empty)
stderr:
Error: workspace not initialized; run `ladon init`
$ ladon init
status: 0
stdout:
Initialized: <WS>
stderr:
(empty)
$ ladon scan
status: 0
stdout:
scanned 2, captured 2 version(s)
stderr:
(empty)
$ ladon history missing.md
status: 1
stdout:
(empty)
stderr:
Error: file not found in ledger
$ ladon verify missing.md
status: 1
stdout:
(empty)
stderr:
Error: file not found in ledger
$ ladon annotate missing.md --section ROOT note
status: 1
stdout:
(empty)
stderr:
Error: file not found in ledger
$ ladon diff unknown-version other-unknown
status: 1
stdout:
(empty)
stderr:
Error: unknown version
$ ladon restore unknown-version
status: 1
stdout:
(empty)
stderr:
Error: restore requires explicit --approve
$ ladon restore unknown-version --approve
status: 1
stdout:
(empty)
stderr:
Error: unknown version
$ ladon archive unknown-version
status: 1
stdout:
(empty)
stderr:
Error: --destination required for local/nas storage
$ ladon archive unknown-version --provider unknown-provider
status: 1
stdout:
(empty)
stderr:
Error: unknown provider: unknown-provider
$ ladon frobnicate
status: 2
stdout:
(empty)
stderr:
error: unrecognized subcommand 'frobnicate'

Usage: ladon [OPTIONS] <COMMAND>

For more information, try '--help'.
"##;
const GOOGLE_AND_WATCH: &str = r##"$ ladon init
status: 0
stdout:
Initialized: <WS>
stderr:
(empty)
$ ladon google-login
status: 1
stdout:
(empty)
stderr:
Error: LADON_GOOGLE_CLIENT_ID is not set
$ ladon google-login
status: 1
stdout:
(empty)
stderr:
Error: no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR
$ ladon google-logout
status: 0
stdout:
Google Drive sign-in removed
stderr:
(empty)
$ ladon google-logout
status: 1
stdout:
(empty)
stderr:
Error: no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR
$ ladon watch (stopped after the start line)
status: stopped by the test
stdout:
watching <WS> — Ctrl+C stops
stderr:
(empty)
"##;

#[test]
fn legacy_workspace_lifecycle() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let mut t = Transcript::new(root);
    t.run(&["init"]);
    write_documents(root);
    t.run(&["scan"]);
    fs::write(root.join("docs/req.md"), "# REQ-001\nApproved with changes\n").expect("edit");
    t.run(&["capture", "docs/req.md", "--actor", "tester", "--reason", "manual check"]);
    t.run(&["files"]);
    let history = t.run(&["history", "docs/req.md"]);
    t.run(&["verify", "docs/req.md"]);
    let first = version_id(&history, 1);
    let second = version_id(&history, 2);
    t.run(&["diff", first.as_str(), second.as_str()]);
    check("legacy_workspace_lifecycle", &t.text, LIFECYCLE);
}

#[test]
fn legacy_queries_and_records() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let mut t = Transcript::new(root);
    t.run(&["init"]);
    write_documents(root);
    t.run(&["scan"]);
    let graph = root.join("graph.nema");
    fs::write(&graph, " \\>9 1\nREQ Requirement\nDES Design\nTEST Test\nREQ>E1>DES Traces\nREQ>E2>TEST Traces").expect("graph");
    t.run(&["ignore-check", "docs/req.md"]);
    t.run(&["grep", "REQ", "--limit", "10"]);
    t.run(&["tag-folder", "--recursive", "--description", "project notes"]);
    t.run(&["tags", "docs/req.md"]);
    t.run(&["annotate", "docs/req.md", "--section", "ROOT", "reviewed by tester"]);
    t.run(&["task", "docs/req.md", "--section", "ROOT", "Write summary", "--description", "due Friday"]);
    let history = t.run(&["history", "docs/req.md"]);
    let first = version_id(&history, 1);
    t.run(&["plan", first.as_str()]);
    t.run(&["parallel", "graph.nema"]);
    check("legacy_queries_and_records", &t.text, QUERIES);
}

#[test]
fn legacy_agent_sensitivity_and_tagging() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let mut t = Transcript::new(root);
    t.run(&["init"]);
    write_documents(root);
    t.run(&["scan"]);
    t.run(&["agent-capabilities"]);
    t.run(&["agent-list", "docs", "--limit", "5"]);
    t.run(&["sensitive-add", "notes.txt"]);
    t.run(&["ignore-check", "notes.txt"]);
    t.run(&["file-space"]);
    t.run(&["tag-folder"]);
    check("legacy_agent_sensitivity_and_tagging", &t.text, AGENT_AND_TAGS);
}

#[test]
fn legacy_error_paths() {
    let empty = tempfile::tempdir().expect("empty folder");
    let mut before_init = Transcript::new(empty.path());
    before_init.run(&["files"]);
    before_init.run(&["tag-folder"]);

    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let mut t = Transcript::new(root);
    t.run(&["init"]);
    write_documents(root);
    t.run(&["scan"]);
    t.run(&["history", "missing.md"]);
    t.run(&["verify", "missing.md"]);
    t.run(&["annotate", "missing.md", "--section", "ROOT", "note"]);
    t.run(&["diff", "unknown-version", "other-unknown"]);
    t.run(&["restore", "unknown-version"]);
    t.run(&["restore", "unknown-version", "--approve"]);
    t.run(&["archive", "unknown-version"]);
    t.run(&["archive", "unknown-version", "--provider", "unknown-provider"]);
    t.run(&["frobnicate"]);
    let combined = format!("{}{}", before_init.text, t.text);
    check("legacy_error_paths", &combined, ERRORS);
}

#[test]
fn legacy_google_offline_and_watch_start() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let settings = tempfile::tempdir().expect("settings folder");
    let settings_text = settings.path().to_string_lossy().into_owned();
    let mut t = Transcript::new(root);
    t.run(&["init"]);
    t.run_with_env(&["google-login"], &[("LADON_SETTINGS_DIR", settings_text.as_str())], &[]);
    t.run_with_env(&["google-login"], &[], &["LOCALAPPDATA"]);
    t.run_with_env(&["google-logout"], &[("LADON_SETTINGS_DIR", settings_text.as_str())], &[]);
    t.run_with_env(&["google-logout"], &[], &["LOCALAPPDATA"]);
    t.watch_start();
    check("legacy_google_offline_and_watch_start", &t.text, GOOGLE_AND_WATCH);
}
