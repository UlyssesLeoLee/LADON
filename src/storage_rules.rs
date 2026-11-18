//! P0: deterministic routing rules for local-first/Drive-first historical objects.
//! Rules are a POLICY LAYER, not a claim that Drive provides SQL-like RLS.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use crate::crypto::digest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveMode { PersonalAppData, TeamDriveFolder, LocalOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectClass { Content, EventSegment, IndexPage, Checkpoint, ChatSegment, Attachment }
impl ObjectClass {
    fn part(self) -> &'static str { match self {
        Self::Content => "content", Self::EventSegment => "events", Self::IndexPage => "indexes",
        Self::Checkpoint => "checkpoints", Self::ChatSegment => "chats", Self::Attachment => "attachments"
    }}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRules {
    pub version: u32,
    pub mode: ArchiveMode,
    pub tenant_id: String,
    pub workspace_id: String,
    pub device_id: String,
    pub allow_remote: bool,
    pub max_local_pending_bytes: u64,
    pub target_segment_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecision { pub logical_key: String, pub remote: bool, pub requires_encryption: bool }

fn opaque_id(kind: &str, raw: &str) -> Result<String> {
    if raw.is_empty() || raw.len() > 256 || raw.chars().any(char::is_control) { bail!("invalid {kind}"); }
    let value = digest(format!("ladon-id-v1:{kind}:{raw}").as_bytes());
    Ok(value[..24].to_string())
}
impl StorageRules {
    /// Logical keys are *not* real Google Drive POSIX paths: the adapter maps
    /// each key to an immutable Drive file ID with appProperties metadata.
    pub fn route(&self, class: ObjectClass, plaintext_hash: &str, sensitive: bool) -> Result<RouteDecision> {
        if self.version != 1 { bail!("unsupported storage rules version"); }
        if !crate::crypto::is_digest(plaintext_hash) { bail!("invalid object digest"); }
        let scope = format!("{}/{}/{}", opaque_id("tenant", &self.tenant_id)?,
            opaque_id("workspace", &self.workspace_id)?, class.part());
        let device = if matches!(class, ObjectClass::EventSegment | ObjectClass::ChatSegment) {
            format!("/{}/", opaque_id("device", &self.device_id)?)
        } else { "/".into() };
        let key = format!("LADON/v1/{scope}{device}{plaintext_hash}.ldn");
        Ok(RouteDecision {
            logical_key: key,
            remote: self.allow_remote && self.mode != ArchiveMode::LocalOnly && !sensitive,
            requires_encryption: true,
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.max_local_pending_bytes == 0 || !(256 * 1024..=32 * 1024 * 1024).contains(&self.target_segment_bytes) {
            bail!("invalid queue/segment capacity");
        }
        for (kind,id) in [("tenant", &self.tenant_id),("workspace", &self.workspace_id),("device", &self.device_id)] {
            opaque_id(kind,id)?;
        }
        if self.mode == ArchiveMode::TeamDriveFolder && self.tenant_id == "personal" {
            bail!("team archive requires a tenant binding");
        }
        Ok(())
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn rules() -> StorageRules { StorageRules {version:1,mode:ArchiveMode::PersonalAppData,
        tenant_id:"user-1".into(),workspace_id:"ws-1".into(),device_id:"device-1".into(),
        allow_remote:true,max_local_pending_bytes:536870912,target_segment_bytes:4*1024*1024} }
    #[test] fn ut_rules_sensitive_stays_local() { let r=rules(); let a=r.route(ObjectClass::Content,&digest(b"abc"),true).unwrap(); assert!(!a.remote);assert!(!a.logical_key.contains("user-1")); }
    #[test] fn ut_rules_key_is_stable() { let r=rules();let hash=digest(b"hello"); assert_eq!(r.route(ObjectClass::EventSegment,&hash,false).unwrap(),r.route(ObjectClass::EventSegment,&hash,false).unwrap()); }
    #[test] fn ut_rules_separate_tenant_keys() { let a=rules();let mut b=rules();b.tenant_id="other".into();let h=digest(b"same"); assert_ne!(a.route(ObjectClass::Content,&h,false).unwrap().logical_key,b.route(ObjectClass::Content,&h,false).unwrap().logical_key); }
    #[test] fn ut_rules_reject_bad_inputs(){ let mut r=rules();r.target_segment_bytes=100;assert!(r.validate().is_err());assert!(r.route(ObjectClass::Content,"../evil",false).is_err()); }
}
