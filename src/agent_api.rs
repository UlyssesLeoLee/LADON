//! Local-first agent-facing folder service; usable from CLI or future MCP/IPC.
//! External callers must first authenticate; `AgentApi` here deliberately does
//! NOT expose a network listener or trust user-provided tenant IDs.
use crate::{engine::Engine,model::VersionRecord,policy::LadonPolicy,search};
use anyhow::{bail, Result};
use serde::Serialize;
use std::{fs,path::Path};

#[derive(Debug, Serialize)]
pub struct ResourceInfo { pub name:String, pub kind:String, pub bytes:u64, pub relative_path:String, pub tracked_file_ref:Option<String> }
#[derive(Debug, Serialize)]
pub struct HistoryView {pub file_ref:String,pub versions:Vec<VersionRecord>,pub chain_valid:bool}
#[derive(Debug, Serialize)]
pub struct ApiCapability {pub name:&'static str,pub status:&'static str,pub requires_approval:bool}

pub fn capabilities()->Vec<ApiCapability>{
    [
        ("folders.list","implemented",false),
        ("files.read","implemented",false),
        ("files.grep","implemented",false),
        ("files.ref.resolve","implemented",false),
        ("files.history","implemented",false),
        ("files.diff","cli-implemented",false),
        ("files.tags.list","cli-implemented",false),
        ("files.tags.batch","cli-implemented",true),
        ("files.ignore.check","implemented",false),
        ("files.ignore.add","cli-implemented",true),
        ("docs.annotations","cli-implemented",true),
        ("docs.tasks","cli-implemented",true),
        ("files.restore.plan","cli-implemented",false),
        ("files.restore.commit","cli-implemented",true),
        ("events.subscribe","design-only",false),
        ("files.write.plan","design-only",false),
        ("files.write.commit","design-only",true),
        ("filespaces.allocate","design-only",true),
        ("worktrees.archive","design-only",true),
        ("storage.migrate","design-only",true),
        ("chat.send","design-only",true),
    ].iter().map(|(name,status,requires_approval)|ApiCapability{name:*name,status:*status,requires_approval:*requires_approval}).collect()
}

pub struct AgentApi<'a>{pub engine:&'a Engine}
impl<'a> AgentApi<'a>{
    pub async fn list_folder(&self,relative:&Path,limit:usize)->Result<Vec<ResourceInfo>>{
        if limit==0 || limit>1000 {bail!("limit must be 1..=1000");}
        let candidate=self.engine.root.join(relative);
        if fs::symlink_metadata(&candidate)?.file_type().is_symlink(){bail!("symlink folder denied");}
        let path=candidate.canonicalize()?;
        if !path.starts_with(&self.engine.root){bail!("folder escapes workspace");}
        if !path.is_dir(){bail!("not a directory");}
        let mut out=Vec::new();
        for entry in fs::read_dir(&path)?{
            let e=entry?;let md=e.metadata()?;
            if md.file_type().is_symlink() {continue;}
            let rel=e.path().strip_prefix(&self.engine.root)?.to_string_lossy().replace('\\',"/");
            if rel.starts_with(".ladon") || rel.starts_with(".git") || rel==".LADONignore" {continue;}
            let r=self.engine.store.find_path(&rel).await?;
            out.push(ResourceInfo{name:e.file_name().to_string_lossy().into(),kind:if md.is_dir(){"directory".into()}else{"file".into()},bytes:md.len(),relative_path:rel,tracked_file_ref:r.map(|x|x.id)});
            if out.len()==limit{break;}
        }
        out.sort_by(|a,b|a.name.cmp(&b.name));Ok(out)
    }
    pub fn read_file_local(&self,relative:&Path,max_bytes:u64)->Result<Vec<u8>>{
        if max_bytes==0 || max_bytes>8*1024*1024 {bail!("read limit must be 1..8MiB");}
        let (path,_) =self.engine.resolve(relative)?;
        let md=path.metadata()?;if md.len()>max_bytes {bail!("file exceeds caller limit");}
        Ok(fs::read(path)?)
    }
    pub fn allow_remote_read(&self,relative:&Path)->Result<()> {
        LadonPolicy::load(&self.engine.root)?.allow_external(relative)
    }
    pub async fn history(&self,relative:&str)->Result<HistoryView>{
        let f=self.engine.file_by_path(relative).await?;
        let versions=self.engine.store.versions(&f.id).await?;
        let chain_valid=self.engine.store.verify_chain(&f.id).await?;
        Ok(HistoryView{file_ref:f.id,versions,chain_valid})
    }
    pub fn grep_local(&self,pattern:&str,limit:usize)->Result<Vec<search::SearchHit>> {
        search::grep(&self.engine.root,pattern,limit)
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn ut_agent_capability_status(){let names=capabilities();assert!(names.iter().any(|x|x.name=="events.subscribe"&&x.status=="design-only"));}
}
