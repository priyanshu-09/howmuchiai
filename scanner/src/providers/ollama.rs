use crate::platform;
use crate::providers::Provider;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct OllamaProvider;

impl Provider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn display_name(&self) -> &'static str {
        "Ollama"
    }

    fn is_available(&self) -> bool {
        platform::ollama_models_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let models_dir = platform::ollama_models_dir()
            .ok_or_else(|| ScanError::NotFound("Ollama models directory not found".into()))?;

        let library_dir = models_dir
            .join("manifests")
            .join("registry.ollama.ai")
            .join("library");

        if !library_dir.exists() {
            let mut result = ProviderResult::new("Ollama");
            result.invocations = Some(0);
            return Ok(result);
        }

        let mut model_names: Vec<String> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&library_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        model_names.push(name.to_string());
                    }
                }
            }
        }

        model_names.sort();

        let model_count = model_names.len() as u64;

        let mut metadata = HashMap::new();
        let models_value: Vec<serde_json::Value> = model_names
            .iter()
            .map(|n| serde_json::Value::String(n.clone()))
            .collect();
        metadata.insert("models".to_string(), serde_json::Value::Array(models_value));
        metadata.insert(
            "model_count".to_string(),
            serde_json::Value::from(model_count),
        );

        let mut result = ProviderResult::new("Ollama");
        result.invocations = Some(model_count);
        result.metadata = Some(metadata);

        Ok(result)
    }
}
