-- LADON Explorer schema v0.1, compatible Turso/SQLite subset.
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS resources (
    id TEXT PRIMARY KEY,
    relative_path TEXT NOT NULL UNIQUE,
    current_hash TEXT NOT NULL,
    version_no INTEGER NOT NULL DEFAULT 0,
    sensitive INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_resources_path ON resources(relative_path);
CREATE TABLE IF NOT EXISTS versions (
    id TEXT PRIMARY KEY,
    file_id TEXT NOT NULL REFERENCES resources(id),
    number INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    sensitive INTEGER NOT NULL DEFAULT 0,
    changed_sections_json TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(file_id,number)
);
CREATE INDEX IF NOT EXISTS idx_versions_file ON versions(file_id,number DESC);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    file_id TEXT NOT NULL REFERENCES resources(id),
    version_id TEXT NOT NULL REFERENCES versions(id),
    kind TEXT NOT NULL,
    old_path TEXT NOT NULL,
    new_path TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    previous_hash TEXT NOT NULL,
    current_hash TEXT NOT NULL,
    ledger_prev_hash TEXT NOT NULL,
    ledger_hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_file ON events(file_id,seq DESC);
CREATE TABLE IF NOT EXISTS archive_objects (
    content_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_locator TEXT NOT NULL,
    encrypted INTEGER NOT NULL,
    verified_at TEXT NOT NULL,
    PRIMARY KEY (content_hash,provider)
);
CREATE TABLE IF NOT EXISTS file_tags (
    file_id TEXT NOT NULL REFERENCES resources(id),
    tag TEXT NOT NULL,
    source TEXT NOT NULL,
    rule_version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(file_id,tag)
);
CREATE TABLE IF NOT EXISTS annotations (
    id TEXT PRIMARY KEY,
    file_id TEXT NOT NULL REFERENCES resources(id),
    section_key TEXT NOT NULL,
    base_version_id TEXT NOT NULL,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_tasks (
    id TEXT PRIMARY KEY,
    file_id TEXT NOT NULL REFERENCES resources(id),
    anchor_section TEXT NOT NULL,
    base_version_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS graph_edges (
    id TEXT PRIMARY KEY,
    from_node TEXT NOT NULL,
    to_node TEXT NOT NULL,
    relation TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS embeddings (
    id TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    model_id TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    vector_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(content_hash,model_id)
);
CREATE TABLE IF NOT EXISTS backup_jobs (
    id TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    provider TEXT NOT NULL,
    state TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS governance_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    file_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    actor TEXT NOT NULL,
    timestamp TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_governance_file ON governance_events(file_id,seq DESC);
