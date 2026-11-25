# Technical Design: `src/doctask_outline.rs`

Source: `src/doctask_outline.rs`

Source-SHA256: `0f9a8609e7698124d33d887c97893809b7a26202302bed7057d0297bbfb62a1e`

## Responsibility

`doctask_outline` is the pure clause layer of the document-task module. From the raw bytes of a file and its extension it computes the clause outline: heading clauses whose keys equal those of `diff::section_hashes`, clause numbers, virtual `ROW` clauses from Markdown table rows and list items, the resolution of section selectors against an outline, the anchor classification of the reconciliation (design 57, section 5.4) and the byte-level splice of a section for edit plans (design 57, section 8.1). It performs no I/O and holds no database connection. It does not modify `diff.rs`, which remains the source of the section fingerprints that existing anchors use.

## Dependencies

- `crate::doctask_model` for `DocError`, `ErrorCode`, `SectionSelector`, `AnchorState`, `sha256_hex`, `check_plan_target` and `normalize_new_text`. The module does not import `crate::store`, `crate::engine`, `crate::doctask_store`, `crate::doctask_service` or any module that depends on them.
- `crate::diff` is not called at run time. The header rule is reproduced exactly from `diff::is_header`, and the fingerprints are the SHA-256 lowercase hexadecimal digests that `diff::section_hashes` produces. The equality with `diff::section_hashes` is asserted by the tests.
- The standard library only for byte and string handling.
- Callers: `doctask_service` (outline of the baseline and latest versions, selector resolution, anchor creation), `doctask_reconcile` (`classify_anchor`, `outline`), `doctask_plan` (`replace_clause`, `insert_after_clause`, `clause_bytes`) and `doctask_report` (outline entries for the audit report).

## Public Contract

### Constants

`ROOT_KEY` is `"ROOT"`. `RELOCATION_MIN_LINES` is 2 and `RELOCATION_MIN_CHARS` is 40: the least number of non-blank body lines and of non-whitespace characters that a body must have for `body_hash` relocation (section 5.4, step 6).

### Clauses and outline

`ClauseKind` has the variants `Root`, `Heading`, `Domain` and `Row`. `Root`, `Heading` and `Domain` are section clauses; `Row` is a virtual clause.

`Clause` has the public fields:

| Field | Meaning |
|---|---|
| `ordinal` | 1-based position among the section clauses in document order, `ROOT` included; `0` for a `Row` |
| `key` | `ROOT`, `H<level>:<title>#<n>`, `DOMAIN:<id>#<n>` or `ROW:<NUMBER>#<n>` |
| `kind` | the `ClauseKind` |
| `number` | the clause number in uppercase ASCII, or `None` |
| `title` | the heading text for `Heading`, `（全文）` for `Root`, the domain id for `Domain`, and the row text for `Row` |
| `level` | 1 to 6 for `Heading`, 0 otherwise |
| `first_line`, `last_line` | 1-based inclusive line numbers; an empty range has `last_line = first_line - 1` |
| `byte_start`, `byte_end` | the byte range in the raw buffer, `byte_end` exclusive |
| `full_hash` | SHA-256 of the clause lines, each followed by `\n`, the heading line included |
| `body_hash` | SHA-256 of the lines after the heading line, each followed by `\n`; equal to `full_hash` for `Root` and `Row` |
| `body_lines` | number of body lines that contain a non-whitespace character |
| `body_chars` | number of non-whitespace Unicode scalar values in the body |

`Outline` has the public fields `sections: Vec<Clause>` (in document order, `ROOT` first) and `rows: Vec<Clause>` (in document order). Its methods are `find_key(&self, key: &str) -> Option<&Clause>`, which searches sections and then rows, and `clauses(&self) -> impl Iterator<Item = &Clause>`, which yields sections and then rows.

`outline(bytes: &[u8], extension: &str) -> Outline` builds the outline. It never fails: bytes that are not valid UTF-8 are read lossily, as `diff::section_hashes` reads them.

Construction rules:

