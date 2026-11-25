//! Binary tests of the one-shot `doc` commands (WP11): the single JSON object on standard output,
//! the refusal of human-only commands on a pipe (IT-060) and the export read-back (IT-066).
//! Human-only steps run in process with a scripted console, because the built binary runs on
//! pipes and never has a console; the binary then runs the remaining commands.
use clap::Parser;
use ladon_explorer::{
    doctask_cli::{self, DocArgs, DocIo},
    engine::Engine,
    entry::{Cli, Command},
    nema,
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::Path,
    process::{Command as Process, Output, Stdio},
};

/// A console stand-in: the console state is fixed and answers come from a queue.
#[derive(Default)]
struct ScriptedIo {
    console: bool,
    answers: VecDeque<String>,
}

impl ScriptedIo {
    fn console(answers: &[&str]) -> ScriptedIo {
        ScriptedIo {
            console: true,
            answers: answers.iter().map(|a| a.to_string()).collect(),
        }
    }
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

/// Runs one `doc` command in process and returns its exit code.
async fn run(engine: &Engine, args: &[&str], io: &mut ScriptedIo) -> i32 {
    doctask_cli::run_doc(engine, &doc_args(args), io)
        .await
        .expect("the command returns an exit code")
}

/// Runs the built `ladon` binary against the workspace, with standard input closed.
fn ladon(root: &Path, args: &[&str]) -> Output {
    Process::new(env!("CARGO_BIN_EXE_ladon"))
        .arg("--root")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("ladon binary runs")
}

fn stdout_json(output: &Output) -> Value {
    let text = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
    serde_json::from_str(text.trim()).expect("stdout is one JSON object")
}

/// A workspace with `a.md` captured and registered as the baseline, in process.
async fn baselined() -> (tempfile::TempDir, Engine) {
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
    assert_eq!(
        run(&engine, &["migrate"], &mut ScriptedIo::default()).await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["baseline", "add", "a.md"],
            &mut ScriptedIo::console(&["y"])
        )
        .await,
        0
    );
    (tmp, engine)
}

#[tokio::test]
async fn it_060_binary_flow_json_and_pipe_refusal() {
    let (tmp, engine) = baselined().await;
    drop(engine);
    let root = tmp.path();

    let created = ladon(
        root,
        &["doc", "new", "a.md", "1", "Write summary", "--json"],
    );
    assert!(created.status.success(), "{created:?}");
    assert_eq!(stdout_json(&created)["ok"], Value::Bool(true));

    let listed = ladon(root, &["doc", "list", "--json"]);
    assert!(listed.status.success());
    assert_eq!(stdout_json(&listed)["ok"], Value::Bool(true));

    let refused = ladon(root, &["doc", "approve", "T-1", "--comment", "x", "--json"]);
    assert_eq!(refused.status.code(), Some(5));
    let refusal = stdout_json(&refused);
    assert_eq!(refusal["ok"], Value::Bool(false));
    assert_eq!(refusal["code"], Value::String("E_NOT_INTERACTIVE".into()));

    let blocked = ladon(
        root,
        &[
            "doc",
            "baseline",
            "add",
            "a.md",
            "--json",
            "--yes",
            "--expect-hash",
            "deadbeef",
        ],
    );
    assert_eq!(blocked.status.code(), Some(5), "no bypass on a pipe");

    let after = ladon(root, &["doc", "list", "--json"]);
    assert_eq!(
        after.stdout, listed.stdout,
        "the refused calls change no state"
    );
}

#[tokio::test]
async fn it_066_binary_export_and_read_back() {
    let (tmp, engine) = baselined().await;
    assert_eq!(
        run(
            &engine,
            &["new", "a.md", "1", "Write summary"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["status", "T-1", "analysis"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["plan", "new", "T-1", "--summary", "analysis only"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["approve", "T-1", "--comment", "ok"],
            &mut ScriptedIo::console(&["T-1"])
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &[
                "evidence",
                "add",
                "T-1",
                "--kind",
                "test_run",
                "--result",
                "pass",
                "--summary",
                "ok"
            ],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["status", "T-1", "verification", "--summary", "done"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["close", "T-1"],
            &mut ScriptedIo::console(&["T-1"])
        )
        .await,
        0
    );
    drop(engine);
    let root = tmp.path();
    let exports = root.join(".ladon").join("exports");

    let nema_run = ladon(root, &["doc", "export", "nema", "--all"]);
    assert!(nema_run.status.success(), "{nema_run:?}");
    let nema_file = fs::read_dir(&exports)
        .expect("exports folder")
        .map(|entry| entry.expect("entry").path())
        .find(|path| path.extension().is_some_and(|ext| ext == "nema"))
        .expect("a .nema file is written");
    let text = fs::read_to_string(&nema_file).expect("read .nema");
    let edges = nema::parse_graph_edges(&text).expect("the .NEMA text parses");
    let nodes: Vec<String> = edges
        .iter()
        .flat_map(|edge| [edge.from.clone(), edge.to.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    nema::topo_batches(&nodes, &edges).expect("the .NEMA graph layers");

    let report_path = exports.join("report-check.json");
    let report = ladon(
        root,
        &[
            "doc",
            "export",
            "report",
            "--format",
            "json",
            "--all",
            "--generated-at",
            "2026-11-25T09:00:00+09:00",
            "--out",
            report_path.to_str().expect("UTF-8 path"),
        ],
    );
    assert!(report.status.success(), "{report:?}");
    let body: Value = serde_json::from_str(&fs::read_to_string(&report_path).expect("read report"))
        .expect("json");

    let engine = Engine::open(root).await.expect("reopen workspace");
    let events = ladon_explorer::doctask_store::DocStore::new(engine.store.clone())
        .events()
        .await
        .expect("events");
    let cutoff = events
        .iter()
        .filter(|event| event.kind != "dt.export.generated")
        .map(|event| event.seq)
        .max()
        .expect("events exist");
    assert_eq!(body["cutoff_seq"].as_i64(), Some(cutoff));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "dt.export.generated"),
        "each export is recorded"
    );
    let done = body["tasks"]
        .as_array()
        .expect("tasks array")
        .iter()
        .any(|task| task["status"] == Value::String("done".into()));
    assert!(done, "the report shows the completed task");
}
