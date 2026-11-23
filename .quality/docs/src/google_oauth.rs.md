# Technical Design: `src/google_oauth.rs`

Source: `src/google_oauth.rs`

Source-SHA256: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`

## Responsibility

`google_oauth` signs the desktop application in to Google Drive with the OAuth 2.0 authorization code flow and PKCE (RFC 7636) over a loopback redirect (RFC 8252). It produces a refresh token, stores that token only in protected form on this machine, exchanges codes, and refreshes access tokens. The only scope requested is `https://www.googleapis.com/auth/drive.file`, which covers only files that LADON creates. The module never writes a token, a code or a verifier to a log, an error message or a reply.

## Dependencies

- `sha2` (SHA-256 for the code challenge), `rand` (verifier and state), `reqwest` with rustls and json (token endpoint), `serde_json` (token response), `anyhow`.
- `std::net::TcpListener`, `std::sync::mpsc` and `std::thread` for the loopback redirect.
- On Windows, `crypt32.dll` (`CryptProtectData`, `CryptUnprotectData`) and `kernel32.dll` (`LocalFree`) for the protected storage.

## Configuration

- `LADON_GOOGLE_CLIENT_ID` (required): OAuth client ID of a Desktop application in the Google Cloud console. The code has no default value.
- `LADON_GOOGLE_CLIENT_SECRET` (optional): client secret. It is sent with token requests when set.

## Public Contract

- `pub struct OAuthClient { pub client_id: String, pub client_secret: Option<String> }`
  - `OAuthClient::from_env() -> Result<OAuthClient>`: reads the two variables. A missing or blank client ID fails with `LADON_GOOGLE_CLIENT_ID is not set`.
- `pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";`
- `pub fn new_code_verifier() -> String`: 64 characters from `A-Z a-z 0-9 - . _ ~`, drawn from `rand::thread_rng`.
- `pub fn code_challenge(verifier: &str) -> String`: `base64url(SHA-256(verifier))` without padding.
- `pub fn base64url(bytes: &[u8]) -> String`: the RFC 4648 section 5 alphabet, without padding.
- `pub fn percent_encode(text: &str) -> String`: every byte outside `A-Z a-z 0-9 - . _ ~` becomes `%XX` with uppercase hexadecimal digits.
- `pub fn percent_decode(text: &str) -> Result<String>`: decodes `%XX` and maps `+` to a space. An invalid escape or a result that is not UTF-8 is an error.
- `pub fn authorization_url(client: &OAuthClient, redirect_uri: &str, challenge: &str, state: &str) -> String`: `https://accounts.google.com/o/oauth2/v2/auth` with the query parameters, in this order: `client_id`, `redirect_uri`, `response_type=code`, `scope`, `code_challenge`, `code_challenge_method=S256`, `state`, `access_type=offline`, `prompt=consent`. Every value is percent-encoded.
- `pub fn parse_callback(request_line: &str, expected_state: &str) -> Result<String>`: accepts `GET /?<query> HTTP/1.x` and returns the authorization code.
  - A query with an `error` parameter fails with `authorization was not completed`.
  - A missing or different `state` fails with `state does not match`.
  - A missing or empty `code` fails with `authorization code is missing`.
  - No error text contains a query value.
- `pub struct Tokens { pub access_token: String, pub refresh_token: Option<String>, pub expires_in: u64 }`. Its `Debug` implementation prints `<redacted>` for both token fields.
- `pub async fn exchange_code(client: &OAuthClient, redirect_uri: &str, code: &str, verifier: &str) -> Result<Tokens>`: POSTs an `application/x-www-form-urlencoded` body to `https://oauth2.googleapis.com/token` with `grant_type=authorization_code`. A non-success status fails with `token exchange failed (HTTP <status>)`, and the response body is not included.
- `pub async fn refresh_access_token(client: &OAuthClient, refresh_token: &str) -> Result<String>`: POSTs `grant_type=refresh_token` and returns the access token. A failure is `access token refresh failed`.
- `pub async fn login(client: &OAuthClient, open_url: impl FnOnce(&str), timeout: Duration) -> Result<String>`:
  1. Binds `127.0.0.1` on an ephemeral port. The redirect URI is `http://127.0.0.1:<port>/`.
  2. Generates the verifier and a 32-character state.
  3. Calls `open_url` with the authorization URL.
  4. Waits up to `timeout` for one connection. The connection is answered with a fixed HTML page, and the listener is closed after one request.
  5. Parses the callback, exchanges the code, and returns the refresh token.
  - Without a request in time the result is `sign-in timed out`.
  - When Google returns no refresh token the result is `Google did not return a refresh token; revoke the access of LADON in the Google account and sign in again`.
- `pub fn settings_dir() -> Option<PathBuf>`: the folder that holds the sign-in. It is the value of `LADON_SETTINGS_DIR` when that variable is set, otherwise `%LOCALAPPDATA%\LADON`. `None` when neither is available.
- `pub fn token_path(settings_dir: &Path) -> PathBuf`: `settings_dir/google-token.bin`.
- `pub fn save_refresh_token(settings_dir: &Path, refresh_token: &str) -> Result<()>`: writes the protected bytes to a temporary file and renames it over the token file. The protection is the current-user DPAPI scope with the description `LADON Google Drive`. Off Windows the result is `protected storage is available on Windows only`.
- `pub fn load_refresh_token(settings_dir: &Path) -> Result<Option<String>>`: `None` when the file is absent. An unreadable file fails with `stored sign-in is unreadable`.
- `pub fn delete_refresh_token(settings_dir: &Path) -> Result<()>`: removes the token file when present. Calling it again is not an error.

## Protection

- The refresh token leaves memory only as the DPAPI output. The plain value is never written to disk.
- DPAPI binds the stored value to the Windows user who saved it.
- The loopback page is fixed HTML and never repeats any part of the request.

## Limits

- The loopback listener accepts one connection per login. A second connection is not served.
- The token refresh runs on every Drive request that needs a token. No access token is cached.

## Change Notes

This module is new. It adds no change to an existing public function.
