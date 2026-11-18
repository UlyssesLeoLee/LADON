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
        if entry.metadata()?.len()>8*1024*1024 {continue;}
        let bytes=fs::read(entry.path())?;
        let Ok(text)=String::from_utf8(bytes) else {continue;};
        for (idx,line) in text.lines().enumerate(){
            if re.is_match(line){
                out.push(SearchHit{path:entry.path().strip_prefix(root)?.to_string_lossy().into_owned(),line:idx+1,snippet:line.chars().take(280).collect()});
                if out.len()>=max_results{break 'files;}
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn ut_search_01_finds_lines(){let t=tempfile::tempdir().unwrap();std::fs::write(t.path().join("a.md"),"foo\nbar\n").unwrap();let r=grep(t.path(),"bar",10).unwrap();assert_eq!(r[0].line,2);}
 #[test]fn ut_search_02_bad_regex(){let t=tempfile::tempdir().unwrap();assert!(grep(t.path(),"(",10).is_err());}
}
