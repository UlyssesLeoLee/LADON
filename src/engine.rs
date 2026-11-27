use crate::{archive::{CasStore,assert_under_workspace},crypto::{digest,load_archive_key,seal,open},diff,model::{CaptureResult,FileRecord,StorageObject,VersionRecord},policy::{LadonPolicy,PendingPolicy,RuleStamp},provider::StorageProvider,store::{PendingVersion,Store}};
use anyhow::{anyhow,bail,Context,Result};
use chrono::Utc;
use sha2::{Digest,Sha256};
use std::{collections::HashSet,fs,io::{self,Read},path::{Path,PathBuf},sync::{Mutex,RwLock,atomic::{AtomicBool,AtomicUsize,Ordering}},time::{Duration,Instant}};

const MAX_STRUCTURED_DIFF_BYTES:usize=8*1024*1024;
const MAX_ARCHIVE_IN_MEMORY:usize=128*1024*1024;
const LIVE_CHANGED_MESSAGE:&str="live file changed outside ledger; scan before restore";

/// Joins a ledger path (always "/"-separated) onto the workspace root one component at a time,
/// so that rule matching sees the same components as it does for absolute paths.
fn workspace_path(root:&Path,relative:&str)->PathBuf{
    relative.split('/').fold(root.to_path_buf(),|path,part|path.join(part))
}

/// Directory under the workspace `.ladon` folder that holds the temporary, backup and aside files of a restore.
/// scan, watch and tag-folder skip `.ladon` at any depth, and the directory is on the workspace volume, so renames stay atomic.
fn restore_staging_dir(root:&Path)->PathBuf{
    root.join(".ladon").join("restore")
}

/// Ledger form of a relative caller path: backslashes become "/", and empty and "." components are dropped.
pub fn normalise_relative(input:&str)->String{
    input.replace('\\',"/").split('/').filter(|part|!part.is_empty()&&*part!=".").collect::<Vec<_>>().join("/")
}

const DIGEST_CHUNK_BYTES:usize=64*1024;

