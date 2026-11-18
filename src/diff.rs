//! Section-aware fingerprints for Markdown and .NEMA domain bodies.
use crate::crypto::digest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Section { pub key: String, pub hash: String }

fn is_header(line: &str, extension: &str) -> Option<String> {
    if matches!(extension, "nema" | "nemat") {
        let tail = line.strip_prefix(" \\")?;
        let tail = tail.trim_start_matches(|c| c == '?' || c == '>');
        let parts: Vec<_> = tail.split(' ').collect();
        if parts.len() == 2 && parts[0].parse::<u64>().is_ok() && parts[1].parse::<u64>().is_ok() {
            return Some(format!("DOMAIN:{}", parts[0]));
        }
    }
    if matches!(extension, "md" | "markdown") {
        let h = line.bytes().take_while(|v| *v == b'#').count();
        if (1..=6).contains(&h) && line.as_bytes().get(h) == Some(&b' ') {
            return Some(format!("H{}:{}", h, line[h+1..].trim()));
        }
    }
    None
}

pub fn section_hashes(bytes: &[u8], extension: &str) -> Vec<Section> {
    let content = String::from_utf8_lossy(bytes);
    let mut sections = Vec::<Section>::new();
    let mut current = "ROOT".to_string();
    let mut body = String::new();
    let mut counter = BTreeMap::<String, usize>::new();
    for line in content.lines() {
        if let Some(key) = is_header(line, extension) {
            sections.push(Section{key:current,hash:digest(body.as_bytes())});
            let n=counter.entry(key.clone()).or_insert(0); *n+=1;
            current=format!("{}#{}",key,n);
            body.clear();
        }
        body.push_str(line); body.push('\n');
    }
    sections.push(Section{key:current,hash:digest(body.as_bytes())});
    sections
}

pub fn changed_sections(before: &[u8], after: &[u8], extension: &str) -> Vec<String> {
    let a: BTreeMap<_,_> = section_hashes(before,extension).into_iter().map(|s|(s.key,s.hash)).collect();
    let b: BTreeMap<_,_> = section_hashes(after,extension).into_iter().map(|s|(s.key,s.hash)).collect();
    let keys: BTreeSet<_>=a.keys().chain(b.keys()).cloned().collect();
    keys.into_iter().filter(|k|a.get(k)!=b.get(k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_diff_01_section_change_only() {
        let a=b"# A\nkeep\n# B\nold\n"; let b=b"# A\nkeep\n# B\nnew\n";
        assert_eq!(changed_sections(a,b,"md"),vec!["H1:B#1"]);
    }
    #[test] fn ut_diff_02_nema_domain_change_only() {
        let a=b" \\>1 1\nA A1\n \\>2 1\nB B1\n";
        let b=b" \\>1 1\nA A1\n \\>2 2\nB B2\n";
        assert_eq!(changed_sections(a,b,"nema"),vec!["DOMAIN:2#1"]);
    }
    #[test] fn ut_diff_03_no_change() {
        assert!(changed_sections(b"same",b"same","txt").is_empty());
    }
}
