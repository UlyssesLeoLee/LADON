//! CLI: native local-first entry point. GUI remains optional and independent.
use anyhow::{anyhow,bail,Context,Result};
use clap::{Parser,Subcommand};
use ladon_explorer::{engine::Engine,model::FileRecord,nema,policy::LadonPolicy,provider::{provider_from_env,StorageProvider},search,tagging,workspace::FileSpace};
use notify::{RecursiveMode,Watcher};
use similar::{ChangeTag,TextDiff};
use std::{collections::HashSet,fs,io::Read,path::{Path,PathBuf},sync::mpsc,time::Duration};

#[derive(Parser)]
#[command(name="ladon",version,about="LADON Explorer: event-first document history kernel")]
struct Cli {
    #[arg(long,global=true,default_value=".")]
    root:PathBuf,
    #[command(subcommand)]
    cmd:Command,
}
#[derive(Subcommand)]
enum Command {
    Init,
    Scan{#[arg(long,default_value="human")]actor:String},
    Watch{#[arg(long,default_value="watcher")]actor:String},
    Capture{file:PathBuf,#[arg(long,default_value="human")]actor:String,#[arg(long,default_value="manual capture")]reason:String},
    Files,
    History{file:PathBuf},
    Verify{file:PathBuf},
    Diff{from:String,to:String},
    IgnoreCheck{file:PathBuf},
    Grep{regex:String,#[arg(long,default_value_t=40)]limit:usize},
    TagFolder{#[arg(long,default_value="topic,project,stage")]presets:String,#[arg(long,default_value="")]description:String,#[arg(long)]endpoint:Option<String>,#[arg(long)]model:Option<String>,#[arg(long)]recursive:bool},
    Tags{file:PathBuf},
    Annotate{file:PathBuf,#[arg(long,default_value="ROOT")]section:String,text:String},
    Task{file:PathBuf,#[arg(long,default_value="ROOT")]section:String,title:String,#[arg(long,default_value="")]description:String},
    Plan{version_id:String},
    Restore{version_id:String,#[arg(long)]approve:bool,#[arg(long)]provider:Option<String>,#[arg(long)]destination:Option<PathBuf>},
    Archive{version_id:String,#[arg(long,default_value="local")]provider:String,#[arg(long)]destination:Option<PathBuf>,#[arg(long)]evict_local:bool},
    Parallel{graph_file:PathBuf},
    AgentCapabilities,
    AgentList{folder:PathBuf,#[arg(long,default_value_t=100)]limit:usize},
    SensitiveAdd{file:PathBuf},
    ShellMenu{file:PathBuf},
    #[command(hide=true)]
    ShellHostInternal{file:PathBuf},
    FileSpace,
}

#[tokio::main]
async fn main()->Result<()> {
    let cli=Cli::parse();
    // Shell menu runs in a separate copy of the executable to isolate third-party COM handlers.
    if let Command::ShellMenu{ref file}=cli.cmd {
        let path=file.canonicalize()?;
        let status=ladon_explorer::shell::spawn_shell_host(&cli.root,&path)?;
        if status!=0 {ladon_explorer::shell::open_in_explorer(&path)?;}
        return Ok(());
    }
    if let Command::ShellHostInternal{ref file}=cli.cmd {
        let path=file.canonicalize()?;
        match ladon_explorer::shell::run_isolated_host(&path) {
            Ok(ladon_explorer::shell::ShellResult::ExecutedOrDismissed)=>return Ok(()),
            Ok(ladon_explorer::shell::ShellResult::MarkSensitive)=>{
                let engine=Engine::open(&cli.root).await?;
                ladon_explorer::policy::add_sensitive(&engine.root,&path)?;
                println!("Sensitive rule added for {}",path.display());return Ok(());
            },
            Ok(ladon_explorer::shell::ShellResult::ViewHistory)=>{
                println!("Use: ladon --root <workspace> history <file> (GUI pane is planned)");return Ok(());
            },
            Ok(ladon_explorer::shell::ShellResult::DocumentTask)=>{
                println!("Open LADON Document Task editor (GUI integration planned)");return Ok(());
            },
            Ok(ladon_explorer::shell::ShellResult::OpenExplorer)=>{
                ladon_explorer::shell::open_in_explorer(&path)?;return Ok(());
            },
            Err(e)=>{eprintln!("{e:#}");std::process::exit(1);}
        }
    }
    match cli.cmd {
        Command::Init=>{let e=Engine::init(&cli.root).await?;println!("Initialized: {}",e.root.display());}
        other=>{
            let engine=Engine::open(&cli.root).await?;
            match other {
                Command::Init=>unreachable!(),
                Command::ShellMenu{..}|Command::ShellHostInternal{..}=>unreachable!(),
                Command::AgentCapabilities=>println!("{}",serde_json::to_string_pretty(&ladon_explorer::agent_api::capabilities())?),
                Command::AgentList{folder,limit}=>{
                    let api=ladon_explorer::agent_api::AgentApi{engine:&engine};
                    println!("{}",serde_json::to_string_pretty(&api.list_folder(&folder,limit).await?)?);
                },
                Command::SensitiveAdd{file}=>{
                    let rule=ladon_explorer::policy::add_sensitive(&engine.root,&file)?;
                    engine.reload_policy()?;
                    println!("Updated sensitivity rules: {}",rule.display());
                },
                Command::Scan{actor}=>{
                    let results=engine.scan(&actor).await?;
                    let changed=results.iter().filter(|r|r.status=="captured").count();
                    println!("scanned {}, captured {} version(s)",results.len(),changed);
                }
                Command::Capture{file,actor,reason}=>println!("{}",serde_json::to_string_pretty(&engine.capture(&file,&actor,&reason).await?)?),
                Command::Watch{actor}=>watch(&engine,&actor).await?,
                Command::Files=>println!("{}",serde_json::to_string_pretty(&engine.store.list_files().await?)?),
                Command::History{file}=>{
                    let record=resolve_record(&engine,&file).await?;
                    let versions=engine.store.versions(&record.id).await?;
                    let events=engine.store.events(&record.id).await?;
                    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"file":record,"versions":versions,"events":events,"chain_valid":engine.store.verify_chain(&record.id).await?}))?);
                }
                Command::Verify{file}=>{
                    let record=resolve_record(&engine,&file).await?;
                    let valid=engine.store.verify_chain(&record.id).await?;
                    println!("event chain {}",if valid{"VALID"}else{"BROKEN"});
                    if !valid{bail!("history chain damaged");}
                }
                Command::Diff{from,to}=>{
                    let left=engine.load_version(&from,None).await?;
                    let right=engine.load_version(&to,None).await?;
                    let a=String::from_utf8_lossy(&left);let b=String::from_utf8_lossy(&right);
                    let diff=TextDiff::from_lines(a.as_ref(),b.as_ref());
                    for (i,c) in diff.iter_all_changes().enumerate(){
                        if i>800{println!("[diff truncated]");break;}
                        let ch=match c.tag(){ChangeTag::Delete=>'-',ChangeTag::Insert=>'+',ChangeTag::Equal=>' '};
                        print!("{}{}",ch,c);
                    }
                }
                Command::IgnoreCheck{file}=>{
                    let p=LadonPolicy::load(&engine.root)?;
                    println!("{}",if p.excluded(&file)?{"EXCLUDED: local only"}else{"ALLOWED: subject to explicit archive action"});
                }
                Command::Grep{regex,limit}=>println!("{}",serde_json::to_string_pretty(&search::grep(&engine.root,&regex,limit)?)?),
                Command::TagFolder{presets,description,endpoint,model,recursive}=>{
                    let count=tag_folder(&engine,&presets,&description,endpoint.as_deref(),model.as_deref(),recursive).await?;
                    println!("tagged {} file(s)",count);
                }
                Command::Tags{file}=>{
                    let rec=resolve_record(&engine,&file).await?;
                    println!("{}",serde_json::to_string_pretty(&engine.store.tags(&rec.id).await?)?);
                }
                Command::Annotate{file,section,text}=>{
                    let rec=resolve_record(&engine,&file).await?;
                    let v=engine.store.latest_version(&rec.id).await?.context("file has no version")?;
                    println!("annotation id: {}",engine.store.add_annotation(&rec.id,&section,&v.id,&text).await?);
                }
                Command::Task{file,section,title,description}=>{
                    let rec=resolve_record(&engine,&file).await?;
                    let v=engine.store.latest_version(&rec.id).await?.context("file has no version")?;
                    println!("task id: {}",engine.store.create_task(&rec.id,&section,&v.id,&title,&description).await?);
                }
                Command::Plan{version_id}=>println!("{}",engine.restore_plan(&version_id).await?),
                Command::Restore{version_id,approve,provider,destination}=>{
                    let adapter=if let Some(provider)=provider{Some(provider_from_env(&provider,destination.as_deref())?)}else{None};
                    let obj=engine.restore(&version_id,approve,adapter.as_deref()).await?;
                    println!("{}",serde_json::to_string_pretty(&obj)?);
                }
                Command::Archive{version_id,provider,destination,evict_local}=>{
                    let p=provider_from_env(&provider,destination.as_deref())?;
                    let obj=engine.archive_version(&version_id,p.as_ref(),evict_local).await?;
                    println!("{}",serde_json::to_string_pretty(&obj)?);
                }
                Command::Parallel{graph_file}=>{
                    let data=fs::read_to_string(graph_file)?;
                    let edges=nema::parse_graph_edges(&data)?;
                    let nodes:HashSet<String>=data.lines().filter(|l|!l.starts_with(' ') && !l.is_empty())
                        .filter_map(|l|{
                            let t=l.split(' ').next()?;
                            if t.contains('>')||t.contains('<')||t.contains('='){None}else{Some(t.to_string())}
                        }).collect();
                    let groups=nema::topo_batches(&nodes.into_iter().collect::<Vec<_>>(),&edges)?;
                    println!("{}",serde_json::to_string_pretty(&groups)?);
                }
                Command::FileSpace=>println!("{}",serde_json::to_string_pretty(&FileSpace::from_files(&engine.store.list_files().await?))?),
            }
        }
    }
    Ok(())
}

async fn resolve_record(engine:&Engine,path:&Path)->Result<FileRecord>{
    let rel=if path.is_absolute(){path.strip_prefix(&engine.root)?.to_string_lossy().to_string()}else{path.to_string_lossy().to_string()};
    engine.file_by_path(&rel).await
}

async fn tag_folder(engine:&Engine,presets:&str,description:&str,endpoint:Option<&str>,model:Option<&str>,recursive:bool)->Result<usize>{
    let policy=LadonPolicy::load(&engine.root)?;
    let mut count=0;
    let iter=walkdir::WalkDir::new(&engine.root).max_depth(if recursive{32}else{1}).follow_links(false);
    for entry in iter.into_iter().filter_entry(|e| e.depth()==0 || !matches!(e.file_name().to_str(),Some(".ladon"|".git")))
        .filter_map(std::result::Result::ok){
        if !entry.file_type().is_file(){continue;}
        if entry.path().components().any(|s|s.as_os_str()==".git"||s.as_os_str()==".ladon") || entry.file_name()==".LADONignore"{continue;}
        let rel=entry.path().strip_prefix(&engine.root)?.to_string_lossy().replace('\\',"/");
        let Some(file)=engine.store.find_path(&rel).await? else {continue;};
        let mut buf=Vec::new();let mut f=fs::File::open(entry.path())?;
        f.by_ref().take(16384).read_to_end(&mut buf)?;
        let content=String::from_utf8_lossy(&buf);
        let mut tags=tagging::heuristic_tags(&rel,&content);
        if let (Some(url),Some(model))=(endpoint,model){
            // Sensitive files may only be sent to a loopback local model if explicitly requested.
            if policy.excluded(entry.path())? && !tagging::local_endpoint(url)? {
                eprintln!("skipped remote AI tagging for excluded file: {}",rel);
            } else {
                tags.extend(tagging::ai_tags(url,model,&format!("{}; dimensions: {}",description,presets),&rel,&content).await?);
            }
        }
        if !description.is_empty(){tags.push("rule-described".into());}
        tags.sort();tags.dedup();
        if !tags.is_empty(){engine.store.set_tags(&file.id,&tags,if endpoint.is_some() && model.is_some(){"ai+rules"}else{"heuristic"},"v1").await?;count+=1;}
    }
    Ok(count)
}

async fn watch(engine:&Engine,actor:&str)->Result<()> {
    let (tx,rx)=mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher=notify::recommended_watcher(move|r|{let _=tx.send(r);})?;
    watcher.watch(&engine.root,RecursiveMode::Recursive)?;
    println!("watching {} — Ctrl+C stops",engine.root.display());
    loop {
        let event=rx.recv().map_err(|_|anyhow!("watcher disconnected"))??;
        let mut paths:HashSet<PathBuf>=event.paths.into_iter().collect();
        std::thread::sleep(Duration::from_millis(350));
        while let Ok(next)=rx.try_recv(){for p in next?.paths{paths.insert(p);}}
        for path in paths {
            if path.components().any(|c|c.as_os_str()==".ladon"||c.as_os_str()==".git"){continue;}
            if path.file_name().map(|s|s==".LADONignore").unwrap_or(false){if let Err(err)=engine.reload_policy(){eprintln!("policy reload failed, check external-transfer protections: {err:#}");}
                else {println!("policy reloaded: {}",path.display());}
                continue;}
            if path.is_file(){
                if let Err(e)=engine.capture(&path,actor,"filesystem stable save").await{eprintln!("capture warning {}: {e:#}",path.display());}
            } else if let Ok(relative)=path.strip_prefix(&engine.root){
                let rel=relative.to_string_lossy().replace('\\',"/");
                if let Some(f)=engine.store.find_path(&rel).await? {
                    if let Err(e)=engine.store.mark_deleted(&f,actor,"watcher removal").await{eprintln!("delete warning: {e:#}");}
                }
            }
        }
    }
}
