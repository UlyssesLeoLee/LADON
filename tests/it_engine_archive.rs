//! Archive lifecycle of `engine::Engine`.
//!
//! This file holds a single test on purpose: it sets the process-wide variable
//! `LADON_ARCHIVE_KEY_HEX`, and each integration test file runs in its own process,
//! so the variable cannot race with any other test.
use ladon_explorer::{engine::Engine, provider::FilesystemProvider};
use std::fs;

/// Fixed random 32-byte test key (64 hex digits). Never a documented example key.
const TEST_KEY: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

#[tokio::test]
async fn tc311_archive_evict_and_load_lifecycle() {
    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
    let tmp = tempfile::tempdir().unwrap();
    let remote_a = tempfile::tempdir().unwrap();
    let remote_b = tempfile::tempdir().unwrap();
    let e = Engine::init(tmp.path()).await.unwrap();
    let nas = FilesystemProvider::new("nas", remote_a.path().to_path_buf());
    let other = FilesystemProvider::new("other", remote_b.path().to_path_buf());

    let doc = tmp.path().join("doc.md");
    fs::write(&doc, "# Doc\nv1\n").unwrap();
    let first_capture = e.capture(&doc, "human", "v1").await.unwrap();
    let version_id = first_capture.version_id.clone().unwrap();

    // Without a key, or with a malformed key, nothing is uploaded.
    let err = e.archive_version(&version_id, &nas, false).await.unwrap_err();
    assert!(err.to_string().contains("LADON_ARCHIVE_KEY_HEX"), "{err}");
    std::env::set_var("LADON_ARCHIVE_KEY_HEX", "not-hex");
    let err = e.archive_version(&version_id, &nas, false).await.unwrap_err();
    assert!(err.to_string().contains("64 hex digits"), "{err}");
    std::env::set_var("LADON_ARCHIVE_KEY_HEX", "abcd");
    let err = e.archive_version(&version_id, &nas, false).await.unwrap_err();
    assert!(err.to_string().contains("exactly 32 bytes"), "{err}");
    let content_hash = e.store.version(&version_id).await.unwrap().unwrap().content_hash;
    assert!(e.store.archives(&content_hash).await.unwrap().is_empty(), "nothing may be recorded when the key is rejected");

    std::env::set_var("LADON_ARCHIVE_KEY_HEX", TEST_KEY);

    // First archive uploads an encrypted, verified object.
    let object = e.archive_version(&version_id, &nas, false).await.unwrap();
    assert!(object.encrypted);
    assert_eq!(object.provider, "nas");

    // A repeat on the same provider returns the stored record untouched and adds nothing.
    let again = e.archive_version(&version_id, &nas, false).await.unwrap();
    assert_eq!(again.provider_locator, object.provider_locator);
    assert_eq!(again.verified_at, object.verified_at, "a stored record must not be re-verified and re-recorded");
    assert_eq!(e.store.archives(&object.content_hash).await.unwrap().len(), 1);

    // Eviction only happens after the remote object authenticates: a damaged remote object
    // makes the request fail and keeps the local copy.
    let remote_file = remote_a.path().join(&object.content_hash[..2]).join(format!("{}.ladon", object.content_hash));
    let good = fs::read(&remote_file).unwrap();
    let mut damaged = good.clone();
    *damaged.last_mut().unwrap() ^= 1;
    fs::write(&remote_file, &damaged).unwrap();
    let err = e.archive_version(&version_id, &nas, true).await.unwrap_err();
    assert!(err.to_string().contains("archive authentication failed"), "{err}");
    assert!(e.cas.has(&object.content_hash), "the local copy must survive a failed verification");
    fs::write(&remote_file, &good).unwrap();

    // With an intact remote object, eviction removes the local copy.
    e.archive_version(&version_id, &nas, true).await.unwrap();
    assert!(!e.cas.has(&object.content_hash));

    // Cold content needs the matching provider.
    let err = e.load_version(&version_id, None).await.unwrap_err();
    assert!(err.to_string().contains("cold"), "{err}");
    let bytes = e.load_version(&version_id, Some(&nas)).await.unwrap();
    assert_eq!(bytes, b"# Doc\nv1\n");
    let err = e.load_version(&version_id, Some(&other)).await.unwrap_err();
    assert!(err.to_string().contains("no archived location found"), "{err}");

    // Capturing while the previous content is cold defers the structured diff.
    fs::write(&doc, "# Doc\nv2\n").unwrap();
    let second = e.capture(&doc, "human", "v2").await.unwrap();
    assert_eq!(second.changed_sections, vec!["DIFF_DEFERRED_PREVIOUS_COLD".to_string()]);

    // A first archive on another provider needs the local copy, which is gone.
    let err = e.archive_version(&version_id, &other, false).await.unwrap_err();
    assert!(err.to_string().contains("local copy unavailable"), "{err}");

    std::env::remove_var("LADON_ARCHIVE_KEY_HEX");
}
