# Test Design: `src/archive.rs`

Source: `src/archive.rs`

Source-SHA256: `753686475c7df58dacbcd31e52bd64bd08860444b805b2623b3559e1fd82b460`

## Test Strategy

Inline unit tests in `src/archive.rs` use a temporary store root and temporary files. They need no network and no process-wide state. The damaged-object cases write the damage straight into the object file, the way an interrupted write would leave it.

## Test Cases

| Case ID | Input / Setup | Expected Outcome | Test Location |
| --- | --- | --- | --- |
| TC-1038 | `put_file` of a file twice, then `load_bytes` of its hash. | Both calls return the same hash and size; the bytes load back unchanged. | src/archive.rs::ut_cas_01_dedup_and_reload |
| TC-1039 | `local_object_path` for the string `../evil`. | The call fails with `invalid digest`. | src/archive.rs::ut_cas_02_traversal |
| TC-1040 | `put_bytes` of 100 zero bytes, then `load_bytes` with a limit of 99 bytes. | `load_bytes` fails with `object exceeds bounded read limit`. | src/archive.rs::ut_cas_03_limit |
| TC-1041 | `put_file` of a file; the object file is truncated to half its length; `put_file` of the same file again; `load_bytes`. | The second call succeeds and `load_bytes` returns the original bytes. | src/archive.rs::ut_cas_04_put_file_replaces_a_damaged_object |
| TC-1052 | `put_bytes` of a byte string; the object file is overwritten with garbage; `put_bytes` of the same bytes again; `load_bytes`. | The second call returns the same hash and `load_bytes` returns the original bytes. | src/archive.rs::ut_cas_05_put_bytes_replaces_a_damaged_object |

## Boundary & Failure Cases

- A hash that is not 64 hexadecimal characters never reaches the file system (TC-1039).
- A file that changes while it is read is refused by `put_file`; this case has no test, because it needs a concurrent writer.
- `assert_under_workspace` is covered through `Engine::resolve` by `tests/it_engine.rs` (`tc306_resolve_rejects_internal_and_outside_paths`).

## Regression Scope

`engine.rs` (`capture`, `load_version`, `archive_version`, `restore`) stores and reads its content through this module, and `tests/it_engine.rs`, `tests/it_engine_archive.rs` and `tests/it_history.rs` must stay green. The `ui-prototype` archive tests evict objects from the store.