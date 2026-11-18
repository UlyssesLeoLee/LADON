//! Minimal, strict subset of .NEMA graph grammar for typed dependency edges.
//! Not a full implementation of the upstream .NEMA language.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency { pub from: String, pub edge_id: String, pub to: String, pub label: String }

pub fn parse_graph_edges(input: &str) -> Result<Vec<Dependency>> {
    if input.contains('\t') || input.contains('\u{3000}') { bail!("NEMA requires ASCII U+0020 spaces"); }
    let mut graph=false; let mut edges=Vec::new(); let mut nodes=HashSet::new();
    for (i,line) in input.lines().enumerate() {
        if let Some(header)=line.strip_prefix(" \\") {
            let mut s=header;
            graph=s.starts_with('>');
            if graph {s=&s[1..];}
            else if s.starts_with('?') {s=&s[1..];}
            let p:Vec<_>=s.split(' ').collect();
            if p.len()!=2 || p[0].parse::<u64>().is_err() || p[1].parse::<u64>().is_err() {
                bail!("bad NEMA domain header on line {}",i+1);
            }
            continue;
        }
        if !graph || line.is_empty() || line.starts_with(' ') { continue; }
        let parts:Vec<_>=line.split(' ').collect();
        if parts.len()<2 { bail!("malformed graph row {}",i+1); }
        let token=parts[0];
        if let Some(a)=token.find('>') {
            let left=&token[..a]; let rest=&token[a+1..];
            let b=rest.find('>').ok_or_else(||anyhow::anyhow!("malformed edge {}",i+1))?;
            let edge_id=&rest[..b]; let right=&rest[b+1..];
            if left.is_empty()||right.is_empty()||edge_id.is_empty()||right.contains('>') {
                bail!("invalid graph edge {}",i+1);
            }
            edges.push(Dependency{from:left.into(),edge_id:edge_id.into(),to:right.into(),label:parts[1..].join(" ")});
        } else if !token.contains('<') && !token.contains('=') { nodes.insert(token.to_string()); }
        else { bail!("only directed > graph edges supported in runtime v0.1"); }
    }
    for edge in &edges {
        if !nodes.contains(&edge.from)||!nodes.contains(&edge.to) {
            bail!("edge {} refers to undeclared node",edge.edge_id);
        }
    }
    Ok(edges)
}

pub fn topo_batches(nodes: &[String], edges: &[Dependency]) -> Result<Vec<Vec<String>>> {
    use std::collections::{BTreeMap,BTreeSet};
    let mut degrees:BTreeMap<String,usize>=nodes.iter().cloned().map(|n|(n,0)).collect();
    let mut adj:BTreeMap<String,Vec<String>>=BTreeMap::new();
    for e in edges {
        if !degrees.contains_key(&e.from)||!degrees.contains_key(&e.to) {bail!("unknown node");}
        *degrees.get_mut(&e.to).unwrap()+=1;
        adj.entry(e.from.clone()).or_default().push(e.to.clone());
    }
    let mut pending:BTreeSet<String>=degrees.keys().cloned().collect();
    let mut result=Vec::new();
    while !pending.is_empty() {
        let ready:Vec<String>=pending.iter().filter(|n|degrees[*n]==0).cloned().collect();
        if ready.is_empty(){ bail!("dependency cycle"); }
        for n in &ready { pending.remove(n); for d in adj.get(n).into_iter().flatten(){ *degrees.get_mut(d).unwrap()-=1; } }
        result.push(ready);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ut_nema_01_graph() {
        let s=" \\>3 1\nA Requirement\nB Design\nC Test\nA>E1>B Depends\nB>E2>C Depends";
        let edges=parse_graph_edges(s).unwrap();
        assert_eq!(edges.len(),2);
        let batches=topo_batches(&["A".into(),"B".into(),"C".into()],&edges).unwrap();
        assert_eq!(batches,vec![vec!["A"],vec!["B"],vec!["C"]]);
    }
    #[test] fn ut_nema_02_parallel() {
        let edges=vec![Dependency{from:"A".into(),edge_id:"E".into(),to:"C".into(),label:"dep".into()}];
        assert_eq!(topo_batches(&["A".into(),"B".into(),"C".into()],&edges).unwrap()[0],vec!["A","B"]);
    }
    #[test] fn ut_nema_03_cycle_reject() {
        let edges=vec![Dependency{from:"A".into(),edge_id:"X".into(),to:"B".into(),label:"".into()},Dependency{from:"B".into(),edge_id:"Y".into(),to:"A".into(),label:"".into()}];
        assert!(topo_batches(&["A".into(),"B".into()],&edges).is_err());
    }
    #[test] fn ut_nema_04_tab_reject() {assert!(parse_graph_edges(" \\>1 1\nA\tNode").is_err());}
}
