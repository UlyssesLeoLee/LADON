# Test Design: `src/provider.rs`

Source: `src/provider.rs`

Source-SHA256: `490aeb44f19e35242dad7b48fd24784deb230c6852214267002d71f5672b0f56`

## Test Strategy

Unit tests in `src/provider.rs` cover the filesystem round trip and the Drive identifier check. The token source of the Drive provider and `provider_from_env` are covered by `tests/it_google_oauth.rs`. No test makes a Drive request, because that needs an account.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-810 | A local provider writes a digest and reads it back. | The bytes read back equal the bytes written. | src/provider.rs::ut_provider_01_local_roundtrip |
| TC-811 | Drive identifiers `abc_123-X` and `../../etc/passwd`. | The first is accepted and the second is refused. | src/provider.rs::ut_provider_02_id_validation |
| TC-809 | As in `google_oauth.rs`. | `provider_from_env` reports the missing connection and then selects the environment token. | tests/it_google_oauth.rs::tc809_provider_token_source |

## Boundary & Failure Cases

- A Drive provider with an empty fixed token is refused at construction.
- `delete` returns an error in every case.

## Regression Scope

`engine.rs` (archive and load through a provider) and `main.rs` (`archive` command with the `google_drive` kind).
