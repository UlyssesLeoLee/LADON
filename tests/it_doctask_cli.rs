//! In-process tests of the one-shot `doc` layer (WP11): the legacy `annotate` and `task` lines
//! (IT-061), the entry-point source guard (IT-938) and the one-shot flow from baseline to export
//! with a scripted console (IT-939). The console answers come from a queue, so the human-only
//! steps run through the same confirmation code as the terminal.
use clap::Parser;
use ladon_explorer::{
    doctask_cli::{self, DocArgs, DocIo},
    engine::Engine,
    entry::{Cli, Command},
};
use serde_json::Value;
use std::{collections::VecDeque, fs, path::Path};

/// A console stand-in: the console state is fixed and answers come from a queue.
#[derive(Default)]
struct ScriptedIo {
    console: bool,
    answers: VecDeque<String>,
    stdout_text: String,
    stderr_text: String,
}

impl ScriptedIo {
    fn console(answers: &[&str]) -> ScriptedIo {
        ScriptedIo {
            console: true,
            answers: answers.iter().map(|a| a.to_string()).collect(),
            ..ScriptedIo::default()
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

    fn out(&mut self, text: &str) {
        self.stdout_text.push_str(text);
    }

    fn err(&mut self, text: &str) {
        self.stderr_text.push_str(text);
    }

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

/// Runs one `doc` command line and returns its exit code.
async fn run(engine: &Engine, args: &[&str], io: &mut ScriptedIo) -> i32 {
    doctask_cli::run_doc(engine, &doc_args(args), io)
        .await
        .expect("the command returns an exit code")
}

/// A workspace with `a.md` captured; the file has two clauses.
async fn workspace() -> (tempfile::TempDir, Engine) {
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
    let mut io = ScriptedIo::default();
    assert_eq!(
        run(&engine, &["migrate"], &mut io).await,
        0,
        "{}",
        io.stderr_text
    );
    (tmp, engine)
}

#[tokio::test]
async fn it_061_legacy_annotate_task_lines() {
    let (_tmp, engine) = workspace().await;

    let mut io = ScriptedIo::default();
    let code = doctask_cli::annotate_legacy(&engine, "human", "a.md", "ROOT", "reviewed", &mut io)
        .await
        .expect("annotate returns");
    assert_eq!(code, 0);
    assert!(
        io.stdout_text.starts_with("annotation id: "),
        "{}",
        io.stdout_text
    );

    let mut io = ScriptedIo::default();
    let code = doctask_cli::annotate_legacy(&engine, "human", "a.md", "Nope", "unknown", &mut io)
        .await
        .expect("annotate returns");
    assert_eq!(
        code, 0,
        "an unknown section is accepted as an unchecked anchor"
    );
    assert!(io.stdout_text.starts_with("annotation id: "));

    let mut io = ScriptedIo::default();
    let code = doctask_cli::task_legacy(
        &engine,
        "human",
        "a.md",
        "ROOT",
        "Write summary",
        "due Friday",
        &mut io,
    )
    .await
    .expect("task returns");
    assert_eq!(code, 0);
    assert!(
        io.stdout_text.starts_with("task id: "),
        "{}",
        io.stdout_text
    );
    assert!(io.stderr_text.contains("T-1"), "{}", io.stderr_text);
    assert!(
        io.stderr_text.contains("ladon doc show T-1"),
        "{}",
        io.stderr_text
    );
}

#[test]
fn it_938_entry_arms_are_single_calls() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let entry = fs::read_to_string(root.join("src").join("entry.rs")).expect("entry.rs");
    for marker in [
        "Command::Annotate {",
        "Command::Task {",
        "Command::Doc(args) =>",
    ] {
        let start = entry
            .find(marker)
            .unwrap_or_else(|| panic!("arm {marker} is present"));
        let rest = &entry[start + marker.len()..];
        let end = rest.find("\n        Command::").unwrap_or(rest.len());
        let body = &rest[..end];
        for token in ["store.", "SELECT", "INSERT", "UPDATE", "DELETE", "sql"] {
            assert!(!body.contains(token), "arm {marker} contains {token}");
        }
        assert!(
            body.contains("doctask_cli::"),
            "arm {marker} calls the doc layer"
        );
    }
    let main = fs::read_to_string(root.join("src").join("main.rs")).expect("main.rs");
    assert_eq!(
        main.matches("main_entry").count(),
        1,
        "main.rs makes one call"
    );
}

#[tokio::test]
async fn it_939_one_shot_flow_in_process() {
    let (tmp, engine) = workspace().await;

    assert_eq!(
        run(
            &engine,
            &["baseline", "add", "a.md"],
            &mut ScriptedIo::console(&["y"])
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["note", "a.md", "1", "reviewed"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
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
    assert_eq!(
        run(&engine, &["verify"], &mut ScriptedIo::default()).await,
        0
    );
    assert_eq!(
        run(&engine, &["history"], &mut ScriptedIo::default()).await,
        0
    );

    let mut io = ScriptedIo::default();
    assert_eq!(run(&engine, &["list", "--all"], &mut io).await, 0);
    assert!(io.stdout_text.contains("T-1"), "{}", io.stdout_text);

    let mut io = ScriptedIo::default();
    assert_eq!(run(&engine, &["show", "T-1", "--json"], &mut io).await, 0);
    let shown: Value = serde_json::from_str(io.stdout_text.trim()).expect("show --json parses");
    assert_eq!(shown["ok"], Value::Bool(true));
    assert!(io.stdout_text.contains("done"), "{}", io.stdout_text);

    assert_eq!(
        run(&engine, &["anchors"], &mut ScriptedIo::default()).await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["export", "nema", "--all"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    let exported = fs::read_dir(tmp.path().join(".ladon").join("exports"))
        .expect("exports folder")
        .count();
    assert_eq!(
        exported, 1,
        "the .NEMA file is written under .ladon/exports"
    );
}

/// The names in the exports folder, so a test can find the file one export wrote.
fn export_names(tmp: &tempfile::TempDir) -> Vec<String> {
    let dir = tmp.path().join(".ladon").join("exports");
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn it_940_export_omits_a_file_excluded_after_capture() {
    let (tmp, engine) = workspace().await;
    assert_eq!(
        run(
            &engine,
            &["baseline", "add", "a.md"],
            &mut ScriptedIo::console(&["y"])
        )
        .await,
        0
    );
    assert_eq!(
        run(
            &engine,
            &["note", "a.md", "1", "reviewed"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    // The rule comes after the capture, so the stored flag of a.md is still false.
    ladon_explorer::policy::add_sensitive(tmp.path(), &tmp.path().join("a.md")).unwrap();

    let before = export_names(&tmp);
    assert_eq!(
        run(
            &engine,
            &["export", "report", "--all"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    let after = export_names(&tmp);
    let first = after
        .iter()
        .find(|name| !before.contains(name))
        .expect("the first report is written");
    let text = fs::read_to_string(tmp.path().join(".ladon").join("exports").join(first)).unwrap();
    assert!(
        !text.contains("a.md"),
        "a file the gate refuses is left out of the report: {text}"
    );

    let before = export_names(&tmp);
    assert_eq!(
        run(
            &engine,
            &["export", "report", "--all", "--include-sensitive"],
            &mut ScriptedIo::default()
        )
        .await,
        0
    );
    let after = export_names(&tmp);
    let second = after
        .iter()
        .find(|name| !before.contains(name))
        .expect("the second report is written");
    let text = fs::read_to_string(tmp.path().join(".ladon").join("exports").join(second)).unwrap();
    assert!(
        text.contains("a.md"),
        "--include-sensitive brings the file back: {text}"
    );
}