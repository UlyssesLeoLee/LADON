# Test Design: `src/main.rs`

Source: `src/main.rs`

Source-SHA256: `0157a7bc33f8567c64d2f4a89a5f804c0d3292f7177f03aa85d32850e58b4df1`

## Test Strategy

The command-line program has no unit tests. The two new commands are checked by hand on Windows with a real Google client, and the logic they call is covered by `tests/it_google_oauth.rs`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-813 | `ladon google-logout` with no stored sign-in. | Prints `Google Drive sign-in removed` and exits with status 0. | Manual, Windows 11 |
| TC-814 | `ladon google-login` with `LADON_GOOGLE_CLIENT_ID` unset. | Exits with status 1 and prints `LADON_GOOGLE_CLIENT_ID is not set`. | Manual, Windows 11 |
| TC-815 | `ladon google-login` with a client ID; complete the browser consent. | Prints `Google Drive sign-in saved`; `google-token.bin` exists in the settings folder and contains no plain token. | Manual, Windows 11 |

## Boundary & Failure Cases

- Closing the browser without consent ends the command after the timeout with `sign-in timed out`.

## Regression Scope

The existing commands; the shell menu entries are not changed.
