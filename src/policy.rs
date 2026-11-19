//! .LADONignore applies to external data flow; local history still permitted.
use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

pub struct LadonPolicy { root: PathBuf, matcher: Gitignore }

impl LadonPolicy {
    pub fn load(root: &Path) -> Result<Self> {
        let root=root.canonicalize().context("workspace not found")?;
        let mut builder=GitignoreBuilder::new(&root);
        // Globally scoped file (optional); project rules and nested rules.
        if let Ok(path)=std::env::var("LADON_GLOBAL_IGNORE") {
            let p=PathBuf::from(path);
            if p.is_file() { if let Some(err)=builder.add(&p) { return Err(err.into()); } }
        }
        for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter()
            .filter_entry(|e|e.depth()==0 || !matches!(e.file_name().to_str(),Some(".ladon"|".git")))
            .filter_map(Result::ok) {
            if entry.file_type().is_dir() && entry.file_name()==".ladon" { continue; }
            if entry.file_type().is_file() && entry.file_name()==".LADONignore" {
                if let Some(err)=builder.add(entry.path()) {return Err(err.into());}
            }
        }
        Ok(Self{root,matcher:builder.build()?})
    }

    pub fn excluded(&self, path: &Path) -> Result<bool> {
        let path=if path.is_absolute(){path.to_path_buf()}else{self.root.join(path)};
        if !path.starts_with(&self.root) {anyhow::bail!("outside workspace");}
        if path.components().any(|c|c.as_os_str()==".ladon"||c.as_os_str()==".git") {return Ok(true);}
        if path.file_name().map(|s|s==".LADONignore").unwrap_or(false){return Ok(true);}
        let is_dir=path.is_dir();
        Ok(self.matcher.matched_path_or_any_parents(&path,is_dir).is_ignore())
    }

    pub fn allow_external(&self, path: &Path)->Result<()> {
        if self.excluded(path)? {anyhow::bail!("LADON policy denies external data transfer: {}",path.display());}
        Ok(())
    }
}

/// Add an existing file or directory to root `.LADONignore` with exact,
/// root-anchored literal matching; symbolic-link escapes are rejected.
/// This affects subsequent transfers; it does NOT revoke previously uploaded data.
pub fn add_sensitive(root:&Path,target:&Path)->Result<PathBuf>{
    let root=root.canonicalize()?;
    let abs=if target.is_absolute(){target.to_path_buf()}else{root.join(target)};
    let target=abs.canonicalize()?;
    if !target.starts_with(&root){anyhow::bail!("cannot exclude resource outside workspace");}
    let rel=target.strip_prefix(&root)?.to_string_lossy().replace('\\',"/");
    if rel.is_empty() || rel.starts_with(".ladon/") || rel==".LADONignore" {anyhow::bail!("invalid ignore target");}
    // Escape path characters interpreted by gitignore globs. Root anchor
    // prevents a same-named file elsewhere matching accidentally.
    let mut escaped=String::new();
    for c in rel.chars(){
        if matches!(c,'*'|'?'|'['|']'|'!'|'#'|'\\'){escaped.push('\\');}
        escaped.push(c);
    }
    let rule=format!("/{}{}",escaped,if target.is_dir(){"/"}else{""});
    let ignore_file=root.join(".LADONignore");
    let mut current=if ignore_file.exists(){std::fs::read_to_string(&ignore_file)?}else{String::new()};
    if !current.lines().any(|l|l==rule){
        if !current.ends_with('\n') && !current.is_empty(){current.push('\n');}
        current.push_str("# LADON sensitive: exclude remote transfers\n");
        current.push_str(&rule);current.push('\n');
        let temp=ignore_file.with_extension("LADONignore.tmp");
        std::fs::write(&temp,current)?;
        std::fs::rename(&temp,&ignore_file)?;
    }
    Ok(ignore_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_policy_01_ignore_and_allow() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"*.pem\nprivate/\n").unwrap();
        std::fs::create_dir(root.join("private")).unwrap();
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("x.pem")).unwrap());
        assert!(policy.excluded(&root.join("private/x.md")).unwrap());
        assert!(!policy.excluded(&root.join("public.txt")).unwrap());
    }
    #[test] fn ut_policy_02_rule_is_ignored() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"").unwrap();
        let p=LadonPolicy::load(&root).unwrap();
        assert!(p.excluded(&root.join(".LADONignore")).unwrap());
    }
    #[test] fn ut_policy_04_add_sensitive() {
        let t=tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("secret project.md"),"private").unwrap();
        add_sensitive(t.path(),Path::new("secret project.md")).unwrap();
        let p=LadonPolicy::load(t.path()).unwrap();
        assert!(p.excluded(Path::new("secret project.md")).unwrap());
    }
    #[test] fn ut_policy_03_outside_workspace() {
        let t=tempfile::tempdir().unwrap(); let p=LadonPolicy::load(t.path()).unwrap();
        assert!(p.excluded(std::path::Path::new("/this/is/outside")).is_err());
    }
}
