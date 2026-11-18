use crate::{archive::{CasStore,assert_under_workspace},crypto::{digest,load_archive_key,seal,open},diff,model::{CaptureResult,FileRecord,StorageObject},policy::LadonPolicy,provider::StorageProvider,store::{PendingVersion,Store}};
use anyhow::{anyhow,bail,Context,Result};
use chrono::Utc;
use std::{collections::HashSet,fs,path::{Path,PathBuf},sync::{RwLock,atomic::{AtomicBool,Ordering}}};

const MAX_STRUCTURED_DIFF_BYTES:usize=8*1024*1024;
const MAX_ARCHIVE_IN_MEMORY:usize=128*1024*1024;

pub struct Engine { pub root:PathBuf,pub cas:CasStore,pub store:Store, policy:RwLock<LadonPolicy>, policy_valid:AtomicBool }

impl Engine {
    pub async fn init(path:&Path)->Result<Self>{
        let root=path.canonicalize().context("directory does not exist")?;
        fs::create_dir_all(root.join(".ladon"))?;
        let db=root.join(".ladon/meta.turso");
        let store=Store::new(db);
        store.init().await?;
        let rule=root.join(".LADONignore");
        if !rule.exists() {fs::write(rule,include_str!("../config/examples/.LADONignore"))?;}
        let policy=LadonPolicy::load(&root)?;
        Ok(Self{root:root.clone(),cas:CasStore::new(root.join(".ladon/cas")),store,policy:RwLock::new(policy),policy_valid:AtomicBool::new(true)})
    }
    pub async fn open(path:&Path)->Result<Self>{
        let root=path.canonicalize().context("workspace does not exist")?;
        let db=root.join(".ladon/meta.turso");
        if !db.exists(){bail!("workspace not initialized; run `ladon init`");}
        let policy=LadonPolicy::load(&root)?;
        Ok(Self{root:root.clone(),cas:CasStore::new(root.join(".ladon/cas")),store:Store::new(db),policy:RwLock::new(policy),policy_valid:AtomicBool::new(true)})
    }
    /// Reload `.LADONignore` after FS events without rescanning all rule files per capture.
    pub fn reload_policy(&self)->Result<()> {
        let updated=match LadonPolicy::load(&self.root){
            Ok(policy)=>policy,
            Err(err)=>{self.policy_valid.store(false,Ordering::SeqCst);return Err(err);}
        };
        *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=updated;
        self.policy_valid.store(true,Ordering::SeqCst);
        Ok(())
    }
    pub fn resolve(&self,path:&Path)->Result<(PathBuf,String)> {
        let path=if path.is_absolute(){path.to_path_buf()}else{self.root.join(path)};
        let canonical=assert_under_workspace(&self.root,&path)?;
        if canonical.components().any(|x|x.as_os_str()==".ladon"||x.as_os_str()==".git") {bail!("internal file cannot be tracked");}
        let rel=canonical.strip_prefix(&self.root)?.to_string_lossy().replace('\\',"/");
        Ok((canonical,rel))
    }
    pub async fn capture(&self,path:&Path,actor:&str,reason:&str)->Result<CaptureResult>{
        let (path,relative)=self.resolve(path)?;
        let sensitive=self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.excluded(&path)?;
        let existing=self.store.find_path(&relative).await?;
        let (hash,size)=self.cas.put_file(&path)?;
        if let Some(e)=&existing {
            if e.current_hash==hash && !e.deleted {
                return Ok(CaptureResult{file_id:e.id.clone(),version_id:None,event_id:None,status:"unchanged".into(),sensitive,changed_sections:vec![]});
            }
        }
        let previous_was_cold=existing.as_ref().map(|e|!self.cas.has(&e.current_hash)).unwrap_or(false);
        let before=if let Some(e)=&existing{
            if self.cas.has(&e.current_hash){self.cas.load_bytes(&e.current_hash,MAX_STRUCTURED_DIFF_BYTES).unwrap_or_default()}else{vec![]}
        }else{vec![]};
        let after=if size <=MAX_STRUCTURED_DIFF_BYTES as u64 {self.cas.load_bytes(&hash,MAX_STRUCTURED_DIFF_BYTES)?}else{vec![]};
        let ext=path.extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase();
        let sections=if previous_was_cold {vec!["DIFF_DEFERRED_PREVIOUS_COLD".into()]}
          else if size>MAX_STRUCTURED_DIFF_BYTES as u64 {vec!["LARGE_BINARY_OR_TEXT".into()]}
          else if !before.is_empty() && after==before {vec![]}
          else {diff::changed_sections(&before,&after,&ext)};
        let (fid,vid,eid)=self.store.append_version(PendingVersion{path:relative,hash,size:size as i64,sections:sections.clone(),actor:actor.into(),reason:reason.into(),sensitive}).await?;
        Ok(CaptureResult{file_id:fid,version_id:Some(vid),event_id:Some(eid),status:"captured".into(),sensitive,changed_sections:sections})
    }
    pub async fn scan(&self,actor:&str)->Result<Vec<CaptureResult>>{
        let mut out=Vec::new();let mut seen=HashSet::new();
        for entry in walkdir::WalkDir::new(&self.root).follow_links(false).into_iter()
            .filter_entry(|e| e.depth()==0 || !matches!(e.file_name().to_str(),Some(".ladon"|".git")))
            .filter_map(std::result::Result::ok) {
            if entry.file_type().is_dir(){continue;}
            if !entry.file_type().is_file(){continue;}
            let rel=entry.path().strip_prefix(&self.root)?.to_string_lossy().replace('\\',"/");
            if rel==".LADONignore"||rel.starts_with(".ladon/")||rel.starts_with(".git/"){continue;}
            seen.insert(rel);
            out.push(self.capture(entry.path(),actor,"scan").await?);
        }
        for f in self.store.list_files().await? {
            if !f.deleted&&!seen.contains(&f.relative_path) {
                self.store.mark_deleted(&f,actor,"missing during full scan").await?;
            }
        }
        Ok(out)
    }
    pub async fn file_by_path(&self,path:&str)->Result<FileRecord>{
        self.store.find_path(&path.replace('\\',"/")).await?.ok_or_else(||anyhow!("file not found in ledger"))
    }
    pub async fn archive_version(&self,version_id:&str,provider:&dyn StorageProvider,evict_local:bool)->Result<StorageObject>{
        if !self.policy_valid.load(Ordering::SeqCst){bail!("policy reload failed: all external archive transfers blocked");}
        let version=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        let current=self.store.list_files().await?.into_iter().find(|f|f.id==version.file_id)
            .ok_or_else(||anyhow!("missing tracked file"))?;
        self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.allow_external(Path::new(&current.relative_path))?;
        if version.sensitive||self.store.is_hash_sensitive(&version.content_hash).await? {bail!("sensitive version blocked from external storage");}
        // Archive may exist on this provider already, verify it before doing anything.
        let key=load_archive_key()?;
        for prior in self.store.archives(&version.content_hash).await? {
            if prior.provider==provider.name(){
                let data=provider.get(&prior.provider_locator).await?;
                open(&data,&key,&version.content_hash)?;
                if evict_local{self.cas.evict(&version.content_hash)?;}
                return Ok(prior);
            }
        }
        let plain=self.cas.load_bytes(&version.content_hash,MAX_ARCHIVE_IN_MEMORY)
            .context("local copy unavailable for first archive; restore from an existing provider first")?;
        let sealed=seal(&plain,&key)?;
        let locator=provider.put(&version.content_hash,&sealed).await?;
        let check=provider.get(&locator).await?;
        open(&check,&key,&version.content_hash)?;
        let obj=StorageObject{content_hash:version.content_hash.clone(),provider:provider.name().into(),provider_locator:locator,encrypted:true,verified_at:Utc::now().to_rfc3339()};
        self.store.archive_record(&obj).await?;
        if evict_local {self.cas.evict(&version.content_hash)?;}
        Ok(obj)
    }
    pub async fn load_version(&self,version_id:&str,provider:Option<&dyn StorageProvider>)->Result<Vec<u8>>{
        let v=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        if self.cas.has(&v.content_hash) {return self.cas.load_bytes(&v.content_hash,MAX_ARCHIVE_IN_MEMORY);}
        let provider=provider.context("historical content is cold; supply matching provider")?;
        let key=load_archive_key()?;
        let archives=self.store.archives(&v.content_hash).await?;
        let record=archives.iter().find(|a|a.provider==provider.name()).context("no archived location found")?;
        let bytes=open(&provider.get(&record.provider_locator).await?,&key,&v.content_hash)?;
        Ok(bytes)
    }
    pub async fn restore_plan(&self,version_id:&str)->Result<String>{
        let v=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        let file=self.store.list_files().await?.into_iter().find(|x|x.id==v.file_id).ok_or_else(||anyhow!("unknown resource"))?;
        Ok(format!("RESTORE PLAN: {} version {} ({}) -> {}\nTarget exists: {}\nAction: require --approve, create new version and preserve current history.\nHistorical bytes: read from verified local CAS/cloud archive.",
            file.id,v.number,&v.content_hash,file.relative_path,self.root.join(file.relative_path).exists()))
    }
    pub async fn restore(&self,version_id:&str,approve:bool,provider:Option<&dyn StorageProvider>)->Result<CaptureResult>{
        if !approve {bail!("restore requires explicit --approve");}
        let v=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        let file=self.store.list_files().await?.into_iter().find(|f|f.id==v.file_id).ok_or_else(||anyhow!("file not found"))?;
        // TOCTOU guard: refuse if current file diverged from last captured hash.
        let target=self.root.join(&file.relative_path);
        if target.exists() {
            let data=fs::read(&target)?;
            if digest(&data)!=file.current_hash {bail!("live file changed outside ledger; scan before restore");}
        }
        let data=self.load_version(version_id,provider).await?;
        if let Some(parent)=target.parent(){fs::create_dir_all(parent)?;}
        let tmp=target.with_extension(format!("ladon-restore-{}",uuid::Uuid::new_v4()));
        fs::write(&tmp,&data)?;
        // Note: atomic replace differs by OS. Current implementation refuses overwrite on Windows failure.
        // Preserve the last file until the restored version has a durable DB record.
        let backup=target.with_extension(format!("ladon-backup-{}",uuid::Uuid::new_v4()));
        let had_target=target.exists();
        if had_target {fs::rename(&target,&backup)?;}
        if let Err(e)=fs::rename(&tmp,&target){
            let _=fs::remove_file(&tmp);
            if had_target{let _=fs::rename(&backup,&target);}
            return Err(e.into());
        }
        let captured=self.capture(&target,"restore",&format!("restore from version {}",v.id)).await;
        if captured.is_ok(){ if had_target{fs::remove_file(&backup)?;} }
        else {
            let _=fs::remove_file(&target);
            if had_target{let _=fs::rename(&backup,&target);}
        }
        captured
    }
}
