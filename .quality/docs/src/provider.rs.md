# Technical Design: `src/provider.rs`

Source: `src/provider.rs`

Source-SHA256: `490aeb44f19e35242dad7b48fd24784deb230c6852214267002d71f5672b0f56`

## Responsibility

`provider` moves encrypted objects to and from storage providers. Objects are addressed by content hash, and every payload is encrypted before it reaches this interface. Two providers exist: a filesystem provider for local and NAS folders, and a Google Drive provider.

## Dependencies

- `async_trait`, `reqwest` (Drive requests), `serde_json`, `crate::crypto::is_digest`, `crate::google_oauth` (token refresh and protected sign-in storage).

## Public Contract

- `pub trait StorageProvider`: `name`, `put(hash, encrypted) -> locator`, `get(locator) -> bytes`, `delete(locator)`.
- `pub struct FilesystemProvider`: `new(name, root)`. Objects are written to `root/<first two hex digits>/<hash>.ladon`.
- `pub struct GoogleDriveProvider { pub folder_id: Option<String>, pub use_appdata: bool }`. It is built by:
  - `GoogleDriveProvider::new(access_token, folder_id)`: a fixed access token from the environment. An empty token fails with `OAuth access token required`.
  - `GoogleDriveProvider::from_sign_in(refresh_token, folder_id)`: refreshes the access token with `google_oauth::refresh_access_token` before each request.
  - `set_appdata(enabled)`: the application data folder and a regular folder are mutually exclusive.
- Token source of a Drive request: a fixed token is used as it is. A sign-in refreshes the token first. A refresh failure fails the request and names no token.
- `pub fn provider_from_env(kind, path) -> Result<Box<dyn StorageProvider>>`:
  - `local` and `nas` need `path`.
  - `google_drive`: uses `LADON_GOOGLE_ACCESS_TOKEN` when set and non-empty. Otherwise it uses the sign-in stored in the settings folder (`google_oauth::load_refresh_token`). When neither exists it fails with `Google Drive is not connected`.
  - The sign-in is read from `google_oauth::settings_dir()`. The refresh token needs the client ID from `OAuthClient::from_env`; without it the provider fails with the same message.
  - The folder comes from `LADON_GOOGLE_FOLDER_ID`. `LADON_GOOGLE_MODE=appdata` selects the application data folder.
- Drive file IDs are validated against `A-Z a-z 0-9 _ -`, at most 256 characters.
- The resumable upload location must start with `https://www.googleapis.com/` or `https://www.google.com/`.

## Limits

- `delete` is disabled in this release; deletion needs a retention and approval step.
- Constructing a Drive provider sends no request. The first request that needs a token refreshes it.
- A sign-in provider keeps no access token between requests. Each request refreshes one, so a revoked sign-in fails on the next request.

## Dependencies (upstream)

`google_oauth` supplies the sign-in storage and the token refresh used by `GoogleDriveProvider::from_sign_in`. `engine` and `main` depend on the provider types.

## Change Notes

The Drive provider stores its token source in a private field. Code outside this module uses `GoogleDriveProvider::new` for a fixed token, and `provider_from_env` for the environment or the stored sign-in. A fixed token is no longer exposed as a public field.
