//! FileSpace snapshot and optimistic concurrency guard.
use crate::{crypto::digest,model::FileRecord};
use anyhow::{bail,Result};
use serde::{Serialize,Deserialize};
use std::{path::{Path,PathBuf},fs};

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct FileSpace {pub id:String,pub scope:Vec<FileRef>}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct FileRef {pub id:String,pub path:String,pub version:i64,pub content_hash:String}
impl FileSpace {
    pub fn from_files(files:&[FileRecord])->Self{
        Self{id:uuid::Uuid::new_v4().to_string(),scope:files.iter().filter(|f|!f.deleted).map(|f|FileRef{id:f.id.clone(),path:f.relative_path.clone(),version:f.version_no,content_hash:f.current_hash.clone()}).collect()}
    }
    pub fn verify_before_write(&self,root:&Path,target:&str)->Result<PathBuf>{
        let r=self.scope.iter().find(|r|r.path==target).ok_or_else(||anyhow::anyhow!("file outside FileSpace"))?;
        let root=root.canonicalize()?;
        let path=root.join(&r.path).canonicalize()?;
        if !path.starts_with(&root){bail!("FileSpace path escaped root");}
        let bytes=fs::read(&path)?;
        if digest(&bytes)!=r.content_hash {bail!("FileSpace version conflict: file changed since snapshot");}
        Ok(path)
    }
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn ut_space_01_conflict(){
   let t=tempfile::tempdir().unwrap();let p=t.path().join("a");fs::write(&p,"before").unwrap();
   let space=FileSpace{ id:"s".into(),scope:vec![FileRef{id:"id".into(),path:"a".into(),version:1,content_hash:digest(b"before")}]};
   assert!(space.verify_before_write(t.path(),"a").is_ok());
   fs::write(&p,"after").unwrap();assert!(space.verify_before_write(t.path(),"a").is_err());
 }
}