/// SHA-256 hex digest of a file's bytes, the same value `digest` gives for those bytes, read in fixed-size chunks.
fn digest_file(path:&Path)->Result<String>{
    let mut file=fs::File::open(path)?;
    let mut hasher=Sha256::new();
    let mut buf=vec![0u8;DIGEST_CHUNK_BYTES];
    loop {
        let n=file.read(&mut buf)?;
        if n==0{break;}
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Places `src` at `dst` without replacing anything: an existing `dst` makes the call fail with `AlreadyExists`.
/// `src` stays in place. A hard link is used where the volume supports one, and a copy otherwise.
fn place_without_replacing(src:&Path,dst:&Path)->io::Result<()>{
    match fs::hard_link(src,dst){
        Err(e) if hard_links_unsupported(&e)=>copy_without_replacing(src,dst),
        other=>other,
    }
}

/// Whether the volume cannot create hard links: `Unsupported`, or the raw codes FAT and exFAT volumes report
/// (Windows ERROR_INVALID_FUNCTION and ERROR_NOT_SUPPORTED, Unix EPERM and EOPNOTSUPP).
/// A false match only costs a copy, because the copy never replaces either.
fn hard_links_unsupported(err:&io::Error)->bool{
    err.kind()==io::ErrorKind::Unsupported||matches!(err.raw_os_error(),Some(1)|Some(50)|Some(95))
}

/// Copies `src` to `dst` through a file created with `create_new`, so an existing `dst` is never replaced.
/// A file this call created is removed again when the copy fails.
fn copy_without_replacing(src:&Path,dst:&Path)->io::Result<()>{
    let mut out=fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
    let copied=fs::File::open(src).and_then(|mut input|io::copy(&mut input,&mut out));
    if copied.is_err(){
        drop(out);
        let _=fs::remove_file(dst);
    }
    copied.map(|_|())
}

/// Refuses a restore target whose existing ancestor directories pass through a link or resolve outside the workspace.
fn assert_restore_target_inside(root:&Path,target:&Path)->Result<()>{
    let mut ancestor=target.parent();
    while let Some(dir)=ancestor{
        if dir==root{return Ok(());}
        match fs::symlink_metadata(dir){
            Ok(meta)=>{
                if meta.file_type().is_symlink(){bail!("restore target is reached through a link: {}",dir.display());}
                if !dir.canonicalize()?.starts_with(root){bail!("path escapes the workspace");}
            }
            Err(err) if err.kind()==io::ErrorKind::NotFound=>{}
            Err(err)=>return Err(err.into()),
        }
        ancestor=dir.parent();
    }
    Ok(())
}

/// Time after which a content-related call starts a background traversal for nested rule files.
const NESTED_REFRESH_INTERVAL:Duration=Duration::from_secs(30);

/// What `refresh_policy_if_stale` tracks between calls.
struct RuleState {
    /// Stamp of the root and global rule files when the rule set in force was started.
    stamp:RuleStamp,
    /// When the last traversal was started by `refresh_policy_if_stale`, or when the engine was opened.
    last_scan:Instant,
    /// A background traversal that has not been installed yet.
    inflight:Option<PendingPolicy>,
}

pub struct Engine { pub root:PathBuf,pub cas:CasStore,pub store:Store, policy:RwLock<PendingPolicy>, policy_valid:AtomicBool, rules:Mutex<RuleState>, rule_traversals:AtomicUsize, nested_interval:Duration }

impl Engine {
    pub async fn init(path:&Path)->Result<Self>{
        let root=path.canonicalize().context("directory does not exist")?;
        fs::create_dir_all(root.join(".ladon"))?;
        let db=root.join(".ladon/meta.turso");
        let store=Store::new(db);
        store.init().await?;
        let rule=root.join(".LADONignore");
        if !rule.exists() {fs::write(rule,include_str!("../config/examples/.LADONignore"))?;}
        LadonPolicy::check_root(&root)?;
        let stamp=RuleStamp::read(&root);
        let policy=PendingPolicy::start(&root);
        Ok(Self{root:root.clone(),cas:CasStore::new(root.join(".ladon/cas")),store,policy:RwLock::new(policy),policy_valid:AtomicBool::new(true),rules:Mutex::new(RuleState{stamp,last_scan:Instant::now(),inflight:None}),rule_traversals:AtomicUsize::new(0),nested_interval:NESTED_REFRESH_INTERVAL})
    }
    pub async fn open(path:&Path)->Result<Self>{
        let root=path.canonicalize().context("workspace does not exist")?;
        let db=root.join(".ladon/meta.turso");
        if !db.exists(){bail!("workspace not initialized; run `ladon init`");}
        LadonPolicy::check_root(&root)?;
        let stamp=RuleStamp::read(&root);
        let policy=PendingPolicy::start(&root);
        Ok(Self{root:root.clone(),cas:CasStore::new(root.join(".ladon/cas")),store:Store::new(db),policy:RwLock::new(policy),policy_valid:AtomicBool::new(true),rules:Mutex::new(RuleState{stamp,last_scan:Instant::now(),inflight:None}),rule_traversals:AtomicUsize::new(0),nested_interval:NESTED_REFRESH_INTERVAL})
    }
    /// Reload `.LADONignore` after FS events without rescanning all rule files per capture.
    /// A missing root rule file also fails the rules in force, so capture refuses as archive does.
    pub fn reload_policy(&self)->Result<()> {
        let mut state=self.rules.lock().map_err(|_|anyhow!("rule state lock poisoned"))?;
        // A traversal that an earlier refresh started may have read older rule files, so it is dropped.
        state.inflight=None;
        if let Err(err)=LadonPolicy::check_root(&self.root){
            self.policy_valid.store(false,Ordering::SeqCst);
            if !LadonPolicy::has_root_rule_file(&self.root){
                *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=PendingPolicy::failed(format!("{err:#}"));
            }
            return Err(err);
        }
        state.stamp=RuleStamp::read(&self.root);
        state.last_scan=Instant::now();
        *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=PendingPolicy::start(&self.root);
        self.policy_valid.store(true,Ordering::SeqCst);
        Ok(())
    }
    /// Reloads the rule set when the root or global rule file changed, and starts a background traversal for the
    /// nested rule files once the nested interval has passed. Both remote gates call this before they decide.
    pub fn refresh_policy_if_stale(&self)->Result<()> {
        let stamp=RuleStamp::read(&self.root);
        let mut state=self.rules.lock().map_err(|_|anyhow!("rule state lock poisoned"))?;
        // A policy marked invalid is rechecked on every call, because restoring a broken file exactly can leave the stamp unchanged.
        if stamp!=state.stamp||!self.policy_valid.load(Ordering::SeqCst) {
            // A traversal in flight may have read the older root or global file, so it is dropped.
            state.inflight=None;
            if let Err(err)=LadonPolicy::check_root(&self.root){
                self.policy_valid.store(false,Ordering::SeqCst);
                if !LadonPolicy::has_root_rule_file(&self.root){
                    *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=PendingPolicy::failed(format!("{err:#}"));
                }
                return Err(err);
            }
            self.rule_traversals.fetch_add(1,Ordering::SeqCst);
            state.last_scan=Instant::now();
            *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=PendingPolicy::ready(LadonPolicy::load(&self.root));
            self.policy_valid.store(true,Ordering::SeqCst);
            state.stamp=stamp;
            return Ok(());
        }
        // Install a finished background traversal, whether it loaded or failed. Until then the rules in force stay in use.
        if let Some(pending)=state.inflight.take(){
            if pending.is_ready(){
                *self.policy.write().map_err(|_|anyhow!("policy lock poisoned"))?=pending;
            } else {
                state.inflight=Some(pending);
            }
        }
        if state.inflight.is_none()&&state.last_scan.elapsed()>=self.nested_interval{
            self.rule_traversals.fetch_add(1,Ordering::SeqCst);
            state.last_scan=Instant::now();
            state.inflight=Some(PendingPolicy::start(&self.root));
        }
        Ok(())
    }
    /// Number of rule-set traversals that `refresh_policy_if_stale` has started since the engine was opened.
    pub fn rule_traversals(&self)->usize{
        self.rule_traversals.load(Ordering::SeqCst)
    }
    /// True when the rule set in force is valid and has loaded. Every remote decision starts with this check.
    fn rules_usable(&self)->Result<bool>{
        if self.refresh_policy_if_stale().is_err(){return Ok(false);}
        if !self.policy_valid.load(Ordering::SeqCst){return Ok(false);}
        Ok(self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.with(|_| Ok(())).is_ok())
    }
    /// File-level gate for data that leaves the machine (design 13.1). Returns false when the rules are not usable,
    /// when the file is flagged sensitive, when the rule set excludes its path, or when its current content is excluded.
    /// Callers treat an error as a refusal too.
    pub async fn remote_read_allowed(&self,file:&FileRecord)->Result<bool>{
        if !self.rules_usable()?||file.sensitive{return Ok(false);}
        let external=self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?
            .with(|policy| policy.allow_external(Path::new(&file.relative_path))).is_ok();
        if !external{return Ok(false);}
        Ok(!self.content_is_excluded(&file.current_hash).await?)
    }
    /// Version-level gate. A historical version is refused when its own flag is set or when its content is excluded,
    /// whether or not the file is allowed now. Callers treat an error as a refusal too.
    pub async fn remote_version_allowed(&self,version:&VersionRecord)->Result<bool>{
        if !self.rules_usable()?||version.sensitive{return Ok(false);}
        Ok(!self.content_is_excluded(&version.content_hash).await?)
    }
    /// Path-level gate for a path that may be untracked (design 13.1). It returns `Ok(false)` when the rules in force
    /// are not usable, and when `LadonPolicy::allow_external` refuses the path. A tracked path is decided by
    /// `remote_read_allowed` on its record, so a flagged file, a tracked copy of excluded content and a directory alias
    /// are refused. Callers treat an error as a refusal too.
    pub async fn remote_path_allowed(&self,relative:&Path)->Result<bool>{
        if !self.rules_usable()?{return Ok(false);}
        let external=self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?
            .with(|policy| policy.allow_external(relative)).is_ok();
        if !external{return Ok(false);}
        let key=self.relative_for_input(relative).unwrap_or_else(|_|relative.to_string_lossy().replace('\\',"/"));
        match self.store.find_path(&key).await?{
            Some(file)=>self.remote_read_allowed(&file).await,
            None=>Ok(true),
        }
    }
    pub fn resolve(&self,path:&Path)->Result<(PathBuf,String)> {
        let path=if path.is_absolute(){path.to_path_buf()}else{self.root.join(path)};
        let canonical=assert_under_workspace(&self.root,&path)?;
        let rel=self.ledger_path(&canonical)?;
        Ok((canonical,rel))
    }
    /// Ledger form of a canonical path. A path outside the workspace or under `.ladon` or `.git` is refused.
    fn ledger_path(&self,canonical:&Path)->Result<String>{
        let relative=canonical.strip_prefix(&self.root).map_err(|_|anyhow!("path escapes the workspace"))?;
        if canonical.components().any(|x|x.as_os_str()==".ladon"||x.as_os_str()==".git") {bail!("internal file cannot be tracked");}
        Ok(relative.to_string_lossy().replace('\\',"/"))
    }
    /// Ledger path for a caller-supplied path. A path the root prefix matches as written is used directly.
    /// An absolute path the root does not prefix (a plain Windows path, since the root is verbatim `\\?\`) is matched
    /// through its canonical parent directory, so the file itself need not exist: a tracked file may have been deleted.
    /// Any other path goes through `resolve`.
    pub fn relative_for_input(&self,path:&Path)->Result<String>{
        if let Ok(relative)=path.strip_prefix(&self.root){
            return Ok(relative.to_string_lossy().replace('\\',"/"));
        }
        if let (true,Some(parent),Some(name))=(path.is_absolute(),path.parent(),path.file_name()){
            return self.ledger_path(&parent.canonicalize()?.join(name));
        }
        let (_,relative)=self.resolve(path)?;
        Ok(relative)
    }
    pub async fn capture(&self,path:&Path,actor:&str,reason:&str)->Result<CaptureResult>{
        let (path,relative)=self.resolve(path)?;
        let sensitive=self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.with(|policy| policy.excluded(&path))?;
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
        let walk=walkdir::WalkDir::new(&self.root).follow_links(false).into_iter()
            .filter_entry(|e| e.depth()==0 || !matches!(e.file_name().to_str(),Some(".ladon"|".git")));
        self.scan_walk(walk,actor).await
    }
    /// Captures each file the walk lists, then records deletions for tracked files it did not list.
    /// A walk error stops the scan before any deletion, so a directory that cannot be listed never marks its files deleted.
    /// A file that cannot be captured does not stop the scan: the others are still captured and the deletions still run,
    /// and the scan then fails naming every file it could not capture. A rule set that did not load stops the scan at once.
    async fn scan_walk(&self,walk:impl Iterator<Item=walkdir::Result<walkdir::DirEntry>>,actor:&str)->Result<Vec<CaptureResult>>{
        let mut out=Vec::new();let mut seen=HashSet::new();let mut failed=Vec::new();
        for step in walk {
            let entry=crate::policy::walk_entry(step)?;
            if entry.file_type().is_dir(){continue;}
            if !entry.file_type().is_file(){continue;}
            let rel=entry.path().strip_prefix(&self.root)?.to_string_lossy().replace('\\',"/");
            if rel.eq_ignore_ascii_case(".LADONignore")||rel.starts_with(".ladon/")||rel.starts_with(".git/"){continue;}
            self.rules_ready()?;
            match self.capture(entry.path(),actor,"scan").await {
                Ok(result)=>out.push(result),
                Err(err)=>failed.push(format!("{rel} ({err:#})")),
            }
            seen.insert(rel);
        }
        for f in self.store.list_files().await? {
            if !f.deleted&&!seen.contains(&f.relative_path) {
                self.store.mark_deleted(&f,actor,"missing during full scan").await?;
            }
        }
        if !failed.is_empty(){
            bail!("scan could not capture {} file(s): {}",failed.len(),failed.join("; "));
        }
        Ok(out)
    }
    /// Fails with the stored message when the rule set did not load, so no file is captured without rules.
    fn rules_ready(&self)->Result<()>{
        self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.with(|_| Ok(()))
    }
    pub async fn file_by_path(&self,path:&str)->Result<FileRecord>{
        self.store.find_path(&path.replace('\\',"/")).await?.ok_or_else(||anyhow!("file not found in ledger"))
    }
    /// True when the bytes behind `hash` must stay off external storage: the hash is marked sensitive,
    /// or a path that has ever held these bytes is excluded by the current rules. Stored flags can predate a rule.
    pub async fn content_is_excluded(&self,hash:&str)->Result<bool>{
        let holders=self.store.paths_for_hash(hash).await?;
        let held_by_excluded_path=self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.with(|policy| {
            for rel in &holders {if policy.excluded(&workspace_path(&self.root,rel))?{return Ok(true);}}
            Ok(false)
        })?;
        Ok(held_by_excluded_path||self.store.is_hash_sensitive(hash).await?)
    }
    pub async fn archive_version(&self,version_id:&str,provider:&dyn StorageProvider,evict_local:bool)->Result<StorageObject>{
        if !self.policy_valid.load(Ordering::SeqCst){bail!("policy reload failed: all external archive transfers blocked");}
        let version=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        let current=self.store.list_files().await?.into_iter().find(|f|f.id==version.file_id)
            .ok_or_else(||anyhow!("missing tracked file"))?;
        self.policy.read().map_err(|_|anyhow!("policy lock poisoned"))?.with(|policy| policy.allow_external(Path::new(&current.relative_path)))?;
        if version.sensitive||self.content_is_excluded(&version.content_hash).await? {bail!("sensitive version blocked from external storage");}
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
            file.id,v.number,&v.content_hash,file.relative_path,self.root.join(&file.relative_path).exists()))
    }
    pub async fn restore(&self,version_id:&str,approve:bool,provider:Option<&dyn StorageProvider>)->Result<CaptureResult>{
        self.restore_with_hook(version_id,approve,provider,|_|{},|_|{},|_|Ok(()),|_|{}).await
    }
    /// `before_backup` runs after the digest check and before the live file is moved aside, so tests can stand in for a concurrent writer.
    /// `before_place` runs once the moved-aside bytes pass the digest check, just before the restored file is placed at the target.
    /// `before_capture` runs once the restored bytes are in place; an error it returns stands in for a failed capture.
    /// `before_rollback_removal` runs in the rollback once the moved-aside target is confirmed to hold the restored bytes, just before they are removed.
    async fn restore_with_hook(&self,version_id:&str,approve:bool,provider:Option<&dyn StorageProvider>,before_backup:impl FnOnce(&Path),before_place:impl FnOnce(&Path),before_capture:impl FnOnce(&Path)->Result<()>,before_rollback_removal:impl FnOnce(&Path))->Result<CaptureResult>{
        if !approve {bail!("restore requires explicit --approve");}
        let v=self.store.version(version_id).await?.ok_or_else(||anyhow!("unknown version"))?;
        let file=self.store.list_files().await?.into_iter().find(|f|f.id==v.file_id).ok_or_else(||anyhow!("file not found"))?;
        // TOCTOU guard: refuse if current file diverged from last captured hash.
        let target=self.root.join(&file.relative_path);
        assert_restore_target_inside(&self.root,&target)?;
        if target.exists() && digest_file(&target)?!=file.current_hash {bail!(LIVE_CHANGED_MESSAGE);}
        let data=self.load_version(version_id,provider).await?;
        if let Some(parent)=target.parent(){fs::create_dir_all(parent)?;}
        // Temporary, backup and aside files live in the restore staging directory, so a watch or scan never records them.
        let name=target.file_name().ok_or_else(||anyhow!("restore target has no file name"))?;
        let staging=restore_staging_dir(&self.root);
        fs::create_dir_all(&staging)?;
        let staged=|kind:&str|staging.join(name).with_extension(format!("ladon-{kind}-{}",uuid::Uuid::new_v4()));
        let tmp=staged("restore");
        fs::write(&tmp,&data)?;
        // The restored file is placed without replacing anything, so a file that appears at the target during the restore is kept.
        // Preserve the last file until the restored version has a durable DB record.
        let backup=staged("backup");
        before_backup(&target);
        let had_target=target.exists();
        if had_target {
            fs::rename(&target,&backup)?;
            // The digest check above ran before the load and the move, so re-check the bytes that were moved aside.
            let moved_intact=digest_file(&backup).map(|hash|hash==file.current_hash).unwrap_or(false);
            if !moved_intact {
                let _=fs::remove_file(&tmp);
                fs::rename(&backup,&target).with_context(|| format!("live file changed during restore and could not be put back; its bytes are at {}",backup.display()))?;
                bail!(LIVE_CHANGED_MESSAGE);
            }
        }
        before_place(&target);
        let placed=place_without_replacing(&tmp,&target);
        let _=fs::remove_file(&tmp);
        if let Err(e)=placed{
            if e.kind()==io::ErrorKind::AlreadyExists{
                // The file that appeared at the target is left as it is; the pre-restore bytes stay in the backup.
                let backup_note=if had_target{format!("; the pre-restore bytes are at {}",backup.display())}else{String::new()};
                bail!("a file appeared at {} during restore and was left as it is{}",target.display(),backup_note);
            }
            if had_target && place_without_replacing(&backup,&target).is_ok(){let _=fs::remove_file(&backup);}
            return Err(e.into());
        }
        let captured=match before_capture(&target){
            Ok(())=>self.capture(&target,"restore",&format!("restore from version {}",v.id)).await,
            Err(err)=>Err(err),
        };
        match captured {
            Ok(result)=>{
                // The restored version is already recorded, so a backup that cannot be removed stays beside the target instead of failing the restore.
                if had_target{let _=fs::remove_file(&backup);}
                Ok(result)
            }
            Err(err)=>{
                // Roll back only while the target still holds the restored bytes; anything written during capture is a live edit.
                // The target is moved aside before its bytes are checked, so a write that lands after the check is never removed with them.
                let restored_digest=digest(&data);
                let backup_note=if had_target{format!("; the pre-restore bytes are at {}",backup.display())}else{String::new()};
                let aside=staged("discard");
                let moved_aside=fs::rename(&target,&aside).is_ok();
                let aside_holds_restore=moved_aside&&digest_file(&aside).map(|hash|hash==restored_digest).unwrap_or(false);
                if !aside_holds_restore {
                    if moved_aside {
                        fs::rename(&aside,&target).with_context(|| format!("live file changed during rollback and could not be put back; its bytes are at {}",aside.display()))?;
                    }
                    bail!("restore capture failed ({:#}); the live file at {} no longer holds the restored bytes and was left as it is{}",err,target.display(),backup_note);
                }
                before_rollback_removal(&target);
                let _=fs::remove_file(&aside);
                // An edit written to the target path during rollback keeps that path; the backup is not moved over it.
                if had_target && target.exists() {
                    bail!("restore capture failed ({:#}); an edit was written to {} during rollback and was left as it is{}",err,target.display(),backup_note);
                }
                if had_target{let _=fs::rename(&backup,&target);}
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restore_rolls_back_when_live_file_changes_after_digest_check() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(&version_id, true, None, |path| fs::write(path, "live edit").unwrap(), |_| {}, |_| Ok(()), |_| {})
            .await;

        let err = match outcome {
            Ok(_) => panic!("restore replaced a live edit made after the digest check"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("live file changed outside ledger"), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), "live edit");
        let events = e.store.events(&first.file_id).await.unwrap();
        let kinds: Vec<&str> = events.iter().map(|ev| ev.kind.as_str()).collect();
        assert_eq!(kinds, vec!["created", "modified"], "a rolled-back restore must not be recorded");
        let leftovers: Vec<String> = fs::read_dir(restore_staging_dir(tmp.path()))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("ladon-restore") || name.contains("ladon-backup"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[tokio::test]
    async fn restore_keeps_live_edit_when_capture_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(&version_id, true, None, |_| {}, |_| {}, |path| {
                fs::write(path, "live edit").unwrap();
                Err(anyhow!("simulated capture failure"))
            }, |_| {})
            .await;

        let err = match outcome {
            Ok(_) => panic!("a failed capture must not report a restore"),
            Err(err) => err,
        };
        assert_eq!(fs::read_to_string(&file).unwrap(), "live edit", "the rollback must not discard a live edit");
        let backups: Vec<PathBuf> = fs::read_dir(restore_staging_dir(&e.root))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("x.ladon-backup-"))
            .collect();
        assert_eq!(backups.len(), 1, "the pre-restore bytes must stay in a backup: {backups:?}");
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "two");
        assert!(err.to_string().contains(&backups[0].display().to_string()), "{err}");
    }

    #[tokio::test]
    async fn restore_keeps_edit_written_during_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(
                &version_id,
                true,
                None,
                |_| {},
                |_| {},
                |_| Err(anyhow!("simulated capture failure")),
                |path| fs::write(path, "user edit").unwrap(),
            )
            .await;

        let err = match outcome {
            Ok(_) => panic!("a failed capture must not report a restore"),
            Err(err) => err,
        };
        assert_eq!(fs::read_to_string(&file).unwrap(), "user edit", "the rollback must not remove an edit written during rollback");
        let backups: Vec<PathBuf> = fs::read_dir(restore_staging_dir(&e.root))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("x.ladon-backup-"))
            .collect();
        assert_eq!(backups.len(), 1, "the pre-restore bytes must stay in a backup: {backups:?}");
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "two");
        assert!(err.to_string().contains(&backups[0].display().to_string()), "{err}");
    }

    #[tokio::test]
    async fn restore_keeps_temporary_files_out_of_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        // A concurrent watch or scan would record any restore file that sits in the workspace while the capture runs.
        let mut seen_in_root: Vec<String> = Vec::new();
        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(&version_id, true, None, |_| {}, |_| {}, |_| {
                for entry in fs::read_dir(tmp.path()).unwrap() {
                    let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                    if ["ladon-backup", "ladon-restore", "ladon-discard"].iter().any(|marker| name.contains(marker)) {
                        seen_in_root.push(name);
                    }
                }
                Ok(())
            }, |_| {})
            .await;

        outcome.expect("restore should succeed");
        assert_eq!(fs::read_to_string(&file).unwrap(), "one");
        assert!(seen_in_root.is_empty(), "restore left temporary files in the workspace root: {seen_in_root:?}");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn restore_succeeds_when_backup_cannot_be_removed() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        // A handle opened without FILE_SHARE_DELETE makes Windows refuse to remove the backup.
        let mut held_backup = None;
        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(&version_id, true, None, |_| {}, |_| {}, |_| {
                let backup = fs::read_dir(restore_staging_dir(tmp.path()))
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|p| p.file_name().unwrap().to_string_lossy().starts_with("x.ladon-backup-"))
                    .unwrap();
                held_backup = Some(fs::OpenOptions::new().read(true).share_mode(0).open(&backup).unwrap());
                Ok(())
            }, |_| {})
            .await;

        let restored = outcome.expect("the restored version is already recorded, so a backup that cannot be removed must not fail the restore");
        assert_eq!(restored.status, "captured");
        assert!(restored.version_id.is_some());
        assert_eq!(fs::read_to_string(&file).unwrap(), "one");
        drop(held_backup);
        let backups: Vec<PathBuf> = fs::read_dir(restore_staging_dir(tmp.path()))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("x.ladon-backup-"))
            .collect();
        assert_eq!(backups.len(), 1, "the pre-restore bytes must stay in the backup: {backups:?}");
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "two");
    }

    #[tokio::test]
    async fn restore_does_not_replace_file_created_during_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("x.txt");
        fs::write(&file, "one").unwrap();
        let first = e.capture(&file, "human", "v1").await.unwrap();
        fs::write(&file, "two").unwrap();
        e.capture(&file, "human", "v2").await.unwrap();

        // A concurrent save creates the target after the moved-aside bytes are checked and before the restored file is placed.
        let version_id = first.version_id.clone().unwrap();
        let outcome = e
            .restore_with_hook(&version_id, true, None, |_| {}, |path| fs::write(path, "user bytes").unwrap(), |_| Ok(()), |_| {})
            .await;

        assert_eq!(fs::read_to_string(&file).unwrap(), "user bytes", "restore replaced a file created during restore");
        let err = match outcome {
            Ok(_) => panic!("a restore that cannot place the restored file must not report success"),
            Err(err) => err,
        };
        let backups: Vec<PathBuf> = fs::read_dir(restore_staging_dir(&e.root))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with("x.ladon-backup-"))
            .collect();
        assert_eq!(backups.len(), 1, "the pre-restore bytes must stay in a backup: {backups:?}");
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "two");
        assert!(err.to_string().contains(&backups[0].display().to_string()), "{err}");
        assert!(err.to_string().contains(&e.root.join("x.txt").display().to_string()), "{err}");
        let events = e.store.events(&first.file_id).await.unwrap();
        let kinds: Vec<&str> = events.iter().map(|ev| ev.kind.as_str()).collect();
        assert_eq!(kinds, vec!["created", "modified"], "a restore that failed to place its file must not be recorded");
    }

    #[test]
    fn copy_without_replacing_keeps_existing_target() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        fs::write(&src, "restored").unwrap();
        fs::write(&dst, "user bytes").unwrap();

        let err = copy_without_replacing(&src, &dst).expect_err("an existing target must not be replaced");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "user bytes");

        let fresh = tmp.path().join("fresh.txt");
        copy_without_replacing(&src, &fresh).unwrap();
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "restored");
    }

    #[tokio::test]
    async fn scan_records_no_deletion_when_walk_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        let file = tmp.path().join("kept.txt");
        fs::write(&file, "one").unwrap();
        e.capture(&file, "human", "v1").await.unwrap();

        // A walk rooted at a missing directory yields a real walk error and no entries, as an unlistable directory does.
        let walk = walkdir::WalkDir::new(tmp.path().join("unlistable")).into_iter();
        let outcome = e.scan_walk(walk, "scanner").await;

        let record = e.file_by_path("kept.txt").await.unwrap();
        assert!(!record.deleted, "a walk error must not mark a tracked file deleted");
        let err = outcome.expect_err("the scan must report the walk error");
        assert!(err.to_string().contains("failed to scan the workspace"), "{err}");
    }

    #[test]
    fn normalise_relative_drops_dot_components() {
        assert_eq!(normalise_relative(r".\docs\a.md"), "docs/a.md");
        assert_eq!(normalise_relative("./docs/a.md"), "docs/a.md");
        assert_eq!(normalise_relative("docs//a.md"), "docs/a.md");
        assert_eq!(normalise_relative(r"docs\a.md"), "docs/a.md");
    }

    #[tokio::test]
    async fn digest_file_matches_whole_file_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("large.bin");
        let first: Vec<u8> = (0..300 * 1024).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &first).unwrap();
        assert_eq!(digest_file(&path).unwrap(), digest(&first));

        let empty = tmp.path().join("empty.bin");
        fs::write(&empty, b"").unwrap();
        assert_eq!(digest_file(&empty).unwrap(), digest(b""));

        let _env = crate::policy::lock_global_ignore();
        let e = Engine::init(tmp.path()).await.unwrap();
        let captured = e.capture(&path, "human", "v1").await.unwrap();
        fs::write(&path, vec![7u8; 300 * 1024]).unwrap();
        e.capture(&path, "human", "v2").await.unwrap();

        let restored = e.restore(captured.version_id.as_deref().unwrap(), true, None).await.unwrap();
        assert_eq!(restored.status, "captured");
        assert_eq!(fs::read(&path).unwrap(), first);
    }

    /// Sets the last traversal time back, so the next content-related call is past the nested interval.
    fn backdate_last_scan(e: &Engine) {
        e.rules.lock().unwrap().last_scan = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
    }

    /// Waits until the background traversal in flight has finished, or until none is in flight.
    fn wait_for_background_scan(e: &Engine) {
        let started = Instant::now();
        loop {
            let finished = e.rules.lock().unwrap().inflight.as_ref().map_or(true, |p| p.is_ready());
            if finished {
                return;
            }
            assert!(started.elapsed() < Duration::from_secs(30), "background rule scan did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[tokio::test]
    async fn ut_nested_rule_edit_is_applied_by_background_scan() {
        let _env = crate::policy::lock_global_ignore();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".LADONignore"), "").unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub").join("a.txt"), "nested one").unwrap();
        e.capture(&tmp.path().join("sub").join("a.txt"), "human", "ut859").await.unwrap();
        let file = e.file_by_path("sub/a.txt").await.unwrap();
        assert!(e.remote_read_allowed(&file).await.unwrap());

        fs::write(tmp.path().join("sub").join(".LADONignore"), "a.txt\n").unwrap();
        backdate_last_scan(&e);
        let base = e.rule_traversals();
        // The first call starts the background traversal and answers from the rules in force.
        assert!(e.remote_read_allowed(&file).await.unwrap(), "the rules in force stay in use while the new ones load");
        assert_eq!(e.rule_traversals(), base + 1, "the nested edit starts one background traversal");
        // A second call does not start another traversal, whether or not the first has finished.
        e.remote_read_allowed(&file).await.unwrap();
        assert_eq!(e.rule_traversals(), base + 1);

        wait_for_background_scan(&e);
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "the finished traversal is in force after the next call");
        assert_eq!(e.rule_traversals(), base + 1);
    }

    #[tokio::test]
    async fn ut_failed_nested_scan_denies_until_fixed() {
        let _env = crate::policy::lock_global_ignore();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".LADONignore"), "").unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub").join("b.txt"), "nested two").unwrap();
        e.capture(&tmp.path().join("sub").join("b.txt"), "human", "ut859").await.unwrap();
        let file = e.file_by_path("sub/b.txt").await.unwrap();

        fs::write(tmp.path().join("sub").join(".LADONignore"), "[z-a]\n").unwrap();
        backdate_last_scan(&e);
        let base = e.rule_traversals();
        e.remote_read_allowed(&file).await.unwrap();
        assert_eq!(e.rule_traversals(), base + 1);
        wait_for_background_scan(&e);
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "a failed nested traversal refuses transfers");

        fs::write(tmp.path().join("sub").join(".LADONignore"), "# fixed\n").unwrap();
        backdate_last_scan(&e);
        assert!(!e.remote_read_allowed(&file).await.unwrap(), "the failed rules stay in force until a traversal succeeds");
        assert_eq!(e.rule_traversals(), base + 2);
        wait_for_background_scan(&e);
        assert!(e.remote_read_allowed(&file).await.unwrap(), "the repaired traversal restores transfers");
        assert_eq!(e.rule_traversals(), base + 2);
    }

    #[tokio::test]
    async fn ut_reload_policy_discards_a_finished_older_traversal() {
        let _env = crate::policy::lock_global_ignore();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".LADONignore"), "").unwrap();
        let e = Engine::init(tmp.path()).await.unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub").join("a.txt"), "nested one").unwrap();
        e.capture(&tmp.path().join("sub").join("a.txt"), "human", "tc1042").await.unwrap();
        let file = e.file_by_path("sub/a.txt").await.unwrap();

        // The first call starts a traversal that reads the rules as they are now; it finishes but is not installed yet.
        backdate_last_scan(&e);
        assert!(e.remote_read_allowed(&file).await.unwrap());
        wait_for_background_scan(&e);

        fs::write(tmp.path().join("sub").join(".LADONignore"), "a.txt\n").unwrap();
        e.reload_policy().unwrap();
        assert!(
            !e.remote_read_allowed(&file).await.unwrap(),
            "the rule set that reload_policy started stays in force; the older traversal is not installed over it"
        );
    }
}
