use crate::{crypto::digest, model::{EventRecord, FileRecord, VersionRecord, StorageObject}};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use rand::Rng;
use std::{fmt, path::{Path, PathBuf}, time::Duration};
use turso::{params, Builder};
use uuid::Uuid;

const MIGRATION_004: &str = include_str!("../sql/004_document_tasks.sql");
const COUNTED_TABLES: [&str; 5] = ["resources", "versions", "doc_tasks", "annotations", "governance_events"];
const DT_MIGRATION_APPLIED: &str = "dt.migration.applied";
const CONNECT_ATTEMPTS: u32 = 6;
const BACKOFF_BASE_MS: u64 = 25;
const BACKOFF_CAP_MS: u64 = 400;
const MAX_JITTER_PERMILLE: u64 = 500;

#[derive(Debug, Clone)]
pub struct Store { pub path: PathBuf }

/// Result of `Store::ensure_schema`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaOutcome {
    /// Migration 4 was already recorded; nothing ran.
    Current,
    /// Migration 4 ran; `backup` states what was done before its first statement.
    Applied { backup: BackupState },
}

/// State of the backup taken before migration 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupState {
    /// The library had no data rows, so there was nothing to back up.
    NotNeeded,
    /// The backup was copied and its row counts match the source.
    Verified { path: PathBuf },
    /// The backup could not be verified; `reason` says why.
    Unverified { reason: String },
}

/// The migration backup could not be verified and the caller did not accept that.
/// Displayed as `E_MIGRATION_BACKUP`.
#[derive(Debug)]
pub struct MigrationBackupUnverified {
    pub reason: String,
}

impl fmt::Display for MigrationBackupUnverified {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E_MIGRATION_BACKUP: the migration backup could not be verified ({})", self.reason)
    }
}

impl std::error::Error for MigrationBackupUnverified {}

/// Every connection attempt failed with a lock error. Displayed as `E_STORE_BUSY`.
#[derive(Debug)]
pub struct StoreBusy {
    pub attempts: u32,
    pub detail: String,
}

impl fmt::Display for StoreBusy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "E_STORE_BUSY: the database is in use by another process; gave up after {} attempts ({})",
            self.attempts, self.detail
        )
    }
}

impl std::error::Error for StoreBusy {}

/// Opens a turso connection to a database file. `TursoOpener` is the production implementation;
/// tests substitute one that fails with lock errors.
#[async_trait]
pub(crate) trait Opener: Send + Sync {
    async fn open(&self, path: &str) -> std::result::Result<turso::Connection, turso::Error>;
}

pub(crate) struct TursoOpener;

#[async_trait]
impl Opener for TursoOpener {
    async fn open(&self, path: &str) -> std::result::Result<turso::Connection, turso::Error> {
        let db = Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        conn.execute("PRAGMA foreign_keys = ON", ()).await?;
        Ok(conn)
    }
}

fn is_lock_error(err: &turso::Error) -> bool {
    matches!(err, turso::Error::Busy(_)) || err.to_string().contains("locked by another process")
}

/// Sleep before retry number `retry` (1 to 5): 25 ms doubling up to 400 ms, plus 0 to 50 percent
/// of that value as `jitter_permille` (0 to 500).
pub(crate) fn backoff_ms(retry: u32, jitter_permille: u64) -> u64 {
    let shift = (retry.max(1) - 1).min(10);
    let base = (BACKOFF_BASE_MS << shift).min(BACKOFF_CAP_MS);
    base + base * jitter_permille.min(MAX_JITTER_PERMILLE) / 1000
}

