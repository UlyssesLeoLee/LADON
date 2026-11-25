//! Smoke tests of the built `ladon` binary and of the command entry point (TC-1025, TC-1026).
use std::process::{Command, Output};

/// Every subcommand listed in the help output of `ladon`.
const VISIBLE_COMMANDS: &[&str] = &[
    "init",
    "scan",
    "watch",
    "capture",
    "files",
    "history",
    "verify",
    "diff",
    "ignore-check",
    "grep",
    "tag-folder",
    "tags",
    "annotate",
    "task",
    "plan",
    "restore",
    "archive",
    "parallel",
    "agent-capabilities",
    "agent-list",
    "sensitive-add",
    "shell-menu",
    "file-space",
    "google-login",
    "google-logout",
];

fn ladon(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ladon"))
        .args(args)
        .output()
        .expect("ladon binary runs")
}

#[test]
fn binary_smoke_version_help_and_workspace() {
    let version = ladon(&["--version"]);
    assert!(version.status.success());
    let version_text = String::from_utf8(version.stdout).expect("version is UTF-8");
    assert_eq!(
        version_text,
        format!("ladon {}\n", env!("CARGO_PKG_VERSION"))
    );

    let help = ladon(&["--help"]);
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout).expect("help is UTF-8");
    for command in VISIBLE_COMMANDS {
        let listed = help_text
            .lines()
            .any(|line| line.trim_start().split_whitespace().next() == Some(command));
        assert!(listed, "help lists {command}");
    }
    assert!(
        !help_text.contains("shell-host-internal"),
        "the hidden shell host is not listed"
    );

    let dir = tempfile::tempdir().expect("temporary workspace");
    let root = dir.path();
    let init = Command::new(env!("CARGO_BIN_EXE_ladon"))
        .arg("--root")
        .arg(root)
        .arg("init")
        .output()
        .expect("ladon binary runs");
    assert!(init.status.success());
    assert!(String::from_utf8_lossy(&init.stdout).starts_with("Initialized: "));

    let scan = Command::new(env!("CARGO_BIN_EXE_ladon"))
        .arg("--root")
        .arg(root)
        .arg("scan")
        .output()
        .expect("ladon binary runs");
    assert!(scan.status.success());
    assert!(String::from_utf8_lossy(&scan.stdout).starts_with("scanned "));

    let files = Command::new(env!("CARGO_BIN_EXE_ladon"))
        .arg("--root")
        .arg(root)
        .arg("files")
        .output()
        .expect("ladon binary runs");
    assert!(files.status.success());
    assert_eq!(String::from_utf8_lossy(&files.stdout).trim(), "[]");
}

#[test]
fn binary_smoke_main_is_one_call() {
    let source = include_str!("../src/main.rs");
    assert!(source.contains("fn main()"));
    assert_eq!(
        source.matches("fn ").count(),
        1,
        "main.rs declares only main"
    );
    assert_eq!(
        source.matches("main_entry()").count(),
        1,
        "main.rs calls main_entry once"
    );
    assert!(
        !source.contains("tokio"),
        "main.rs does not build a runtime"
    );
    assert!(!source.contains("clap"), "main.rs does not parse arguments");
}

#[test]
fn binary_smoke_entry_is_reachable() {
    let entry: fn() -> anyhow::Result<()> = ladon_explorer::entry::main_entry;
    let _ = entry;
}
