-- V1.3 cloud archive rules; control-plane metadata only.
-- IMPORTANT: each table is tenant-scoped. Vercel server must check membership on every query.
CREATE TABLE IF NOT EXISTS storage_rule_sets (
  id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
  rule_version INTEGER NOT NULL, archive_mode TEXT NOT NULL CHECK(archive_mode IN ('personal_app_data','team_drive_folder','local_only')),
  allow_remote INTEGER NOT NULL DEFAULT 0, max_pending_bytes INTEGER NOT NULL,
  target_segment_bytes INTEGER NOT NULL, created_at TEXT NOT NULL,
  UNIQUE(workspace_id,rule_version)
);
CREATE TABLE IF NOT EXISTS archive_event_segments (
  id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
  device_id TEXT NOT NULL, first_seq INTEGER NOT NULL, last_seq INTEGER NOT NULL,
  sha256 TEXT NOT NULL, provider_binding_id TEXT NOT NULL,
  opaque_remote_locator TEXT NOT NULL, verified_at TEXT, archived_at TEXT NOT NULL,
  CHECK(first_seq <= last_seq),UNIQUE(workspace_id,device_id,first_seq,last_seq)
);
CREATE INDEX IF NOT EXISTS idx_archive_segment_cursor ON archive_event_segments(tenant_id,workspace_id,device_id,first_seq);
CREATE TABLE IF NOT EXISTS archive_index_pages (
  id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
  page_no INTEGER NOT NULL, object_sha256 TEXT NOT NULL, provider_binding_id TEXT NOT NULL,
  opaque_remote_locator TEXT NOT NULL, verified_at TEXT,
  UNIQUE(workspace_id,page_no,object_sha256)
);
CREATE TABLE IF NOT EXISTS sync_merge_conflicts (
  id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
  file_ref_hash TEXT NOT NULL, left_version TEXT NOT NULL, right_version TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('open','reviewing','resolved','rejected')),
  opened_at TEXT NOT NULL, resolved_at TEXT
);
CREATE TABLE IF NOT EXISTS tenant_control_audit (
  id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL,workspace_id TEXT,
  identity_id TEXT NOT NULL,action TEXT NOT NULL,at TEXT NOT NULL,
  detail_digest TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS device_sync_heads (
  tenant_id TEXT NOT NULL, workspace_id TEXT NOT NULL, device_id TEXT NOT NULL,
  next_seq INTEGER NOT NULL, last_segment_sha256 TEXT,updated_at TEXT NOT NULL,
  PRIMARY KEY(tenant_id,workspace_id,device_id)
);
-- Optional short-lived relay; encrypted chat segment on Drive remains authoritative.
CREATE TABLE IF NOT EXISTS chat_delivery_queue (
  id TEXT PRIMARY KEY,tenant_id TEXT NOT NULL,workspace_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL,sender_identity_id TEXT NOT NULL,
  client_message_id TEXT NOT NULL,body_ciphertext BLOB NOT NULL,
  created_at TEXT NOT NULL,expires_at TEXT NOT NULL,ack_at TEXT,
  UNIQUE(conversation_id,client_message_id)
);
CREATE INDEX IF NOT EXISTS idx_chat_relay_scope ON chat_delivery_queue(tenant_id,workspace_id,conversation_id,created_at);
