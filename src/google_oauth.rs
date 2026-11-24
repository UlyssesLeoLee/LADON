//! Google Drive sign-in: OAuth 2.0 authorization code flow with PKCE over a loopback redirect.
//! The refresh token is stored only in DPAPI-protected form. No token, code, verifier or secret
//! appears in an error message, a reply or a log line.
use anyhow::{anyhow, bail, Context, Result};
use rand::distributions::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

/// The only scope requested. It covers files that LADON creates.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const TOKEN_FILE: &str = "google-token.bin";
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const CALLBACK_PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>LADON</title></head><body><p>Google sign-in was received. Return to LADON to finish.</p></body></html>";
const REFRESH_FAILED: &str = "access token refresh failed";
const NO_REFRESH_TOKEN: &str = "Google did not return a refresh token; revoke the access of LADON in the Google account and sign in again";

/// OAuth client of a Desktop application. The client secret is never printed.
#[derive(Clone)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: Option<String>,
}

struct Redacted;
impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| Redacted))
            .finish()
    }
}

impl OAuthClient {
    /// Reads `LADON_GOOGLE_CLIENT_ID` (required) and `LADON_GOOGLE_CLIENT_SECRET` (optional).
    pub fn from_env() -> Result<OAuthClient> {
        let client_id = std::env::var("LADON_GOOGLE_CLIENT_ID").unwrap_or_default();
        if client_id.trim().is_empty() {
            bail!("LADON_GOOGLE_CLIENT_ID is not set");
        }
        let client_secret = std::env::var("LADON_GOOGLE_CLIENT_SECRET")
            .ok()
            .filter(|secret| !secret.trim().is_empty());
        Ok(OAuthClient { client_id: client_id.trim().to_string(), client_secret })
    }
}

/// Token response of the token endpoint. Its `Debug` output hides both token values.
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &Redacted)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| Redacted))
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(serde::Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

/// 64 characters from the RFC 7636 unreserved set.
pub fn new_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    (0..64).map(|_| UNRESERVED[rng.gen_range(0..UNRESERVED.len())] as char).collect()
}

fn random_state() -> String {
    rand::thread_rng().sample_iter(&Alphanumeric).take(32).map(char::from).collect()
}

/// `base64url(SHA-256(verifier))` without padding.
pub fn code_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// RFC 4648 section 5 alphabet, without padding.
pub fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 3);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64URL[(n >> 18) as usize & 63] as char);
        out.push(BASE64URL[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(BASE64URL[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(BASE64URL[n as usize & 63] as char);
        }
    }
    out
}

/// Every byte outside the unreserved set becomes `%XX` with uppercase hexadecimal digits.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// Decodes `%XX` escapes and maps `+` to a space. An invalid escape or a result that is not UTF-8 is an error.
pub fn percent_decode(text: &str) -> Result<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes.get(index + 1).and_then(|c| hex_value(*c)).ok_or_else(|| anyhow!("invalid percent escape"))?;
                let low = bytes.get(index + 2).and_then(|c| hex_value(*c)).ok_or_else(|| anyhow!("invalid percent escape"))?;
                out.push(high * 16 + low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| anyhow!("decoded text is not UTF-8"))
}

/// Authorization URL with the parameters in a fixed order. Every value is percent-encoded.
pub fn authorization_url(client: &OAuthClient, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let params = [
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{AUTHORIZATION_ENDPOINT}?{query}")
}

/// Reads the authorization code from `GET /?<query> HTTP/1.x`. No error text contains a query value.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<String> {
    let query = request_line
        .strip_prefix("GET /?")
        .and_then(|rest| rest.split_once(" HTTP/1."))
        .map(|(query, _)| query)
        .ok_or_else(|| anyhow!("authorization was not completed"))?;
    let mut code = None::<String>;
    let mut state = None::<String>;
    let mut failed = false;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value)?;
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => failed = true,
            _ => {}
        }
    }
    if failed {
        bail!("authorization was not completed");
    }
    if state.as_deref() != Some(expected_state) {
        bail!("state does not match");
    }
    match code {
        Some(code) if !code.is_empty() => Ok(code),
        _ => bail!("authorization code is missing"),
    }
}

async fn post_form(endpoint: &str, params: &[(&str, &str)]) -> Result<(reqwest::StatusCode, String)> {
    let body = params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(60)).build()?;
    let response = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;
    let status = response.status();
    Ok((status, response.text().await?))
}

async fn exchange_at(endpoint: &str, client: &OAuthClient, redirect_uri: &str, code: &str, verifier: &str) -> Result<Tokens> {
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = client.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    let (status, text) = post_form(endpoint, &params)
        .await
        .map_err(|_| anyhow!("token exchange failed (network error)"))?;
    if !status.is_success() {
        bail!("token exchange failed (HTTP {})", status.as_u16());
    }
    let reply: TokenReply = serde_json::from_str(&text).map_err(|_| anyhow!("token exchange failed (invalid response)"))?;
    Ok(Tokens { access_token: reply.access_token, refresh_token: reply.refresh_token, expires_in: reply.expires_in })
}