1. The raw buffer is split after each `\n` byte. A line's text excludes the `\n`, and a `\r` immediately before a `\n` is excluded as well. A final piece without `\n` keeps a trailing `\r`, which matches `str::lines`. Each line is decoded with `String::from_utf8_lossy`.
2. The extension is compared after ASCII lowercasing. `md` and `markdown` use the Markdown rules; `nema` and `nemat` use the domain rules; any other extension yields only the `ROOT` section.
3. Heading rules for `md` and `markdown`: a line whose leading `#` run has length 1 to 6 followed by a space is a heading of that level, and its title is the rest of the line trimmed. For `nema` and `nemat`: a line that starts with ` \` and, after removing the following `?` and `>` characters, splits on spaces into exactly two decimal integers is a domain header with id equal to the first integer text. Keys are `H<level>:<title>` and `DOMAIN:<id>` with a per-base-key counter `#n` starting at 1, exactly as `diff::section_hashes` assigns them.
4. The sections partition the buffer. `ROOT` covers the bytes before the first heading, and each heading section runs from its heading line up to the next heading line, or to the end of the buffer. `ROOT` is always present, so an empty buffer or a buffer that starts with a heading has the empty `ROOT` range `[0, 0)` with `first_line` 1 and `last_line` 0.
5. A byte-order mark is part of the first line and therefore never a heading, as in `diff.rs`.
6. Clause number of a heading (`md` and `markdown` only): a title that ends with `{#X}`, where `X` is a clause token, gives `X`; otherwise the first whitespace-separated word of the title that begins with a clause token gives the number. `Root` and `Domain` clauses have no number.
7. A clause token is an ASCII letter, followed by ASCII letters or digits, followed by `-`, followed by one or more ASCII digits, such that the token is not followed by an ASCII letter, an ASCII digit or `-`. The number is stored in uppercase.
8. Virtual `Row` clauses (`md` and `markdown` only) come from lines that are not headings. A table row is a line whose first non-blank character is `|` and whose first cell, the text between the first and second `|` (or the end of the line), starts with a clause token after leading blanks. A list item is a line whose first non-blank characters are `-`, `*` or `+`, or one or more digits followed by `.` or `)`, each followed by a space or tab; its first word after the marker must begin with a clause token. The row key is `ROW:<NUMBER>#<n>`, where `n` counts the rows with the same number in document order. A row covers its own line, its byte range includes the terminating `\n` when present, and `full_hash` and `body_hash` both equal the SHA-256 of the row text followed by `\n`.
9. Code fences are not tracked. Headings and rows inside a fenced block are classified as in the rules above, consistent with `diff.rs`.

Known limits, fixed by the tests: fenced `#` lines are headings; Setext headings are not recognised; `Row` clauses do not nest into sections; the outline does not validate that a document is well formed.

### Selector resolution

`resolve<'a>(outline: &'a Outline, selector: &SectionSelector) -> Result<&'a Clause, DocError>`:

| Selector | Result |
|---|---|
| `Root` | the `ROOT` section |
| `Ordinal(n)` | the section with `ordinal == n`; `E_SECTION_NOT_FOUND` when there is none; rows are never returned |
| `Clause(c)` | every clause, section or row, whose number equals `c` ignoring ASCII case; none gives `E_SECTION_NOT_FOUND`; one gives that clause; several give `E_AMBIGUOUS` |
| `Key(k)` | the clause whose key equals `k` exactly; none gives `E_SECTION_NOT_FOUND` |
| `Text(t)` | the heading sections whose title equals `t` exactly; exactly one gives it; several give `E_AMBIGUOUS`; none falls back to the heading sections whose title contains `t`: one gives it, several give `E_AMBIGUOUS`, none gives `E_SECTION_NOT_FOUND` |

`E_AMBIGUOUS` carries in its detail the candidate keys, separated by `, `, and the ordinal of each section candidate (`#2 H1:Scope#1`).

### Byte-level access and splice

`clause_bytes(bytes: &[u8], clause: &Clause) -> Option<&[u8]>` returns the raw bytes of the clause range, or `None` when the range is outside `bytes`.

`replace_clause(bytes: &[u8], extension: &str, clause: &Clause, new_text: &str) -> Result<Vec<u8>, DocError>` and `insert_after_clause(bytes: &[u8], extension: &str, clause: &Clause, new_text: &str) -> Result<Vec<u8>, DocError>` return the new buffer. The checks run in this order and the first failure is returned:

1. `check_plan_target(extension, bytes)`: `E_UNSUPPORTED_FILE` for an extension other than `md`, `markdown`, `nema` or `nemat`, for a NUL byte, for a UTF-16 byte-order mark at the start, and for bytes that are not valid UTF-8; `E_PLAN_TOO_LARGE` above 1 MiB. A UTF-8 byte-order mark is accepted.
2. `clause.kind == Row` gives `E_UNSUPPORTED_FILE`.
3. `clause.byte_end > bytes.len()` or `clause.byte_start > clause.byte_end` gives `E_INVALID_ARG`, detail `clause range is outside the target bytes`.
4. `normalize_new_text(new_text)`: `\r\n` becomes `\n`, a remaining `\r` or other control character is `E_INVALID_ARG`, more than 262144 bytes is `E_PLAN_TOO_LARGE`. An empty result gives `E_INVALID_ARG`.

