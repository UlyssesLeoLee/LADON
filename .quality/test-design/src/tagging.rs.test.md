# Test Design: `src/tagging.rs`

Source: `src/tagging.rs`

Source-SHA256: `ae4d8561bcfe1e5c68928c33914df0138edacdc21308b422fa0e73ab2d8a3560`

## Test Strategy

Integration tests call the public functions directly. `heuristic_tags` and `local_endpoint` need no fixtures. `ai_tags` and `ai_tags_with_timeout` are tested against a real HTTP listener built with `std::net::TcpListener` on `127.0.0.1:0` in a helper thread (`serve_once`) that serves one canned HTTP/1.1 response per listener (a test that needs two responses calls `serve_once` twice). `serve_silent` accepts one connection and never replies (TC-213); `serve_redirect` answers one 302 response with a `Location` header (TC-216). The timeout test passes a one-second budget to `ai_tags_with_timeout`; `ai_tags` applies a 60-second budget (`AI_REQUEST_TIMEOUT`) that no test lets expire. The size tests build bodies of an exact byte length with `chat_body_of_len` (TC-214, TC-215). The production request code therefore runs end to end without mocks and without extra dependencies. Because the HTTP client honours proxy environment variables for remote endpoints, the helper sets `NO_PROXY` for loopback addresses once, before the first listener of the test process is bound. `tc217` removes every proxy variable, including `NO_PROXY`, for its own duration and sets `HTTP_PROXY` to an address with no listener, so it passes only when the loopback endpoint bypasses the proxy in the code. The other tests that send a request use loopback endpoints, which bypass the proxy in the code as well, so the temporary variables do not change their outcome (TC-210 returns its error before any request is sent); apart from the proxy variables, no environment variable is touched. `src/tagging.rs` also holds the inline unit tests `ut_tag_01_heuristic` and `ut_tag_02_localhost`.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-201 | Eight inputs, one per label, such as path requirements.md, text 基本設計, text unit test, path lib.rs, path x.nema | Each input yields exactly its own label | Pass; tests/it_tagging.rs::tc201_every_rule_label_reachable |
| TC-202 | Path README.MD with text Requirement list | Result contains requirements | Pass; tests/it_tagging.rs::tc202_matching_is_case_insensitive |
| TC-203 | Path design.rs with text architecture requirement | Exactly requirements, basic-design, rust in that order | Pass; tests/it_tagging.rs::tc203_labels_in_rule_table_order |
| TC-204 | Path notes.txt with text hello world; and two empty strings | Empty vectors | Pass; tests/it_tagging.rs::tc204_no_match_returns_empty |
| TC-205 | Text containing requirement, requirements and the Chinese keyword together | The requirements label appears once | Pass; tests/it_tagging.rs::tc205_label_emitted_once |
| TC-206 | Local stub returns a chat body whose content is a JSON array of two tags | ai_tags returns those two tags | Pass; tests/it_tagging.rs::tc206_ai_tags_parses_plain_json_array |
| TC-207 | Stub content wrapped in a code fence with the json marker, and a second stub with a plain code fence | Fences stripped in both cases and the tags parsed | Pass; tests/it_tagging.rs::tc207_ai_tags_accepts_fenced_json |
| TC-208 | Stub returns an empty tag, a whitespace-only tag, an 81-byte tag, a 41-character tag of 82 bytes, an 80-byte tag and nine short tags | Only valid tags remain, the 80-byte tag is kept, and exactly eight tags are returned | Pass; tests/it_tagging.rs::tc208_ai_tags_filters_and_caps_results |
| TC-209 | Stub returns content that is not JSON; stub returns a body whose choices array is empty | Errors containing not a JSON tag array and missing chat content | Pass; tests/it_tagging.rs::tc209_ai_tags_rejects_non_json_and_missing_content |
| TC-210 | Endpoint http://example.invalid/v1 (remote, plain HTTP) | Error remote AI requires https, with no network attempt | Pass; tests/it_tagging.rs::tc210_ai_tags_rejects_remote_http_endpoint |
| TC-211 | Stub answers HTTP 500 with a valid chat body | Error text contains HTTP status server error, so the failure comes from the status check | Pass; tests/it_tagging.rs::tc211_ai_tags_http_error_status_fails |
| TC-212 | local_endpoint for localhost, 127.0.0.1 and an IPv6 loopback URL, a remote https URL, an ftp URL and a malformed string | True for the three loopback URLs, false for the remote URL, errors for the ftp URL and the malformed string | Pass; tests/it_tagging.rs::tc212_local_endpoint_variants |
| TC-213 | Loopback endpoint that accepts one connection and never replies; ai_tags_with_timeout with a one-second budget, inside a ten-second outer timeout | Returns within ten seconds with an error that downcasts to a reqwest timeout error | Pass; tests/it_tagging.rs::tc213_ai_tags_silent_endpoint_times_out |
| TC-214 | Stub answers HTTP 200 with a chat body of 8 MiB plus one byte | Error containing AI response exceeds | Pass; tests/it_tagging.rs::tc214_ai_tags_rejects_response_over_size_cap |
| TC-215 | Stub answers HTTP 200 with a chat body of exactly 8 MiB whose content is a one-tag array | Returns the single tag alpha | Pass; tests/it_tagging.rs::tc215_ai_tags_accepts_response_at_size_cap |
| TC-216 | Stub answers 302 with a Location header pointing at a second loopback listener; the redirect response body carries the tag from-redirect and the target would answer with the tag leaked | Error containing redirects are not followed; the redirect target receives no request within 500 ms | Pass; tests/it_tagging.rs::tc216_ai_tags_does_not_follow_redirects |
| TC-217 | Loopback stub answers 200 with a chat body whose content is ["loopback"]; every proxy variable is removed, then `HTTP_PROXY` is set to `http://127.0.0.1:9`, where nothing listens; `ai_tags_with_timeout` with a ten-second budget | Returns `loopback` and the request reaches the stub without using the proxy | Pass; tests/it_tagging.rs::tc217_loopback_endpoint_ignores_http_proxy |

