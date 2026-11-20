# Technical Design: `src/tagging.rs`

Source: `src/tagging.rs`

Source-SHA256: `ff15537fb063c1e1e90e47c37880b21648f9e566e1b25b3308c8b9ac31f21cda`

## Responsibility

Produces classification tags for tracked documents in two ways: a deterministic keyword rule table (`heuristic_tags`) that works offline, and an optional call to an OpenAI-compatible chat endpoint (`ai_tags`) that returns a short JSON array of tags. It also decides whether an endpoint is local (`local_endpoint`). It never contacts a remote service unless the caller passes an endpoint, and it refuses remote plain-HTTP endpoints.

## Dependencies

- `anyhow` for errors, `serde_json` for request and response bodies, `url` for endpoint parsing, `reqwest` (rustls, JSON) for the chat request, `std::env` for the optional `LADON_AI_API_KEY` bearer token. The HTTP client honours the standard proxy environment variables.
- Callers: the `tag-folder` CLI command in `src/main.rs`. For each tracked file it reads at most the first 16384 bytes and calls `heuristic_tags` on that text. It calls `ai_tags` only when an endpoint and a model are given; for a file excluded by `.LADONignore` it calls `ai_tags` only when `local_endpoint` accepts the endpoint, and otherwise skips the AI call with a message. The returned tags are persisted through `store::Store::set_tags`.

## Public Contract

`heuristic_tags(path, text) -> Vec<String>`: lowercases `"{path} {text}"` and walks the rule table in order. A label is emitted once when any of its keywords is a substring of that string. The table is `[(&str, &[&str]); 8]` with these labels in this order: `requirements`, `basic-design`, `detailed-design`, `unit-test`, `integration-test`, `system-test`, `rust`, `nema`. Each row stores its keywords as a slice; the keyword counts are 4, 4, 4, 4, 3, 3, 3 and 4 in the label order above. Keywords are compared as written against the lowercased string, so every keyword in the table is lowercase, and Chinese, Japanese and English keywords are matched alike. Matching is a plain substring test, so the short keywords `ut-`, `it-` and `st-` also match inside longer words: the path `input-a.md` yields `unit-test`. The result is empty when nothing matches.

`local_endpoint(endpoint) -> Result<bool>`: parses the URL, accepts only `http` and `https` schemes (error `http(s) endpoint only`), and returns true for the hosts `localhost`, `127.0.0.1`, `::1` and `[::1]`.

`ai_tags(endpoint, model, description, filename, preview) -> Result<Vec<String>>`: first calls `local_endpoint`, so a non-http(s) scheme (`http(s) endpoint only`) or an unparsable URL fails before any request. It rejects a non-local endpoint that does not start with `https://` (error `remote AI requires https`). It posts a chat completion request (temperature 0, a system prompt requiring a JSON array of 1 to 8 short lowercase tags, the preview limited to its first 6000 characters); it adds a bearer header when `LADON_AI_API_KEY` is set and fails on a non-success HTTP status. A response body that is not JSON fails during response decoding. It reads `choices[0].message.content`, which is the error `missing chat content` when absent, strips optional Markdown code fences (with or without the `json` marker) and parses a JSON array of strings (error `AI response was not a JSON tag array`). It drops blank or whitespace-only tags and tags longer than 80 bytes (the length is counted in bytes, not characters), leaves the remaining tags unchanged, and returns at most the first 8 of them. The cap applies after filtering.

## Change Notes

The rule table is typed `[(&str, &[&str]); 8]` with slice keyword lists, so rows with three or four keywords share one table and all eight labels are reachable. Matching is a case-insensitive substring test over the path and the text, label order is the table order, endpoint validation is as specified above, and the chat protocol is the OpenAI-compatible request and response shape described under Public Contract.
