# Technical Design: `src/archive.rs`

Source: `src/archive.rs`

Source-SHA256: `753686475c7df58dacbcd31e52bd64bd08860444b805b2623b3559e1fd82b460`

## Responsibility

`archive.rs` holds the local content-addressed store (`CasStore`) and the workspace path assertion `assert_under_workspace`. The store keeps each distinct file content once, compressed with zstd, under the name of its SHA-256 digest. `assert_under_workspace` accepts only a regular file that lies inside the workspace and is not a symbolic link.

## Dependencies

- `crypto::{digest, is_digest}` (digest of bytes and the digest format check), `sha2` (streaming digest), `zstd` (compression), `uuid` (temporary file names), `anyhow` and `std::fs`.
- Callers: `engine::Engine` (the `cas` field: `put_file` in `capture`, `has`, `load_bytes`, `evict`; `assert_under_workspace` in `resolve`), and the integration tests of `ui-prototype` that evict an object.

## Public Contract

- `CasStore::new(base)`: the store root. Objects live at `<base>/<first two digest characters>/<digest>.zst`; `local_object_path` fails with `invalid digest` for a string that is not 64 hexadecimal characters.
- `has(hash) -> bool`: true when the object file exists.
- `put_file(path) -> Result<(hash, bytes)>`: reads a regular file in 64 KiB chunks, hashes and compresses it into a temporary file `<base>/.tmp-<uuid>`, syncs it, and fails with `file changed during capture: retry after stable save` when the length or modification time differs afterwards. When an object for the digest already exists and its decompressed bytes hash to the digest, the temporary file is removed. When the existing object does not verify, it is replaced by the temporary file. Otherwise the temporary file is renamed into place. The temporary file is removed when the call fails.
- `put_bytes(bytes) -> Result<hash>`: the same rules for a byte slice: an existing object that verifies is kept, one that does not verify is replaced, and the temporary file is synced before it is renamed.
- `load_bytes(hash, max_bytes) -> Result<Vec<u8>>`: decompresses at most `max_bytes` plus one byte and fails with `object exceeds bounded read limit` beyond the limit, and with `local CAS hash validation failed` when the bytes do not hash to the digest.
- `evict(hash)`: removes the object file when it exists.
- `assert_under_workspace(root, path) -> Result<PathBuf>`: fails with `symlink capture forbidden` when `path` itself is a symbolic link, with `path escapes the workspace` when its canonical form is outside the canonical root, and with `symlink/nonfile capture forbidden` when the canonical path is a link or not a regular file. It returns the canonical path.

## Change Notes

An existing object is trusted only after its bytes hash to its digest, so a truncated or damaged object left by an interrupted write is replaced by the next capture of the same bytes instead of failing every later load. The verification reads the object in chunks and holds no more than one chunk in memory.