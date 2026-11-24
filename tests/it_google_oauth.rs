//! Integration tests for the Google Drive sign-in (TC-801 to TC-809).
use ladon_explorer::google_oauth::{self, OAuthClient};
use ladon_explorer::provider::provider_from_env;
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Serializes the tests that change process environment variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Returns the redirect URI and state carried by an authorization URL.
fn redirect_and_state(url: &str) -> (String, String) {
    let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
    let mut redirect = String::new();
    let mut state = String::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "redirect_uri" => redirect = google_oauth::percent_decode(value).unwrap(),
            "state" => state = google_oauth::percent_decode(value).unwrap(),
            _ => {}
        }
    }
    (redirect, state)
}

#[test]
fn tc801_pkce_vector_and_verifier() {
    assert_eq!(
        google_oauth::code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let first = google_oauth::new_code_verifier();
    let second = google_oauth::new_code_verifier();
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c)));
    assert_ne!(first, second);
}

#[test]
fn tc802_encoding_round_trip_and_errors() {
    assert_eq!(google_oauth::base64url(&[0xfb, 0xff]), "-_8");
    let encoded = google_oauth::percent_encode("a b/é");
    assert_eq!(encoded, "a%20b%2F%C3%A9");
    assert_eq!(google_oauth::percent_decode(&encoded).unwrap(), "a b/é");
    assert_eq!(google_oauth::percent_decode("a+b%2F").unwrap(), "a b/");
    assert!(google_oauth::percent_decode("%zz").is_err());
    assert!(google_oauth::percent_decode("%FF").is_err());
}

#[test]
fn tc803_authorization_url_parameters() {
    let client = OAuthClient { client_id: "id.apps".into(), client_secret: Some("s".into()) };
    let url = google_oauth::authorization_url(&client, "http://127.0.0.1:5000/", "CHAL", "STATE");
    let expected = "https://accounts.google.com/o/oauth2/v2/auth?client_id=id.apps\
        &redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2F&response_type=code\
        &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.file&code_challenge=CHAL\
        &code_challenge_method=S256&state=STATE&access_type=offline&prompt=consent";
    assert_eq!(url, expected);
    assert!(!url.contains("client_secret"));
}

#[test]
fn tc804_callback_parsing() {
    assert_eq!(google_oauth::parse_callback("GET /?code=abc&state=S HTTP/1.1", "S").unwrap(), "abc");
    let mismatch = google_oauth::parse_callback("GET /?code=abc&state=S HTTP/1.1", "T").unwrap_err();
    assert_eq!(mismatch.to_string(), "state does not match");
    let denied = google_oauth::parse_callback("GET /?error=access_denied&state=S HTTP/1.1", "S").unwrap_err();
    assert_eq!(denied.to_string(), "authorization was not completed");
    let no_code = google_oauth::parse_callback("GET /?state=S HTTP/1.1", "S").unwrap_err();
    assert_eq!(no_code.to_string(), "authorization code is missing");
    let empty_code = google_oauth::parse_callback("GET /?code=&state=S HTTP/1.1", "S").unwrap_err();
    assert_eq!(empty_code.to_string(), "authorization code is missing");
    assert!(!mismatch.to_string().contains("abc"));
}

#[cfg(windows)]
#[test]
fn tc805_protected_storage_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    google_oauth::save_refresh_token(dir.path(), "refresh-value-for-test").unwrap();
    let bytes = std::fs::read(google_oauth::token_path(dir.path())).unwrap();
    let plain = b"refresh-value-for-test";
    assert!(!bytes.windows(plain.len()).any(|window| window == plain));
    assert_eq!(google_oauth::load_refresh_token(dir.path()).unwrap().as_deref(), Some("refresh-value-for-test"));
    google_oauth::delete_refresh_token(dir.path()).unwrap();
    assert_eq!(google_oauth::load_refresh_token(dir.path()).unwrap(), None);
    google_oauth::delete_refresh_token(dir.path()).unwrap();
}

#[cfg(not(windows))]
#[test]
fn tc805_protected_storage_is_windows_only() {
    let dir = tempfile::tempdir().unwrap();
    let error = google_oauth::save_refresh_token(dir.path(), "refresh-value-for-test").unwrap_err();
    assert_eq!(error.to_string(), "protected storage is available on Windows only");
    assert_eq!(google_oauth::load_refresh_token(dir.path()).unwrap(), None);
}

