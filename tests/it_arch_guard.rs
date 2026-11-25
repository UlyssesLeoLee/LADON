//! Architecture guard of the MCP layer (WP13, IT-067). A token scan of the source files checks that
//! the tool layer names no human-only operation, that the pure protocol module reaches no store or
//! engine, and that the command entry holds no SQL. Comments, string literals and character
//! literals are skipped, and identifiers match whole words only.
use std::fs;
use std::path::Path;

/// Human-only identifiers that must not appear in the tool layer (design 57 section 13.4).
const HUMAN_ONLY: [&str; 20] = [
    "HumanProof",
    "Presence",
    "HumanOps",
    "approve",
    "reject",
    "apply",
    "recover",
    "close_task",
    "cancel",
    "reopen",
    "adopt",
    "dismiss",
    "relocate",
    "rebase",
    "acknowledge",
    "confirm",
    "revoke",
    "baseline_add",
    "restore",
    "export_to_file",
];

/// Identifiers that must not appear in the command entry, which holds no SQL.
const NO_SQL: [&str; 4] = ["DocStore", "execute", "query", "params"];

fn source(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .expect("source file is readable")
}

/// The identifiers of a Rust source, in order, outside comments and literals.
fn identifiers(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            if chars.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
            continue;
        }
        i += 1;
    }
    out
}

#[test]
fn it_067_tool_layer_names_no_human_only_operation() {
    for file in ["src/mcp.rs", "src/doctask_mcp.rs"] {
        let ids = identifiers(&source(file));
        for banned in HUMAN_ONLY {
            assert!(
                !ids.iter().any(|id| id == banned),
                "{file} names the human-only identifier {banned}"
            );
        }
    }
}

#[test]
fn it_067_protocol_module_reaches_no_store_or_engine() {
    let ids = identifiers(&source("src/mcp.rs"));
    for pair in ids.windows(2) {
        if pair[0] == "crate" {
            assert!(
                pair[1] != "store" && pair[1] != "engine",
                "src/mcp.rs reaches crate::{}",
                pair[1]
            );
        }
    }
}

#[test]
fn it_067_command_entry_holds_no_sql() {
    let ids = identifiers(&source("src/entry.rs"));
    for banned in NO_SQL {
        assert!(
            !ids.iter().any(|id| id == banned),
            "src/entry.rs names {banned}"
        );
    }
}

#[test]
fn it_067_scanner_skips_comments_and_literals() {
    let text = "// approve in a comment\nlet s = \"reject\"; let c = 'x'; let l: &'a str = t;";
    let ids = identifiers(text);
    assert!(!ids.iter().any(|id| id == "approve" || id == "reject"));
    assert!(ids.iter().any(|id| id == "let"));
}
