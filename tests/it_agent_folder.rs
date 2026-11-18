use ladon_explorer::{agent_api::AgentApi,engine::Engine};
#[tokio::test]
async fn it_agent_folder_snapshot_history(){
    let root=tempfile::tempdir().unwrap();
    let engine=Engine::init(root.path()).await.unwrap();
    std::fs::write(root.path().join("requirements.md"),"# REQ-01\ninitial\n").unwrap();
    let cap=engine.capture(std::path::Path::new("requirements.md"),"UT-agent","created").await.unwrap();
    let api=AgentApi{engine:&engine};
    let folder=api.list_folder(std::path::Path::new("."),100).await.unwrap();
    assert!(folder.iter().any(|x|x.name=="requirements.md"));
    let hist=api.history("requirements.md").await.unwrap();
    assert_eq!(hist.file_ref,cap.file_id);
    assert_eq!(hist.versions.len(),1);
    assert!(hist.chain_valid);
}
