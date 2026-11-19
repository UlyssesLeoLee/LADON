use ladon_explorer::{agent_api::AgentApi,engine::Engine};
use std::{fs,path::Path};

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

async fn workspace()->(tempfile::TempDir,Engine){
    let tmp=tempfile::tempdir().unwrap();
    let engine=Engine::init(tmp.path()).await.unwrap();
    (tmp,engine)
}

#[tokio::test]
async fn tc401_list_folder_filters_internal_entries_and_sorts(){
    let (tmp,engine)=workspace().await;
    let root=tmp.path();
    // B.md sorts before a.md bytewise, whereas NTFS enumerates a.md before B.md,
    // so only a real sort by name produces the expected order on Windows.
    fs::write(root.join("B.md"),"bb").unwrap();
    fs::write(root.join("a.md"),"aaaa").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    engine.capture(&root.join("a.md"),"tester","track").await.unwrap();

    let api=AgentApi{engine:&engine};
    let list=api.list_folder(Path::new("."),100).await.unwrap();
    let names:Vec<&str>=list.iter().map(|x|x.name.as_str()).collect();
    assert_eq!(names,vec!["B.md","a.md","sub"]);
    let a=list.iter().find(|x|x.name=="a.md").unwrap();
    assert_eq!((a.kind.as_str(),a.bytes,a.relative_path.as_str()),("file",4,"a.md"));
    assert!(a.tracked_file_ref.is_some());
    let b=list.iter().find(|x|x.name=="B.md").unwrap();
    assert_eq!((b.kind.as_str(),b.bytes),("file",2));
    assert!(b.tracked_file_ref.is_none());
    let sub=list.iter().find(|x|x.name=="sub").unwrap();
    assert_eq!(sub.kind,"directory");
}

#[tokio::test]
async fn tc402_list_folder_limit_bounds(){
    let (tmp,engine)=workspace().await;
    for name in ["a.md","b.md","c.md"]{fs::write(tmp.path().join(name),"x").unwrap();}
    let api=AgentApi{engine:&engine};
    for bad in [0usize,1001]{
        let err=api.list_folder(Path::new("."),bad).await.unwrap_err();
        assert!(err.to_string().contains("limit must be 1..=1000"),"{err}");
    }
    assert_eq!(api.list_folder(Path::new("."),2).await.unwrap().len(),2);
    assert_eq!(api.list_folder(Path::new("."),1).await.unwrap().len(),1);
    assert_eq!(api.list_folder(Path::new("."),1000).await.unwrap().len(),3);
}

#[tokio::test]
async fn tc403_list_folder_rejects_file_escape_and_missing(){
    let (tmp,engine)=workspace().await;
    fs::write(tmp.path().join("a.md"),"x").unwrap();
    let api=AgentApi{engine:&engine};
    let err=api.list_folder(Path::new("a.md"),10).await.unwrap_err();
    assert!(err.to_string().contains("not a directory"),"{err}");
    // A parent reference must never list anything outside the workspace. The exact error depends
    // on how the platform resolves ".." below an extended-length root, so only failure is asserted.
    assert!(api.list_folder(Path::new(".."),10).await.is_err());
    assert!(api.list_folder(Path::new("missing"),10).await.is_err());

    // An absolute path replaces the root in Path::join and resolves outside the workspace:
    // an existing, empty folder there would list successfully if the containment guard were absent.
    let outside=tempfile::tempdir().unwrap();
    let err=api.list_folder(outside.path(),10).await.unwrap_err();
    assert!(err.to_string().contains("folder escapes workspace"),"{err}");
}

#[cfg(unix)]
fn make_link(target:&Path,link:&Path)->bool{std::os::unix::fs::symlink(target,link).is_ok()}
#[cfg(windows)]
fn make_link(target:&Path,link:&Path)->bool{std::os::windows::fs::symlink_file(target,link).is_ok()}
#[cfg(not(any(unix,windows)))]
fn make_link(_target:&Path,_link:&Path)->bool{false}