async fn refresh_at(endpoint: &str, client: &OAuthClient, refresh_token: &str) -> Result<String> {
    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client.client_id.as_str()),
    ];
    if let Some(secret) = client.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    let (status, text) = post_form(endpoint, &params).await.map_err(|_| anyhow!(REFRESH_FAILED))?;
    if !status.is_success() {
        bail!(REFRESH_FAILED);
    }
    let reply: TokenReply = serde_json::from_str(&text).map_err(|_| anyhow!(REFRESH_FAILED))?;
    if reply.access_token.is_empty() {
        bail!(REFRESH_FAILED);
    }
    Ok(reply.access_token)
}

/// Exchanges an authorization code for tokens.
pub async fn exchange_code(client: &OAuthClient, redirect_uri: &str, code: &str, verifier: &str) -> Result<Tokens> {
    exchange_at(TOKEN_ENDPOINT, client, redirect_uri, code, verifier).await
}

/// Returns a new access token for a stored refresh token.
pub async fn refresh_access_token(client: &OAuthClient, refresh_token: &str) -> Result<String> {
    refresh_at(TOKEN_ENDPOINT, client, refresh_token).await
}

/// Runs one sign-in. `open_url` receives the authorization URL. Returns the refresh token.
pub async fn login(client: &OAuthClient, open_url: impl FnOnce(&str), timeout: Duration) -> Result<String> {
    login_at(TOKEN_ENDPOINT, client, open_url, timeout).await
}

async fn login_at(token_endpoint: &str, client: &OAuthClient, open_url: impl FnOnce(&str), timeout: Duration) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).context("the local sign-in port could not be opened")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/");
    let verifier = new_code_verifier();
    let state = random_state();
    // The listener must accept before the browser is opened, so the redirect is never missed.
    let (sender, receiver) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = sender.send(accept_callback(listener, timeout));
    });
    let url = authorization_url(client, &redirect_uri, &code_challenge(&verifier), &state);
    open_url(url.as_str());
    let request_line = receiver
        .recv_timeout(timeout)
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("sign-in timed out"))?;
    let code = parse_callback(&request_line, &state)?;
    let tokens = exchange_at(token_endpoint, client, &redirect_uri, &code, &verifier).await?;
    tokens.refresh_token.ok_or_else(|| anyhow!(NO_REFRESH_TOKEN))
}

/// Accepts one connection before the deadline, answers it with the fixed page, and returns its request line.
fn accept_callback(listener: TcpListener, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    listener.set_nonblocking(true).ok()?;
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    };
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let request_line = read_request_line(&mut stream)?;
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        CALLBACK_PAGE.len(),
        CALLBACK_PAGE
    );
    let _ = stream.write_all(reply.as_bytes());
    Some(request_line)
}

fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    while data.len() < 8192 && !data.contains(&b'\n') {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);
    }
    let end = data.iter().position(|byte| *byte == b'\n')?;
    Some(String::from_utf8_lossy(&data[..end]).trim_end_matches('\r').to_string())
}

/// Folder that holds the sign-in: `LADON_SETTINGS_DIR` when set, otherwise `%LOCALAPPDATA%\LADON`.
pub fn settings_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LADON_SETTINGS_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(|base| PathBuf::from(base).join("LADON"))
}

pub fn token_path(settings_dir: &Path) -> PathBuf {
    settings_dir.join(TOKEN_FILE)
}

/// Writes the protected refresh token. The file is replaced in one rename.
pub fn save_refresh_token(settings_dir: &Path, refresh_token: &str) -> Result<()> {
    let protected = protection::protect(refresh_token.as_bytes())?;
    fs::create_dir_all(settings_dir)?;
    let temporary = settings_dir.join(format!("{TOKEN_FILE}.tmp"));
    fs::write(&temporary, &protected)?;
    fs::rename(&temporary, token_path(settings_dir))?;
    Ok(())
}

