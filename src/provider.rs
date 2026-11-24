//! Provider adapters are addressable by content hash, not by user path.
//! Every remote payload is encrypted before entering this interface.
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::json;
use std::{path::{Path,PathBuf},fs,time::Duration};
use crate::crypto::is_digest;
use crate::google_oauth;

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

/// Where a Drive request gets its bearer token. A sign-in refreshes the token before each request.
enum TokenSource { Fixed(String), SignIn { client:google_oauth::OAuthClient, refresh_token:String } }

pub struct GoogleDriveProvider { pub folder_id:Option<String>, pub use_appdata:bool, token_source:TokenSource, client:reqwest::Client, upload_client:reqwest::Client }
impl GoogleDriveProvider {
    pub fn new(access_token:String,folder_id:Option<String>)->Result<Self>{
        if access_token.trim().is_empty(){bail!("OAuth access token required");}
        Self::with_source(TokenSource::Fixed(access_token),folder_id)
    }
    /// Uses a stored sign-in. The client ID comes from the environment; no request is sent here.
    pub fn from_sign_in(refresh_token:String,folder_id:Option<String>)->Result<Self>{
        if refresh_token.trim().is_empty(){bail!("stored sign-in is unreadable");}
        let client=google_oauth::OAuthClient::from_env()?;
        Self::with_source(TokenSource::SignIn{client,refresh_token},folder_id)
    }
    fn with_source(token_source:TokenSource,folder_id:Option<String>)->Result<Self>{
        Ok(Self{folder_id,use_appdata:false,token_source,
            client:http_client(DRIVE_CONNECT_TIMEOUT,Some(DRIVE_READ_TIMEOUT))?,
            upload_client:http_client(DRIVE_CONNECT_TIMEOUT,None)?})
    }
    async fn bearer(&self)->Result<String>{
        match &self.token_source {
            TokenSource::Fixed(token)=>Ok(token.clone()),
            TokenSource::SignIn{client,refresh_token}=>google_oauth::refresh_access_token(client,refresh_token).await,
        }
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
        // Drive v3 resumable upload: POST metadata, then PUT complete payload. One token serves both requests.
        let token=self.bearer().await?;
        let res=self.client.post("https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id")
            .bearer_auth(&token)
            .header("X-Upload-Content-Type","application/octet-stream")
            .header("X-Upload-Content-Length",encrypted.len().to_string())
            .json(&meta).send().await?.error_for_status()?;
        let loc=res.headers().get(reqwest::header::LOCATION)
            .context("Drive did not return resumable session location")?.to_str()?.to_string();
        if !loc.starts_with("https://www.googleapis.com/") && !loc.starts_with("https://www.google.com/") {
            bail!("unexpected upload location origin");
        }
        let reply:DriveCreate=self.upload_client.put(loc).bearer_auth(&token)
            .header(reqwest::header::CONTENT_TYPE,"application/octet-stream")
            .timeout(upload_deadline(encrypted.len()))
            .body(encrypted.to_vec()).send().await?.error_for_status()?.json().await?;
        Self::validate_id(&reply.id)?;
        Ok(reply.id)
    }
    async fn get(&self,locator:&str)->Result<Vec<u8>>{
        Self::validate_id(locator)?;
        let url=format!("https://www.googleapis.com/drive/v3/files/{locator}?alt=media");
        let token=self.bearer().await?;
        let r=self.client.get(url).bearer_auth(&token).send().await?.error_for_status()?;
        Ok(r.bytes().await?.to_vec())
    }
    async fn delete(&self,_locator:&str)->Result<()>{
        bail!("remote deletion disabled in prototype; retention/approval adapter required")
    }
}
/// Connect timeout for every Drive request.
const DRIVE_CONNECT_TIMEOUT:Duration=Duration::from_secs(30);
/// Longest wait for response headers or for the next body chunk. A progressing transfer is not cut off.
const DRIVE_READ_TIMEOUT:Duration=Duration::from_secs(60);
/// Upload deadline: base plus one second per MIN_UPLOAD_BYTES_PER_SEC bytes.
const UPLOAD_BASE_DEADLINE:Duration=Duration::from_secs(180);
const MIN_UPLOAD_BYTES_PER_SEC:usize=64*1024;

