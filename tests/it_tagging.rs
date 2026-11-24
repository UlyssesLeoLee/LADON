//! Integration tests for `tagging`: rule table, endpoint checks and the chat client.
use ladon_explorer::tagging::{ai_tags, ai_tags_with_timeout, heuristic_tags, local_endpoint};
use serde_json::json;
use std::{
    ffi::OsString,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

static NO_PROXY_INIT: std::sync::Once = std::sync::Once::new();

/// Binds a loopback port and returns the listener with its endpoint URL.
/// Loopback traffic must never go through a developer's proxy, so NO_PROXY is set once
/// before the first HTTP client of this test binary is built.
fn loopback_listener() -> (TcpListener, String) {
    NO_PROXY_INIT.call_once(|| std::env::set_var("NO_PROXY", "127.0.0.1,localhost,::1"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, format!("http://127.0.0.1:{port}/v1/chat/completions"))
}

/// Reads one HTTP/1.1 request from `stream`, through the end of its Content-Length body.
fn read_request(stream: &mut TcpStream) {
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
}

/// Writes one HTTP/1.1 JSON response with the given status and a Content-Length body.
fn write_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 { "OK" } else { "Error" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Serves exactly one HTTP/1.1 response on a loopback port and returns the endpoint URL.
fn serve_once(status: u16, body: String) -> String {
    let (listener, url) = loopback_listener();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        write_response(&mut stream, status, &body);
    });
    url
}

/// Serves exactly one loopback 302 response to `location` that carries `body`, and returns the endpoint URL.
fn serve_redirect(location: &str, body: String) -> String {
    let (listener, url) = loopback_listener();
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        let _ = stream.write_all(response.as_bytes());
    });
    url
}

fn chat_body(content: &str) -> String {
    json!({"choices": [{"message": {"content": content}}]}).to_string()
}

/// Largest response body `ai_tags` buffers; mirrors `AI_RESPONSE_MAX_BYTES` in `tagging.rs`.
const AI_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Builds a chat response whose serialized body is exactly `total_len` bytes.
fn chat_body_of_len(content: &str, total_len: usize) -> String {
    let mut value = json!({"choices": [{"message": {"content": content}}], "padding": ""});
    let base_len = value.to_string().len();
    value["padding"] = json!("x".repeat(total_len - base_len));
    let body = value.to_string();
    assert_eq!(body.len(), total_len);
    body
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

    let url = serve_once(200, chat_body("```\n[\"beta\"]\n```"));
    let tags = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap();
    assert_eq!(tags, vec!["beta".to_string()]);
}

#[tokio::test]
async fn tc208_ai_tags_filters_and_caps_results() {
    let exactly_80 = "e".repeat(80);
    // Dropped: empty, whitespace only, 81 bytes, and 41 characters that occupy 82 bytes.
    let mut list = vec![String::new(), "   ".to_string(), "x".repeat(81), "é".repeat(41), exactly_80.clone()];
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
    // A valid body behind a 500 status proves the failure comes from the status check.
    let url = serve_once(500, chat_body("[\"alpha\"]"));
    let err = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("HTTP status server error"), "{err}");
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

/// Accepts one loopback connection and keeps it open without ever writing a reply.
fn serve_silent() -> String {
    let (listener, url) = loopback_listener();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_secs(60));
        drop(stream);
    });
    url
}

#[tokio::test]
async fn tc213_ai_tags_silent_endpoint_times_out() {
    let url = serve_silent();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        ai_tags_with_timeout(&url, "model", "rule", "a.md", "preview", Duration::from_secs(1)),
    )
    .await;
    let result = outcome.expect("ai_tags_with_timeout must return within 10 seconds when the endpoint never answers");
    let err = result.unwrap_err();
    assert!(
        err.downcast_ref::<reqwest::Error>().is_some_and(|e| e.is_timeout()),
        "expected a request timeout, got: {err:?}"
    );
}

#[tokio::test]
async fn tc214_ai_tags_rejects_response_over_size_cap() {
    let url = serve_once(200, chat_body_of_len("[\"alpha\"]", AI_RESPONSE_MAX_BYTES + 1));
    let err = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("AI response exceeds"), "{err}");
}

#[tokio::test]
async fn tc215_ai_tags_accepts_response_at_size_cap() {
    let url = serve_once(200, chat_body_of_len("[\"alpha\"]", AI_RESPONSE_MAX_BYTES));
    let tags = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap();
    assert_eq!(tags, vec!["alpha".to_string()]);
}

#[tokio::test]
async fn tc216_ai_tags_does_not_follow_redirects() {
    // The redirect response and its target both carry valid tags, so either path would succeed without the guard.
    let (target, target_url) = loopback_listener();
    let (reached_tx, reached_rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = target.accept() {
            read_request(&mut stream);
            write_response(&mut stream, 200, &chat_body("[\"leaked\"]"));
            let _ = reached_tx.send(());
        }
    });
    let url = serve_redirect(&target_url, chat_body("[\"from-redirect\"]"));
    let err = ai_tags(&url, "model", "rule", "a.md", "preview").await.unwrap_err();
    assert!(err.to_string().contains("redirects are not followed"), "{err}");
    assert!(
        reached_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "the redirect target must not receive a request"
    );
}

/// Proxy variables that reqwest reads from the environment when a client is built.
const PROXY_ENV_VARS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// Clears every proxy variable and routes HTTP through `proxy` until dropped. Drop restores the previous values, even after a panic.
struct ProxyEnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl ProxyEnvGuard {
    fn route_http_through(proxy: &str) -> Self {
        let saved = PROXY_ENV_VARS.iter().map(|name| (*name, std::env::var_os(name))).collect();
        for name in PROXY_ENV_VARS {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", proxy);
        Self { saved }
    }
}

impl Drop for ProxyEnvGuard {
    fn drop(&mut self) {
        for (name, value) in &self.saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[tokio::test]
async fn tc217_loopback_endpoint_ignores_http_proxy() {
    // serve_once runs the one-time NO_PROXY setup before the guard, so no later listener can re-add the bypass while it is active.
    let url = serve_once(200, chat_body("[\"loopback\"]"));
    let _env = ProxyEnvGuard::route_http_through("http://127.0.0.1:9");
    let tags = ai_tags_with_timeout(&url, "model", "rule", "a.md", "preview", Duration::from_secs(10))
        .await
        .expect("loopback endpoint must be reached directly, not through HTTP_PROXY");
    assert_eq!(tags, vec!["loopback".to_string()]);
}
