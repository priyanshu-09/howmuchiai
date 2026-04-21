use crate::platform;
use crate::providers::Provider;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct ChatGPTDesktopProvider;

impl Provider for ChatGPTDesktopProvider {
    fn name(&self) -> &'static str {
        "chatgpt_desktop"
    }

    fn display_name(&self) -> &'static str {
        "ChatGPT Desktop"
    }

    fn is_available(&self) -> bool {
        platform::chatgpt_desktop_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let base_dir = platform::chatgpt_desktop_dir()
            .ok_or_else(|| ScanError::NotFound("ChatGPT Desktop not found".into()))?;

        // Count conversations from conversations-v3-* directories
        // The .data files are binary-encoded so we can only count them
        let mut conversation_count: u64 = 0;
        let mut first_seen: Option<i64> = None;
        let mut last_seen: Option<i64> = None;

        let pattern = format!("{}/conversations-v3-*/*.data", base_dir.display());
        if let Ok(entries) = glob::glob(&pattern) {
            for entry in entries.flatten() {
                conversation_count += 1;

                // Use file modification time as a proxy for conversation time
                if let Ok(metadata) = entry.metadata() {
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

        if conversation_count == 0 {
            return Err(ScanError::NotFound(
                "No ChatGPT Desktop conversations found".into(),
            ));
        }

        let mut result = ProviderResult::new("ChatGPT Desktop");
        result.sessions = Some(conversation_count);
        result.first_seen = first_seen;
        result.last_seen = last_seen;

        let mut metadata = HashMap::new();
        metadata.insert(
            "note".to_string(),
            serde_json::json!("Conversation count from .data files — content is binary-encoded"),
        );
        result.metadata = Some(metadata);

        Ok(result)
    }
}