Byte rules, applied to the raw buffer and never to a `String` rebuilt from it:

- The dominant newline is `\r\n` when the buffer contains more `\r\n` pairs than bare `\n` bytes, and `\n` otherwise. Each `\n` of the normalised text becomes the dominant newline.
- Bytes outside the replaced or inserted range are copied unchanged: each line's own terminator, mixed newlines, the byte-order mark and a missing final newline.
- A byte-order mark at offset 0 is kept. When the range starts at 0 and the buffer begins with `EF BB BF`, the splice starts at 3.
- `replace_clause`: the range `[start, end)` is replaced. When the range is not empty and its last byte is `\n`, or when `end` is less than the buffer length, the converted text is given a final dominant newline if it lacks one. A range that ends at the end of the buffer without a newline receives text as given.
- `insert_after_clause`: the insertion point is `clause.byte_end`. When the insertion point is greater than 0 and the byte before it is not `\n`, a dominant newline is inserted first. The converted text receives a final dominant newline when the insertion point is at the start of the buffer, when the byte before it is `\n`, or when more bytes follow. An insertion at the end of a buffer that lacks a final newline keeps that absence.

`new_text` replaces or follows the whole clause range. For a `Heading` clause the caller supplies the heading line; the splice does not check that the result keeps the key. The caller recomputes the outline of the result before it accepts a plan.

### Anchor classification

```
pub enum Method { SameHash, FullHash, BodyHash, None }   // as_str: same_hash, full_hash, body_hash, none
pub struct AnchorRef<'a> { pub key: &'a str, pub full_hash: &'a str, pub body_hash: &'a str }
pub enum LatestVersion<'a> { Deleted, Cold, Present(&'a Outline) }
pub struct Verdict { pub state: AnchorState, pub key: String, pub method: Method }
pub fn classify_anchor(anchor: &AnchorRef<'_>, latest: LatestVersion<'_>) -> Verdict
```

`classify_anchor` applies the order of design section 5.4:

1. `Deleted`: `FileMissing`, key unchanged, method `None`. The caller returns this result only when no lineage is confirmed for the file; a confirmed lineage is resolved before classification.
2. `Cold`: `Unknown`, key unchanged, method `None`.
3. `Present(outline)`, and the anchor key exists among all clauses of the outline:
   - the clause with the anchor key has `full_hash` equal to the anchor's: `Valid`, same key, method `SameHash`;
   - otherwise the clauses with a different key and an equal `full_hash` are counted: one gives `Relocated` to its key with method `FullHash`; several give `Ambiguous`; none gives `NeedsReview`, same key, method `None`.
4. `Present(outline)`, and the anchor key does not exist: the section clauses of kind `Heading` or `Domain` whose `body_hash` equals the anchor's are counted. Rows and `Root` are not candidates.
   - none gives `Orphaned`, key unchanged, method `None`;
   - exactly one with `body_lines >= 2` and `body_chars >= 40` gives `Relocated` to its key with method `BodyHash`;
   - exactly one below those bounds, or several, give `Ambiguous`, key unchanged, method `None`.

Step 3 of section 5.4 (filling an unchecked fingerprint from the baseline version) belongs to the caller: it reads the clause of the baseline outline with the anchor key and passes its `full_hash` and `body_hash` in `AnchorRef`. `AnchorState::Unchecked` is never returned by this function.

## Change Notes

- Clause numbers are stored in uppercase ASCII; matching of numbers and of selectors ignores ASCII case.
- Ordinals number the section clauses from 1 with `ROOT` as 1, because the selector grammar rejects `#0` and the keys of `diff::section_hashes` include `ROOT`. Rows have no ordinal and are reachable by number or by key only.
- Only `md` and `markdown` files produce `Row` clauses and clause numbers for headings. `nema` and `nemat` files produce domain sections only.
- The per-clause annotation and task counts of section 5.1 are not computed here. The service adds them to the outline entries it displays.
- The extension is compared after ASCII lowercasing. The equality with `diff::section_hashes` holds for the lowercase extension names that `diff.rs` matches.
- A file that is not valid UTF-8 is outlined with lossy decoding, as `diff.rs` does, but it can never be the target of a splice.
- The module adds no dependency. `Cargo.toml`, `Cargo.lock` and `ui-prototype/Cargo.lock` are unchanged.