/// Directory junctions need no special privilege on Windows, unlike symbolic links.
#[cfg(windows)]
fn make_dir_junction(target:&Path,link:&Path)->bool{
    std::process::Command::new("cmd").args(["/C","mklink","/J"]).arg(link).arg(target)
        .output().map(|o|o.status.success()).unwrap_or(false)
}
#[cfg(not(windows))]
fn make_dir_junction(_target:&Path,_link:&Path)->bool{false}

#[tokio::test]
async fn tc404_list_folder_skips_symlink_entries(){
    let (tmp,engine)=workspace().await;
    let root=tmp.path();
    fs::write(root.join("a.md"),"x").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    let file_link=make_link(&root.join("a.md"),&root.join("link.md"));
    let dir_link=make_dir_junction(&root.join("sub"),&root.join("jct"));
    if !file_link && !dir_link {
        assert!(std::env::var_os("LADON_REQUIRE_LINK_TESTS").is_none(),"link creation is required but unavailable on this host");
        eprintln!("neither symbolic links nor junctions can be created here; skipping assertions");
        return;
    }
    let api=AgentApi{engine:&engine};
    let list=api.list_folder(Path::new("."),100).await.unwrap();
    let names:Vec<&str>=list.iter().map(|x|x.name.as_str()).collect();
    assert_eq!(names,vec!["a.md","sub"],"link entries must be skipped");
}

#[tokio::test]
async fn tc405_read_file_local_limits(){
    let (tmp,engine)=workspace().await;
    fs::write(tmp.path().join("big.bin"),"0123456789").unwrap();
    let api=AgentApi{engine:&engine};
    for bad in [0u64,8*1024*1024+1]{
        let err=api.read_file_local(Path::new("big.bin"),bad).unwrap_err();
        assert!(err.to_string().contains("read limit must be 1..8MiB"),"{err}");
    }
    let err=api.read_file_local(Path::new("big.bin"),5).unwrap_err();
    assert!(err.to_string().contains("file exceeds caller limit"),"{err}");
    assert_eq!(api.read_file_local(Path::new("big.bin"),10).unwrap(),b"0123456789");
    assert_eq!(api.read_file_local(Path::new("big.bin"),8*1024*1024).unwrap().len(),10);

    let outside=tempfile::tempdir().unwrap();
    fs::write(outside.path().join("x.txt"),"x").unwrap();
    assert!(api.read_file_local(&outside.path().join("x.txt"),10).is_err());
}

#[tokio::test]
async fn tc406_history_unknown_and_grep_limit(){
    let (tmp,engine)=workspace().await;
    fs::write(tmp.path().join("notes.txt"),"alpha\nneedle one\nneedle two\n").unwrap();
    let api=AgentApi{engine:&engine};
    let err=api.history("nope.md").await.unwrap_err();
    assert!(err.to_string().contains("file not found in ledger"),"{err}");
    let hits=api.grep_local("needle",1).unwrap();
    assert_eq!(hits.len(),1);
    assert_eq!(hits[0].line,2);
    assert_eq!(hits[0].path,"notes.txt");
}

#[tokio::test]
async fn tc407_allow_remote_read_respects_ignore_rules(){
    let (tmp,engine)=workspace().await;
    fs::write(tmp.path().join("ok.md"),"ok").unwrap();
    fs::write(tmp.path().join("secret.md"),"secret").unwrap();
    let api=AgentApi{engine:&engine};
    api.allow_remote_read(Path::new("ok.md")).unwrap();
    ladon_explorer::policy::add_sensitive(tmp.path(),&tmp.path().join("secret.md")).unwrap();
    let err=api.allow_remote_read(Path::new("secret.md")).unwrap_err();
    assert!(err.to_string().contains("denies external data transfer"),"{err}");
}
