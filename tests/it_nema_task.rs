use ladon_explorer::{engine::Engine,nema};
use std::fs;

#[tokio::test]
async fn it_05_task_annotation_bind_approved_version(){
 let tmp=tempfile::tempdir().unwrap();let e=Engine::init(tmp.path()).await.unwrap();
 fs::write(tmp.path().join("req.md"),"# REQ-001\nApproved\n").unwrap();
 let result=e.capture(&tmp.path().join("req.md"),"human","approved").await.unwrap();
 let vid=result.version_id.unwrap();
 let a=e.store.add_annotation(&result.file_id,"H1:REQ-001#1",&vid,"add review").await.unwrap();
 let t=e.store.create_task(&result.file_id,"H1:REQ-001#1",&vid,"Review","check impacts").await.unwrap();
 assert!(!a.is_empty()&&!t.is_empty());
}

#[test]
fn it_06_nema_dag_parallel_and_cycle(){
 let graph=" \\>9 1\nREQ Requirement\nDES Design\nTEST Test\nREQ>E1>DES Traces\nREQ>E2>TEST Traces";
 let edges=nema::parse_graph_edges(graph).unwrap();
 let batches=nema::topo_batches(&["REQ".into(),"DES".into(),"TEST".into()],&edges).unwrap();
 assert_eq!(batches[0],vec!["REQ"]);
 assert_eq!(batches[1],vec!["DES","TEST"]);
}
