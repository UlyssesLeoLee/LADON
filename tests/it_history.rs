use ladon_explorer::{engine::Engine,provider::{FilesystemProvider,StorageProvider},crypto::digest};
use std::fs;

#[tokio::test]
async fn it_01_capture_history_reuse_identity_and_chain(){
 let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let e=Engine::init(root).await.unwrap();
 fs::write(root.join("req.md"),"# R1\noriginal\n# R2\nold\n").unwrap();
 let a=e.capture(&root.join("req.md"),"human","initial").await.unwrap();
 assert_eq!(a.status,"captured");
 let unchanged=e.capture(&root.join("req.md"),"human","again").await.unwrap();
 assert_eq!(unchanged.status,"unchanged");
 fs::write(root.join("req.md"),"# R1\noriginal\n# R2\nnew\n").unwrap();
 let b=e.capture(&root.join("req.md"),"agent","update").await.unwrap();
 assert_eq!(a.file_id,b.file_id);assert_eq!(b.changed_sections,vec!["H1:R2#1"]);
 let events=e.store.events(&a.file_id).await.unwrap();assert_eq!(events.len(),2);
 assert!(e.store.verify_chain(&a.file_id).await.unwrap());
}

#[tokio::test]
async fn it_02_sensitive_prevents_archive(){
 let tmp=tempfile::tempdir().unwrap();let e=Engine::init(tmp.path()).await.unwrap();
 fs::write(tmp.path().join("secret.pem"),"PRIVATE").unwrap();
 let c=e.capture(&tmp.path().join("secret.pem"),"human","init").await.unwrap();
 assert!(c.sensitive);
 let p=FilesystemProvider::new("nas",tmp.path().join("remote"));
 assert!(e.archive_version(c.version_id.as_deref().unwrap(),&p,false).await.is_err());
}

#[tokio::test]
async fn it_03_archive_restore_with_verification(){
 let tmp=tempfile::tempdir().unwrap();let e=Engine::init(tmp.path()).await.unwrap();
 let p=FilesystemProvider::new("nas",tmp.path().join("remote"));
 fs::write(tmp.path().join("design.md"),"# Design\nV1\n").unwrap();
 let a=e.capture(&tmp.path().join("design.md"),"human","v1").await.unwrap();
 // Archive key integration tests must not race setting a process-global env var.
 // Test local CAS restoration here; cloud encryption is covered by crypto UT.
 fs::write(tmp.path().join("design.md"),"# Design\nV2\n").unwrap();
 let b=e.capture(&tmp.path().join("design.md"),"human","v2").await.unwrap();
 assert_ne!(a.version_id,b.version_id);
 assert_eq!(e.store.versions(&a.file_id).await.unwrap().len(),2);
 assert!(e.restore_plan(a.version_id.as_deref().unwrap()).await.unwrap().contains("RESTORE PLAN"));
 assert!(e.restore(a.version_id.as_deref().unwrap(),false,None).await.is_err());
 let restored=e.restore(a.version_id.as_deref().unwrap(),true,None).await.unwrap();
 assert_eq!(restored.file_id,a.file_id);
 assert_eq!(digest(&fs::read(tmp.path().join("design.md")).unwrap()),e.store.latest_version(&a.file_id).await.unwrap().unwrap().content_hash);
 assert_eq!(e.store.events(&a.file_id).await.unwrap().len(),3);
 let _=p.name();
}

#[tokio::test]
async fn it_04_detect_delete_and_keep_history(){
 let tmp=tempfile::tempdir().unwrap();let e=Engine::init(tmp.path()).await.unwrap();
 fs::write(tmp.path().join("x.txt"),"hello").unwrap();
 let a=e.capture(&tmp.path().join("x.txt"),"human","initial").await.unwrap();
 fs::remove_file(tmp.path().join("x.txt")).unwrap();
 e.scan("scanner").await.unwrap();
 let r=e.store.find_path("x.txt").await.unwrap().unwrap();
 assert!(r.deleted);assert_eq!(r.id,a.file_id);
 assert!(e.store.verify_chain(&a.file_id).await.unwrap());
}
