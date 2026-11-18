use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub id: String,
    pub relative_path: String,
    pub current_hash: String,
    pub version_no: i64,
    pub sensitive: bool,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRecord {
    pub id: String,
    pub file_id: String,
    pub number: i64,
    pub content_hash: String,
    pub byte_size: i64,
    pub sensitive: bool,
    pub changed_sections: Vec<String>,
    pub actor: String,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub file_id: String,
    pub version_id: String,
    pub kind: String,
    pub old_path: String,
    pub new_path: String,
    pub timestamp: String,
    pub actor: String,
    pub reason: String,
    pub previous_hash: String,
    pub current_hash: String,
    pub ledger_prev_hash: String,
    pub ledger_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureResult {
    pub file_id: String,
    pub version_id: Option<String>,
    pub event_id: Option<String>,
    pub status: String,
    pub sensitive: bool,
    pub changed_sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageObject {
    pub content_hash: String,
    pub provider: String,
    pub provider_locator: String,
    pub encrypted: bool,
    pub verified_at: String,
}
