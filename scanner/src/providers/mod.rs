use crate::types::{ProviderResult, ScanError};

pub mod aider;
pub mod browser;
pub mod chatgpt_desktop;
pub mod claude_code;
pub mod codex;
pub mod continue_ext;
pub mod copilot;
pub mod cursor;
pub mod detection;
pub mod gemini;
pub mod jetbrains;
pub mod ollama;
pub mod shell_history;
pub mod warp;

/// Trait for all AI usage data providers.
/// Each provider scans a specific data source independently.
pub trait Provider: Send + Sync {
    /// Machine-readable name: "claude_code", "chrome_browser", etc.
    fn name(&self) -> &'static str;

    /// Display name shown in terminal output
    fn display_name(&self) -> &'static str;

    /// Quick check: does this data source exist on this machine?
    fn is_available(&self) -> bool;

    /// Perform the scan. Returns partial results on partial failure.
    fn scan(&self) -> Result<ProviderResult, ScanError>;
}

/// Returns all registered providers
pub fn all_providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude_code::ClaudeCodeProvider),
        Box::new(chatgpt_desktop::ChatGPTDesktopProvider),
        Box::new(browser::ChromeProvider),
        Box::new(browser::ArcProvider),
        Box::new(browser::BraveProvider),
        Box::new(browser::EdgeProvider),
        Box::new(browser::DiaProvider),
        Box::new(browser::SafariProvider),
        Box::new(browser::FirefoxProvider),
        Box::new(codex::CodexProvider),
        Box::new(cursor::CursorProvider),
        Box::new(shell_history::ShellHistoryProvider),
        Box::new(copilot::CopilotProvider),
        Box::new(warp::WarpProvider),
        Box::new(gemini::GeminiProvider),
        Box::new(aider::AiderProvider),
        Box::new(continue_ext::ContinueProvider),
        Box::new(ollama::OllamaProvider),
        Box::new(jetbrains::JetBrainsProvider),
    ]
}