#[test]
fn tc806_client_from_environment() {
    let _guard = lock_env();
    std::env::remove_var("LADON_GOOGLE_CLIENT_ID");
    std::env::remove_var("LADON_GOOGLE_CLIENT_SECRET");
    assert_eq!(OAuthClient::from_env().err().unwrap().to_string(), "LADON_GOOGLE_CLIENT_ID is not set");
    std::env::set_var("LADON_GOOGLE_CLIENT_ID", "   ");
    assert_eq!(OAuthClient::from_env().err().unwrap().to_string(), "LADON_GOOGLE_CLIENT_ID is not set");
    std::env::set_var("LADON_GOOGLE_CLIENT_ID", "id.apps");
    std::env::set_var("LADON_GOOGLE_CLIENT_SECRET", "s");
    let client = OAuthClient::from_env().unwrap();
    assert_eq!(client.client_id, "id.apps");
    assert_eq!(client.client_secret.as_deref(), Some("s"));
    assert!(format!("{client:?}").contains("<redacted>"));
    assert!(!format!("{client:?}").contains("\"s\""));
    std::env::remove_var("LADON_GOOGLE_CLIENT_ID");
    std::env::remove_var("LADON_GOOGLE_CLIENT_SECRET");
}

#[tokio::test]
async fn tc807_login_times_out() {
    let client = OAuthClient { client_id: "id.apps".into(), client_secret: None };
    let started = Instant::now();
    let error = google_oauth::login(&client, |_| {}, Duration::from_millis(200)).await.unwrap_err();
    assert_eq!(error.to_string(), "sign-in timed out");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn tc808_loopback_receives_callback() {
    let client = OAuthClient { client_id: "id.apps".into(), client_secret: None };
    let mut reply = String::new();
    let mut state_seen = String::new();
    let result = google_oauth::login(
        &client,
        |url| {
            let (redirect, state) = redirect_and_state(url);
            state_seen = state.clone();
            let address = redirect.trim_start_matches("http://").trim_end_matches('/').to_string();
            let mut stream = TcpStream::connect(address).unwrap();
            write!(stream, "GET /?code=test-code&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            stream.read_to_string(&mut reply).unwrap();
        },
        Duration::from_secs(5),
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200 OK"));
    assert!(reply.contains("<!doctype html>"));
    let error = result.unwrap_err().to_string();
    assert!(error.starts_with("token exchange failed"), "unexpected message: {error}");
    assert!(!error.contains("test-code"));
    assert!(!error.contains(&state_seen));
}

#[test]
fn tc809_provider_token_source() {
    let _guard = lock_env();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("LADON_SETTINGS_DIR", dir.path());
    std::env::remove_var("LADON_GOOGLE_CLIENT_ID");
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");

    assert_eq!(google_oauth::settings_dir().unwrap(), dir.path().to_path_buf());
    let missing = provider_from_env("google_drive", None).err().unwrap();
    assert_eq!(missing.to_string(), "Google Drive is not connected");

    std::env::set_var("LADON_GOOGLE_ACCESS_TOKEN", "   ");
    let blank = provider_from_env("google_drive", None).err().unwrap();
    assert_eq!(blank.to_string(), "Google Drive is not connected");

    std::env::set_var("LADON_GOOGLE_ACCESS_TOKEN", "static-token");
    assert!(provider_from_env("google_drive", None).is_ok());

    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
    std::env::remove_var("LADON_SETTINGS_DIR");
}

#[cfg(windows)]
#[test]
fn tc809_stored_sign_in_needs_client_id() {
    let _guard = lock_env();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("LADON_SETTINGS_DIR", dir.path());
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
    std::env::remove_var("LADON_GOOGLE_CLIENT_ID");
    google_oauth::save_refresh_token(dir.path(), "stored-refresh-for-test").unwrap();

    let without_client = provider_from_env("google_drive", None).err().unwrap();
    assert_eq!(without_client.to_string(), "Google Drive is not connected");

    std::env::set_var("LADON_GOOGLE_CLIENT_ID", "id.apps");
    assert!(provider_from_env("google_drive", None).is_ok());

    std::env::remove_var("LADON_GOOGLE_CLIENT_ID");
    std::env::remove_var("LADON_SETTINGS_DIR");
}
