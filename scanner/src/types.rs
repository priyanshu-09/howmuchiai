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
        // Total = input + output + cache_read + cache_creation. This matches
        // what ccusage / openusage / CodexBar report ("tokens processed by
        // the model"), so users comparing across tools see consistent
        // numbers. Cost math doesn't read this — it walks the per-field
        // values with per-field rates from src/lib/pricing.ts on the web.
        self.total = self
            .input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_creation_tokens);
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

/// Per-day usage aggregation keyed by UTC ISO date ("YYYY-MM-DD").
/// Providers may optionally populate this for streak + heatmap widgets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyBucket {
    pub hours: f64,
    pub tokens: u64,
    pub sessions: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocations: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_buckets: Option<std::collections::HashMap<String, DailyBucket>>,
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
            daily_buckets: None,
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

/// JSON schema version for ScanResult.
///
/// History:
/// * v1 — initial release
/// * v2 — added daily_buckets to provider results
/// * v3 — added device_id + device_label; share hash is gzip-then-base64url
pub const SCHEMA_VERSION: u32 = 3;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub scanned_at: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub platform: String,
    pub scan_duration_ms: u64,
    /// Stable UUID v4 identifying this machine. Persisted at the OS-appropriate
    /// config dir (see `device_id::load_or_create`). Lets the web dashboard
    /// aggregate scans across multiple devices under one account.
    pub device_id: String,
    /// User-friendly label (typically hostname) — best-effort, may be `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
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