/// Builds a Drive HTTP client. `read` bounds each wait for response data; `None` leaves it unbounded.
fn http_client(connect:Duration,read:Option<Duration>)->Result<reqwest::Client>{
    let builder=reqwest::Client::builder().connect_timeout(connect);
    let builder=match read { Some(t)=>builder.read_timeout(t), None=>builder };
    Ok(builder.build()?)
}
/// Per-request deadline for a resumable upload of `len` bytes. The PUT is answered only after its whole body is sent,
/// so a read timeout cannot bound it.
fn upload_deadline(len:usize)->Duration{
    UPLOAD_BASE_DEADLINE+Duration::from_secs((len/MIN_UPLOAD_BYTES_PER_SEC) as u64)
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
            let folder_id=std::env::var("LADON_GOOGLE_FOLDER_ID").ok();
            let appdata=std::env::var("LADON_GOOGLE_MODE").ok().as_deref()==Some("appdata");
            // Order: a non-empty environment token, then the stored sign-in, then the error.
            let provider=match std::env::var("LADON_GOOGLE_ACCESS_TOKEN").ok().filter(|t|!t.trim().is_empty()) {
                Some(token)=>GoogleDriveProvider::new(token,folder_id)?,
                None=>{
                    let stored=match google_oauth::settings_dir() {
                        Some(dir)=>google_oauth::load_refresh_token(&dir)?,
                        None=>None,
                    };
                    let refresh=stored.ok_or_else(||anyhow!("Google Drive is not connected"))?;
                    GoogleDriveProvider::from_sign_in(refresh,folder_id).map_err(|_|anyhow!("Google Drive is not connected"))?
                }
            };
            Ok(Box::new(provider.set_appdata(appdata)?))
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
    /// Reads one HTTP request (headers and any Content-Length body) from the stream.
    fn read_request(stream:&mut std::net::TcpStream){
        use std::io::Read;
        let mut head=Vec::new();
        let mut byte=[0u8;1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).unwrap()==0 {break;}
            head.push(byte[0]);
        }
        let text=String::from_utf8_lossy(&head).to_ascii_lowercase();
        let len=text.lines().find_map(|l|l.strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
        let mut body=vec![0u8;len];
        stream.read_exact(&mut body).unwrap();
    }
    /// Serves one connection on a background thread and returns the base URL.
    fn serve(respond:impl FnOnce(&mut std::net::TcpStream)+Send+'static)->String{
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr=listener.local_addr().unwrap();
        std::thread::spawn(move||{
            let (mut stream,_)=listener.accept().unwrap();
            read_request(&mut stream);
            respond(&mut stream);
        });
        format!("http://{addr}/")
    }
    const HEAD_10K:&[u8]=b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n";
    #[tokio::test]
    async fn ut_provider_03_drive_download_survives_slow_transfer(){
        let url=serve(|s|{
            use std::io::Write;
            s.write_all(HEAD_10K).unwrap();
            for _ in 0..10 { s.write_all(&[7u8;1000]).unwrap(); s.flush().unwrap(); std::thread::sleep(Duration::from_millis(100)); }
        });
        let client=http_client(Duration::from_secs(1),Some(Duration::from_millis(300))).unwrap();
        let body=client.get(url).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(body.len(),10000);
    }
    #[tokio::test]
    async fn ut_provider_04_drive_download_stall_fails(){
        let url=serve(|s|{
            use std::io::Write;
            s.write_all(HEAD_10K).unwrap();
            s.write_all(&[7u8;1000]).unwrap();
            s.flush().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let client=http_client(Duration::from_secs(1),Some(Duration::from_millis(300))).unwrap();
        let result=client.get(url).send().await.unwrap().bytes().await;
        assert!(result.unwrap_err().is_timeout());
    }
    #[tokio::test]
    async fn ut_provider_05_drive_upload_response_may_wait_past_read_timeout(){
        let url=serve(|s|{
            use std::io::Write;
            std::thread::sleep(Duration::from_millis(600));
            let body=r#"{"id":"abc"}"#;
            write!(s,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let client=http_client(Duration::from_secs(1),None).unwrap();
        let res=client.put(url).body(vec![7u8;4096]).timeout(Duration::from_secs(5)).send().await.unwrap();
        assert!(res.status().is_success());
    }
}
