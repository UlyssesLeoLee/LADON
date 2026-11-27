//! Archive refusal while a nested rule file fails to load (TC-317).
//!
//! The test clears the process-wide variable `LADON_ARCHIVE_KEY_HEX`. Each integration test file runs in
//! its own process, so the variable cannot race with any other test.
use ladon_explorer::{engine::Engine, provider::FilesystemProvider};
use std::fs;

#[tokio::test]
async fn tc317_archive_returns_scan_error_before_missing_key_error() {
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
    let tmp = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let provider = FilesystemProvider::new("nas", remote.path().to_path_buf());

    let top = tmp.path().join("top.txt");
    fs::write(&top, "top bytes").unwrap();
    let captured = e.capture(&top, "human", "tc317").await.unwrap();
    let version_id = captured.version_id.clone().unwrap();
    let content_hash = e.store.version(&version_id).await.unwrap().unwrap().content_hash;

    // The root rule files still pass their check; the new complete scan fails and stays in force.
    fs::create_dir_all(tmp.path().join("sub")).unwrap();
    fs::write(tmp.path().join("sub/.LADONignore"), "[z-a]\n").unwrap();
    e.reload_policy().unwrap();

    let err = e.archive_version(&version_id, &provider, false).await.unwrap_err();
    let text = format!("{err:#}");
    assert!(!text.contains("LADON_ARCHIVE_KEY_HEX"), "the scan error must come before the key error: {text}");
    assert!(text.contains("LADONignore"), "the error must name the rule file that failed to load: {text}");
    assert!(e.store.archives(&content_hash).await.unwrap().is_empty(), "no archive record may be stored");
    assert_eq!(fs::read_dir(remote.path()).unwrap().count(), 0, "the provider must receive no object");
}
