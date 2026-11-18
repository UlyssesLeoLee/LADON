//! LADON Explorer core. Real local history/archiving services, future UI adapters.
pub mod archive;
pub mod crypto;
pub mod diff;
pub mod engine;
pub mod model;
pub mod nema;
pub mod policy;
pub mod provider;
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
