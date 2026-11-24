//! Bounded local exact/regex content search. Never contacts external AI.
use anyhow::{bail,Result};
use regex::RegexBuilder;
use std::{fs, path::Path};
use serde::{Serialize,Deserialize};

#[derive(Debug,Serialize,Deserialize)]
pub struct SearchHit {pub path:String,pub line:usize,pub snippet:String}

pub fn grep(root:&Path,query:&str,max_results:usize)->Result<Vec<SearchHit>>{
    if query.len()>512{bail!("regex too long");}
    let re=RegexBuilder::new(query).size_limit(2*1024*1024).build()?;
    let mut out=Vec::new();
    'files:for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(std::result::Result::ok){
        if !entry.file_type().is_file(){continue;}
        if entry.path().components().any(|s|s.as_os_str()==".git"||s.as_os_str()==".ladon"){continue;}
        let Ok(meta)=entry.metadata() else {continue;};
        if meta.len()>8*1024*1024 {continue;}
        let Some(text)=read_text_file(entry.path())? else {continue;};
        for (idx,line) in text.lines().enumerate(){
            if re.is_match(line){
                out.push(SearchHit{path:entry.path().strip_prefix(root)?.to_string_lossy().into_owned(),line:idx+1,snippet:line.chars().take(280).collect()});
                if out.len()>=max_results{break 'files;}
            }
        }
    }
    Ok(out)
}

/// Reads a file as UTF-8 text. Returns `Ok(None)` when the file cannot be read
/// (missing, locked, permission denied) or is not valid UTF-8, so the search skips it.
fn read_text_file(path:&Path)->Result<Option<String>>{
    let Ok(bytes)=fs::read(path) else {return Ok(None);};
    Ok(String::from_utf8(bytes).ok())
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn ut_search_01_finds_lines(){let t=tempfile::tempdir().unwrap();std::fs::write(t.path().join("a.md"),"foo\nbar\n").unwrap();let r=grep(t.path(),"bar",10).unwrap();assert_eq!(r[0].line,2);}
 #[test]fn ut_search_02_bad_regex(){let t=tempfile::tempdir().unwrap();assert!(grep(t.path(),"(",10).is_err());}
 #[cfg(windows)]
 #[test]fn ut_search_03_skips_unreadable_file(){
  use std::os::windows::fs::OpenOptionsExt;
  let t=tempfile::tempdir().unwrap();
  std::fs::write(t.path().join("a.md"),"needle\n").unwrap();
  std::fs::write(t.path().join("b.md"),"needle\n").unwrap();
  let _lock=std::fs::OpenOptions::new().read(true).share_mode(0).open(t.path().join("a.md")).unwrap();
  let r=grep(t.path(),"needle",10).unwrap();
  assert!(r.iter().any(|h|h.path=="b.md"));
 }
 #[test]fn ut_search_04_helper_reads_utf8(){let t=tempfile::tempdir().unwrap();std::fs::write(t.path().join("a.md"),"héllo\n").unwrap();assert_eq!(read_text_file(&t.path().join("a.md")).unwrap().as_deref(),Some("héllo\n"));}
 #[test]fn ut_search_05_helper_none_for_missing_file(){let t=tempfile::tempdir().unwrap();assert!(read_text_file(&t.path().join("gone.md")).unwrap().is_none());}
 #[test]fn ut_search_06_helper_none_for_non_utf8(){let t=tempfile::tempdir().unwrap();std::fs::write(t.path().join("a.bin"),[0xffu8,0xfe,0x00]).unwrap();assert!(read_text_file(&t.path().join("a.bin")).unwrap().is_none());}
 #[cfg(windows)]
 #[test]fn ut_search_07_helper_none_for_locked_file(){
  use std::os::windows::fs::OpenOptionsExt;
  let t=tempfile::tempdir().unwrap();
  std::fs::write(t.path().join("a.md"),"needle\n").unwrap();
  let _lock=std::fs::OpenOptions::new().read(true).share_mode(0).open(t.path().join("a.md")).unwrap();
  assert!(read_text_file(&t.path().join("a.md")).unwrap().is_none());
 }
}
