use crate::providers::Provider;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct JetBrainsProvider;

impl Provider for JetBrainsProvider {
    fn name(&self) -> &'static str {
        "jetbrains_ai"
    }

    fn display_name(&self) -> &'static str {
        "JetBrains AI Assistant"
    }

    fn is_available(&self) -> bool {
        if cfg!(target_os = "macos") {
            let jetbrains_dir = crate::platform::home_dir()
                .join("Library")
                .join("Application Support")
                .join("JetBrains");
            jetbrains_dir.exists()
        } else {
            // Linux: ~/.config/JetBrains/
            let jetbrains_dir = crate::platform::home_dir()
                .join(".config")
                .join("JetBrains");
            jetbrains_dir.exists()
        }
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let jetbrains_base = if cfg!(target_os = "macos") {
            crate::platform::home_dir()
                .join("Library")
                .join("Application Support")
                .join("JetBrains")
        } else {
            crate::platform::home_dir()
                .join(".config")
                .join("JetBrains")
        };

        if !jetbrains_base.exists() {
            return Err(ScanError::NotFound("JetBrains directory not found".into()));
        }

        let pattern = format!("{}/*/options/other.xml", jetbrains_base.to_string_lossy());
        let xml_files: Vec<_> = glob::glob(&pattern)
            .map(|paths| paths.filter_map(|p| p.ok()).collect())
            .unwrap_or_default();

        if xml_files.is_empty() {
            let mut result = ProviderResult::new("JetBrains AI Assistant");
            result.invocations = Some(0);
            return Ok(result);
        }

        let mut ides_with_ai: Vec<String> = Vec::new();

        for xml_file in &xml_files {
            let content = match std::fs::read_to_string(xml_file) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if content.contains("AIAssistant") || content.contains("ai.assistant") {
                // Extract IDE name from path: .../JetBrains/<IdeName>/options/other.xml
                if let Some(ide_dir) = xml_file.parent().and_then(|p| p.parent()) {
                    if let Some(ide_name) = ide_dir.file_name().and_then(|n| n.to_str()) {
                        ides_with_ai.push(ide_name.to_string());
                    }
                }
            }
        }

        ides_with_ai.sort();

        let ide_count = ides_with_ai.len() as u64;

        let mut metadata = HashMap::new();
        let ides_value: Vec<serde_json::Value> = ides_with_ai
            .iter()
            .map(|n| serde_json::Value::String(n.clone()))
            .collect();
        metadata.insert(
            "ides_with_ai".to_string(),
            serde_json::Value::Array(ides_value),
        );

        let mut result = ProviderResult::new("JetBrains AI Assistant");
        result.invocations = Some(ide_count);
        result.metadata = Some(metadata);

        Ok(result)
    }
}
