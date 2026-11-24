# Test Design: `src/main.rs`

Source: `src/main.rs`

Source-SHA256: `426480dffa98a8695fcc16ce10882fa115edb229f6c80ff3b7905c4eafc7c08e`

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
