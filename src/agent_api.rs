//! Local-first agent-facing folder service; usable from CLI or future MCP/IPC.
//! External callers must first authenticate; `AgentApi` here deliberately does
//! NOT expose a network listener or trust user-provided tenant IDs.
use crate::{engine::Engine,model::VersionRecord,search};
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
            if rel.starts_with(".ladon") || rel.starts_with(".git") || rel.eq_ignore_ascii_case(".LADONignore") {continue;}
            if !self.engine.remote_path_allowed(Path::new(&rel)).await.unwrap_or(false) {continue;}
            let r=self.engine.store.find_path(&rel).await?;
            out.push(ResourceInfo{name:e.file_name().to_string_lossy().into(),kind:if md.is_dir(){"directory".into()}else{"file".into()},bytes:md.len(),relative_path:rel,tracked_file_ref:r.map(|x|x.id)});
            if out.len()==limit{break;}
        }
        out.sort_by(|a,b|a.name.cmp(&b.name));Ok(out)
    }
    pub async fn read_file_local(&self,relative:&Path,max_bytes:u64)->Result<Vec<u8>>{
        if max_bytes==0 || max_bytes>8*1024*1024 {bail!("read limit must be 1..8MiB");}
        let (path,_) =self.engine.resolve(relative)?;
        self.allow_remote_read(relative).await?;
        let md=path.metadata()?;if md.len()>max_bytes {bail!("file exceeds caller limit");}
        Ok(fs::read(path)?)
    }
    /// Refreshes the rules first, so a rule file that cannot be loaded fails the call with its error. The decision is then
    /// taken by `Engine::remote_path_allowed`, the gate that the remote transfer paths use.
    pub async fn allow_remote_read(&self,relative:&Path)->Result<()> {
        self.engine.refresh_policy_if_stale()?;
        if !self.engine.remote_path_allowed(relative).await? {
            bail!("LADON policy denies external data transfer: {}",relative.display());
        }
        Ok(())
    }
    pub async fn history(&self,relative:&str)->Result<HistoryView>{
        let f=self.engine.file_by_path(relative).await?;
        let versions=self.engine.store.versions(&f.id).await?;
        let chain_valid=self.engine.store.verify_chain(&f.id).await?;
        Ok(HistoryView{file_ref:f.id,versions,chain_valid})
    }
    /// Keeps the hits whose path the transfer gate allows. `search::grep` is asked for `limit` hits, and again for twice
    /// as many (up to `MAX_FETCH`) while refused hits leave fewer than `limit` and more hits may exist.
    pub async fn grep_local(&self,pattern:&str,limit:usize)->Result<Vec<search::SearchHit>> {
        const MAX_FETCH:usize=100_000;
        let want=limit.max(1);
        let mut fetch=want;
        let mut allowed=std::collections::HashMap::<String,bool>::new();
        loop {
            let hits=search::grep(&self.engine.root,pattern,fetch)?;
            let total=hits.len();
            let mut out=Vec::new();
            for hit in hits {
                let ok=match allowed.get(&hit.path){
                    Some(known)=>*known,
                    None=>{
                        let ok=self.engine.remote_path_allowed(Path::new(&hit.path)).await.unwrap_or(false);
                        allowed.insert(hit.path.clone(),ok);
                        ok
                    }
                };
                if ok {
                    out.push(hit);
                    if out.len()>=want {break;}
                }
            }
            if out.len()>=want || total<fetch || fetch>=MAX_FETCH {return Ok(out);}
            fetch=fetch.saturating_mul(2).min(MAX_FETCH);
        }
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn ut_agent_capability_status(){let names=capabilities();assert!(names.iter().any(|x|x.name=="events.subscribe"&&x.status=="design-only"));}
}
