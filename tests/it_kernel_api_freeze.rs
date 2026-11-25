//! Compile-only freeze of the kernel signatures used by `ui-prototype` (WP01, IT-068).
//!
//! The GUI crate is compiled only by the release workflow, after the kernel tests have run, so a
//! signature it depends on could change unnoticed here. The functions below call every such
//! signature with the argument and result types the GUI relies on. They are never executed; the
//! test passes when this file compiles.
#![allow(dead_code)]
use ladon_explorer::{
    engine::Engine,
    model::{CaptureResult, EventRecord, FileRecord, StorageObject, VersionRecord},
    provider::StorageProvider,
    store::{PendingVersion, Store},
};
use std::path::{Path, PathBuf};

fn frozen_pending_version() -> PendingVersion {
    PendingVersion {
        path: String::new(),
        hash: String::new(),
        size: 0,
        sections: Vec::new(),
        actor: String::new(),
        reason: String::new(),
        sensitive: false,
    }
}

async fn frozen_store_calls(store: &Store, file: &FileRecord) -> anyhow::Result<()> {
    let files: Vec<FileRecord> = store.list_files().await?;
    let found: Option<FileRecord> = store.find_path("a.md").await?;
    let by_id: Option<FileRecord> = store.file_by_id("id").await?;
    let version: Option<VersionRecord> = store.version("version").await?;
    let latest: Option<VersionRecord> = store.latest_version("file").await?;
    let versions: Vec<VersionRecord> = store.versions("file").await?;
    let events: Vec<EventRecord> = store.events("file").await?;
    let chain: bool = store.verify_chain("file").await?;
    let (_file_id, _version_id, _event_id): (String, String, String) =
        store.append_version(frozen_pending_version()).await?;
    store.mark_deleted(file, "human", "moved to recycle bin").await?;
    let _ = (files, found, by_id, version, latest, versions, events, chain);
    Ok(())
}

async fn frozen_engine_calls(root: &Path, engine: &Engine, provider: &dyn StorageProvider) -> anyhow::Result<()> {
    let _opened: Engine = Engine::open(root).await?;
    let _initialised: Engine = Engine::init(root).await?;
    let _captured: CaptureResult = engine.capture(root, "human", "gui-capture").await?;
    let _archived: StorageObject = engine.archive_version("version", provider, false).await?;
    let _bytes: Vec<u8> = engine.load_version("version", None).await?;
    let _restored: CaptureResult = engine.restore("version", true, None).await?;
    let _plan: String = engine.restore_plan("version").await?;
    engine.reload_policy()?;
    let _cached: bool = engine.cas.has("hash");
    let _root: &PathBuf = &engine.root;
    let _store: &Store = &engine.store;
    Ok(())
}

/// The signatures are checked when this file compiles; the functions above are never run.
#[test]
fn it_068_kernel_signatures_are_frozen() {}
