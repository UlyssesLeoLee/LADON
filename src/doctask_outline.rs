//! Pure clause layer of the document-task module: the clause outline of a file
//! (keys equal to `diff::section_hashes`), clause numbers, virtual `ROW`
//! clauses, selector resolution, anchor classification and the byte-level
//! section splice. The module performs no I/O and does not depend on the
//! store or the engine.

use crate::doctask_model::{
    check_plan_target, normalize_new_text, sha256_hex, AnchorState, DocError, ErrorCode,
    SectionSelector,
};
use std::collections::BTreeMap;

/// Key of the section that holds the bytes before the first heading.
pub const ROOT_KEY: &str = "ROOT";
/// Least number of non-blank body lines for `body_hash` relocation.
pub const RELOCATION_MIN_LINES: usize = 2;
/// Least number of non-whitespace characters for `body_hash` relocation.
pub const RELOCATION_MIN_CHARS: usize = 40;

const ROOT_TITLE: &str = "（全文）";
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Kind of a clause. `Root`, `Heading` and `Domain` are sections; `Row` is virtual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseKind {
    Root,
    Heading,
    Domain,
    Row,
}

/// One clause of an outline with its keys, fingerprints and byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub ordinal: usize,
    pub key: String,
    pub kind: ClauseKind,
    pub number: Option<String>,
    pub title: String,
    pub level: u8,
    pub first_line: usize,
    pub last_line: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub full_hash: String,
    pub body_hash: String,
    pub body_lines: usize,
    pub body_chars: usize,
}

/// The clause outline of one file version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    pub sections: Vec<Clause>,
    pub rows: Vec<Clause>,
}

impl Outline {
    /// The clause with the given key, searching sections and then rows.
    pub fn find_key(&self, key: &str) -> Option<&Clause> {
        self.clauses().find(|c| c.key == key)
    }

    /// Every clause: sections in document order, then rows in document order.
    pub fn clauses(&self) -> impl Iterator<Item = &Clause> {
        self.sections.iter().chain(self.rows.iter())
    }
}

struct Line {
    start: usize,
    end: usize,
    text: String,
}

enum Header {
    Heading { level: u8, title: String },
    Domain { id: String },
}

/// Splits the raw buffer into lines the way `str::lines` does, keeping offsets.
fn split_lines(bytes: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let (content_end, end) = match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(offset) => {
                let newline = start + offset;
                let content_end = if newline > start && bytes[newline - 1] == b'\r' {
                    newline - 1
                } else {
                    newline
                };
                (content_end, newline + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        lines.push(Line {
            start,
            end,
            text: String::from_utf8_lossy(&bytes[start..content_end]).into_owned(),
        });
        start = end;
    }
    lines
}

/// Header rule of `diff::is_header`, reproduced exactly.
fn header_of(text: &str, markdown: bool, nema: bool) -> Option<Header> {
    if nema {
        let tail = text.strip_prefix(" \\")?;
        let tail = tail.trim_start_matches(|c| c == '?' || c == '>');
        let parts: Vec<&str> = tail.split(' ').collect();
        if parts.len() == 2 && parts[0].parse::<u64>().is_ok() && parts[1].parse::<u64>().is_ok() {
            return Some(Header::Domain {
                id: parts[0].to_string(),
            });
        }
        return None;
    }
    if markdown {
        let hashes = text.bytes().take_while(|v| *v == b'#').count();
        if (1..=6).contains(&hashes) && text.as_bytes().get(hashes) == Some(&b' ') {
            return Some(Header::Heading {
                level: hashes as u8,
                title: text[hashes + 1..].trim().to_string(),
            });
        }
    }
    None
}

/// Length of the clause token at the start of `word`, if there is one.
fn clause_token_len(word: &str) -> Option<usize> {
    let bytes = word.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'-' {
        return None;
    }
    i += 1;
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        return None;
    }
    Some(i)
}

fn leading_clause(text: &str) -> Option<String> {
    let len = clause_token_len(text)?;
    Some(text[..len].to_ascii_uppercase())
}

fn braced_clause(title: &str) -> Option<String> {
    let rest = title.strip_suffix('}')?;
    let open = rest.rfind("{#")?;
    let inner = &rest[open + 2..];
    if clause_token_len(inner)? == inner.len() {
        Some(inner.to_ascii_uppercase())
    } else {
        None
    }
}

fn heading_number(title: &str) -> Option<String> {
    braced_clause(title).or_else(|| title.split_whitespace().find_map(leading_clause))
}

/// The text after a list marker, when the line is a list item.
fn list_item_body(trimmed: &str) -> Option<&str> {
    let bytes = trimmed.as_bytes();
    let rest = match bytes.first().copied() {
        Some(b'-') | Some(b'*') | Some(b'+') => &trimmed[1..],
        _ => {
            let digits = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
            if digits == 0 {
                return None;
            }
            match bytes.get(digits).copied() {
                Some(b'.') | Some(b')') => {}
                _ => return None,
            }
            &trimmed[digits + 1..]
        }
    };
    if !rest.starts_with(|c: char| c == ' ' || c == '\t') {
        return None;
    }
    Some(rest.trim_start())
}

/// The clause number of a Markdown table row or list item, if it has one.
fn row_number(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('|') {
        let first_cell = rest.split('|').next().unwrap_or("");
        return leading_clause(first_cell.trim_start());
    }
    let item = list_item_body(trimmed)?;
    leading_clause(item.split_whitespace().next()?)
}

