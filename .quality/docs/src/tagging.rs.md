# Technical Design: `src/tagging.rs`

Source: `src/tagging.rs`

Source-SHA256: `a2ecffcde1f39870957499e594297273fc6129c71bd3e03630b74801b82ae786`

## Responsibility

Produces classification tags for tracked documents in two ways: a deterministic keyword rule table (`heuristic_tags`) that works offline, and an optional call to an OpenAI-compatible chat endpoint (`ai_tags`) that returns a short JSON array of tags. It also decides whether an endpoint is local (`local_endpoint`). It never contacts a remote service unless the caller passes an endpoint, and it refuses remote plain-HTTP endpoints.

## Dependencies

- `anyhow` for errors, `serde_json` for request and response bodies, `url` for endpoint parsing, `reqwest` (rustls, JSON) for the chat request, `std::env` for the optional `LADON_AI_API_KEY` bearer token.
- Callers: the `tag-folder` CLI command and `store::Store::set_tags`.

## Public Contract

`heuristic_tags(path, text) -> Vec<String>`: lowercases `"{path} {text}"` and walks the rule table in order. A label is emitted once when any of its keywords is a substring of that string. The table is `[(&str, &[&str]); 8]` with these labels in this order: `requirements`, `basic-design`, `detailed-design`, `unit-test`, `integration-test`, `system-test`, `rust`, `nema`. Each row stores its keywords as a slice, so rows may have different keyword counts (Chinese, Japanese and English keywords are all compared in lowercase). The result is empty when nothing matches.

`local_endpoint(endpoint) -> Result<bool>`: parses the URL, accepts only `http` and `https` schemes (error `http(s) endpoint only`), and returns true for the hosts `localhost`, `127.0.0.1`, `::1` and `[::1]`.

`ai_tags(endpoint, model, description, filename, preview) -> Result<Vec<String>>`: rejects a non-local endpoint that does not start with `https://` (error `remote AI requires https`); posts a chat completion request (temperature 0, a system prompt requiring a JSON array of 1 to 8 short lowercase tags, preview limited to 6000 characters); adds a bearer header when `LADON_AI_API_KEY` is set; fails on a non-success HTTP status; reads `choices[0].message.content`, strips optional Markdown code fences, parses a JSON array of strings (error `AI response was not a JSON tag array`), drops blank tags and tags longer than 80 bytes, and returns at most 8 tags. A missing content field is the error `missing chat content`.

## Change Notes

The rule table is typed as `[(&str, &[&str]); 8]` with slice keyword lists, which lets rows with three or four keywords share one table and makes all eight labels reachable. Matching semantics, label order, endpoint validation and the chat protocol are unchanged.
