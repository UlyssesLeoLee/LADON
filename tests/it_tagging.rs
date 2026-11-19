//! Integration tests for `tagging`: rule table, endpoint checks and the chat client.
use ladon_explorer::tagging::{ai_tags, heuristic_tags, local_endpoint};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Serves exactly one HTTP/1.1 response on a loopback port and returns the endpoint URL.
fn serve_once(status: u16, body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..n]);
            if let Some(end) = find(&received, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&received[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    .unwrap_or(0);
                if received.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let reason = if status == 200 { "OK" } else { "Error" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });
    format!("http://127.0.0.1:{port}/v1/chat/completions")
}

fn chat_body(content: &str) -> String {
    json!({"choices": [{"message": {"content": content}}]}).to_string()
}

#[test]
fn tc201_every_rule_label_reachable() {
    let cases = [
        ("requirements.md", "", "requirements"),
        ("", "基本設計", "basic-design"),
        ("", "詳細設計", "detailed-design"),
        ("", "unit test", "unit-test"),
        ("", "integration test", "integration-test"),
        ("", "システムテスト", "system-test"),
        ("lib.rs", "", "rust"),
        ("x.nema", "", "nema"),
    ];
    for (path, text, label) in cases {
        assert_eq!(heuristic_tags(path, text), vec![label.to_string()], "path={path:?} text={text:?}");
    }
}

#[test]
fn tc202_matching_is_case_insensitive() {
    assert!(heuristic_tags("README.MD", "Requirement list").contains(&"requirements".to_string()));
}

#[test]
fn tc203_labels_in_rule_table_order() {
    assert_eq!(
        heuristic_tags("design.rs", "architecture requirement"),
        vec!["requirements".to_string(), "basic-design".to_string(), "rust".to_string()]
    );
}

#[test]
fn tc204_no_match_returns_empty() {
    assert!(heuristic_tags("notes.txt", "hello world").is_empty());
    assert!(heuristic_tags("", "").is_empty());
}

#[test]
fn tc205_label_emitted_once() {
    assert_eq!(heuristic_tags("a.md", "requirement requirements 需求"), vec!["requirements".to_string()]);
}

#[tokio::test]
async fn tc206_ai_tags_parses_plain_json_array() {
    let url = serve_once(200, chat_body("[\"alpha\",\"beta\"]"));
    let tags = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap();
    assert_eq!(tags, vec!["alpha".to_string(), "beta".to_string()]);
}

#[tokio::test]
async fn tc207_ai_tags_accepts_fenced_json() {
    let url = serve_once(200, chat_body("```json\n[\"alpha\"]\n```"));
    let tags = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap();
    assert_eq!(tags, vec!["alpha".to_string()]);
}

#[tokio::test]
async fn tc208_ai_tags_filters_and_caps_results() {
    let exactly_80 = "e".repeat(80);
    let mut list = vec![String::new(), "x".repeat(81), exactly_80.clone()];
    list.extend((1..=9).map(|n| format!("t{n}")));
    let url = serve_once(200, chat_body(&serde_json::to_string(&list).unwrap()));
    let tags = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap();
    assert_eq!(tags.len(), 8);
    assert_eq!(tags[0], exactly_80);
    assert_eq!(tags[7], "t7");
    assert!(tags.iter().all(|t| !t.is_empty() && t.len() <= 80));
}

#[tokio::test]
async fn tc209_ai_tags_rejects_non_json_and_missing_content() {
    let url = serve_once(200, chat_body("not json at all"));
    let err = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("not a JSON tag array"), "{err}");

    let url = serve_once(200, json!({"choices": []}).to_string());
    let err = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("missing chat content"), "{err}");
}

#[tokio::test]
async fn tc210_ai_tags_rejects_remote_http_endpoint() {
    let err = ai_tags("http://example.invalid/v1", "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("remote AI requires https"), "{err}");
}

#[tokio::test]
async fn tc211_ai_tags_http_error_status_fails() {
    let url = serve_once(500, "{}".to_string());
    assert!(ai_tags(&url, "model", "rule", "a.md", "preview").await.is_err());
}

#[test]
fn tc212_local_endpoint_variants() {
    assert!(local_endpoint("http://localhost:11434/v1/chat/completions").unwrap());
    assert!(local_endpoint("http://127.0.0.1:8080/v1").unwrap());
    assert!(local_endpoint("http://[::1]:9/v1").unwrap());
    assert!(!local_endpoint("https://api.openai.com/v1/chat/completions").unwrap());
    let err = local_endpoint("ftp://localhost/x").unwrap_err();
    assert!(err.to_string().contains("http(s) endpoint only"), "{err}");
    assert!(local_endpoint("not a url").is_err());
}
