# Technical Design: `src/main.rs`

Source: `src/main.rs`

Source-SHA256: `0157a7bc33f8567c64d2f4a89a5f804c0d3292f7177f03aa85d32850e58b4df1`

## Responsibility

`main.rs` is the command-line program `ladon`. It parses the command, opens the workspace given by `--root`, and runs the command. Two commands are handled before a workspace is opened: `shell-menu` and `shell-host-internal`, which are used by the Windows shell menu.

## Commands added for Google Drive

- `google-login`: runs `google_oauth::login` with the client from `OAuthClient::from_env`. The authorization URL is opened with `explorer.exe`. The timeout is five minutes. On success the refresh token is saved with `save_refresh_token` in the settings folder, and the command prints `Google Drive sign-in saved`.
- `google-logout`: deletes the stored sign-in with `delete_refresh_token` and prints `Google Drive sign-in removed`.
- Neither command needs a workspace. Neither prints a token, a code or a URL that contains a state value.
- The settings folder is `LADON_SETTINGS_DIR`, or `%LOCALAPPDATA%\LADON` when that variable is unset.

## Other commands

The other commands are unchanged: `init`, `capture`, `scan`, `watch`, `files`, `history`, `restore`, `archive`, `ignore-check`, `sensitive-add`, `tag-folder`, `search`, `agent-*`, `shell-menu` and `shell-host-internal`. Their behaviour is described in `docs/` and is not changed by this design.

## Dependencies

`clap` (command parsing), `tokio` (runtime), `ladon_explorer` (engine, provider, policy, Google sign-in).

## Public Contract

The command-line program has these commands, in order of their handling: `shell-menu` and `shell-host-internal` (before a workspace is opened), `google-login` and `google-logout` (before a workspace is opened), then `init`, `agent-*`, `sensitive-add`, `scan`, `capture`, `watch`, `files`, `history`, `restore`, `archive`, `ignore-check` and `tag-folder`. Each command returns status 0 on success and a non-zero status with a message on failure.

## Change Notes

`google-login` and `google-logout` are added. They need no workspace and print no token.