/// Opens `path` with up to `CONNECT_ATTEMPTS` attempts. Only lock errors are retried.
pub(crate) async fn open_with_retry(opener: &dyn Opener, path: &str) -> Result<turso::Connection> {
    let mut attempt: u32 = 1;
    loop {
        match opener.open(path).await {
            Ok(conn) => return Ok(conn),
            Err(err) if is_lock_error(&err) => {
                if attempt >= CONNECT_ATTEMPTS {
                    return Err(StoreBusy { attempts: attempt, detail: err.to_string() }.into());
                }
                let jitter = rand::thread_rng().gen_range(0..=MAX_JITTER_PERMILLE);
                tokio::time::sleep(Duration::from_millis(backoff_ms(attempt, jitter))).await;
                attempt += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingVersion {
    pub path: String,
    pub hash: String,
    pub size: i64,
    pub sections: Vec<String>,
    pub actor: String,
    pub reason: String,
    pub sensitive: bool,
}

impl Store {
    pub fn new(path: PathBuf)->Self{Self{path}}
    pub(crate) async fn connect(&self)->Result<turso::Connection>{
        self.connect_via(&TursoOpener).await
    }
    pub(crate) async fn connect_via(&self,opener:&dyn Opener)->Result<turso::Connection>{
        let p=self.path.to_string_lossy().to_string();
        open_with_retry(opener,&p).await
    }
    pub async fn init(&self)->Result<()> {
        if let Some(parent)=self.path.parent(){std::fs::create_dir_all(parent)?;}
        let conn=self.connect().await?;
        for statement in include_str!("../sql/001_initial.sql").split(';') {
            let statement=statement.trim();
            if !statement.is_empty() {conn.execute(statement,()).await?;}
        }
        conn.execute("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",()).await?;
        let mut rows=conn.query("SELECT COUNT(*) FROM schema_migrations WHERE version=2",()).await?;
        let is_applied=rows.next().await?.map(|r|r.get::<i64>(0).unwrap_or(0)>0).unwrap_or(false);
        drop(rows);
        if !is_applied {
            // Safe to retry: all CREATE TABLE / INDEX statements have IF NOT EXISTS.
            for statement in include_str!("../sql/002_collaboration_shell_agent.sql").split(';') {
                let statement=statement.trim();
                if !statement.is_empty() {conn.execute(statement,()).await?;}
            }
            conn.execute("INSERT OR IGNORE INTO schema_migrations(version,applied_at) VALUES (2,datetime('now'))",()).await?;
        }
        Ok(())
    }
    /// Applies migration 4 (document tasks). This is the only path that applies it.
    /// The migration backup runs first when the library holds data; an unverified backup stops
    /// the migration unless `accept_unverified_backup` is set, in which case the event records it.
    pub async fn ensure_schema(&self,accept_unverified_backup:bool)->Result<SchemaOutcome>{
        self.init().await?;
        let conn=self.connect().await?;
        if scalar_count(&conn,"SELECT COUNT(*) FROM schema_migrations WHERE version=4").await?>0 {
            return Ok(SchemaOutcome::Current);
        }
        let counts=table_counts(&conn).await?;
        let backup=if counts.iter().all(|(_,rows)|*rows==0) {
            BackupState::NotNeeded
        } else {
            self.backup_before_migration(&counts).await?
        };
        if let BackupState::Unverified{reason}=&backup {
            if !accept_unverified_backup {
                return Err(MigrationBackupUnverified{reason:reason.clone()}.into());
            }
        }
        for statement in MIGRATION_004.split(';') {
            let statement=statement.trim();
            if !statement.is_empty() {conn.execute(statement,()).await?;}
        }
        record_migration_event(&conn,&backup,&counts).await?;
        conn.execute("INSERT OR IGNORE INTO schema_migrations(version,applied_at) VALUES (4,datetime('now'))",()).await?;
        Ok(SchemaOutcome::Applied{backup})
    }
    /// Copies the database and its write-ahead log to `backup/` beside the file and compares the
    /// five counted tables of the copy with `counts`, read from the open source connection.
    async fn backup_before_migration(&self,counts:&[(&'static str,i64)])->Result<BackupState> {
        let backup_dir=self.path.parent().unwrap_or_else(||Path::new(".")).join("backup");
        // An earlier backup file is never overwritten: a new attempt takes the first free name.
        let mut copy_no=1u32;
        let (main_copy,wal_copy)=loop{
            let suffix=if copy_no==1{String::new()}else{format!("-{copy_no}")};
            let main=backup_dir.join(format!("meta.turso.pre-004{suffix}"));
            if !main.is_file(){break (main,backup_dir.join(format!("meta.turso.pre-004{suffix}-wal")));}
            copy_no+=1;
        };
        if let Err(err)=copy_backup_pair(&self.path,&wal_file_of(&self.path),&main_copy,&wal_copy) {
            return Ok(BackupState::Unverified{reason:format!("copy failed: {err}")});
        }
        let copy=match open_with_retry(&TursoOpener,&main_copy.to_string_lossy()).await {
            Ok(copy)=>copy,
            Err(err)=>return Ok(BackupState::Unverified{reason:format!("copy cannot be opened: {err}")}),
        };
        let copied=match table_counts(&copy).await {
            Ok(copied)=>copied,
            Err(err)=>return Ok(BackupState::Unverified{reason:format!("copy cannot be read: {err}")}),
        };
        for ((table,source_rows),(_,copy_rows)) in counts.iter().zip(copied.iter()) {
            if source_rows!=copy_rows {
                return Ok(BackupState::Unverified{reason:format!("row count of {table} differs: source {source_rows}, copy {copy_rows}")});
            }
        }
        Ok(BackupState::Verified{path:main_copy})
    }
    pub async fn find_path(&self,path:&str)->Result<Option<FileRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,relative_path,current_hash,version_no,sensitive,deleted FROM resources WHERE relative_path=?1",[path]).await?;
        if let Some(row)=r.next().await?{
            Ok(Some(FileRecord{id:row.get(0)?,relative_path:row.get(1)?,current_hash:row.get(2)?,version_no:row.get(3)?,sensitive:row.get::<i64>(4)?!=0,deleted:row.get::<i64>(5)?!=0}))
        } else {Ok(None)}
    }
    pub async fn file_by_id(&self,file_id:&str)->Result<Option<FileRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,relative_path,current_hash,version_no,sensitive,deleted FROM resources WHERE id=?1",[file_id]).await?;
        if let Some(row)=r.next().await?{Ok(Some(parse_file(&row)?))}else{Ok(None)}
    }
    pub async fn list_files(&self)->Result<Vec<FileRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,relative_path,current_hash,version_no,sensitive,deleted FROM resources ORDER BY relative_path",()).await?;
        let mut out=Vec::new();
        while let Some(row)=r.next().await?{
            out.push(FileRecord{id:row.get(0)?,relative_path:row.get(1)?,current_hash:row.get(2)?,version_no:row.get(3)?,sensitive:row.get::<i64>(4)?!=0,deleted:row.get::<i64>(5)?!=0});
        }
        Ok(out)
    }
    pub async fn latest_version(&self,file_id:&str)->Result<Option<VersionRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,file_id,number,content_hash,byte_size,sensitive,changed_sections_json,actor,reason,created_at FROM versions WHERE file_id=?1 ORDER BY number DESC LIMIT 1",[file_id]).await?;
        if let Some(row)=r.next().await?{Ok(Some(parse_version(&row)?))}else{Ok(None)}
    }
    pub async fn version(&self,version_id:&str)->Result<Option<VersionRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,file_id,number,content_hash,byte_size,sensitive,changed_sections_json,actor,reason,created_at FROM versions WHERE id=?1",[version_id]).await?;
        if let Some(row)=r.next().await?{Ok(Some(parse_version(&row)?))}else{Ok(None)}
    }
    pub async fn versions(&self,file_id:&str)->Result<Vec<VersionRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,file_id,number,content_hash,byte_size,sensitive,changed_sections_json,actor,reason,created_at FROM versions WHERE file_id=?1 ORDER BY number DESC",[file_id]).await?;
        let mut out=Vec::new();while let Some(row)=r.next().await?{out.push(parse_version(&row)?);}Ok(out)
    }
    pub async fn events(&self,file_id:&str)->Result<Vec<EventRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,file_id,version_id,kind,old_path,new_path,timestamp,actor,reason,previous_hash,current_hash,ledger_prev_hash,ledger_hash FROM events WHERE file_id=?1 ORDER BY seq",[file_id]).await?;
        let mut out=Vec::new();
        while let Some(row)=r.next().await?{
            out.push(EventRecord{id:row.get(0)?,file_id:row.get(1)?,version_id:row.get(2)?,kind:row.get(3)?,old_path:row.get(4)?,new_path:row.get(5)?,timestamp:row.get(6)?,actor:row.get(7)?,reason:row.get(8)?,previous_hash:row.get(9)?,current_hash:row.get(10)?,ledger_prev_hash:row.get(11)?,ledger_hash:row.get(12)?});
        }
        Ok(out)
    }
    pub async fn append_version(&self,p:PendingVersion)->Result<(String,String,String)> {
        let mut conn=self.connect().await?;
        let tx=conn.transaction().await?;
        let mut rs=tx.query("SELECT id,version_no,current_hash,deleted FROM resources WHERE relative_path=?1",[p.path.as_str()]).await?;
        let existing=if let Some(row)=rs.next().await?{Some((row.get::<String>(0)?,row.get::<i64>(1)?,row.get::<String>(2)?,row.get::<i64>(3)?))}else{None};
        drop(rs);
        let (file_id,number,previous,was_deleted) = if let Some((id,n,h,d))=existing{(id,n+1,h,d!=0)}else{(Uuid::new_v4().to_string(),1,String::new(),false)};
        if number>1 && previous==p.hash && !was_deleted { return Err(anyhow!("no content change")); }
        let now=Utc::now().to_rfc3339();
        let vid=Uuid::new_v4().to_string(); let eid=Uuid::new_v4().to_string();
        let kind=if number==1 {"created"}else if was_deleted{"restored"}else{"modified"};
        tx.execute("INSERT INTO resources(id,relative_path,current_hash,version_no,sensitive,deleted,updated_at) VALUES(?1,?2,?3,?4,?5,0,?6) ON CONFLICT(relative_path) DO UPDATE SET current_hash=excluded.current_hash,version_no=excluded.version_no,sensitive=excluded.sensitive,deleted=0,updated_at=excluded.updated_at",
            params![file_id.as_str(),p.path.as_str(),p.hash.as_str(),number,if p.sensitive{1i64}else{0i64},now.as_str()]).await?;
        let sections_json=serde_json::to_string(&p.sections)?;
        tx.execute("INSERT INTO versions(id,file_id,number,content_hash,byte_size,sensitive,changed_sections_json,actor,reason,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![vid.as_str(),file_id.as_str(),number,p.hash.as_str(),p.size,if p.sensitive{1i64}else{0i64},sections_json.as_str(),p.actor.as_str(),p.reason.as_str(),now.as_str()]).await?;
        let mut prev_row=tx.query("SELECT ledger_hash FROM events WHERE file_id=?1 ORDER BY seq DESC LIMIT 1",[file_id.as_str()]).await?;
        let chain_prev=if let Some(row)=prev_row.next().await?{row.get::<String>(0)?}else{String::new()};drop(prev_row);
        let chain_hash=event_hash(&chain_prev,&eid,&file_id,&vid,kind,&p.path,&p.path,&now,&p.actor,&p.reason,&previous,&p.hash);
        tx.execute("INSERT INTO events(id,file_id,version_id,kind,old_path,new_path,timestamp,actor,reason,previous_hash,current_hash,ledger_prev_hash,ledger_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![eid.as_str(),file_id.as_str(),vid.as_str(),kind,p.path.as_str(),p.path.as_str(),now.as_str(),p.actor.as_str(),p.reason.as_str(),previous.as_str(),p.hash.as_str(),chain_prev.as_str(),chain_hash.as_str()]).await?;
        tx.commit().await?;
        Ok((file_id,vid,eid))
    }
    pub async fn mark_deleted(&self,file:&FileRecord,actor:&str,reason:&str)->Result<()> {
        if file.deleted{return Ok(());}
        let mut conn=self.connect().await?;let tx=conn.transaction().await?;
        let now=Utc::now().to_rfc3339();let eid=Uuid::new_v4().to_string();
        let mut rs=tx.query("SELECT id FROM versions WHERE file_id=?1 ORDER BY number DESC LIMIT 1",[file.id.as_str()]).await?;
        let vid=rs.next().await?.ok_or_else(||anyhow!("missing version"))?.get::<String>(0)?;drop(rs);
        let mut r=tx.query("SELECT ledger_hash FROM events WHERE file_id=?1 ORDER BY seq DESC LIMIT 1",[file.id.as_str()]).await?;
        let prev=if let Some(row)=r.next().await?{row.get::<String>(0)?}else{String::new()};drop(r);
        let next=event_hash(&prev,&eid,&file.id,&vid,"deleted",&file.relative_path,"",&now,actor,reason,&file.current_hash,"");
        tx.execute("UPDATE resources SET deleted=1,updated_at=?1 WHERE id=?2",params![now.as_str(),file.id.as_str()]).await?;
        tx.execute("INSERT INTO events(id,file_id,version_id,kind,old_path,new_path,timestamp,actor,reason,previous_hash,current_hash,ledger_prev_hash,ledger_hash) VALUES(?1,?2,?3,'deleted',?4,'',?5,?6,?7,?8,'',?9,?10)",
            params![eid.as_str(),file.id.as_str(),vid.as_str(),file.relative_path.as_str(),now.as_str(),actor,reason,file.current_hash.as_str(),prev.as_str(),next.as_str()]).await?;
        tx.commit().await?;Ok(())
    }
    pub async fn archive_record(&self,o:&StorageObject)->Result<()> {
        let conn=self.connect().await?;
        conn.execute("INSERT INTO archive_objects(content_hash,provider,provider_locator,encrypted,verified_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(content_hash,provider) DO UPDATE SET provider_locator=excluded.provider_locator,encrypted=excluded.encrypted,verified_at=excluded.verified_at",
            params![o.content_hash.as_str(),o.provider.as_str(),o.provider_locator.as_str(),if o.encrypted{1i64}else{0i64},o.verified_at.as_str()]).await?;
        Ok(())
    }
    pub async fn archives(&self,hash:&str)->Result<Vec<StorageObject>> {
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT content_hash,provider,provider_locator,encrypted,verified_at FROM archive_objects WHERE content_hash=?1 ORDER BY verified_at DESC",[hash]).await?;
        let mut out=Vec::new();while let Some(row)=r.next().await?{out.push(StorageObject{content_hash:row.get(0)?,provider:row.get(1)?,provider_locator:row.get(2)?,encrypted:row.get::<i64>(3)?!=0,verified_at:row.get(4)?});}Ok(out)
    }
    pub async fn is_hash_sensitive(&self,hash:&str)->Result<bool>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT COUNT(*) FROM versions WHERE content_hash=?1 AND sensitive=1",[hash]).await?;
        Ok(r.next().await?.map(|row|row.get::<i64>(0).unwrap_or(0)>0).unwrap_or(false))
    }
    /// Every tracked path that has held `hash`, including paths that were later moved or deleted.
    pub async fn paths_for_hash(&self,hash:&str)->Result<Vec<String>> {
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT DISTINCT f.relative_path FROM versions v JOIN resources f ON f.id=v.file_id WHERE v.content_hash=?1 ORDER BY f.relative_path",[hash]).await?;
        let mut out=Vec::new();
        while let Some(row)=r.next().await?{out.push(row.get::<String>(0)?);}
        Ok(out)
    }
    pub async fn set_tags(&self,file_id:&str,tags:&[String],source:&str,rule_version:&str)->Result<()> {
        let mut conn=self.connect().await?;let tx=conn.transaction().await?;
        let now=Utc::now().to_rfc3339();
        for tag in tags {
            let clean=tag.trim();if clean.is_empty()||clean.len()>80 {continue;}
            tx.execute("INSERT INTO file_tags(file_id,tag,source,rule_version,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(file_id,tag) DO UPDATE SET source=excluded.source,rule_version=excluded.rule_version",
                params![file_id,clean,source,rule_version,now.as_str()]).await?;
        }
        tx.commit().await?;Ok(())
    }
    pub async fn tags(&self,file_id:&str)->Result<Vec<String>>{
        let conn=self.connect().await?;let mut r=conn.query("SELECT tag FROM file_tags WHERE file_id=?1 ORDER BY tag",[file_id]).await?;
        let mut out=Vec::new();while let Some(row)=r.next().await?{out.push(row.get::<String>(0)?)}Ok(out)
    }
    pub async fn add_annotation(&self,file_id:&str,section:&str,vid:&str,text:&str)->Result<String>{
        let id=Uuid::new_v4().to_string();let now=Utc::now().to_rfc3339();let conn=self.connect().await?;
        conn.execute("INSERT INTO annotations(id,file_id,section_key,base_version_id,text,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id.as_str(),file_id,section,vid,text,now.as_str()]).await?;
        self.add_governance_event(file_id,"annotation.created",&id,text,"human").await?;Ok(id)
    }
    pub async fn create_task(&self,file_id:&str,section:&str,vid:&str,title:&str,description:&str)->Result<String>{
        let id=Uuid::new_v4().to_string();let now=Utc::now().to_rfc3339();let conn=self.connect().await?;
        conn.execute("INSERT INTO doc_tasks(id,file_id,anchor_section,base_version_id,title,status,description,created_at) VALUES(?1,?2,?3,?4,?5,'open',?6,?7)",params![id.as_str(),file_id,section,vid,title,description,now.as_str()]).await?;
        self.add_governance_event(file_id,"task.created",&id,title,"human").await?;Ok(id)
    }
    pub async fn add_governance_event(&self,file_id:&str,kind:&str,subject_id:&str,payload:&str,actor:&str)->Result<String>{
        let id=Uuid::new_v4().to_string();let now=Utc::now().to_rfc3339();let conn=self.connect().await?;
        conn.execute("INSERT INTO governance_events(id,file_id,kind,subject_id,payload,actor,timestamp) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id.as_str(),file_id,kind,subject_id,payload,actor,now.as_str()]).await?;
        Ok(id)
    }
    pub async fn governance_events(&self,file_id:&str)->Result<Vec<serde_json::Value>>{
        let conn=self.connect().await?;
        let mut rows=conn.query("SELECT id,kind,subject_id,payload,actor,timestamp FROM governance_events WHERE file_id=?1 ORDER BY seq",[file_id]).await?;
        let mut out=Vec::new();
        while let Some(r)=rows.next().await?{
            out.push(serde_json::json!({"id":r.get::<String>(0)?,"kind":r.get::<String>(1)?,"subject_id":r.get::<String>(2)?,"payload":r.get::<String>(3)?,"actor":r.get::<String>(4)?,"timestamp":r.get::<String>(5)?}));
        }Ok(out)
    }
    pub async fn list_tasks(&self,file_id:&str)->Result<Vec<serde_json::Value>>{
        let conn=self.connect().await?;
        let mut rows=conn.query("SELECT id,anchor_section,base_version_id,title,status,description,created_at FROM doc_tasks WHERE file_id=?1 ORDER BY created_at",[file_id]).await?;
        let mut out=Vec::new();
        while let Some(r)=rows.next().await?{
            out.push(serde_json::json!({"id":r.get::<String>(0)?,"section":r.get::<String>(1)?,"base_version_id":r.get::<String>(2)?,"title":r.get::<String>(3)?,"status":r.get::<String>(4)?,"description":r.get::<String>(5)?,"created_at":r.get::<String>(6)?}));
        }Ok(out)
    }
    pub async fn set_task_status(&self,task_id:&str,status:&str,actor:&str)->Result<()> {
        if !matches!(status,"open"|"analysis"|"approval"|"working"|"verification"|"done"|"cancelled"){anyhow::bail!("invalid task state");}
        let conn=self.connect().await?;
        let mut rs=conn.query("SELECT file_id FROM doc_tasks WHERE id=?1",[task_id]).await?;
        let file_id=rs.next().await?.ok_or_else(||anyhow!("task missing"))?.get::<String>(0)?;drop(rs);
        conn.execute("UPDATE doc_tasks SET status=?1 WHERE id=?2",params![status,task_id]).await?;
        self.add_governance_event(&file_id,"task.state_changed",task_id,status,actor).await?;Ok(())
    }
    pub async fn verify_chain(&self,file_id:&str)->Result<bool>{
        let events=self.events(file_id).await?;let mut prev=String::new();
        for e in events {
            if e.ledger_prev_hash!=prev{return Ok(false);}
            let actual=event_hash(&prev,&e.id,&e.file_id,&e.version_id,&e.kind,&e.old_path,&e.new_path,&e.timestamp,&e.actor,&e.reason,&e.previous_hash,&e.current_hash);
            if actual!=e.ledger_hash{return Ok(false);}
            prev=e.ledger_hash;
        }
        Ok(true)
    }
}

fn parse_version(r:&turso::Row)->Result<VersionRecord>{
    let sections:String=r.get(6)?;
    Ok(VersionRecord{id:r.get(0)?,file_id:r.get(1)?,number:r.get(2)?,content_hash:r.get(3)?,byte_size:r.get(4)?,sensitive:r.get::<i64>(5)?!=0,changed_sections:serde_json::from_str(&sections)?,actor:r.get(7)?,reason:r.get(8)?,created_at:r.get(9)?})
}
fn event_hash(prev:&str,eid:&str,fid:&str,vid:&str,kind:&str,old:&str,new:&str,ts:&str,actor:&str,reason:&str,previous:&str,current:&str)->String{
    let parts=(prev,eid,fid,vid,kind,old,new,ts,actor,reason,previous,current);
    digest(serde_json::to_vec(&parts).unwrap_or_default().as_slice())
}
fn parse_file(r:&turso::Row)->Result<FileRecord>{
    Ok(FileRecord{id:r.get(0)?,relative_path:r.get(1)?,current_hash:r.get(2)?,version_no:r.get(3)?,sensitive:r.get::<i64>(4)?!=0,deleted:r.get::<i64>(5)?!=0})
}
/// First column of the first row of a count query; 0 when there is no row.
async fn scalar_count(conn:&turso::Connection,sql:&str)->Result<i64>{
    let mut rows=conn.query(sql,()).await?;
    Ok(match rows.next().await? {Some(row)=>row.get::<i64>(0)?,None=>0})
}
async fn table_counts(conn:&turso::Connection)->Result<Vec<(&'static str,i64)>>{
    let mut counts=Vec::with_capacity(COUNTED_TABLES.len());
    for table in COUNTED_TABLES {
        counts.push((table,scalar_count(conn,&format!("SELECT COUNT(*) FROM {table}")).await?));
    }
    Ok(counts)
}
/// Appends `dt.migration.applied` to the document-task chain, once. Hash tuple: design 57 section 4.7.
async fn record_migration_event(conn:&turso::Connection,backup:&BackupState,counts:&[(&'static str,i64)])->Result<()> {
    if scalar_count(conn,"SELECT COUNT(*) FROM doc_task_events WHERE kind='dt.migration.applied'").await?>0 {return Ok(());}
    let prev={
        let mut rows=conn.query("SELECT hash FROM doc_task_events ORDER BY seq DESC LIMIT 1",()).await?;
        match rows.next().await? {Some(row)=>row.get::<String>(0)?,None=>"0".repeat(64)}
    };
    let (state,backup_verified,reason)=match backup {
        BackupState::NotNeeded=>("not_needed",true,None),
        BackupState::Verified{..}=>("verified",true,None),
        BackupState::Unverified{reason}=>("unverified",false,Some(reason.clone())),
    };
    let row_counts:serde_json::Map<String,serde_json::Value>=counts.iter().map(|(table,rows)|(table.to_string(),serde_json::json!(rows))).collect();
    let payload=serde_json::json!({"version":4,"backup":state,"backup_verified":backup_verified,"backup_reason":reason,"row_counts":row_counts}).to_string();
    let id=Uuid::new_v4().to_string();let timestamp=Utc::now().to_rfc3339();
    let hash=doc_event_hash(&prev,&id,"","","schema","4",DT_MIGRATION_APPLIED,"","","ladon","system","","",&payload,&timestamp);
    conn.execute("INSERT INTO doc_task_events(id,task_id,file_id,subject_kind,subject_id,kind,from_status,to_status,actor,actor_kind,session,claimed_session,payload_json,timestamp,prev_hash,hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![id.as_str(),"","","schema","4",DT_MIGRATION_APPLIED,"","","ladon","system","","",payload.as_str(),timestamp.as_str(),prev.as_str(),hash.as_str()]).await?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn doc_event_hash(prev:&str,id:&str,task_id:&str,file_id:&str,subject_kind:&str,subject_id:&str,kind:&str,from_status:&str,to_status:&str,actor:&str,actor_kind:&str,session:&str,claimed_session:&str,payload_json:&str,timestamp:&str)->String{
    let parts=(prev,id,task_id,file_id,subject_kind,subject_id,kind,from_status,to_status,actor,actor_kind,session,claimed_session,payload_json,timestamp);
    digest(serde_json::to_vec(&parts).unwrap_or_default().as_slice())
}
fn wal_file_of(database:&Path)->PathBuf {
    let mut name=database.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}
fn copy_backup_pair(source:&Path,source_wal:&Path,main_copy:&Path,wal_copy:&Path)->std::io::Result<()> {
    if let Some(dir)=main_copy.parent() {std::fs::create_dir_all(dir)?;}
    std::fs::copy(source,main_copy)?;
    if source_wal.exists() {
        std::fs::copy(source_wal,wal_copy)?;
    } else if wal_copy.exists() {
        std::fs::remove_file(wal_copy)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    #[derive(Clone, Copy)]
    enum Script {
        /// Lock error for the first n calls, then the real opener.
        BusyThen(u32),
        /// Lock error on every call.
        BusyAlways,
        /// Non-lock error on every call.
        NonLock,
        /// Lock error on the first call, non-lock error afterwards.
        BusyThenNonLock,
    }

    struct ScriptedOpener {
        script: Script,
        calls: AtomicU32,
    }

    impl ScriptedOpener {
        fn new(script: Script) -> Self {
            Self { script, calls: AtomicU32::new(0) }
        }
        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn busy() -> turso::Error {
        turso::Error::Busy("Locking error: Failed locking file. File is locked by another process".into())
    }

    #[async_trait]
    impl Opener for ScriptedOpener {
        async fn open(&self, path: &str) -> std::result::Result<turso::Connection, turso::Error> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            match self.script {
                Script::BusyThen(n) if call <= n => Err(busy()),
                Script::BusyAlways => Err(busy()),
                Script::NonLock => Err(turso::Error::Error("no such table".into())),
                Script::BusyThenNonLock if call == 1 => Err(busy()),
                Script::BusyThenNonLock => Err(turso::Error::Error("no such table".into())),
                _ => TursoOpener.open(path).await,
            }
        }
    }

    fn store_in(dir: &tempfile::TempDir) -> Store {
        std::fs::create_dir_all(dir.path().join(".ladon")).unwrap();
        Store::new(dir.path().join(".ladon/meta.turso"))
    }

    #[tokio::test]
    async fn ut_162_connect_retry_with_injected_opener() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(&tmp);
        let opener = ScriptedOpener::new(Script::BusyThen(2));

        let started = Instant::now();
        let conn = store.connect_via(&opener).await.unwrap();
        let elapsed = started.elapsed();

        assert_eq!(opener.calls(), 3);
        // Two sleeps: at least 25 + 50 ms, and at most 37 + 75 ms plus scheduling slack.
        assert!(elapsed >= Duration::from_millis(75), "elapsed {elapsed:?}");
        assert!(elapsed < Duration::from_millis(1500), "elapsed {elapsed:?}");
        conn.execute("CREATE TABLE IF NOT EXISTS retry_probe (x INTEGER)", ()).await.unwrap();

        let without_jitter: Vec<u64> = (1..=5).map(|r| backoff_ms(r, 0)).collect();
        assert_eq!(without_jitter, vec![25, 50, 100, 200, 400]);
        let max_jitter: Vec<u64> = (1..=5).map(|r| backoff_ms(r, 500)).collect();
        assert_eq!(max_jitter, vec![37, 75, 150, 300, 600]);
    }

    #[tokio::test]
    async fn ut_162_connect_exhaustion_and_non_lock_errors_are_not_retried() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(&tmp);

        let always = ScriptedOpener::new(Script::BusyAlways);
        let err = store.connect_via(&always).await.err().expect("exhausted retries must fail");
        assert_eq!(always.calls(), CONNECT_ATTEMPTS);
        let busy = err.downcast_ref::<StoreBusy>().expect("typed E_STORE_BUSY");
        assert_eq!(busy.attempts, 6);
        assert!(err.to_string().starts_with("E_STORE_BUSY"), "{err}");

        let non_lock = ScriptedOpener::new(Script::NonLock);
        let err = store.connect_via(&non_lock).await.err().expect("non-lock error must fail");
        assert_eq!(non_lock.calls(), 1);
        assert!(err.downcast_ref::<StoreBusy>().is_none());
        assert!(err.to_string().contains("no such table"), "{err}");

        let mixed = ScriptedOpener::new(Script::BusyThenNonLock);
        let err = store.connect_via(&mixed).await.err().expect("non-lock error after busy must fail");
        assert_eq!(mixed.calls(), 2);
        assert!(err.to_string().contains("no such table"), "{err}");
    }
}
