//! Integration tests for `policy`: rule files, exclusion decisions and `add_sensitive`.
//! These tests assume `LADON_GLOBAL_IGNORE` is unset (see `it_policy_global.rs`).
use ladon_explorer::policy::{add_sensitive, LadonPolicy};
use std::{fs, path::{Path, PathBuf}};

/// A temporary workspace and its canonical root, which is the form the policy compares against.
fn workspace() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

#[test]
fn tc505_add_sensitive_writes_anchored_rule_once() {
    let (_tmp, root) = workspace();
    fs::write(root.join("doc.md"), "x").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub").join("doc.md"), "y").unwrap();

    let rule_file = add_sensitive(&root, Path::new("doc.md")).unwrap();
    assert_eq!(rule_file.file_name().unwrap(), ".LADONignore");
    add_sensitive(&root, Path::new("doc.md")).unwrap();

    let text = fs::read_to_string(&rule_file).unwrap();
    assert_eq!(text.lines().filter(|l| *l == "/doc.md").count(), 1, "{text}");
    assert_eq!(text.matches("# LADON sensitive: exclude remote transfers").count(), 1, "{text}");
    let policy = LadonPolicy::load(&root).unwrap();
    assert!(policy.excluded(&root.join("doc.md")).unwrap());
    assert!(!policy.excluded(&root.join("sub").join("doc.md")).unwrap(), "the rule is anchored at the root");
}

#[test]
fn tc506_add_sensitive_directory_rule() {
    let (_tmp, root) = workspace();
    fs::create_dir(root.join("private")).unwrap();
    fs::write(root.join("private").join("a.md"), "x").unwrap();
    fs::write(root.join("public.md"), "y").unwrap();

    add_sensitive(&root, Path::new("private")).unwrap();
    let text = fs::read_to_string(root.join(".LADONignore")).unwrap();
    assert!(text.lines().any(|l| l == "/private/"), "{text}");
    let policy = LadonPolicy::load(&root).unwrap();
    assert!(policy.excluded(&root.join("private")).unwrap());
    assert!(policy.excluded(&root.join("private").join("a.md")).unwrap());
    assert!(!policy.excluded(&root.join("public.md")).unwrap());
}

#[test]
fn tc507_add_sensitive_escapes_glob_characters() {
    let (_tmp, root) = workspace();
    let name = "a[1]#!.md";
    fs::write(root.join(name), "x").unwrap();
    fs::write(root.join("a1.md"), "y").unwrap();

    add_sensitive(&root, Path::new(name)).unwrap();
    let text = fs::read_to_string(root.join(".LADONignore")).unwrap();
    assert!(text.lines().any(|l| l == r"/a\[1\]\#\!.md"), "{text}");
    let policy = LadonPolicy::load(&root).unwrap();
    assert!(policy.excluded(&root.join(name)).unwrap());
    assert!(!policy.excluded(&root.join("a1.md")).unwrap(), "only the exact file may match");
}

#[test]
fn tc508_add_sensitive_rejects_invalid_targets() {
    let (_tmp, root) = workspace();
    fs::write(root.join(".LADONignore"), "# rules\n").unwrap();
    fs::create_dir(root.join(".ladon")).unwrap();
    fs::write(root.join(".ladon").join("x"), "x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("o.md"), "x").unwrap();
    let before = fs::read_to_string(root.join(".LADONignore")).unwrap();

    for target in [".", ".LADONignore", ".ladon/x"] {
        let err = add_sensitive(&root, Path::new(target)).unwrap_err();
        assert!(err.to_string().contains("invalid ignore target"), "{target}: {err}");
    }
    let err = add_sensitive(&root, &outside.path().join("o.md")).unwrap_err();
    assert!(err.to_string().contains("cannot exclude resource outside workspace"), "{err}");
    assert!(add_sensitive(&root, Path::new("missing.md")).is_err());
    assert_eq!(fs::read_to_string(root.join(".LADONignore")).unwrap(), before, "a refused target must not change the rules");
}

#[test]
fn tc509_internal_directories_always_excluded() {
    let (_tmp, root) = workspace();
    let policy = LadonPolicy::load(&root).unwrap();
    for path in [".git/config", ".ladon/meta.turso", "nested/.git/x"] {
        assert!(policy.excluded(&root.join(path)).unwrap(), "{path}");
    }
    assert!(!policy.excluded(&root.join("plain.md")).unwrap());
}

#[test]
fn tc510_allow_external_follows_rules() {
    let (_tmp, root) = workspace();
    fs::write(root.join(".LADONignore"), "*.pem\n").unwrap();
    let policy = LadonPolicy::load(&root).unwrap();
    policy.allow_external(&root.join("ok.md")).unwrap();
    let err = policy.allow_external(&root.join("key.pem")).unwrap_err();
    assert!(err.to_string().contains("denies external data transfer"), "{err}");
    assert!(policy.allow_external(Path::new("key.pem")).is_err(), "relative paths are checked below the root");
}

#[test]
fn tc511_invalid_rule_file_fails_load() {
    let (_tmp, root) = workspace();
    // A reversed character range is a syntax error even though an unclosed `[` is read as a literal.
    fs::write(root.join(".LADONignore"), "[z-a]\n").unwrap();
    assert!(LadonPolicy::load(&root).is_err());
}

#[test]
fn tc512_load_requires_existing_workspace() {
    let (_tmp, root) = workspace();
    let err = LadonPolicy::load(&root.join("does-not-exist")).err().unwrap();
    assert!(err.to_string().contains("workspace not found"), "{err}");
}
