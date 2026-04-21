use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total: u64,
}

impl TokenUsage {
    pub fn compute_total(&mut self) {
        // Total = input + output only. Cache tokens are tracked separately
        // because cache_read inflates totals by 10-100x (re-reads of cached prompts).
        self.total = self.input_tokens + self.output_tokens;
    }

    pub fn merge(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.compute_total();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub tokens: TokenUsage,
    pub sessions: u64,
    pub hours: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResult {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visits: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocations: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelUsage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

impl ProviderResult {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            hours: None,
            tokens: None,
            sessions: None,
            visits: None,
            invocations: None,
            models: None,
            first_seen: None,
            last_seen: None,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Totals {
    pub hours: f64,
    pub tokens: u64,
    pub sessions: u64,
    pub visits: u64,
    pub invocations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub scanned_at: String,
    pub platform: String,
    pub scan_duration_ms: u64,
    pub sources: HashMap<String, ProviderResult>,
    pub totals: Totals,
    pub detected_tools: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON parse error: {0}")]
    Json(String),
    #[error("Data source not found: {0}")]
    NotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}
