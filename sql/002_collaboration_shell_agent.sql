-- LADON v0.2 additive migration: identity, tenants, chat, Agent API, Shell audit.
-- Existing v0.1 tables are local-only until explicit tenant-scoping migration/reconciliation.
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS identities (
    id TEXT PRIMARY KEY, issuer TEXT NOT NULL, external_subject TEXT NOT NULL,
    display_name TEXT NOT NULL, created_at TEXT NOT NULL,
    UNIQUE(issuer,external_subject)
);
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, owner_identity_id TEXT NOT NULL REFERENCES identities(id),
    created_at TEXT NOT NULL, lifecycle_state TEXT NOT NULL DEFAULT 'active'
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, primary_archive_binding_id TEXT,
    created_at TEXT NOT NULL, UNIQUE(tenant_id,name)
);
CREATE INDEX IF NOT EXISTS idx_workspaces_tenant ON workspaces(tenant_id);
CREATE TABLE IF NOT EXISTS memberships (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id), identity_id TEXT NOT NULL REFERENCES identities(id),
    role TEXT NOT NULL CHECK(role IN ('owner','admin','editor','reviewer','viewer','agent')),
    state TEXT NOT NULL CHECK(state IN ('active','invited','revoked')),
    joined_at TEXT NOT NULL, UNIQUE(workspace_id,identity_id)
);
CREATE INDEX IF NOT EXISTS idx_membership_tenant ON memberships(tenant_id,workspace_id,identity_id);
CREATE TABLE IF NOT EXISTS device_sessions (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),identity_id TEXT NOT NULL REFERENCES identities(id),
    device_public_key TEXT NOT NULL,refresh_token_digest TEXT NOT NULL,
    created_at TEXT NOT NULL,expires_at TEXT NOT NULL,revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_owner ON device_sessions(tenant_id,workspace_id,identity_id);
CREATE TABLE IF NOT EXISTS archive_bindings (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),provider TEXT NOT NULL,
    external_account_subject TEXT NOT NULL,credential_secret_ref TEXT NOT NULL,
    root_object_id TEXT NOT NULL,state TEXT NOT NULL,created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_archive_bindings_space ON archive_bindings(tenant_id,workspace_id);
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),kind TEXT NOT NULL CHECK(kind IN ('ai','team')),
    title TEXT NOT NULL,created_by TEXT NOT NULL REFERENCES identities(id),
    created_at TEXT NOT NULL,closed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_conversations_scope ON conversations(tenant_id,workspace_id,created_at);
CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    sender_identity_id TEXT, sender_agent_id TEXT,client_message_id TEXT NOT NULL,
    body_ciphertext BLOB NOT NULL,created_at TEXT NOT NULL,
    edited_at TEXT,deleted_at TEXT,UNIQUE(conversation_id,client_message_id)
);
CREATE INDEX IF NOT EXISTS idx_chat_history ON chat_messages(tenant_id,workspace_id,conversation_id,created_at);
CREATE TABLE IF NOT EXISTS chat_attachments (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    message_id TEXT NOT NULL REFERENCES chat_messages(id),
    file_ref TEXT NOT NULL,base_version_id TEXT,access_policy_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_task_links (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    message_id TEXT NOT NULL REFERENCES chat_messages(id),
    doc_task_id TEXT NOT NULL,created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_grants (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    agent_id TEXT NOT NULL,scope TEXT NOT NULL,allowed_operations TEXT NOT NULL,
    expires_at TEXT NOT NULL,revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_grant ON agent_grants(tenant_id,workspace_id,agent_id);
CREATE TABLE IF NOT EXISTS sync_cursors (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),device_id TEXT NOT NULL,
    stream TEXT NOT NULL,cursor_seq INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,UNIQUE(device_id,stream)
);
CREATE TABLE IF NOT EXISTS shell_audit (
    id TEXT PRIMARY KEY,tenant_id TEXT,workspace_id TEXT,resource_ref TEXT,
    operation TEXT NOT NULL,origin TEXT NOT NULL,host_status TEXT NOT NULL,
    occurred_at TEXT NOT NULL,diagnostic_code TEXT
);
CREATE TABLE IF NOT EXISTS external_history_exports (
    id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL REFERENCES tenants(id),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    archive_binding_id TEXT NOT NULL REFERENCES archive_bindings(id),
    segment_hash TEXT NOT NULL,segment_locator TEXT NOT NULL,
    verified_at TEXT NOT NULL,UNIQUE(archive_binding_id,segment_hash)
);
