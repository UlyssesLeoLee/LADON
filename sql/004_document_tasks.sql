CREATE TABLE IF NOT EXISTS doc_counters (
 name TEXT PRIMARY KEY,
 value INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS doc_baselines (
 id TEXT PRIMARY KEY,
 file_id TEXT NOT NULL REFERENCES resources(id),
 version_id TEXT NOT NULL REFERENCES versions(id),
 version_no INTEGER NOT NULL,
 content_hash TEXT NOT NULL,
 approver TEXT NOT NULL,
 approver_kind TEXT NOT NULL,
 note TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 revoked_at TEXT,
 revoked_by TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_baselines_file ON doc_baselines(file_id, created_at);
CREATE INDEX IF NOT EXISTS idx_doc_baselines_version ON doc_baselines(file_id, version_id);
CREATE TABLE IF NOT EXISTS doc_anchors (
 id TEXT PRIMARY KEY,
 file_id TEXT NOT NULL REFERENCES resources(id),
 base_version_id TEXT NOT NULL,
 baseline_id TEXT NOT NULL DEFAULT '',
 section_key TEXT NOT NULL,
 clause_id TEXT NOT NULL DEFAULT '',
 title TEXT NOT NULL DEFAULT '',
 level INTEGER NOT NULL DEFAULT 0,
 line_start INTEGER NOT NULL DEFAULT 0,
 line_end INTEGER NOT NULL DEFAULT 0,
 full_hash TEXT NOT NULL DEFAULT '',
 body_hash TEXT NOT NULL DEFAULT '',
 state TEXT NOT NULL DEFAULT 'valid',
 current_version_id TEXT NOT NULL,
 current_key TEXT NOT NULL,
 current_hash TEXT NOT NULL DEFAULT '',
 reviewed_version_id TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL,
 UNIQUE (file_id, base_version_id, section_key));
CREATE INDEX IF NOT EXISTS idx_doc_anchors_file ON doc_anchors(file_id, state);
CREATE TABLE IF NOT EXISTS doc_task_meta (
 task_id TEXT PRIMARY KEY REFERENCES doc_tasks(id),
 task_no INTEGER NOT NULL UNIQUE,
 anchor_id TEXT NOT NULL REFERENCES doc_anchors(id),
 owner TEXT NOT NULL DEFAULT '',
 priority INTEGER NOT NULL DEFAULT 2,
 proposal_state TEXT NOT NULL DEFAULT 'accepted',
 proposal_decided_by TEXT NOT NULL DEFAULT '',
 proposal_decided_at TEXT,
 visibility TEXT NOT NULL DEFAULT 'shared',
 source_kind TEXT NOT NULL DEFAULT 'cli',
 source_ref TEXT NOT NULL DEFAULT '',
 created_by TEXT NOT NULL,
 created_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 last_approval_id TEXT NOT NULL DEFAULT '',
 cancel_reason TEXT NOT NULL DEFAULT '',
 updated_at TEXT NOT NULL,
 closed_at TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_task_meta_anchor ON doc_task_meta(anchor_id);
CREATE INDEX IF NOT EXISTS idx_doc_task_meta_proposal ON doc_task_meta(proposal_state);
CREATE TABLE IF NOT EXISTS doc_annotation_meta (
 annotation_id TEXT PRIMARY KEY REFERENCES annotations(id),
 annotation_no INTEGER NOT NULL UNIQUE,
 anchor_id TEXT NOT NULL REFERENCES doc_anchors(id),
 author TEXT NOT NULL,
 author_kind TEXT NOT NULL,
 rationale TEXT NOT NULL DEFAULT '',
 visibility TEXT NOT NULL DEFAULT 'shared',
 quote TEXT NOT NULL DEFAULT '',
 line_start INTEGER NOT NULL DEFAULT 0,
 line_end INTEGER NOT NULL DEFAULT 0,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_annotation_meta_anchor ON doc_annotation_meta(anchor_id);
CREATE TABLE IF NOT EXISTS doc_task_deps (
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 depends_on TEXT NOT NULL REFERENCES doc_tasks(id),
 created_by TEXT NOT NULL,
 created_at TEXT NOT NULL,
 PRIMARY KEY (task_id, depends_on));
CREATE INDEX IF NOT EXISTS idx_doc_task_deps_rev ON doc_task_deps(depends_on);
CREATE TABLE IF NOT EXISTS doc_task_events (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 task_id TEXT NOT NULL DEFAULT '',
 file_id TEXT NOT NULL DEFAULT '',
 subject_kind TEXT NOT NULL,
 subject_id TEXT NOT NULL DEFAULT '',
 kind TEXT NOT NULL,
 from_status TEXT NOT NULL DEFAULT '',
 to_status TEXT NOT NULL DEFAULT '',
 actor TEXT NOT NULL,
 actor_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 claimed_session TEXT NOT NULL DEFAULT '',
 payload_json TEXT NOT NULL,
 timestamp TEXT NOT NULL,
 prev_hash TEXT NOT NULL,
 hash TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_events_task ON doc_task_events(task_id, seq);
CREATE INDEX IF NOT EXISTS idx_doc_task_events_file ON doc_task_events(file_id, seq);
CREATE UNIQUE INDEX IF NOT EXISTS idx_doc_task_events_prev ON doc_task_events(prev_hash);
CREATE TABLE IF NOT EXISTS doc_task_plans (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 plan_no INTEGER NOT NULL,
 summary TEXT NOT NULL,
 rationale TEXT NOT NULL DEFAULT '',
 impact TEXT NOT NULL DEFAULT '',
 risk TEXT NOT NULL DEFAULT 'medium',
 options_json TEXT NOT NULL DEFAULT '[]',
 chosen_option TEXT NOT NULL DEFAULT '',
 edit_kind TEXT NOT NULL DEFAULT '',
 target_file_id TEXT NOT NULL DEFAULT '',
 target_section TEXT NOT NULL DEFAULT '',
 new_text TEXT NOT NULL DEFAULT '',
 reads_json TEXT NOT NULL DEFAULT '[]',
 expected_version_id TEXT NOT NULL DEFAULT '',
 expected_hash TEXT NOT NULL DEFAULT '',
 result_hash TEXT NOT NULL DEFAULT '',
 plan_hash TEXT NOT NULL,
 status TEXT NOT NULL,
 submitted_by TEXT NOT NULL,
 submitted_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 decided_at TEXT,
 applied_at TEXT,
 applied_version_id TEXT NOT NULL DEFAULT '',
 UNIQUE (task_id, plan_no));
CREATE TABLE IF NOT EXISTS doc_task_approvals (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 kind TEXT NOT NULL,
 decision TEXT NOT NULL,
 approver TEXT NOT NULL,
 approver_kind TEXT NOT NULL,
 plan_id TEXT NOT NULL DEFAULT '',
 subject_hash TEXT NOT NULL,
 base_version_id TEXT NOT NULL,
 comment TEXT NOT NULL DEFAULT '',
 event_id TEXT NOT NULL,
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_approvals_task ON doc_task_approvals(task_id, created_at);
CREATE TABLE IF NOT EXISTS doc_task_links (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 link_kind TEXT NOT NULL,
 target_kind TEXT NOT NULL,
 target_file_id TEXT NOT NULL DEFAULT '',
 target_section TEXT NOT NULL DEFAULT '',
 target_ref TEXT NOT NULL DEFAULT '',
 label TEXT NOT NULL DEFAULT '',
 created_by TEXT NOT NULL,
 created_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 removed_at TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_task_links_task ON doc_task_links(task_id);
CREATE TABLE IF NOT EXISTS doc_task_evidence (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 kind TEXT NOT NULL,
 result TEXT NOT NULL,
 source TEXT NOT NULL,
 ref TEXT NOT NULL DEFAULT '',
 version_id TEXT NOT NULL DEFAULT '',
 content_hash TEXT NOT NULL DEFAULT '',
 summary TEXT NOT NULL,
 actor TEXT NOT NULL,
 actor_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 event_id TEXT NOT NULL,
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_evidence_task ON doc_task_evidence(task_id, created_at);
CREATE TABLE IF NOT EXISTS doc_file_lineage (
 id TEXT PRIMARY KEY,
 old_file_id TEXT NOT NULL REFERENCES resources(id),
 new_file_id TEXT NOT NULL REFERENCES resources(id),
 evidence TEXT NOT NULL,
 score_permille INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'proposed',
 decided_by TEXT NOT NULL DEFAULT '',
 decided_at TEXT,
 created_at TEXT NOT NULL,
 UNIQUE (old_file_id, new_file_id));
CREATE TABLE IF NOT EXISTS doc_agent_sessions (
 launch_id TEXT PRIMARY KEY,
 hermes_slug TEXT NOT NULL,
 hermes_exe TEXT NOT NULL,
 focus_task_no INTEGER,
 started_by TEXT NOT NULL,
 started_at TEXT NOT NULL,
 ended_at TEXT,
 exit_code INTEGER,
 policy_fp TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS doc_agent_disclosures (
 launch_id TEXT NOT NULL REFERENCES doc_agent_sessions(launch_id),
 file_id TEXT NOT NULL REFERENCES resources(id),
 version_id TEXT NOT NULL DEFAULT '',
 content_hash TEXT NOT NULL DEFAULT '',
 first_at TEXT NOT NULL,
 PRIMARY KEY (launch_id, file_id, version_id));
CREATE INDEX IF NOT EXISTS idx_doc_tasks_file ON doc_tasks(file_id, status);
CREATE INDEX IF NOT EXISTS idx_doc_tasks_status ON doc_tasks(status);
CREATE INDEX IF NOT EXISTS idx_annotations_file ON annotations(file_id, section_key);
