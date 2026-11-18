//! Explainable, quota-bound event fusion -> suggestions, NOT autonomous writes.
use serde::{Deserialize,Serialize};
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct Signal {pub kind:String,pub anchor:String,pub confidence:f32,pub evidence_count:u32}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct Suggestion {pub anchor:String,pub reason:String,pub confidence:f32,pub requires_approval:bool}
pub fn propose(signals:&[Signal],max_suggestions:usize)->Vec<Suggestion>{
    let mut anchors=std::collections::BTreeMap::<&str,Vec<&Signal>>::new();
    for s in signals {if s.confidence.is_finite()&&s.confidence>=0.6&&s.evidence_count>=1&&!s.anchor.is_empty(){anchors.entry(&s.anchor).or_default().push(s);}}
    let mut result=Vec::new();
    for (anchor,values) in anchors {let unique:std::collections::HashSet<&str>=values.iter().map(|v|v.kind.as_str()).collect();
        if unique.len()<2 {continue;}
        let confidence=(values.iter().map(|v|v.confidence).sum::<f32>()/values.len() as f32).min(0.99);
        result.push(Suggestion{anchor:anchor.into(),reason:format!("{} independent event signals; review document anchor impact",unique.len()),confidence,requires_approval:true});
    }
    result.sort_by(|a,b|b.confidence.total_cmp(&a.confidence));result.truncate(max_suggestions);result
}
#[cfg(test)]mod tests {use super::*;
#[test]fn ut_emergence_requires_multiple_signals(){let a=Signal{kind:"file_changed".into(),anchor:"REQ-011".into(),confidence:0.9,evidence_count:1};assert!(propose(&[a.clone()],2).is_empty());let b=Signal{kind:"test_failed".into(),..a};assert_eq!(propose(&[b.clone(),Signal{kind:"file_changed".into(),..b}],2).len(),1);}
#[test]fn ut_emergence_never_auto_approves(){let a=Signal{kind:"a".into(),anchor:"REQ".into(),confidence:0.9,evidence_count:1};let b=Signal{kind:"b".into(),..a.clone()};assert!(propose(&[a,b],1)[0].requires_approval);}
}
