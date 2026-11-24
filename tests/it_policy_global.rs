//! The optional global rule file named by `LADON_GLOBAL_IGNORE`.
//!
//! This file holds a single test on purpose: it sets a process-wide environment variable,
//! and each integration test file runs in its own process, so nothing can race with it.
use ladon_explorer::policy::LadonPolicy;
use std::fs;

#[test]
fn tc513_global_ignore_file_applies() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let rules = tempfile::tempdir().unwrap();
    let global = rules.path().join("global.ignore");
    fs::write(&global, "*.secret\n").unwrap();

    std::env::set_var("LADON_GLOBAL_IGNORE", &global);
    let policy = LadonPolicy::load(&root).unwrap();
    assert!(policy.excluded(&root.join("x.secret")).unwrap());
    assert!(!policy.excluded(&root.join("x.md")).unwrap());

    // A variable that names a file which does not exist fails the load; the rules are never dropped.
    std::env::set_var("LADON_GLOBAL_IGNORE", rules.path().join("missing.ignore"));
    let err = LadonPolicy::load(&root).err().expect("a missing global rule file must fail the load");
    assert!(format!("{err:#}").contains("missing.ignore"), "the error must name the file: {err:#}");

    std::env::remove_var("LADON_GLOBAL_IGNORE");
    let policy = LadonPolicy::load(&root).unwrap();
    assert!(!policy.excluded(&root.join("x.secret")).unwrap());
}
