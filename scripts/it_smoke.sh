#!/usr/bin/env bash
set -euo pipefail
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cargo run --bin ladon -- --root "$TMP" init
printf '# REQ-1\nfirst\n' > "$TMP/req.md"
cargo run --bin ladon -- --root "$TMP" scan
printf '# REQ-1\nsecond\n' > "$TMP/req.md"
cargo run --bin ladon -- --root "$TMP" capture req.md
cargo run --bin ladon -- --root "$TMP" history req.md
cargo run --bin ladon -- --root "$TMP" verify req.md
