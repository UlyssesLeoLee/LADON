# Test Design: `src/google_oauth.rs`

Source: `src/google_oauth.rs`

Source-SHA256: `4cf5b483493fe0a1c63ec332f1a6d6227c4c6a0e265bfb7845c7f6aca5caa4c5`

## Test Strategy

Integration tests in `tests/it_google_oauth.rs`. The pure functions (verifier, challenge, encoding, URL and callback parsing) are tested directly against their specified values. Storage is tested with a temporary folder and runs only on Windows, where DPAPI exists. The loopback login is tested with a real socket and a simulated browser on the same machine. The token endpoint is not contacted with real credentials, so the exchange step only checks that a failure names no secret. The success and failure paths of the token requests and of the login are tested in `src/google_oauth.rs` against a one-shot loopback mock endpoint (TC-816).

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-801 | Verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` from RFC 7636 appendix B; `new_code_verifier()` called twice. | The challenge is `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`. The generated verifiers are 64 characters from the unreserved set and differ. | tests/it_google_oauth.rs::tc801_pkce_vector_and_verifier |
| TC-802 | `base64url` of bytes `[0xfb, 0xff]`; `percent_encode("a b/é")`; `percent_decode` of that result and of `a+b%2F`; `percent_decode("%zz")`. | `base64url` gives `-_8`. Encoding gives `a%20b%2F%C3%A9`. Decoding round-trips, `a+b%2F` gives `a b/`, and the invalid escape is an error. | tests/it_google_oauth.rs::tc802_encoding_round_trip_and_errors |
| TC-803 | A client with ID `id.apps` and secret `s`; `authorization_url` with a redirect of `http://127.0.0.1:5000/` and a challenge and state. | The URL starts with the Google authorization endpoint and carries each parameter in the specified order, with `scope` equal to `https://www.googleapis.com/auth/drive.file`, `code_challenge_method=S256`, `access_type=offline`, `prompt=consent`, and the redirect percent-encoded. | tests/it_google_oauth.rs::tc803_authorization_url_parameters |
| TC-804 | Request lines: a valid `GET /?code=abc&state=S HTTP/1.1` with state `S`; state `T` for the same line; `GET /?error=access_denied&state=S HTTP/1.1`; `GET /?state=S HTTP/1.1`; `GET /?code=&state=S HTTP/1.1`. | The first returns `abc`. The others fail with `state does not match`, `authorization was not completed`, `authorization code is missing` and `authorization code is missing`. No error text contains `abc` or `S`. | tests/it_google_oauth.rs::tc804_callback_parsing |
| TC-805 | Windows only. A temporary settings folder; `save_refresh_token` with `refresh-value-for-test`; the bytes of the token file; `load_refresh_token`; `delete_refresh_token` twice; `load_refresh_token` afterwards. | The token file exists and its bytes do not contain the plain value. The load returns the value. After deletion, the load returns `None`, and the second deletion succeeds. | tests/it_google_oauth.rs::tc805_protected_storage_round_trip |
| TC-806 | `LADON_GOOGLE_CLIENT_ID` unset, then set to a blank value, then set to `id.apps`; `LADON_GOOGLE_CLIENT_SECRET` set to `s` in the last case. | The first two fail with `LADON_GOOGLE_CLIENT_ID is not set`. The last gives client ID `id.apps` and secret `Some("s")`. | tests/it_google_oauth.rs::tc806_client_from_environment |
| TC-807 | Loopback login with a timeout of 200 ms and no request from the browser. | The result is `sign-in timed out`, and the call returns within 5 seconds. | tests/it_google_oauth.rs::tc807_login_times_out |
| TC-808 | Loopback login with a 5-second timeout. The `open_url` callback parses the state from the URL and sends `GET /?code=test-code&state=<state> HTTP/1.1` to the redirect address, then reads the HTTP reply. | The reply is HTTP 200 with the fixed HTML page. The login then fails at the token exchange with a message that starts with `token exchange failed`, and the message contains neither `test-code` nor the state. | tests/it_google_oauth.rs::tc808_loopback_receives_callback |
| TC-809 | `LADON_GOOGLE_ACCESS_TOKEN` unset and a settings folder with no stored sign-in: `provider_from_env("google_drive", None)`; then the environment token set to `static-token` and the same call. | The first fails with `Google Drive is not connected`. The second succeeds and makes no network request at construction. `settings_dir()` returns the `LADON_SETTINGS_DIR` value when that variable is set. | tests/it_google_oauth.rs::tc809_provider_token_source |
| TC-816 | One-shot loopback mock endpoint. Token exchange answers 200 with both tokens; exchange answers 400 with a body that holds a detail value; refresh answers 200 with an access token and 401 for a refresh token; login with a simulated browser answers with a refresh token, then with no refresh token. | Exchange returns both tokens, and its `Debug` output hides them. The 400 reply fails with `token exchange failed (HTTP 400)`, and the detail value is absent. Refresh returns the access token. The 401 reply fails with `access token refresh failed` and names no refresh token. Login returns the refresh token. Login without a refresh token fails with the Google message. | src/google_oauth.rs::tests::ut_google_01_exchange_returns_tokens_without_printing_them to ut_google_06_login_without_refresh_token_is_refused |
| TC-817 | `Debug` output of an `OAuthClient` with a secret. | The output shows the client ID and `<redacted>`, and does not show the secret. | src/google_oauth.rs::tests::ut_google_07_client_debug_hides_secret |
| TC-818 | Off Windows only: `save_refresh_token` and `load_refresh_token` in an empty settings folder. | Saving fails with `protected storage is available on Windows only`. Loading returns `None`. | tests/it_google_oauth.rs::tc805_protected_storage_is_windows_only |

## Boundary & Failure Cases

- An invalid percent escape, a bad UTF-8 result, and a state mismatch are all refused without a partial result (TC-802, TC-804).
- A blank client ID is treated as missing (TC-806).
- A token file that cannot be unprotected fails without exposing the content (see `stored sign-in is unreadable`).
- Off Windows, saving a token fails with `protected storage is available on Windows only` (TC-818).
- A stored sign-in that is empty after unprotecting is reported as `stored sign-in is unreadable`.

## Regression Scope

`provider.rs` (the Drive provider and `provider_from_env`) and `main.rs` (`google-login`, `google-logout`) depend on this module.
