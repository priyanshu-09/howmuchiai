use crate::platform;
use crate::providers::Provider;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct ContinueProvider;

impl Provider for ContinueProvider {
    fn name(&self) -> &'static str {
        "continue_dev"
    }

    fn display_name(&self) -> &'static str {
        "Continue"
    }

    fn is_available(&self) -> bool {
        platform::continue_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let continue_dir = platform::continue_dir()
            .ok_or_else(|| ScanError::NotFound("Continue directory not found".into()))?;

        let sessions_dir = continue_dir.join("sessions");
        if !sessions_dir.exists() {
            let mut result = ProviderResult::new("Continue");
            result.sessions = Some(0);
            return Ok(result);
        }

        let pattern = format!("{}/*.json", sessions_dir.to_string_lossy());
        let session_files: Vec<_> = glob::glob(&pattern)
            .map(|paths| paths.filter_map(|p| p.ok()).collect())
            .unwrap_or_default();

        let session_count = session_files.len() as u64;

        let mut metadata = HashMap::new();
        metadata.insert(
            "session_files".to_string(),
            serde_json::Value::from(session_count),
        );

        let mut result = ProviderResult::new("Continue");
        result.sessions = Some(session_count);
        result.metadata = Some(metadata);

        Ok(result)
    }
}
