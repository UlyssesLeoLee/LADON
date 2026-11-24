//! Rules-based fallback + OpenAI-compatible chat endpoint, no fake vector output.
use anyhow::{anyhow,bail,Result};
use serde_json::{json,Value};
use std::time::Duration;
use url::Url;

/// Upper bound for one chat request, so a silent endpoint cannot block tag-folder runs forever.
const AI_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Largest chat response body that is buffered before parsing; larger bodies are rejected.
const AI_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn heuristic_tags(path:&str,text:&str)->Vec<String>{
    let q=format!("{} {}",path,text).to_lowercase();
    let rules:[(&str,&[&str]);8]=[("requirements",&["要件定義","requirement","requirements","需求"]),
        ("basic-design",&["基本設計","architecture","基本设计","概要设计"]),
        ("detailed-design",&["詳細設計","detailed design","详细设计","仕様"]),
        ("unit-test",&["単体テスト","unit test","ut-","単体"]),
        ("integration-test",&["結合テスト","integration test","it-"]),
        ("system-test",&["システムテスト","system test","st-"]),
        ("rust",&[".rs","rust","cargo"]),
        ("nema",&[".nema","node-edge","5w1h","nema"]),
    ];
    let mut out=Vec::new();
    for (label,keys) in rules {if keys.iter().any(|k|q.contains(k)){out.push(label.to_string());}}
    out
}

pub fn local_endpoint(endpoint:&str)->Result<bool>{
    let url=Url::parse(endpoint)?;
    if !matches!(url.scheme(),"http"|"https"){bail!("http(s) endpoint only");}
    let host=url.host_str().unwrap_or_default();
    Ok(matches!(host,"localhost"|"127.0.0.1"|"::1"|"[::1]"))
}

pub async fn ai_tags(endpoint:&str,model:&str,description:&str,filename:&str,preview:&str)->Result<Vec<String>>{
    ai_tags_with_timeout(endpoint,model,description,filename,preview,AI_REQUEST_TIMEOUT).await
}

/// `ai_tags` with an explicit time budget for the whole request. Callers use `ai_tags`; this entry point exists for tests.
#[doc(hidden)]
pub async fn ai_tags_with_timeout(endpoint:&str,model:&str,description:&str,filename:&str,preview:&str,timeout:Duration)->Result<Vec<String>>{
    let remote=!local_endpoint(endpoint)?;
    if remote && !endpoint.starts_with("https://") {bail!("remote AI requires https");}
    let mut builder=reqwest::Client::builder().timeout(timeout).redirect(reqwest::redirect::Policy::none());
    // Loopback traffic must not follow HTTP(S)_PROXY or ALL_PROXY, or the excluded-file preview would leave the machine.
    if !remote {builder=builder.no_proxy();}
    let client=builder.build()?;
    let mut req=client.post(endpoint)
        .json(&json!({"model":model,"temperature":0,"messages":[
          {"role":"system","content":"Return a JSON array of 1-8 short lowercase tags, and nothing else. Do not include secrets."},
          {"role":"user","content":format!("Rule: {description}\nFilename: {filename}\nContent preview:\n{}",preview.chars().take(6000).collect::<String>())}]}));
    if let Ok(key)=std::env::var("LADON_AI_API_KEY"){req=req.bearer_auth(key);}
    let reply=req.send().await?;
    if reply.status().is_redirection(){bail!("AI endpoint answered with HTTP {}; redirects are not followed",reply.status());}
    let response=read_capped_json(reply.error_for_status()?).await?;
    let body=response.pointer("/choices/0/message/content").and_then(Value::as_str)
        .ok_or_else(||anyhow!("missing chat content"))?;
    let clean=body.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let tags:Vec<String>=serde_json::from_str(clean).map_err(|_|anyhow!("AI response was not a JSON tag array"))?;
    Ok(tags.into_iter().filter(|t|!t.trim().is_empty()&&t.len()<=80).take(8).collect())
}

/// Reads a response body in chunks and parses it as JSON, failing once it exceeds `max_bytes`.
async fn read_capped_json(mut response:reqwest::Response)->Result<Value>{
    let mut body=Vec::new();
    while let Some(chunk)=response.chunk().await?{
        if body.len()+chunk.len()>AI_RESPONSE_MAX_BYTES{bail!("AI response exceeds {AI_RESPONSE_MAX_BYTES} bytes");}
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests{
 use super::*;
 #[test]fn ut_tag_01_heuristic(){assert!(heuristic_tags("requirements.md","要件定義").contains(&"requirements".into()));}
 #[test]fn ut_tag_02_localhost(){assert!(local_endpoint("http://localhost:11434/v1/chat/completions").unwrap());assert!(!local_endpoint("https://api.openai.com/v1/chat/completions").unwrap());}
}
