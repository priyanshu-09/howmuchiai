use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct CopilotProvider;

impl Provider for CopilotProvider {
    fn name(&self) -> &'static str {
        "github_copilot"
    }

    fn display_name(&self) -> &'static str {
        "GitHub Copilot"
    }

    fn is_available(&self) -> bool {
        platform::vscode_workspace_storage_dir().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let storage_dir = platform::vscode_workspace_storage_dir()
            .ok_or_else(|| ScanError::NotFound("VS Code workspace storage not found".into()))?;

        let pattern = format!("{}/*/state.vscdb", storage_dir.to_string_lossy());
        let db_files: Vec<_> = glob::glob(&pattern)
            .map(|paths| paths.filter_map(|p| p.ok()).collect())
            .unwrap_or_default();

        if db_files.is_empty() {
            let mut result = ProviderResult::new("GitHub Copilot");
            result.invocations = Some(0);
            return Ok(result);
        }

        let mut workspaces_with_copilot: u64 = 0;
        let mut total_workspaces_scanned: u64 = 0;

        for db_file in &db_files {
            total_workspaces_scanned += 1;

            let db = match SafeSqlite::open(db_file) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let conn = db.conn();

            // Check if this workspace has any copilot-related keys
            let has_copilot: bool = conn
                .prepare("SELECT key FROM ItemTable WHERE key LIKE '%copilot%' LIMIT 1")
                .and_then(|mut stmt| stmt.query_row([], |_row| Ok(true)))
                .unwrap_or(false);

            if has_copilot {
                workspaces_with_copilot += 1;
            }
        }

        let mut metadata = HashMap::new();
        metadata.insert(
            "workspaces_scanned".to_string(),
            serde_json::Value::from(total_workspaces_scanned),
        );
        metadata.insert(
            "workspaces_with_copilot".to_string(),
            serde_json::Value::from(workspaces_with_copilot),
        );

        let mut result = ProviderResult::new("GitHub Copilot");
        result.invocations = Some(workspaces_with_copilot);
        result.metadata = Some(metadata);

        Ok(result)
    }
}