fn digest_lines(lines: &[Line]) -> String {
    let mut buffer = Vec::new();
    for line in lines {
        buffer.extend_from_slice(line.text.as_bytes());
        buffer.push(b'\n');
    }
    sha256_hex(&buffer)
}

fn body_metrics(lines: &[Line]) -> (usize, usize) {
    let body_lines = lines.iter().filter(|l| !l.text.trim().is_empty()).count();
    let body_chars = lines
        .iter()
        .map(|l| l.text.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    (body_lines, body_chars)
}

/// Builds a section clause over the line indices `first..end`.
#[allow(clippy::too_many_arguments)]
fn section_clause(
    lines: &[Line],
    total: usize,
    range: (usize, usize),
    has_header: bool,
    ordinal: usize,
    key: String,
    kind: ClauseKind,
    number: Option<String>,
    title: String,
    level: u8,
) -> Clause {
    let (first, end) = range;
    let byte_start = if first < lines.len() {
        lines[first].start
    } else {
        total
    };
    let byte_end = if end > first {
        lines[end - 1].end
    } else {
        byte_start
    };
    let body_start = if has_header { first + 1 } else { first };
    let body = &lines[body_start..end];
    let (body_lines, body_chars) = body_metrics(body);
    Clause {
        ordinal,
        key,
        kind,
        number,
        title,
        level,
        first_line: first + 1,
        last_line: end,
        byte_start,
        byte_end,
        full_hash: digest_lines(&lines[first..end]),
        body_hash: digest_lines(body),
        body_lines,
        body_chars,
    }
}

/// Builds the clause outline of `bytes` for a file with the given extension.
///
/// The extension is compared after ASCII lowercasing. The section keys and
/// full hashes equal those of `diff::section_hashes` for the same bytes.
/// Bytes that are not valid UTF-8 are read lossily, as `diff.rs` reads them.
pub fn outline(bytes: &[u8], extension: &str) -> Outline {
    let extension = extension.to_ascii_lowercase();
    let markdown = matches!(extension.as_str(), "md" | "markdown");
    let nema = matches!(extension.as_str(), "nema" | "nemat");
    let lines = split_lines(bytes);
    let headers: Vec<Option<Header>> = lines
        .iter()
        .map(|l| header_of(&l.text, markdown, nema))
        .collect();

    let mut sections = Vec::new();
    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    let header_lines: Vec<usize> = (0..lines.len()).filter(|i| headers[*i].is_some()).collect();
    let first_header = header_lines.first().copied().unwrap_or(lines.len());
    sections.push(section_clause(
        &lines,
        bytes.len(),
        (0, first_header),
        false,
        1,
        ROOT_KEY.to_string(),
        ClauseKind::Root,
        None,
        ROOT_TITLE.to_string(),
        0,
    ));
    for (position, &index) in header_lines.iter().enumerate() {
        let end = header_lines
            .get(position + 1)
            .copied()
            .unwrap_or(lines.len());
        let (base, kind, level, title, number) = match headers[index].as_ref() {
            Some(Header::Heading { level, title }) => (
                format!("H{}:{}", level, title),
                ClauseKind::Heading,
                *level,
                title.clone(),
                if markdown {
                    heading_number(title)
                } else {
                    None
                },
            ),
            Some(Header::Domain { id }) => (
                format!("DOMAIN:{}", id),
                ClauseKind::Domain,
                0,
                id.clone(),
                None,
            ),
            None => continue,
        };
        let counter = counters.entry(base.clone()).or_insert(0);
        *counter += 1;
        let key = format!("{}#{}", base, counter);
        sections.push(section_clause(
            &lines,
            bytes.len(),
            (index, end),
            true,
            sections.len() + 1,
            key,
            kind,
            number,
            title,
            level,
        ));
    }

    let mut rows = Vec::new();
    if markdown {
        let mut row_counts: BTreeMap<String, usize> = BTreeMap::new();
        for (index, line) in lines.iter().enumerate() {
            if headers[index].is_some() {
                continue;
            }
            let Some(number) = row_number(&line.text) else {
                continue;
            };
            let counter = row_counts.entry(number.clone()).or_insert(0);
            *counter += 1;
            let row_lines = std::slice::from_ref(line);
            let hash = digest_lines(row_lines);
            let (body_lines, body_chars) = body_metrics(row_lines);
            rows.push(Clause {
                ordinal: 0,
                key: format!("ROW:{}#{}", number, counter),
                kind: ClauseKind::Row,
                number: Some(number),
                title: line.text.clone(),
                level: 0,
                first_line: index + 1,
                last_line: index + 1,
                byte_start: line.start,
                byte_end: line.end,
                full_hash: hash.clone(),
                body_hash: hash,
                body_lines,
                body_chars,
            });
        }
    }
    Outline { sections, rows }
}

/// Resolves a section selector against an outline.
///
/// `Ordinal` selects sections only. `Clause` matches the number of any clause
/// ignoring ASCII case. `Text` matches heading titles exactly, and otherwise
/// by a substring of a heading title. Several matches give `E_AMBIGUOUS`.
pub fn resolve<'a>(
    outline: &'a Outline,
    selector: &SectionSelector,
) -> Result<&'a Clause, DocError> {
    match selector {
        SectionSelector::Root => outline
            .sections
            .iter()
            .find(|c| c.key == ROOT_KEY)
            .ok_or_else(not_found),
        SectionSelector::Ordinal(n) => n
            .checked_sub(1)
            .and_then(|i| outline.sections.get(i))
            .ok_or_else(not_found),
        SectionSelector::Key(key) => outline.find_key(key).ok_or_else(not_found),
        SectionSelector::Clause(number) => {
            let wanted = number.to_ascii_uppercase();
            pick(
                outline
                    .clauses()
                    .filter(|c| c.number.as_deref() == Some(wanted.as_str()))
                    .collect(),
            )
        }
        SectionSelector::Text(text) => {
            let headings: Vec<&Clause> = outline
                .sections
                .iter()
                .filter(|c| c.kind == ClauseKind::Heading)
                .collect();
            let exact: Vec<&Clause> = headings
                .iter()
                .copied()
                .filter(|c| c.title == *text)
                .collect();
            if !exact.is_empty() {
                return pick(exact);
            }
            pick(
                headings
                    .into_iter()
                    .filter(|c| c.title.contains(text.as_str()))
                    .collect(),
            )
        }
    }
}

