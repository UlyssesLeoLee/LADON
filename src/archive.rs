use crate::crypto::{digest, is_digest};
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}};
use uuid::Uuid;

#[derive(Clone)]
pub struct CasStore { pub base: PathBuf }

impl CasStore {
    pub fn new(base:PathBuf)->Self{Self{base}}
    pub fn local_object_path(&self, hash:&str)->Result<PathBuf>{
        if !is_digest(hash){bail!("invalid digest");}
        Ok(self.base.join(&hash[..2]).join(format!("{hash}.zst")))
    }
    pub fn has(&self,hash:&str)->bool{self.local_object_path(hash).map(|p|p.exists()).unwrap_or(false)}
    pub fn put_file(&self,path:&Path)->Result<(String,u64)>{
        fs::create_dir_all(&self.base)?;
        let before=fs::metadata(path)?;
        if !before.is_file(){bail!("not a regular file");}
        let tmp=self.base.join(format!(".tmp-{}",Uuid::new_v4()));
        let result=(||->Result<(String,u64)>{
            let mut input=fs::File::open(path)?;
            let output=fs::File::create(&tmp)?;
            let mut encoder=zstd::Encoder::new(output,3)?;
            let mut hasher=Sha256::new();let mut bytes=0u64;let mut buf=[0u8;65536];
            loop {
                let n=input.read(&mut buf)?;if n==0{break;}
                hasher.update(&buf[..n]);encoder.write_all(&buf[..n])?;bytes+=n as u64;
            }
            let mut finished=encoder.finish()?;finished.flush()?;finished.sync_all()?;
            let after=fs::metadata(path)?;
            if before.len()!=after.len() || before.modified().ok()!=after.modified().ok() {
                bail!("file changed during capture: retry after stable save");
            }
            let hash=hex::encode(hasher.finalize());let target=self.local_object_path(&hash)?;
            fs::create_dir_all(target.parent().unwrap())?;
            // An object that already exists is kept only when its bytes hash to its name; a damaged one is replaced.
            if target.exists()&&object_matches(&target,&hash){fs::remove_file(&tmp)?;}
            else {
                if target.exists(){fs::remove_file(&target)?;}
                fs::rename(&tmp,&target)?;
            }
            Ok((hash,bytes))
        })();
        if result.is_err(){let _=fs::remove_file(tmp);}
        result
    }
    pub fn put_bytes(&self,bytes:&[u8])->Result<String>{
        let hash=digest(bytes);let target=self.local_object_path(&hash)?;
        if target.exists()&&object_matches(&target,&hash){return Ok(hash);}
        fs::create_dir_all(target.parent().unwrap())?;
        let tmp=self.base.join(format!(".tmp-{}",Uuid::new_v4()));
        let result=(||->Result<()>{
            let compressed=zstd::stream::encode_all(bytes,3)?;
            let mut out=fs::File::create(&tmp)?;
            out.write_all(&compressed)?;out.sync_all()?;
            drop(out);
            if target.exists(){fs::remove_file(&target)?;}
            fs::rename(&tmp,&target)?;
            Ok(())
        })();
        if result.is_err(){let _=fs::remove_file(tmp);}
        result.map(|_|hash)
    }
    /// Loading whole historical object is currently capped to avoid unbounded memory use.
    pub fn load_bytes(&self,hash:&str,max_bytes:usize)->Result<Vec<u8>>{
        let path=self.local_object_path(hash)?;
        let mut decoder=zstd::Decoder::new(fs::File::open(&path).context("CAS object not local")?)?;
        let mut output=Vec::new();
        decoder.by_ref().take((max_bytes as u64)+1).read_to_end(&mut output)?;
        if output.len()>max_bytes{bail!("object exceeds bounded read limit");}
        if digest(&output)!=hash{bail!("local CAS hash validation failed");}
        Ok(output)
    }
    pub fn evict(&self,hash:&str)->Result<()>{
        let p=self.local_object_path(hash)?;
        if p.exists(){fs::remove_file(p)?;}Ok(())
    }
}

/// True when the object at `target` decompresses to bytes whose digest is `hash`. The object is read in chunks.
fn object_matches(target:&Path,hash:&str)->bool{
    let Ok(file)=fs::File::open(target) else {return false;};
    let Ok(mut decoder)=zstd::Decoder::new(file) else {return false;};
    let mut hasher=Sha256::new();let mut buf=[0u8;65536];
    loop {
        match decoder.read(&mut buf){
            Ok(0)=>break,
            Ok(n)=>hasher.update(&buf[..n]),
            Err(_)=>return false,
        }
    }
    hex::encode(hasher.finalize())==hash
}

pub fn assert_under_workspace(root:&Path,path:&Path)->Result<PathBuf>{
    let root=root.canonicalize()?;
    if fs::symlink_metadata(path)?.file_type().is_symlink(){bail!("symlink capture forbidden");}
    let path=path.canonicalize()?;
    if !path.starts_with(&root){return Err(anyhow!("path escapes the workspace"));}
    let meta=fs::symlink_metadata(&path)?;
    if meta.file_type().is_symlink() || !meta.is_file(){bail!("symlink/nonfile capture forbidden");}
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_cas_01_dedup_and_reload(){
        let tmp=tempfile::tempdir().unwrap();let c=CasStore::new(tmp.path().join("cas"));
        let path=tmp.path().join("x.txt");fs::write(&path,"hello hello").unwrap();
        let (a,_)=c.put_file(&path).unwrap();let (b,_)=c.put_file(&path).unwrap();
        assert_eq!(a,b);assert_eq!(c.load_bytes(&a,128).unwrap(),b"hello hello");
    }
    #[test] fn ut_cas_02_traversal(){
        let tmp=tempfile::tempdir().unwrap();let c=CasStore::new(tmp.path().join("cas"));
        assert!(c.local_object_path("../evil").is_err());
    }
    #[test] fn ut_cas_03_limit(){
        let tmp=tempfile::tempdir().unwrap();let c=CasStore::new(tmp.path().join("cas"));
        let hash=c.put_bytes(&[0u8;100]).unwrap();assert!(c.load_bytes(&hash,99).is_err());
    }
    #[test] fn ut_cas_04_put_file_replaces_a_damaged_object(){
        let tmp=tempfile::tempdir().unwrap();let c=CasStore::new(tmp.path().join("cas"));
        let path=tmp.path().join("x.txt");fs::write(&path,"hello hello hello").unwrap();
        let (hash,_)=c.put_file(&path).unwrap();
        let object=c.local_object_path(&hash).unwrap();
        let bytes=fs::read(&object).unwrap();fs::write(&object,&bytes[..bytes.len()/2]).unwrap();
        assert!(c.load_bytes(&hash,128).is_err());
        let (again,_)=c.put_file(&path).unwrap();
        assert_eq!(again,hash);assert_eq!(c.load_bytes(&hash,128).unwrap(),b"hello hello hello");
    }
    #[test] fn ut_cas_05_put_bytes_replaces_a_damaged_object(){
        let tmp=tempfile::tempdir().unwrap();let c=CasStore::new(tmp.path().join("cas"));
        let hash=c.put_bytes(b"some bytes to store").unwrap();
        fs::write(c.local_object_path(&hash).unwrap(),b"garbage").unwrap();
        assert!(c.load_bytes(&hash,128).is_err());
        assert_eq!(c.put_bytes(b"some bytes to store").unwrap(),hash);
        assert_eq!(c.load_bytes(&hash,128).unwrap(),b"some bytes to store");
    }
}
