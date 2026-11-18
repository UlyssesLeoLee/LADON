//! Provider adapters are addressable by content hash, not by user path.
//! Every remote payload is encrypted before entering this interface.
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::json;
use std::{path::{Path,PathBuf},fs};
use crate::crypto::is_digest;

#[async_trait]
pub trait StorageProvider: Send+Sync {
    fn name(&self)->&str;
    async fn put(&self,hash:&str,encrypted:&[u8])->Result<String>;
    async fn get(&self,locator:&str)->Result<Vec<u8>>;
    async fn delete(&self,locator:&str)->Result<()>;
}

#[derive(Clone)]
pub struct FilesystemProvider { name:String, root:PathBuf }
impl FilesystemProvider {
    pub fn new(name:impl Into<String>,root:PathBuf)->Self{Self{name:name.into(),root}}
    fn target(&self,hash:&str)->Result<PathBuf>{
        if !is_digest(hash){bail!("invalid content hash");}
        Ok(self.root.join(&hash[..2]).join(format!("{hash}.ladon")))
    }
}
#[async_trait]
impl StorageProvider for FilesystemProvider {
    fn name(&self)->&str{&self.name}
    async fn put(&self,hash:&str,encrypted:&[u8])->Result<String>{
        let target=self.target(hash)?;fs::create_dir_all(target.parent().unwrap())?;
        if !target.exists(){
            let tmp=target.with_extension("ladon.pending");
            fs::write(&tmp,encrypted)?;fs::rename(tmp,&target)?;
        }
        Ok(hash.to_string())
    }
    async fn get(&self,locator:&str)->Result<Vec<u8>>{Ok(fs::read(self.target(locator)?)?)}
    async fn delete(&self,locator:&str)->Result<()>{
        let p=self.target(locator)?;if p.exists(){fs::remove_file(p)?;}Ok(())
    }
}

pub struct GoogleDriveProvider { pub access_token:String, pub folder_id:Option<String>, pub use_appdata:bool, client:reqwest::Client }
impl GoogleDriveProvider {
    pub fn new(access_token:String,folder_id:Option<String>)->Result<Self>{
        if access_token.trim().is_empty(){bail!("OAuth access token required");}
        Ok(Self{access_token,folder_id,use_appdata:false,client:reqwest::Client::builder().timeout(std::time::Duration::from_secs(180)).build()?})
    }
    pub fn set_appdata(mut self, enabled:bool)->Result<Self>{
        if enabled && self.folder_id.is_some(){bail!("appDataFolder and regular Drive folder are mutually exclusive");}
        self.use_appdata=enabled;Ok(self)
    }
    fn validate_id(id:&str)->Result<()> {
        if id.is_empty()||id.len()>256||!id.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'_'||c==b'-') {
            bail!("unsafe Google Drive file ID");
        }
        Ok(())
    }
}
#[async_trait]
impl StorageProvider for GoogleDriveProvider {
    fn name(&self)->&str{"google_drive"}
    async fn put(&self,hash:&str,encrypted:&[u8])->Result<String>{
        if !is_digest(hash){bail!("invalid digest");}
        let mut meta=json!({"name":format!("{hash}.ladon"),"mimeType":"application/octet-stream"});
        if self.use_appdata {meta["parents"]=json!(["appDataFolder"]);} else if let Some(folder)=&self.folder_id {Self::validate_id(folder)?;meta["parents"]=json!([folder]);}
        // Drive v3 resumable upload: POST metadata, then PUT complete payload.
        let res=self.client.post("https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id")
            .bearer_auth(&self.access_token)
            .header("X-Upload-Content-Type","application/octet-stream")
            .header("X-Upload-Content-Length",encrypted.len().to_string())
            .json(&meta).send().await?.error_for_status()?;
        let loc=res.headers().get(reqwest::header::LOCATION)
            .context("Drive did not return resumable session location")?.to_str()?.to_string();
        if !loc.starts_with("https://www.googleapis.com/") && !loc.starts_with("https://www.google.com/") {
            bail!("unexpected upload location origin");
        }
        let reply:DriveCreate=self.client.put(loc).bearer_auth(&self.access_token)
            .header(reqwest::header::CONTENT_TYPE,"application/octet-stream")
            .body(encrypted.to_vec()).send().await?.error_for_status()?.json().await?;
        Self::validate_id(&reply.id)?;
        Ok(reply.id)
    }
    async fn get(&self,locator:&str)->Result<Vec<u8>>{
        Self::validate_id(locator)?;
        let url=format!("https://www.googleapis.com/drive/v3/files/{locator}?alt=media");
        let r=self.client.get(url).bearer_auth(&self.access_token).send().await?.error_for_status()?;
        Ok(r.bytes().await?.to_vec())
    }
    async fn delete(&self,_locator:&str)->Result<()>{
        bail!("remote deletion disabled in prototype; retention/approval adapter required")
    }
}
#[derive(serde::Deserialize)]
struct DriveCreate{ id:String }

pub fn provider_from_env(kind:&str,path:Option<&Path>)->Result<Box<dyn StorageProvider>>{
    match kind {
        "local"|"nas"=>{
            let root=path.context("--destination required for local/nas storage")?;
            Ok(Box::new(FilesystemProvider::new(kind,root.to_path_buf())))
        }
        "google_drive"=>{
            let token=std::env::var("LADON_GOOGLE_ACCESS_TOKEN").context("LADON_GOOGLE_ACCESS_TOKEN not set")?;
            let folder_id=std::env::var("LADON_GOOGLE_FOLDER_ID").ok();
            let appdata=std::env::var("LADON_GOOGLE_MODE").ok().as_deref()==Some("appdata");
            Ok(Box::new(GoogleDriveProvider::new(token,folder_id)?.set_appdata(appdata)?))
        }
        _=>bail!("unknown provider: {}",kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ut_provider_01_local_roundtrip(){
        let dir=tempfile::tempdir().unwrap();let p=FilesystemProvider::new("local",dir.path().join("objects"));
        let hash=crate::crypto::digest(b"hello");
        let locator=p.put(&hash,b"encrypted payload").await.unwrap();
        assert_eq!(p.get(&locator).await.unwrap(),b"encrypted payload");
    }
    #[test]
    fn ut_provider_02_id_validation(){
        assert!(GoogleDriveProvider::validate_id("abc_123-X").is_ok());
        assert!(GoogleDriveProvider::validate_id("../../etc/passwd").is_err());
    }
}