fn not_found() -> DocError {
    DocError::new(
        ErrorCode::SectionNotFound,
        "未找到匹配的条款",
        "使用大纲列出的序号、条款编号或 key 指定条款",
    )
}

fn pick(candidates: Vec<&Clause>) -> Result<&Clause, DocError> {
    match candidates.len() {
        0 => Err(not_found()),
        1 => Ok(candidates[0]),
        _ => Err(DocError::new(
            ErrorCode::Ambiguous,
            "条款选择器匹配多个条款",
            "使用 #序号 或 key 指定条款",
        )
        .with_detail(candidate_list(&candidates))),
    }
}

fn candidate_list(candidates: &[&Clause]) -> String {
    candidates
        .iter()
        .map(|c| {
            if c.ordinal > 0 {
                format!("#{} {}", c.ordinal, c.key)
            } else {
                c.key.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The raw bytes of a clause, or `None` when its range is outside `bytes`.
pub fn clause_bytes<'a>(bytes: &'a [u8], clause: &Clause) -> Option<&'a [u8]> {
    bytes.get(clause.byte_start..clause.byte_end)
}

fn dominant_newline(bytes: &[u8]) -> &'static str {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let crlf = bytes
        .windows(2)
        .filter(|w| w[0] == b'\r' && w[1] == b'\n')
        .count();
    if crlf > newlines - crlf {
        "\r\n"
    } else {
        "\n"
    }
}

fn converted_text(new_text: &str) -> Result<String, DocError> {
    let text = normalize_new_text(new_text)?;
    if text.is_empty() {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            "新文本不能为空",
            "提供至少一个字符的章节内容",
        ));
    }
    Ok(text)
}

fn checked_target(bytes: &[u8], extension: &str, clause: &Clause) -> Result<(), DocError> {
    check_plan_target(extension, bytes)?;
    if clause.kind == ClauseKind::Row {
        return Err(DocError::new(
            ErrorCode::UnsupportedFile,
            "行条款不能作为编辑目标",
            "选择标题条款作为目标",
        ));
    }
    if clause.byte_start > clause.byte_end || clause.byte_end > bytes.len() {
        return Err(DocError::new(
            ErrorCode::InvalidArg,
            "条款范围不属于目标文件",
            "在目标文件的当前版本上重新解析大纲",
        )
        .with_detail("clause range is outside the target bytes"));
    }
    Ok(())
}

/// Replaces the byte range of a section clause with `new_text`.
///
/// `new_text` replaces the whole clause range, heading line included. The
/// splice starts after a byte-order mark at offset 0, keeps every byte outside
/// the range, and writes the new text with the dominant newline of the target.
pub fn replace_clause(
    bytes: &[u8],
    extension: &str,
    clause: &Clause,
    new_text: &str,
) -> Result<Vec<u8>, DocError> {
    checked_target(bytes, extension, clause)?;
    let text = converted_text(new_text)?;
    let newline = dominant_newline(bytes);
    let mut start = clause.byte_start;
    if start == 0 && bytes.starts_with(UTF8_BOM) {
        start = UTF8_BOM.len().min(clause.byte_end);
    }
    let end = clause.byte_end;
    let mut body = text.replace('\n', newline);
    let terminated = end > start && bytes[end - 1] == b'\n';
    if (terminated || end < bytes.len()) && !body.ends_with('\n') {
        body.push_str(newline);
    }
    let mut out = Vec::with_capacity(bytes.len() + body.len());
    out.extend_from_slice(&bytes[..start]);
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(&bytes[end..]);
    Ok(out)
}

/// Inserts `new_text` after the last line of a section clause.
///
/// A last line without a newline receives one first, and the insertion at the
/// end of a buffer that lacks a final newline keeps that absence.
pub fn insert_after_clause(
    bytes: &[u8],
    extension: &str,
    clause: &Clause,
    new_text: &str,
) -> Result<Vec<u8>, DocError> {
    checked_target(bytes, extension, clause)?;
    let text = converted_text(new_text)?;
    let newline = dominant_newline(bytes);
    let at = clause.byte_end;
    let line_ended = at == 0 || bytes[at - 1] == b'\n';
    let mut body = String::new();
    if !line_ended {
        body.push_str(newline);
    }
    body.push_str(&text.replace('\n', newline));
    if (line_ended || at < bytes.len()) && !body.ends_with('\n') {
        body.push_str(newline);
    }
    let mut out = Vec::with_capacity(bytes.len() + body.len());
    out.extend_from_slice(&bytes[..at]);
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(&bytes[at..]);
    Ok(out)
}

