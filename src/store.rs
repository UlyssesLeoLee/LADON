use crate::{crypto::digest, model::{EventRecord, FileRecord, VersionRecord, StorageObject}};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;
use turso::{params, Builder};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Store { pub path: PathBuf }

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
    async fn connect(&self)->Result<turso::Connection>{
        let p=self.path.to_string_lossy().to_string();
        let db=Builder::new_local(&p).build().await?;
        let conn=db.connect()?;
        conn.execute("PRAGMA foreign_keys = ON",()).await?;
        Ok(conn)
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
    pub async fn find_path(&self,path:&str)->Result<Option<FileRecord>>{
        let conn=self.connect().await?;
        let mut r=conn.query("SELECT id,relative_path,current_hash,version_no,sensitive,deleted FROM resources WHERE relative_path=?1",[path]).await?;
        if let Some(row)=r.next().await?{
            Ok(Some(FileRecord{id:row.get(0)?,relative_path:row.get(1)?,current_hash:row.get(2)?,version_no:row.get(3)?,sensitive:row.get::<i64>(4)?!=0,deleted:row.get::<i64>(5)?!=0}))
        } else {Ok(None)}
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
