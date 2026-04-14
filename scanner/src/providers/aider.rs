use crate::platform;
use crate::providers::Provider;
use crate::types::{ProviderResult, ScanError};

pub struct AiderProvider;

impl Provider for AiderProvider {
    fn name(&self) -> &'static str {
        "aider"
    }

    fn display_name(&self) -> &'static str {
        "Aider"
    }

    fn is_available(&self) -> bool {
        platform::aider_history_path().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let history_path = platform::aider_history_path()
            .ok_or_else(|| ScanError::NotFound("Aider history file not found".into()))?;

        let content = std::fs::read_to_string(&history_path)?;

        let invocations = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count() as u64;

        let mut result = ProviderResult::new("Aider");
        result.invocations = Some(invocations);

        Ok(result)
    }
}