/// How an anchor was matched in the latest version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    SameHash,
    FullHash,
    BodyHash,
    None,
}

impl Method {
    /// The stored method name.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::SameHash => "same_hash",
            Method::FullHash => "full_hash",
            Method::BodyHash => "body_hash",
            Method::None => "none",
        }
    }
}

/// The stored fingerprint of an anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorRef<'a> {
    pub key: &'a str,
    pub full_hash: &'a str,
    pub body_hash: &'a str,
}

/// The state of the file in its latest version.
#[derive(Debug, Clone, Copy)]
pub enum LatestVersion<'a> {
    /// The file is deleted and no lineage is confirmed.
    Deleted,
    /// The bytes of the latest version are not held locally.
    Cold,
    /// The outline of the latest version.
    Present(&'a Outline),
}

/// The classification of one anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub state: AnchorState,
    pub key: String,
    pub method: Method,
}

/// Classifies an anchor against the latest version (design 57, section 5.4).
pub fn classify_anchor(anchor: &AnchorRef<'_>, latest: LatestVersion<'_>) -> Verdict {
    let unchanged = |state: AnchorState, method: Method| Verdict {
        state,
        key: anchor.key.to_string(),
        method,
    };
    match latest {
        LatestVersion::Deleted => unchanged(AnchorState::FileMissing, Method::None),
        LatestVersion::Cold => unchanged(AnchorState::Unknown, Method::None),
        LatestVersion::Present(outline) => classify_present(anchor, outline),
    }
}