## Boundary & Failure Cases

- Rows with three keywords and rows with four keywords are both reachable (TC-201).
- A tag of exactly 80 bytes is kept and 81 bytes is dropped; a tag of 82 bytes that is only 41 characters is dropped, so the limit counts bytes (TC-208).
- Whitespace-only tags are dropped (TC-208).
- At most eight tags are returned: of the ten tags that pass the filters, the first eight are kept and the last two are cut (TC-208).
- Both code-fence styles are accepted (TC-207).
- Mixed-script keywords (Japanese, Chinese, English) match after lowercasing (TC-201, TC-202, TC-205).
- Remote plain HTTP is refused before any socket is opened (TC-210).
- Server errors and malformed bodies surface as errors, never as empty success (TC-209, TC-211).
- A silent endpoint fails with a request timeout instead of blocking the caller (TC-213).
- `ai_tags` applies a 60-second request budget (`AI_REQUEST_TIMEOUT` in `src/tagging.rs`); the tests that call `ai_tags` run under it, but none lets it expire, and the one-second budget of TC-213 is a test seam of `ai_tags_with_timeout`.
- A chat body of exactly 8 MiB is accepted and one byte more is rejected (TC-214, TC-215).
- A 3xx response is an error, and the redirect target receives no request (TC-216).
- A loopback endpoint bypasses the proxy variables even when `HTTP_PROXY` points at an address with no listener (TC-217); remote endpoints use those variables by default, and no test covers that.
- The `tag-folder` command (`tag_folder` in `src/entry.rs`) is not exercised by this module's tests or by `tests/it_tagging.rs`. The unit test `tag_folder_skips_excluded_files_for_remote_endpoint` in `src/entry.rs` checks, with a fake host, that a remote endpoint receives no AI call for an excluded file, and `tag_folder_without_model_never_calls_ai` checks that no AI call is made without a model. The binary-level tests in `tests/it_entry_legacy.rs` run `tag-folder` without an endpoint. No test covers an excluded file with a loopback endpoint, the skip message (`skipped remote AI tagging` in `src/entry.rs`), a copy of excluded content (excluded by its content rather than its path), or the `ai+rules` source value (set in `src/entry.rs` when both an endpoint and a model are given).

## Regression Scope

The `tag-folder` command and `Store::set_tags` consume these outputs. `tests/it_store.rs` TC-107 (`tc107_set_tags_filters_and_sorts`) covers the storage side of tags. The inline tests `ut_tag_01_heuristic` and `ut_tag_02_localhost` in `src/tagging.rs` must stay green.
