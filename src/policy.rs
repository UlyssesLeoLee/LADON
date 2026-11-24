//! .LADONignore applies to external data flow; local history still permitted.
use anyhow::{anyhow, Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

pub struct LadonPolicy { root: PathBuf, matcher: Gitignore }

/// A rule set loaded on a background thread. `start` returns at once; `with` waits for the
/// load to finish and then runs its closure on the rule set. A failed load keeps its message.
pub struct PendingPolicy { cell: Arc<(Mutex<Option<Result<LadonPolicy, String>>>, Condvar)> }

impl PendingPolicy {
    /// Starts the full rule scan of `root` on one background thread.
    pub fn start(root: &Path) -> PendingPolicy {
        let cell = Arc::new((Mutex::new(None), Condvar::new()));
        let worker = Arc::clone(&cell);
        let root = root.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("ladon-rule-scan".into())
            .spawn(move || {
                let result = LadonPolicy::load(&root).map_err(|err| format!("{err:#}"));
                let (lock, ready) = &*worker;
                if let Ok(mut slot) = lock.lock() {
                    *slot = Some(result);
                }
                ready.notify_all();
            });
        if spawned.is_err() {
            let (lock, ready) = &*cell;
            if let Ok(mut slot) = lock.lock() {
                *slot = Some(Err("rule scan could not start".to_string()));
            }
            ready.notify_all();
        }
        PendingPolicy { cell }
    }

    /// A rule set whose load has already failed with `message`, so `with` returns that error.
    pub fn failed(message: String) -> PendingPolicy {
        PendingPolicy { cell: Arc::new((Mutex::new(Some(Err(message))), Condvar::new())) }
    }

    /// Waits for the scan to finish, then runs `f` on the rule set. A failed scan returns an
    /// error with the stored message on every call.
    pub fn with<R>(&self, f: impl FnOnce(&LadonPolicy) -> Result<R>) -> Result<R> {
        let (lock, ready) = &*self.cell;
        let mut slot = lock.lock().map_err(|_| anyhow!("rule scan lock poisoned"))?;
        while slot.is_none() {
            slot = ready.wait(slot).map_err(|_| anyhow!("rule scan lock poisoned"))?;
        }
        match slot.as_ref() {
            Some(Ok(policy)) => f(policy),
            Some(Err(message)) => Err(anyhow!("{message}")),
            None => Err(anyhow!("rule scan did not finish")),
        }
    }
}

/// Rule-set builder for `root`. Patterns match ASCII case-insensitively on Windows, where the
/// file system does, so `*.PEM` also excludes `key.pem`.
fn rule_builder(root: &Path) -> Result<GitignoreBuilder> {
    let mut builder = GitignoreBuilder::new(root);
    builder.case_insensitive(cfg!(windows))?;
    Ok(builder)
}

/// Files directly under `root` whose name is `.LADONignore` in any ASCII case and that are
/// regular files or links to files. Entries that cannot be read are skipped.
fn root_rule_files(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root).into_iter().flatten().flatten()
        .filter(|e| e.file_name().eq_ignore_ascii_case(".LADONignore") && e.path().is_file())
        .map(|e| e.path())
        .collect()
}

/// Turns one step of a directory walk into the entry it yields. A walk error fails the scan,
/// so a subtree that cannot be listed never leaves its nested rule file unread.
pub(crate) fn walk_entry(step: walkdir::Result<walkdir::DirEntry>) -> Result<walkdir::DirEntry> {
    step.map_err(|err| anyhow!("failed to scan the workspace: {err}"))
}

