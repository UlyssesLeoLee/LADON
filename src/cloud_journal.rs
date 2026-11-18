//! Encrypted, immutable event segments; remote manifests are hints only.
//! Each device has a monotonic sequence; no shared overwriteable "latest.json".
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use crate::{crypto::{digest,open,seal},provider::StorageProvider,storage_rules::{ObjectClass,StorageRules}};

#[derive(Debug,Clone,Serialize,Deserialize,PartialEq,Eq)]
pub struct JournalEvent {
    pub event_id:String, pub file_ref:String, pub operation:String, pub occurred_at:String,
    pub base_version:String, pub result_version:String, pub actor_id:String, pub sensitive:bool,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct JournalSegment {
    pub schema:u32, pub tenant_id:String, pub workspace_id:String, pub device_id:String,
    pub first_seq:u64, pub last_seq:u64, pub prev_segment_hash:Option<String>, pub events:Vec<JournalEvent>,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct PublishReceipt { pub logical_key:String, pub plain_sha256:String, pub provider:String, pub locator:String,
    pub first_seq:u64, pub last_seq:u64, pub verified:bool }
impl JournalSegment {
    pub fn validate(&self)->Result<()> {
        if self.schema!=1 || self.events.is_empty() || self.first_seq==0 { bail!("invalid journal header"); }
        if self.last_seq.checked_sub(self.first_seq).and_then(|n|n.checked_add(1))!=Some(self.events.len() as u64) { bail!("noncontiguous sequence interval"); }
        if let Some(prev)=&self.prev_segment_hash { if !crate::crypto::is_digest(prev){bail!("bad predecessor digest");} }
        let mut seen=std::collections::HashSet::new();
        for e in &self.events { if e.event_id.is_empty() || e.file_ref.is_empty() || !seen.insert(&e.event_id) {bail!("missing / duplicate journal identity");} }
        Ok(())
    }
    pub fn reconcile(a:&Self,b:&Self)->Result<Reconcile> {
        a.validate()?; b.validate()?;
        if a.tenant_id!=b.tenant_id || a.workspace_id!=b.workspace_id {bail!("cross-tenant merge forbidden");}
        if a.device_id==b.device_id && !(a.last_seq<b.first_seq || b.last_seq<a.first_seq) {return Ok(Reconcile::ConflictingDeviceSequence);}
        let a_refs: std::collections::HashSet<_>=a.events.iter().map(|e|(&e.file_ref,&e.base_version)).collect();
        let conflicted=b.events.iter().any(|e|a_refs.contains(&(&e.file_ref,&e.base_version)) && e.result_version!=e.base_version);
        Ok(if conflicted {Reconcile::RequiresReview} else {Reconcile::Independent})
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Reconcile { Independent, RequiresReview, ConflictingDeviceSequence }

pub async fn publish_segment(segment:&JournalSegment,rules:&StorageRules,provider:&dyn StorageProvider,key:&[u8;32])->Result<PublishReceipt>{
    rules.validate()?;segment.validate()?;
    if segment.events.iter().any(|e|e.sensitive) { bail!("LADON Ignore: sensitive journal cannot be published"); }
    if segment.tenant_id!=rules.tenant_id || segment.workspace_id!=rules.workspace_id || segment.device_id!=rules.device_id {bail!("scope mismatch");}
    let bytes=serde_json::to_vec(segment)?;let hash=digest(&bytes);
    let route=rules.route(ObjectClass::EventSegment,&hash,false)?;
    if !route.remote {bail!("remote publication disabled by rules");}
    let payload=seal(&bytes,key)?;
    let locator=provider.put(&hash,&payload).await?;
    let readback=provider.get(&locator).await?;
    let check=open(&readback,key,&hash)?;
    if check!=bytes{bail!("readback content mismatch");}
    Ok(PublishReceipt{logical_key:route.logical_key,plain_sha256:hash,provider:provider.name().into(),locator,
        first_seq:segment.first_seq,last_seq:segment.last_seq,verified:true})
}

#[cfg(test)] mod tests {
    use super::*;
    use crate::provider::FilesystemProvider;
    fn seg(device:&str,base:&str) -> JournalSegment {JournalSegment{schema:1,tenant_id:"t".into(),workspace_id:"w".into(),device_id:device.into(),first_seq:1,last_seq:1,prev_segment_hash:None,events:vec![JournalEvent{event_id:format!("event-{device}"),file_ref:"f".into(),operation:"modify".into(),occurred_at:"2026-11-18T00:00:00Z".into(),base_version:base.into(),result_version:format!("new-{device}"),actor_id:"actor".into(),sensitive:false}]}}
    #[tokio::test] async fn ut_journal_sensitive_denied() { let tmp=tempfile::tempdir().unwrap();let p=FilesystemProvider::new("local",tmp.path().to_path_buf());let mut s=seg("d","v1");s.events[0].sensitive=true;let rules=StorageRules{version:1,mode:crate::storage_rules::ArchiveMode::TeamDriveFolder,tenant_id:"t".into(),workspace_id:"w".into(),device_id:"d".into(),allow_remote:true,max_local_pending_bytes:1024,target_segment_bytes:4*1024*1024};assert!(publish_segment(&s,&rules,&p,&[1u8;32]).await.is_err());}
    #[test] fn ut_journal_reject_gaps(){let mut a=seg("d1","v1");a.last_seq=3;assert!(a.validate().is_err());}
    #[test] fn ut_journal_detect_conflicting_edit(){assert_eq!(JournalSegment::reconcile(&seg("a","v1"),&seg("b","v1")).unwrap(),Reconcile::RequiresReview);}
    #[test] fn ut_journal_reject_tenant_merge(){let a=seg("a","v1");let mut b=seg("b","v2");b.tenant_id="other".into();assert!(JournalSegment::reconcile(&a,&b).is_err());}
    #[tokio::test] async fn ut_journal_real_encrypted_roundtrip(){let tmp=tempfile::tempdir().unwrap();let p=FilesystemProvider::new("local",tmp.path().to_path_buf());let s=seg("d","v1");let rules=StorageRules{version:1,mode:crate::storage_rules::ArchiveMode::TeamDriveFolder,tenant_id:"t".into(),workspace_id:"w".into(),device_id:"d".into(),allow_remote:true,max_local_pending_bytes:1024,target_segment_bytes:4*1024*1024};let result=publish_segment(&s,&rules,&p,&[17u8;32]).await.unwrap();assert!(result.verified);let ciphertext=p.get(&result.locator).await.unwrap();assert!(!String::from_utf8_lossy(&ciphertext).contains("event-d"));}
}