fn classify_present(anchor: &AnchorRef<'_>, outline: &Outline) -> Verdict {
    let unchanged = |state: AnchorState, method: Method| Verdict {
        state,
        key: anchor.key.to_string(),
        method,
    };
    if let Some(current) = outline.find_key(anchor.key) {
        if current.full_hash == anchor.full_hash {
            return unchanged(AnchorState::Valid, Method::SameHash);
        }
        let others: Vec<&Clause> = outline
            .clauses()
            .filter(|c| c.key != anchor.key && c.full_hash == anchor.full_hash)
            .collect();
        return match others.as_slice() {
            [only] => Verdict {
                state: AnchorState::Relocated,
                key: only.key.clone(),
                method: Method::FullHash,
            },
            [] => unchanged(AnchorState::NeedsReview, Method::None),
            _ => unchanged(AnchorState::Ambiguous, Method::None),
        };
    }
    let candidates: Vec<&Clause> = outline
        .sections
        .iter()
        .filter(|c| matches!(c.kind, ClauseKind::Heading | ClauseKind::Domain))
        .filter(|c| c.body_hash == anchor.body_hash)
        .collect();
    match candidates.as_slice() {
        [] => unchanged(AnchorState::Orphaned, Method::None),
        [only]
            if only.body_lines >= RELOCATION_MIN_LINES
                && only.body_chars >= RELOCATION_MIN_CHARS =>
        {
            Verdict {
                state: AnchorState::Relocated,
                key: only.key.clone(),
                method: Method::BodyHash,
            }
        }
        _ => unchanged(AnchorState::Ambiguous, Method::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::section_hashes;

    fn digest(text: &str) -> String {
        sha256_hex(text.as_bytes())
    }

    fn sel_clause(number: &str) -> SectionSelector {
        SectionSelector::Clause(number.to_string())
    }

    fn outline_of(text: &str) -> Outline {
        outline(text.as_bytes(), "md")
    }

    /// Anchor fingerprint of the section with `key` in `text`.
    fn fingerprint(text: &str, key: &str) -> (String, String) {
        let o = outline_of(text);
        let c = o.find_key(key).expect("key present in fixture");
        (c.full_hash.clone(), c.body_hash.clone())
    }

    fn classify_md(
        anchor_key: &str,
        anchor_full: &str,
        anchor_body: &str,
        latest: &str,
    ) -> Verdict {
        let o = outline_of(latest);
        classify_anchor(
            &AnchorRef {
                key: anchor_key,
                full_hash: anchor_full,
                body_hash: anchor_body,
            },
            LatestVersion::Present(&o),
        )
    }

    const PARA: &str = "Alpha clause body sentence one is long enough.\nAlpha clause body sentence two follows here.\n";

    #[test]
    fn ut_147_outline_keys_match_section_hashes() {
        let a = "# Scope\nalpha\n## Scope\nbeta\n# Scope\ngamma\n```\n# not a heading\n```\n### Deep {#REQ-101}\nx\n";
        let b = " \\>1 1\nA A1\n \\>2 1\nB B1\n \\>2 2\nB B2\n";
        let c = a.replace('\n', "\r\n");
        let d = "# Scope\r\nalpha\n## Scope\r\nbeta\n# Scope\ngamma\r\n```\n# not a heading\r\n```\n### Deep {#REQ-101}\nx\r\n";
        let e = "\u{feff}# A\nbody\n";
        let corpora: [(&[u8], &str); 5] = [
            (a.as_bytes(), "md"),
            (b.as_bytes(), "nema"),
            (c.as_bytes(), "md"),
            (d.as_bytes(), "md"),
            (e.as_bytes(), "md"),
        ];
        for (bytes, ext) in corpora {
            let ours = outline(bytes, ext);
            let expected: Vec<(String, String)> = section_hashes(bytes, ext)
                .into_iter()
                .map(|s| (s.key, s.hash))
                .collect();
            let got: Vec<(String, String)> = ours
                .sections
                .iter()
                .map(|c| (c.key.clone(), c.full_hash.clone()))
                .collect();
            assert_eq!(got, expected, "keys and full hashes for extension {ext}");
            let mut previous_end = 0;
            for section in &ours.sections {
                assert_eq!(section.byte_start, previous_end);
                previous_end = section.byte_end;
                let raw = clause_bytes(bytes, section).expect("range inside corpus");
                let text = String::from_utf8_lossy(raw);
                let mut lines = text.lines();
                if section.kind != ClauseKind::Root {
                    lines.next();
                }
                let mut body = String::new();
                for line in lines {
                    body.push_str(line);
                    body.push('\n');
                }
                assert_eq!(
                    section.body_hash,
                    sha256_hex(body.as_bytes()),
                    "body of {}",
                    section.key
                );
            }
            assert_eq!(previous_end, bytes.len());
        }
        let duplicated = outline(a.as_bytes(), "md");
        let keys: Vec<&str> = duplicated.sections.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "ROOT",
                "H1:Scope#1",
                "H2:Scope#1",
                "H1:Scope#2",
                "H1:not a heading#1",
                "H3:Deep {#REQ-101}#1"
            ]
        );
        let nema = outline(b.as_bytes(), "nema");
        assert!(nema.sections.iter().any(|c| c.key == "DOMAIN:2#2"));
    }

    #[test]
    fn tc841_root_only_and_degenerate_inputs() {
        let txt = outline(b"# A\n", "txt");
        assert_eq!(txt.sections.len(), 1);
        assert_eq!(txt.sections[0].key, "ROOT");
        assert_eq!(
            (txt.sections[0].byte_start, txt.sections[0].byte_end),
            (0, 4)
        );
        assert_eq!(txt.sections[0].full_hash, digest("# A\n"));
        assert!(txt.rows.is_empty());

        let empty = outline(b"", "md");
        assert_eq!(empty.sections.len(), 1);
        assert_eq!(
            (empty.sections[0].byte_start, empty.sections[0].byte_end),
            (0, 0)
        );
        assert_eq!(
            (empty.sections[0].first_line, empty.sections[0].last_line),
            (1, 0)
        );
        assert_eq!(empty.sections[0].full_hash, digest(""));

        let no_newline = outline(b"# A\nx", "md");
        let keys: Vec<&str> = no_newline.sections.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["ROOT", "H1:A#1"]);
        assert_eq!(no_newline.sections[1].body_hash, digest("x\n"));

        let upper = outline(b"# A\n", "MD");
        assert_eq!(upper.sections[1].key, "H1:A#1");

        let bom = outline("\u{feff}plain text\n".as_bytes(), "md");
        assert_eq!(bom.sections.len(), 1);
        assert_eq!(bom.sections[0].byte_end, "\u{feff}plain text\n".len());
    }

    #[test]
    fn ut_148_clause_numbers_and_row() {
        let doc = "# Head\n## REQ-101 Scope\n### Scope {#req-102}\n# Scope REQ-103\n# REQ-104a scope\n# Notes REQ-105-x\n# REQ-106: text\n# Scope\n| REQ-201 | Shall |\n| req-201 | duplicate |\n| Title | REQ-202 |\n- REQ-203: text\n1. REQ-204 text\n- REQ-2050x\n| REQ-205x |\n# REQ-207 heading\n";
        let o = outline_of(doc);
        let numbers: Vec<Option<&str>> = o.sections.iter().map(|c| c.number.as_deref()).collect();
        assert_eq!(
            numbers,
            vec![
                None,
                None,
                Some("REQ-101"),
                Some("REQ-102"),
                Some("REQ-103"),
                None,
                None,
                Some("REQ-106"),
                None,
                Some("REQ-207"),
            ]
        );
        let ordinals: Vec<usize> = o.sections.iter().map(|c| c.ordinal).collect();
        assert_eq!(ordinals, (1..=10).collect::<Vec<usize>>());
        let row_keys: Vec<&str> = o.rows.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            row_keys,
            vec![
                "ROW:REQ-201#1",
                "ROW:REQ-201#2",
                "ROW:REQ-203#1",
                "ROW:REQ-204#1"
            ]
        );
        assert!(o.sections.iter().all(|c| c.kind != ClauseKind::Row));

        let line = "| REQ-201 | Shall |\n";
        let first = &o.rows[0];
        let start = doc.find(line).unwrap();
        assert_eq!(
            (first.byte_start, first.byte_end),
            (start, start + line.len())
        );
        assert_eq!(first.full_hash, digest(line));
        assert_eq!(first.body_hash, digest(line));
        assert_eq!(first.ordinal, 0);
        assert_eq!(o.rows[1].number.as_deref(), Some("REQ-201"));
    }

    #[test]
    fn tc843_selector_resolution() {
        let doc = "# Overview\n## REQ-301 Scope\nx\n## Overview\ny\n| REQ-301 | row |\n";
        let o = outline_of(doc);
        assert_eq!(resolve(&o, &SectionSelector::Root).unwrap().key, "ROOT");
        assert_eq!(
            resolve(&o, &SectionSelector::Ordinal(2)).unwrap().key,
            "H1:Overview#1"
        );
        assert!(matches!(
            resolve(&o, &SectionSelector::Ordinal(9)).unwrap_err().code,
            ErrorCode::SectionNotFound
        ));
        let ambiguous = resolve(&o, &sel_clause("REQ-301")).unwrap_err();
        assert!(matches!(ambiguous.code, ErrorCode::Ambiguous));
        assert!(ambiguous.detail.contains("H2:REQ-301 Scope#1"));
        assert!(ambiguous.detail.contains("ROW:REQ-301#1"));
        assert!(matches!(
            resolve(&o, &sel_clause("REQ-999")).unwrap_err().code,
            ErrorCode::SectionNotFound
        ));
        assert_eq!(
            resolve(&o, &SectionSelector::Key("H2:REQ-301 Scope#1".into()))
                .unwrap()
                .kind,
            ClauseKind::Heading
        );
        assert_eq!(
            resolve(&o, &SectionSelector::Key("ROW:REQ-301#1".into()))
                .unwrap()
                .kind,
            ClauseKind::Row
        );
        assert!(matches!(
            resolve(&o, &SectionSelector::Key("H9:x#1".into()))
                .unwrap_err()
                .code,
            ErrorCode::SectionNotFound
        ));
        assert!(matches!(
            resolve(&o, &SectionSelector::Text("Overview".into()))
                .unwrap_err()
                .code,
            ErrorCode::Ambiguous
        ));
        assert_eq!(
            resolve(&o, &SectionSelector::Text("Scope".into()))
                .unwrap()
                .key,
            "H2:REQ-301 Scope#1"
        );
        assert!(matches!(
            resolve(&o, &SectionSelector::Text("Nope".into()))
                .unwrap_err()
                .code,
            ErrorCode::SectionNotFound
        ));
        assert!(matches!(
            resolve(&o, &SectionSelector::Text("ew".into()))
                .unwrap_err()
                .code,
            ErrorCode::Ambiguous
        ));
        let only_heading = outline_of("## REQ-302 Only\nbody\n");
        assert_eq!(
            resolve(&only_heading, &sel_clause("req-302")).unwrap().key,
            "H2:REQ-302 Only#1"
        );
    }

    #[test]
    fn ut_149_anchor_classification_matrix() {
        let base = format!("# Scope\n{PARA}");
        let (full, body) = fingerprint(&base, "H1:Scope#1");
        let key = "H1:Scope#1";

        let same = classify_md(key, &full, &body, &base);
        assert_eq!(
            (same.state, same.key.as_str(), same.method),
            (AnchorState::Valid, key, Method::SameHash)
        );

        let changed = format!("# Scope\n{}", PARA.replace("sentence one", "sentence uno"));
        let v = classify_md(key, &full, &body, &changed);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::NeedsReview, key, Method::None)
        );

        let renamed = format!("# Range\n{PARA}");
        let v = classify_md(key, &full, &body, &renamed);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Relocated, "H1:Range#1", Method::BodyHash)
        );

        let level = format!("## Scope\n{PARA}");
        let v = classify_md(key, &full, &body, &level);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Relocated, "H2:Scope#1", Method::BodyHash)
        );

        let crlf = format!("# Scope\r\n{}", PARA.replace('\n', "\r\n"));
        let v = classify_md(key, &full, &body, &crlf);
        assert_eq!((v.state, v.method), (AnchorState::Valid, Method::SameHash));

        let fenced = "# Scope\n```\n# inner\n```\n";
        let fenced_outline = outline_of(fenced);
        assert!(fenced_outline.find_key("H1:inner#1").is_some());

        let front_base = format!("# A\n{PARA}");
        let (f_full, f_body) = fingerprint(&front_base, "H1:A#1");
        let moved = format!("# A\nnew\n# A\n{PARA}");
        let v = classify_md("H1:A#1", &f_full, &f_body, &moved);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Relocated, "H1:A#2", Method::FullHash)
        );

        let edited_front = format!(
            "# A\nnew\n# A\n{}",
            PARA.replace("sentence one", "sentence uno")
        );
        let v = classify_md("H1:A#1", &f_full, &f_body, &edited_front);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::NeedsReview, "H1:A#1", Method::None)
        );

        let multiple = format!("# A\nnew\n# A\n{PARA}# A\n{PARA}");
        let v = classify_md("H1:A#1", &f_full, &f_body, &multiple);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Ambiguous, "H1:A#1", Method::None)
        );

        let root_text = "alpha beta\n";
        let root_outline = outline(root_text.as_bytes(), "txt");
        let root = &root_outline.sections[0];
        let anchor = AnchorRef {
            key: ROOT_KEY,
            full_hash: &root.full_hash,
            body_hash: &root.body_hash,
        };
        let same_root = outline(root_text.as_bytes(), "txt");
        assert_eq!(
            classify_anchor(&anchor, LatestVersion::Present(&same_root)).state,
            AnchorState::Valid
        );
        let other_root = outline(b"alpha gamma\n", "txt");
        let v = classify_anchor(&anchor, LatestVersion::Present(&other_root));
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::NeedsReview, ROOT_KEY, Method::None)
        );
    }

    fn single_body_verdict(body: &str) -> (Verdict, Outline) {
        let base = format!("# Old\n{body}");
        let (full, anchor_body) = fingerprint(&base, "H1:Old#1");
        let latest = outline_of(&format!("# New\n{body}"));
        let v = classify_anchor(
            &AnchorRef {
                key: "H1:Old#1",
                full_hash: &full,
                body_hash: &anchor_body,
            },
            LatestVersion::Present(&latest),
        );
        (v, latest)
    }

    #[test]
    fn tc845_body_hash_floor_boundaries() {
        let one_line = format!("{}\n", "x".repeat(60));
        let (v, _) = single_body_verdict(&one_line);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Ambiguous, "H1:Old#1", Method::None)
        );

        let thirty_nine = format!("{}\n{}\n", "a".repeat(19), "b".repeat(20));
        let (v, _) = single_body_verdict(&thirty_nine);
        assert_eq!(v.state, AnchorState::Ambiguous);

        let forty = format!("{}\n{}\n", "a".repeat(19), "b".repeat(21));
        let (v, latest) = single_body_verdict(&forty);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Relocated, "H1:New#1", Method::BodyHash)
        );
        let candidate = latest.find_key("H1:New#1").unwrap();
        assert_eq!((candidate.body_lines, candidate.body_chars), (2, 40));

        let padded = format!("{}\n   \n", "a".repeat(40));
        let (v, _) = single_body_verdict(&padded);
        assert_eq!(v.state, AnchorState::Ambiguous);

        let table_row = format!("| {} |\n", "a".repeat(28));
        let (v, _) = single_body_verdict(&table_row);
        assert_eq!(v.state, AnchorState::Ambiguous);

        let base = format!("# Old\n{PARA}");
        let (full, anchor_body) = fingerprint(&base, "H1:Old#1");
        let twins = outline_of(&format!("# New\n{PARA}# Other\n{PARA}"));
        let v = classify_anchor(
            &AnchorRef {
                key: "H1:Old#1",
                full_hash: &full,
                body_hash: &anchor_body,
            },
            LatestVersion::Present(&twins),
        );
        assert_eq!(v.state, AnchorState::Ambiguous);

        let blanks = format!("{}\n\n\n{}\n", "a".repeat(22), "b".repeat(23));
        let (v, latest) = single_body_verdict(&blanks);
        assert_eq!(v.state, AnchorState::Relocated);
        let candidate = latest.find_key("H1:New#1").unwrap();
        assert_eq!((candidate.body_lines, candidate.body_chars), (2, 45));
    }

    #[test]
    fn tc846_row_and_cold_exclusions() {
        let row = "| REQ-401 | long text of more than forty characters in total |\n";
        let row_only = outline_of(row);
        let anchor_body = digest(row);
        let v = classify_anchor(
            &AnchorRef {
                key: "H1:Gone#1",
                full_hash: &anchor_body,
                body_hash: &anchor_body,
            },
            LatestVersion::Present(&row_only),
        );
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Orphaned, "H1:Gone#1", Method::None)
        );

        let row_anchor = AnchorRef {
            key: "ROW:REQ-401#1",
            full_hash: &anchor_body,
            body_hash: &anchor_body,
        };
        let v = classify_anchor(&row_anchor, LatestVersion::Present(&row_only));
        assert_eq!((v.state, v.method), (AnchorState::Valid, Method::SameHash));

        let changed = "| REQ-401 | changed text of more than forty characters in total |\n";
        let v = classify_anchor(&row_anchor, LatestVersion::Present(&outline_of(changed)));
        assert_eq!(
            (v.state, v.key.as_str()),
            (AnchorState::NeedsReview, "ROW:REQ-401#1")
        );

        let base = format!("# A\n{PARA}");
        let (full, body) = fingerprint(&base, "H1:A#1");
        let anchor = AnchorRef {
            key: "H1:A#1",
            full_hash: &full,
            body_hash: &body,
        };
        let v = classify_anchor(&anchor, LatestVersion::Cold);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::Unknown, "H1:A#1", Method::None)
        );
        let v = classify_anchor(&anchor, LatestVersion::Deleted);
        assert_eq!(
            (v.state, v.key.as_str(), v.method),
            (AnchorState::FileMissing, "H1:A#1", Method::None)
        );
    }

    #[test]
    fn ut_150_byte_level_splice() {
        let input: &[u8] = b"\xEF\xBB\xBF# A\r\nalpha\nbeta\r\n## B\ngamma\r\n# C\r\nlast line";
        let o = outline(input, "md");
        let b = o.find_key("H2:B#1").unwrap().clone();
        let replaced = replace_clause(input, "md", &b, "## B\nnew\n").unwrap();
        assert_eq!(
            replaced,
            b"\xEF\xBB\xBF# A\r\nalpha\nbeta\r\n## B\r\nnew\r\n# C\r\nlast line".to_vec()
        );

        let inserted = insert_after_clause(input, "md", &b, "## B2\nx\n").unwrap();
        assert_eq!(
            inserted,
            b"\xEF\xBB\xBF# A\r\nalpha\nbeta\r\n## B\ngamma\r\n## B2\r\nx\r\n# C\r\nlast line"
                .to_vec()
        );

        let c = o.find_key("H1:C#1").unwrap().clone();
        let final_replaced = replace_clause(input, "md", &c, "# C\nchanged").unwrap();
        assert_eq!(
            final_replaced,
            b"\xEF\xBB\xBF# A\r\nalpha\nbeta\r\n## B\ngamma\r\n# C\r\nchanged".to_vec()
        );

        let bom_input: &[u8] = b"\xEF\xBB\xBFpre\r\n# A\nx\n";
        let bom_outline = outline(bom_input, "md");
        let root = bom_outline.find_key("ROOT").unwrap().clone();
        let root_replaced = replace_clause(bom_input, "md", &root, "intro\n").unwrap();
        assert_eq!(root_replaced, b"\xEF\xBB\xBFintro\n# A\nx\n".to_vec());

        for (result, original_range) in [
            (&replaced, (0usize, b.byte_start)),
            (&inserted, (0usize, b.byte_end)),
        ] {
            assert_eq!(&result[..original_range.1], &input[..original_range.1]);
        }
        assert!(final_replaced.starts_with(&input[..c.byte_start]));
    }

    #[test]
    fn tc848_splice_newline_and_new_text_rules() {
        let lf_doc = "## B\nold\n# C\n";
        let o = outline_of(lf_doc);
        let b = o.find_key("H2:B#1").unwrap().clone();
        let out = replace_clause(lf_doc.as_bytes(), "md", &b, "## B\r\nnew").unwrap();
        assert_eq!(out, b"## B\nnew\n# C\n".to_vec());

        let no_final = b"# A\nx";
        let a = outline(no_final, "md").find_key("H1:A#1").unwrap().clone();
        let out = insert_after_clause(no_final, "md", &a, "# D\nd").unwrap();
        assert_eq!(out, b"# A\nx\n# D\nd".to_vec());

        let with_final = b"# A\nx\n";
        let a = outline(with_final, "md")
            .find_key("H1:A#1")
            .unwrap()
            .clone();
        let out = insert_after_clause(with_final, "md", &a, "# D\nd\n").unwrap();
        assert_eq!(out, b"# A\nx\n# D\nd\n".to_vec());

        let mixed = b"a\r\nb\n# A\n";
        let root = outline(mixed, "md").find_key("ROOT").unwrap().clone();
        let out = replace_clause(mixed, "md", &root, "intro").unwrap();
        assert_eq!(out, b"intro\n# A\n".to_vec());

        let empty_root_doc = b"# A\nx\n";
        let root = outline(empty_root_doc, "md")
            .find_key("ROOT")
            .unwrap()
            .clone();
        let out = replace_clause(empty_root_doc, "md", &root, "intro").unwrap();
        assert_eq!(out, b"intro\n# A\nx\n".to_vec());

        let a = outline(with_final, "md")
            .find_key("H1:A#1")
            .unwrap()
            .clone();
        assert!(matches!(
            replace_clause(with_final, "md", &a, "a\rb")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArg
        ));
        assert!(matches!(
            replace_clause(with_final, "md", &a, "a\u{1}b")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArg
        ));
        assert!(matches!(
            replace_clause(with_final, "md", &a, "").unwrap_err().code,
            ErrorCode::InvalidArg
        ));
        let huge = "x".repeat(262_145);
        assert!(matches!(
            replace_clause(with_final, "md", &a, &huge)
                .unwrap_err()
                .code,
            ErrorCode::PlanTooLarge
        ));
    }

    #[test]
    fn tc849_splice_rejects_unsupported_targets() {
        let valid = outline(b"# A\nx\n", "md");
        let section = valid.find_key("H1:A#1").unwrap().clone();

        let invalid_utf8: &[u8] = b"# A\n\xFF\n";
        assert!(matches!(
            replace_clause(invalid_utf8, "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));
        assert!(matches!(
            insert_after_clause(invalid_utf8, "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));
        assert!(matches!(
            replace_clause(b"# A\n\0\n", "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));
        assert!(matches!(
            replace_clause(b"\xFF\xFE# A\n", "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));
        assert!(matches!(
            replace_clause(b"\xFE\xFF# A\n", "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));
        let txt = b"# A\nx\n";
        let txt_clause = outline(txt, "txt").sections[0].clone();
        assert!(matches!(
            replace_clause(txt, "txt", &txt_clause, "y")
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedFile
        ));

        let row_bytes = b"| REQ-501 | x |\n";
        let row_outline = outline(row_bytes, "md");
        let row = row_outline.rows[0].clone();
        assert!(matches!(
            replace_clause(row_bytes, "md", &row, "y").unwrap_err().code,
            ErrorCode::UnsupportedFile
        ));

        let oversize = vec![b'a'; 1_048_576 + 1];
        assert!(matches!(
            replace_clause(&oversize, "md", &section, "y")
                .unwrap_err()
                .code,
            ErrorCode::PlanTooLarge
        ));

        let longer = outline(b"# A\nxxxxxxxxxxxxxxxxxxxxxx\n", "md");
        let wide = longer.find_key("H1:A#1").unwrap().clone();
        let short: &[u8] = b"# A\nx\n";
        let error = replace_clause(short, "md", &wide, "y").unwrap_err();
        assert!(matches!(error.code, ErrorCode::InvalidArg));
        assert_eq!(error.detail, "clause range is outside the target bytes");
    }
}
