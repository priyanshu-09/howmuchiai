use crate::platform;
use std::path::PathBuf;

/// Detect AI tools that are installed but lack detailed usage data.
/// Returns a list of tool names that were found on this machine.
pub fn detect_tools() -> Vec<String> {
    let mut detected = Vec::new();

    // Codeium
    if platform::codeium_dir().is_some() {
        detected.push("Codeium".to_string());
    }

    // Tabnine
    if platform::tabnine_dir().is_some() {
        detected.push("Tabnine".to_string());
    }

    // Amazon Q
    if platform::amazon_q_dir().is_some() {
        detected.push("Amazon Q".to_string());
    }

    // Supermaven
    if let Some(home) = dirs::home_dir() {
        if home.join(".supermaven").exists() {
            detected.push("Supermaven".to_string());
        }
    }

    // Roo Code
    if let Some(home) = dirs::home_dir() {
        if home.join(".roo").exists() {
            detected.push("Roo Code".to_string());
        }
    }

    // Windsurf -- check common binary locations
    let windsurf_paths: Vec<PathBuf> = vec![
        PathBuf::from("/usr/local/bin/windsurf"),
        PathBuf::from("/opt/homebrew/bin/windsurf"),
    ];
    let windsurf_home = dirs::home_dir()
        .map(|h| h.join(".local/bin/windsurf"))
        .unwrap_or_default();

    if windsurf_paths.iter().any(|p| p.exists()) || windsurf_home.exists() {
        detected.push("Windsurf".to_string());
    }

    detected
}