/// Reads the stored refresh token. `None` when no sign-in is stored.
pub fn load_refresh_token(settings_dir: &Path) -> Result<Option<String>> {
    let protected = match fs::read(token_path(settings_dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("stored sign-in is unreadable"),
    };
    let plain = protection::unprotect(&protected).map_err(|_| anyhow!("stored sign-in is unreadable"))?;
    let token = String::from_utf8(plain).map_err(|_| anyhow!("stored sign-in is unreadable"))?;
    if token.trim().is_empty() {
        bail!("stored sign-in is unreadable");
    }
    Ok(Some(token))
}

/// Removes the stored sign-in. Calling it again is not an error.
pub fn delete_refresh_token(settings_dir: &Path) -> Result<()> {
    match fs::remove_file(token_path(settings_dir)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
mod protection {
    use anyhow::{anyhow, bail, Result};
    use std::ffi::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *const c_void,
            prompt: *const c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    fn take_output(output: DataBlob) -> Vec<u8> {
        if output.pb_data.is_null() {
            return Vec::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();
        unsafe { LocalFree(output.pb_data as *mut c_void) };
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
        let length = u32::try_from(plain.len()).map_err(|_| anyhow!("the sign-in is too large to protect"))?;
        let input = DataBlob { cb_data: length, pb_data: plain.as_ptr() as *mut u8 };
        let description: Vec<u16> = "LADON Google Drive".encode_utf16().chain(std::iter::once(0)).collect();
        let mut output = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
        let ok = unsafe {
            CryptProtectData(
                &input,
                description.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            bail!("the sign-in could not be protected");
        }
        Ok(take_output(output))
    }

    pub fn unprotect(protected: &[u8]) -> Result<Vec<u8>> {
        let length = u32::try_from(protected.len()).map_err(|_| anyhow!("stored sign-in is unreadable"))?;
        let input = DataBlob { cb_data: length, pb_data: protected.as_ptr() as *mut u8 };
        let mut output = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            bail!("stored sign-in is unreadable");
        }
        Ok(take_output(output))
    }
}

#[cfg(not(windows))]
mod protection {
    use anyhow::{bail, Result};

    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>> {
        bail!("protected storage is available on Windows only")
    }

    pub fn unprotect(_protected: &[u8]) -> Result<Vec<u8>> {
        bail!("protected storage is available on Windows only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-shot token endpoint on the loopback interface. It answers the first request with `status` and `body`.
    fn mock_endpoint(status: u16, body: &'static str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let read = stream.read(&mut chunk).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok()))
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let reply = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        });
        format!("http://{address}/token")
    }

    /// Plays the browser: follows the authorization URL and sends the callback to the loopback listener.
    fn answer_like_browser(url: &str, code: &str) {
        let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
        let mut redirect = String::new();
        let mut state = String::new();
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            match key {
                "redirect_uri" => redirect = percent_decode(value).unwrap(),
                "state" => state = percent_decode(value).unwrap(),
                _ => {}
            }
        }
        let address = redirect.trim_start_matches("http://").trim_end_matches('/').to_string();
        let mut stream = TcpStream::connect(address).unwrap();
        write!(stream, "GET /?code={code}&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200 OK"));
    }

    fn client() -> OAuthClient {
        OAuthClient { client_id: "id.apps".into(), client_secret: Some("client-secret-value".into()) }
    }

    #[tokio::test]
    async fn ut_google_01_exchange_returns_tokens_without_printing_them() {
        let endpoint = mock_endpoint(200, r#"{"access_token":"access-value","refresh_token":"refresh-value","expires_in":3599}"#);
        let tokens = exchange_at(&endpoint, &client(), "http://127.0.0.1:1/", "code", "verifier").await.unwrap();
        assert_eq!(tokens.access_token, "access-value");
        assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-value"));
        assert_eq!(tokens.expires_in, 3599);
        let shown = format!("{tokens:?}");
        assert!(shown.contains("<redacted>"));
        assert!(!shown.contains("access-value"));
        assert!(!shown.contains("refresh-value"));
    }

    #[tokio::test]
    async fn ut_google_02_exchange_failure_names_only_the_status() {
        let endpoint = mock_endpoint(400, r#"{"error":"invalid_grant","detail":"body-detail-value"}"#);
        let error = exchange_at(&endpoint, &client(), "http://127.0.0.1:1/", "code", "verifier").await.err().unwrap();
        assert_eq!(error.to_string(), "token exchange failed (HTTP 400)");
    }

    #[tokio::test]
    async fn ut_google_03_refresh_returns_access_token() {
        let endpoint = mock_endpoint(200, r#"{"access_token":"fresh-access","expires_in":3599}"#);
        assert_eq!(refresh_at(&endpoint, &client(), "stored-refresh").await.unwrap(), "fresh-access");
    }

    #[tokio::test]
    async fn ut_google_04_refresh_failure_names_no_token() {
        let endpoint = mock_endpoint(401, r#"{"error":"invalid_grant"}"#);
        let error = refresh_at(&endpoint, &client(), "stored-refresh-value").await.err().unwrap();
        assert_eq!(error.to_string(), REFRESH_FAILED);
        assert!(!error.to_string().contains("stored-refresh-value"));
    }

    #[tokio::test]
    async fn ut_google_05_login_returns_refresh_token() {
        let endpoint = mock_endpoint(200, r#"{"access_token":"a-value","refresh_token":"login-refresh-value","expires_in":3599}"#);
        let refresh = login_at(&endpoint, &client(), |url| answer_like_browser(url, "code-1"), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(refresh, "login-refresh-value");
    }

    #[tokio::test]
    async fn ut_google_06_login_without_refresh_token_is_refused() {
        let endpoint = mock_endpoint(200, r#"{"access_token":"a-value","expires_in":3599}"#);
        let error = login_at(&endpoint, &client(), |url| answer_like_browser(url, "code-2"), Duration::from_secs(5))
            .await
            .err()
            .unwrap();
        assert_eq!(error.to_string(), NO_REFRESH_TOKEN);
    }

    #[test]
    fn ut_google_07_client_debug_hides_secret() {
        let shown = format!("{:?}", client());
        assert!(shown.contains("id.apps"));
        assert!(!shown.contains("client-secret-value"));
    }
}
