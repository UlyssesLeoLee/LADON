//! LADON Explorer core. Real local history/archiving services, future UI adapters.
pub mod archive;
pub mod crypto;
pub mod diff;
pub mod engine;
pub mod entry;
pub mod model;
pub mod nema;
pub mod policy;
pub mod provider;
pub mod google_oauth;
pub mod search;
pub mod store;
pub mod tagging;
pub mod workspace;

pub mod shell;
pub mod auth;
pub mod agent_api;

// V1.3 additions: verified archival journal, deterministic policies, adaptive layout, grounded suggestions.
pub mod storage_rules;
pub mod cloud_journal;
pub mod ui_layout;
pub mod emergence;

// Document-task pure model: states, transitions, errors, hashes, limits, scheduling.
pub mod doctask_model;

// Document-task pure clause layer: outline, clause numbers, anchor classification, byte-level splice.
pub mod doctask_outline;

// Document-task persistence: immediate transactions, counters, governance chain, legacy backfill.
pub mod doctask_store;

// Document-task service core: caller, transfer gate, agent view, baselines, annotations, tasks.
pub mod doctask_service;

// Document-task workflow: transitions, approvals, proposals, dependencies, links, evidence,
// quotas, inbox, schedule and verification.
pub mod doctask_flow;

// Document-task reconciliation: anchor review against the latest version, owner decisions on
// moved anchors, and file-move lineage.
pub mod doctask_reconcile;
