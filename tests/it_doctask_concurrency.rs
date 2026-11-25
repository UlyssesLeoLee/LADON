//! Cross-process concurrency of the document-task store (WP13, IT-069). The test process writes
//! annotations through the owner service while `ladon mcp-serve` in a child process reads and
//! writes through the MCP tools. Each side pauses 10 to 30 milliseconds between store calls. A busy
//! reply is retried, every write succeeds, and the event chain verifies afterwards.
#[path = "support/mcp_stub.rs"]
mod mcp_stub;

use ladon_explorer::{
    doctask_flow::DocFlow,
    doctask_model::SourceKind,
    doctask_service::{Caller, DocTaskService, Shown},
    engine::Engine,
};
use mcp_stub::{envelope, Stub};
use serde_json::json;
use std::{fs, time::Duration};

const LAUNCH: &str = "20261125-101010-ab";
const ROUNDS: u64 = 12;

/// A pause of 10 to 30 milliseconds that varies with the round.
fn pause(round: u64) {
    std::thread::sleep(Duration::from_millis(10 + (round * 7) % 21));
}

#[tokio::test]
async fn it_069_cross_process_reads_and_writes_all_succeed() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let engine = Engine::init(tmp.path()).await.expect("workspace");
    fs::write(
        tmp.path().join("a.md"),
        "# 1 Scope\nThe scope text.\n\n# 2 Rules\nThe rule text.\n",
    )
    .expect("write a.md");
    let captured = engine
        .capture(&tmp.path().join("a.md"), "human", "test")
        .await
        .expect("capture");
    let version_id = captured.version_id.expect("capture records a version");
    let content_hash = engine
        .store
        .version(&version_id)
        .await
        .expect("version lookup")
        .expect("the captured version exists")
        .content_hash;
    let owner = Caller::human("owner", SourceKind::Cli);
    let service = DocTaskService::open(&engine, owner.clone())
        .await
        .expect("owner service");
    service
        .register_baseline(
            &captured.file_id,
            None,
            "",
            &Shown {
                version_id,
                hash: content_hash,
            },
        )
        .await
        .expect("baseline registered");
    let file_ref = service
        .resolve_file("a.md")
        .await
        .expect("a.md is tracked")
        .id;

    let mut stub = Stub::spawn(tmp.path(), LAUNCH, None);
    let init = stub.request("initialize", json!({"protocolVersion": "2025-11-25"}));
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");

    for round in 0..ROUNDS {
        let mut last_error = String::new();
        let mut written = false;
        for _ in 0..6 {
            match service
                .add_annotation("a.md", "1", &format!("owner note {round}"), "", "", false)
                .await
            {
                Ok(_) => {
                    written = true;
                    break;
                }
                Err(err) => last_error = format!("{err:#}"),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(written, "owner write {round} succeeds: {last_error}");
        pause(round);

        let mut agent_written = false;
        for _ in 0..6 {
            let reply = envelope(&stub.call(
                "doc_annotation_add",
                json!({"file_ref": file_ref, "section": "1", "text": format!("agent note {round}")}),
            ));
            if reply["ok"] == true {
                agent_written = true;
                break;
            }
            assert_eq!(reply["code"], "E_STORE_BUSY", "{reply}");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(agent_written, "agent write {round} succeeds");
        pause(round + 1);

        let listed = envelope(&stub.call("doc_task_list", json!({})));
        assert_eq!(listed["ok"], true, "{listed}");
        pause(round + 2);
    }

    let (status, _) = stub.close();
    assert!(status.is_some_and(|status| status.success()));
    let flow = DocFlow::open(&engine, owner).await.expect("owner flow");
    let report = flow.verify(None).await.expect("verification runs");
    assert!(
        report.ok,
        "the event chain verifies after the concurrent run"
    );
}