/// Adds a nested rule file. A negated pattern in it is rewritten to match only below the
/// file's own directory, so a re-include cannot reach other directories. Other lines are
/// added the same way `GitignoreBuilder::add` adds them.
fn add_nested_rule_file(builder: &mut GitignoreBuilder, root: &Path, rule: &Path) -> Result<()> {
    let text = std::fs::read_to_string(rule)
        .with_context(|| format!("failed to read {}", rule.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let scope = nested_scope(root, rule)?;
    for (i, line) in text.lines().enumerate() {
        let scoped = scope_negation(&scope, line);
        builder.add_line(Some(rule.to_path_buf()), &scoped)
            .map_err(|err| anyhow!("{}: line {}: {err}", rule.display(), i + 1))?;
    }
    Ok(())
}

/// Directory of a rule file relative to `root`, with each name escaped for a glob.
fn nested_scope(root: &Path, rule: &Path) -> Result<String> {
    let dir = rule.parent().and_then(|d| d.strip_prefix(root).ok())
        .ok_or_else(|| anyhow!("rule file is outside the workspace: {}", rule.display()))?;
    Ok(dir.components()
        .map(|c| escape_glob(&c.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/"))
}

/// Escapes the characters that a gitignore glob reads as syntax.
fn escape_glob(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '!' | '#' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Rewrites a negated pattern so it matches only below `scope`, a root-relative directory.
/// A pattern with a slash, or one starting with a slash, stays anchored to that directory;
/// any other pattern matches at any depth below it. Other lines are returned unchanged.
fn scope_negation(scope: &str, line: &str) -> String {
    let Some(rest) = line.strip_prefix('!') else { return line.to_string(); };
    let (anchored, body) = match rest.strip_prefix('/') {
        Some(body) => (true, body),
        None => (false, rest),
    };
    let core = body.trim_end().trim_end_matches('/');
    if core.is_empty() {
        return line.to_string();
    }
    let body = if anchored || core.contains('/') { body.to_string() } else { format!("**/{body}") };
    format!("!{scope}/{body}")
}

/// Adds the global rule file that `LADON_GLOBAL_IGNORE` names, when the variable is set. A file
/// that cannot be read is an error, so mandatory global rules never drop out without notice.
fn add_global_rule_file(builder: &mut GitignoreBuilder) -> Result<()> {
    if let Some(path) = std::env::var_os("LADON_GLOBAL_IGNORE") {
        if let Some(err) = builder.add(Path::new(&path)) {
            return Err(err.into());
        }
    }
    Ok(())
}

impl LadonPolicy {
    /// True when the root holds a `.LADONignore` in any ASCII case. Without one the rule set
    /// would be empty, so the workspace must not be read as having no rules.
    pub fn has_root_rule_file(root: &Path) -> bool {
        !root_rule_files(root).is_empty()
    }

    /// Checks the global rule file and the root `.LADONignore` only; nested rule files are
    /// not read, so the cost does not depend on the size of the tree.
    pub fn check_root(root: &Path) -> Result<()> {
        let root = root.canonicalize().context("workspace not found")?;
        if !Self::has_root_rule_file(&root) {
            anyhow::bail!("root rule file .LADONignore is missing");
        }
        let mut builder = rule_builder(&root)?;
        add_global_rule_file(&mut builder)?;
        for rule in root_rule_files(&root) {
            if let Some(err) = builder.add(&rule) {
                return Err(err.into());
            }
        }
        builder.build()?;
        Ok(())
    }

    pub fn load(root: &Path) -> Result<Self> {
        let root=root.canonicalize().context("workspace not found")?;
        let mut builder=rule_builder(&root)?;
        // Globally scoped file (when set); project rules and nested rules.
        add_global_rule_file(&mut builder)?;
        // The root rule file uses the same lookup as check_root and is added before nested files.
        for rule in root_rule_files(&root) {
            if let Some(err)=builder.add(&rule) {return Err(err.into());}
        }
        for step in walkdir::WalkDir::new(&root).follow_links(false).into_iter()
            .filter_entry(|e|e.depth()==0 || !matches!(e.file_name().to_str(),Some(".ladon"|".git"))) {
            let entry = walk_entry(step)?;
            if entry.file_type().is_dir() && entry.file_name()==".ladon" { continue; }
            // Nested rule files only; the root rule file was added above.
            if entry.depth()>=2 && entry.file_type().is_file() && entry.file_name()==".LADONignore" {
                add_nested_rule_file(&mut builder, &root, entry.path())?;
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
    let rule=format!("/{}{}",escape_glob(&rel),if target.is_dir(){"/"}else{""});
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

/// True when a change to `changed` is a root rule file event. The root rule file is found in any
/// ASCII case, matching `root_rule_files`, so a rename or edit of `.ladonignore` reloads the rules.
pub fn is_root_rule_event(root: &Path, changed: &Path) -> bool {
    changed.parent() == Some(root)
        && changed.file_name().map(|s| s.eq_ignore_ascii_case(".LADONignore")).unwrap_or(false)
}

/// True when a watch event for `path` requires a full workspace scan. A path that is not a
/// regular file, such as a renamed or moved folder, can leave tracked files at stale paths, so it
/// needs a scan. The root rule file is excluded because the watcher reloads the rules for it.
pub fn needs_full_scan(path: &Path, is_file: bool, root: &Path) -> bool {
    !is_file && !is_root_rule_event(root, path)
}

/// Serializes the tests that load rules, because `LADON_GLOBAL_IGNORE` is process-wide. A test
/// that points it at a missing file would otherwise make every concurrent load fail.
#[cfg(test)]
static GLOBAL_IGNORE_LOCK: Mutex<()> = Mutex::new(());

/// Holds `GLOBAL_IGNORE_LOCK` and, when dropped, restores `LADON_GLOBAL_IGNORE` to the value it had
/// at `lock_global_ignore`, so a failed assertion cannot leave the variable changed.
#[cfg(test)]
pub(crate) struct GlobalIgnoreGuard {
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

/// Takes the rule-loading test lock. Hold the guard for as long as a test loads rules.
#[cfg(test)]
pub(crate) fn lock_global_ignore() -> GlobalIgnoreGuard {
    let lock = GLOBAL_IGNORE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    GlobalIgnoreGuard { previous: std::env::var_os("LADON_GLOBAL_IGNORE"), _lock: lock }
}

#[cfg(test)]
impl Drop for GlobalIgnoreGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("LADON_GLOBAL_IGNORE", value),
            None => std::env::remove_var("LADON_GLOBAL_IGNORE"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_policy_01_ignore_and_allow() {
        let _env=lock_global_ignore();
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
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"").unwrap();
        let p=LadonPolicy::load(&root).unwrap();
        assert!(p.excluded(&root.join(".LADONignore")).unwrap());
    }
    #[test] fn ut_policy_lowercase_rule_file_is_applied() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".ladonignore"),"*.pem\n").unwrap();
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("key.pem")).unwrap());
    }
    #[cfg(windows)]
    #[test] fn ut_policy_case_insensitive_on_windows() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"*.PEM\n").unwrap();
        std::fs::write(root.join("key.pem"),"").unwrap();
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("key.pem")).unwrap());
    }
    #[cfg(unix)]
    #[test] fn ut_policy_symlinked_root_rule_file_is_applied() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join("rules.txt"),"*.pem\n").unwrap();
        std::os::unix::fs::symlink(root.join("rules.txt"),root.join(".LADONignore")).unwrap();
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("key.pem")).unwrap());
    }
    #[test] fn ut_policy_nested_negation_stays_in_its_dir() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"*.pem\n").unwrap();
        std::fs::create_dir(root.join("public")).unwrap();
        std::fs::create_dir(root.join("keys")).unwrap();
        std::fs::write(root.join("public/.LADONignore"),"!cert.pem\n").unwrap();
        std::fs::write(root.join("public/cert.pem"),"").unwrap();
        std::fs::write(root.join("keys/cert.pem"),"").unwrap();
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("keys/cert.pem")).unwrap());
        assert!(!policy.excluded(&root.join("public/cert.pem")).unwrap());
    }
    #[test] fn ut_policy_nested_anchored_negation_stays_in_its_dir() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"*.pem\n").unwrap();
        std::fs::create_dir_all(root.join("docs/keys")).unwrap();
        std::fs::create_dir(root.join("keys")).unwrap();
        std::fs::write(root.join("docs/.LADONignore"),"!/cert.pem\n!keys/cert.pem\n").unwrap();
        for p in ["cert.pem","keys/cert.pem","docs/cert.pem","docs/keys/cert.pem"] {
            std::fs::write(root.join(p),"").unwrap();
        }
        let policy=LadonPolicy::load(&root).unwrap();
        assert!(policy.excluded(&root.join("cert.pem")).unwrap());
        assert!(policy.excluded(&root.join("keys/cert.pem")).unwrap());
        assert!(!policy.excluded(&root.join("docs/cert.pem")).unwrap());
        assert!(!policy.excluded(&root.join("docs/keys/cert.pem")).unwrap());
    }
    #[test] fn ut_policy_04_add_sensitive() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("secret project.md"),"private").unwrap();
        add_sensitive(t.path(),Path::new("secret project.md")).unwrap();
        let p=LadonPolicy::load(t.path()).unwrap();
        assert!(p.excluded(Path::new("secret project.md")).unwrap());
    }
    #[test] fn ut_add_sensitive_escapes_braces() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join("x{a,b}.md"),"private").unwrap();
        std::fs::write(root.join("xa.md"),"public").unwrap();
        add_sensitive(&root,Path::new("x{a,b}.md")).unwrap();
        let p=LadonPolicy::load(&root).unwrap();
        assert!(p.excluded(Path::new("x{a,b}.md")).unwrap());
        assert!(!p.excluded(Path::new("xa.md")).unwrap());
    }
    #[test] fn ut_policy_03_outside_workspace() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap(); let p=LadonPolicy::load(t.path()).unwrap();
        assert!(p.excluded(std::path::Path::new("/this/is/outside")).is_err());
    }
    #[test] fn ut_unreadable_global_ignore_fails_closed() {
        let _env=lock_global_ignore();
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        std::fs::write(root.join(".LADONignore"),"").unwrap();
        let missing=root.join("missing-global.ignore");
        std::env::set_var("LADON_GLOBAL_IGNORE",&missing);
        let shown=missing.display().to_string();
        let load_err=LadonPolicy::load(&root).err()
            .expect("load must fail when LADON_GLOBAL_IGNORE names a file that does not exist");
        assert!(format!("{load_err:#}").contains(&shown), "the load error must name the file: {load_err:#}");
        let check_err=LadonPolicy::check_root(&root).err()
            .expect("check_root must fail when LADON_GLOBAL_IGNORE names a file that does not exist");
        assert!(format!("{check_err:#}").contains(&shown), "the check error must name the file: {check_err:#}");
    }
    #[test] fn ut_policy_walk_error_fails_closed() {
        let t=tempfile::tempdir().unwrap();
        // A walk of a missing path yields a real walk error on every platform.
        let step=walkdir::WalkDir::new(t.path().join("missing")).into_iter().next().unwrap();
        assert!(walk_entry(step).is_err());
    }
    #[test] fn ut_root_rule_event_matches_any_case() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path();
        assert!(is_root_rule_event(root,&root.join(".LADONignore")), "exact root rule name must reload");
        assert!(is_root_rule_event(root,&root.join(".ladonignore")), "lowercase root rule name must reload");
        assert!(is_root_rule_event(root,&root.join(".LadonIgnore")), "mixed-case root rule name must reload");
        assert!(!is_root_rule_event(root,&root.join("sub").join(".ladonignore")), "nested rule files keep the exact name");
        assert!(!is_root_rule_event(root,&root.join("sub").join(".LADONignore")), "a nested rule file is not a root event");
        assert!(!is_root_rule_event(root,&root.join("other.txt")), "unrelated file is not a rule event");
    }
    #[test] fn ut_needs_full_scan_for_directory_events() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path().canonicalize().unwrap();
        let moved=root.join("moved");
        std::fs::create_dir(&moved).unwrap();
        let file=root.join("note.md");
        std::fs::write(&file,"text").unwrap();
        assert!(needs_full_scan(&moved,false,&root), "a directory event must trigger a full scan");
        assert!(!needs_full_scan(&file,true,&root), "a regular file event is captured directly");
        assert!(!needs_full_scan(&root.join(".LADONignore"),false,&root), "the root rule file reloads the rules instead");
    }
}
