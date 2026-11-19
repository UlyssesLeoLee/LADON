# Test Design: `src/tagging.rs`

Source: `src/tagging.rs`

Source-SHA256: `a2ecffcde1f39870957499e594297273fc6129c71bd3e03630b74801b82ae786`

## Test Strategy

Integration tests call the public functions directly. `heuristic_tags` and `local_endpoint` need no fixtures. `ai_tags` is tested against a real HTTP listener built with `std::net::TcpListener` on `127.0.0.1:0` in a helper thread that serves one canned HTTP/1.1 response per test, so the production request code runs end to end without mocks and without extra dependencies. No test sets process-wide environment variables, so the tests can run in parallel. The inline unit tests in `src/tagging.rs` stay in place.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-201 | Eight inputs, one per label, such as path requirements.md, text 基本設計, text unit test, path lib.rs, path x.nema | Each input yields exactly its own label | tests/it_tagging.rs::tc201_every_rule_label_reachable |
| TC-202 | Path README.MD with text Requirement list | Result contains requirements | tests/it_tagging.rs::tc202_matching_is_case_insensitive |
| TC-203 | Path design.rs with text architecture requirement | Exactly requirements, basic-design, rust in that order | tests/it_tagging.rs::tc203_labels_in_rule_table_order |
| TC-204 | Path notes.txt with text hello world; and two empty strings | Empty vectors | tests/it_tagging.rs::tc204_no_match_returns_empty |
| TC-205 | Text containing requirement, requirements and the Chinese keyword together | The requirements label appears once | tests/it_tagging.rs::tc205_label_emitted_once |
| TC-206 | Local stub returns a chat body whose content is a JSON array of two tags | ai_tags returns those two tags | tests/it_tagging.rs::tc206_ai_tags_parses_plain_json_array |
| TC-207 | Stub content wrapped in a json code fence | Fences stripped and tags parsed | tests/it_tagging.rs::tc207_ai_tags_accepts_fenced_json |
| TC-208 | Stub returns ten tags including a blank one and one of 81 characters | Blank and oversize tags dropped and at most eight returned | tests/it_tagging.rs::tc208_ai_tags_filters_and_caps_results |
| TC-209 | Stub returns content that is not JSON; stub returns a body without choices | Errors AI response was not a JSON tag array and missing chat content | tests/it_tagging.rs::tc209_ai_tags_rejects_non_json_and_missing_content |
| TC-210 | Endpoint http://example.invalid/v1 (remote, plain HTTP) | Error remote AI requires https, with no network attempt | tests/it_tagging.rs::tc210_ai_tags_rejects_remote_http_endpoint |
| TC-211 | Stub answers HTTP 500 | ai_tags returns an error | tests/it_tagging.rs::tc211_ai_tags_http_error_status_fails |
| TC-212 | local_endpoint for localhost, 127.0.0.1 and an IPv6 loopback URL, a remote https URL, an ftp URL and a malformed string | True for the three loopback URLs, false for the remote URL, errors for the ftp URL and the malformed string | tests/it_tagging.rs::tc212_local_endpoint_variants |

## Boundary & Failure Cases

- Rows with three keywords and rows with four keywords are both reachable (TC-201).
- A tag of exactly 80 bytes is kept and 81 bytes is dropped (TC-208).
- Exactly eight tags pass the cap and the ninth is cut (TC-208).
- Mixed-script keywords (Japanese, Chinese, English) match after lowercasing (TC-201, TC-205).
- Remote plain HTTP is refused before any socket is opened (TC-210).
- Server errors and malformed bodies surface as errors, never as empty success (TC-209, TC-211).

## Regression Scope

The `tag-folder` command and `Store::set_tags` consume these outputs. `tests/it_store.rs` TC-107 covers the storage side of tags. The inline tests `ut_tag_01_heuristic` and `ut_tag_02_localhost` in `src/tagging.rs` must stay green.
