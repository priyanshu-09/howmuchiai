use crate::platform;
use crate::providers::Provider;
use crate::token_estimate::ESTIMATE_METHOD_PREFIX;
use crate::types::{ProviderResult, ScanError, TokenUsage};
use std::collections::HashMap;

/// Encrypted Atlas `.data` payloads correlate loosely with conversation size.
const ATLAS_BYTES_PER_ESTIMATED_TOKEN: u64 = 4;

pub struct ChatGPTDesktopProvider;

impl Provider for ChatGPTDesktopProvider {
    fn name(&self) -> &'static str {
        "chatgpt_desktop"
    }

    fn display_name(&self) -> &'static str {
        "ChatGPT Desktop"
    }

    fn is_available(&self) -> bool {
        !platform::chatgpt_desktop_dirs().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let dirs = platform::chatgpt_desktop_dirs();
        if dirs.is_empty() {
            return Err(ScanError::NotFound(
                "ChatGPT Desktop/Atlas not found".into(),
            ));
        }

        let mut conversation_count: u64 = 0;
        let mut payload_bytes: u64 = 0;
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        for base_dir in &dirs {
            let pattern = format!("{}/**/conversations-v3/*/*.data", base_dir.display());
            if let Ok(entries) = glob::glob(&pattern) {
                for entry in entries.flatten() {
                    conversation_count += 1;
                    if let Ok(metadata) = entry.metadata() {
                        payload_bytes += metadata.len();
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                                let ts = duration.as_secs() as i64;
                                first_seen = Some(first_seen.map_or(ts, |fs: i64| fs.min(ts)));
                                last_seen = Some(last_seen.map_or(ts, |ls: i64| ls.max(ts)));
                            }
                        }
                    }
                }
            }
        }

        if conversation_count == 0 {
            return Err(ScanError::NotFound(
                "No ChatGPT Desktop/Atlas conversations found".into(),
            ));
        }

        let estimated_tokens = (payload_bytes / ATLAS_BYTES_PER_ESTIMATED_TOKEN).max(1);

        let mut usage = TokenUsage {
            input_tokens: 0,
            output_tokens: estimated_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total: 0,
        };
        usage.compute_total();

        let mut result = ProviderResult::new("ChatGPT Desktop");
        result.sessions = Some(conversation_count);
        result.first_seen = first_seen;
        result.last_seen = last_seen;
        result.tokens = Some(usage);
        result.tokens_estimated = true;
        result.tokens_estimate_method = Some(format!(
            "{ESTIMATE_METHOD_PREFIX}; estimated from {conversation_count} encrypted Atlas .data payloads ({payload_bytes} bytes ÷ {ATLAS_BYTES_PER_ESTIMATED_TOKEN} bytes/token heuristic)"
        ));

        let mut metadata = HashMap::new();
        metadata.insert(
            "note".to_string(),
            serde_json::json!("Atlas/legacy desktop .data files are encrypted; token count is payload-size heuristic"),
        );
        metadata.insert(
            "payload_bytes".to_string(),
            serde_json::json!(payload_bytes),
        );
        result.metadata = Some(metadata);

        Ok(result)
    }
}
