use std::path::PathBuf;

pub fn detect_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().expect("Could not determine home directory")
}

// --- Claude Code ---

pub fn claude_projects_dir() -> PathBuf {
    home_dir().join(".claude").join("projects")
}

// --- Codex ---

pub fn codex_dir() -> PathBuf {
    home_dir().join(".codex")
}

pub fn codex_sqlite() -> PathBuf {
    codex_dir().join("state_5.sqlite")
}

pub fn codex_sessions_dir() -> PathBuf {
    codex_dir().join("sessions")
}

// --- Browser History ---

pub fn chrome_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut paths = Vec::new();

    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Google/Chrome");
        paths.push(base.join("Default/History"));
        // Check numbered profiles
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("Profile ") {
                    paths.push(entry.path().join("History"));
                }
            }
        }
    } else {
        let base = home.join(".config/google-chrome");
        paths.push(base.join("Default/History"));
    }

    paths.into_iter().filter(|p| p.exists()).collect()
}

pub fn arc_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut paths = Vec::new();

    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Arc/User Data");
        paths.push(base.join("Default/History"));
    }

    paths.into_iter().filter(|p| p.exists()).collect()
}

pub fn brave_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut paths = Vec::new();

    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/BraveSoftware/Brave-Browser");
        paths.push(base.join("Default/History"));
    } else {
        let base = home.join(".config/BraveSoftware/Brave-Browser");
        paths.push(base.join("Default/History"));
    }

    paths.into_iter().filter(|p| p.exists()).collect()
}

pub fn edge_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut paths = Vec::new();

    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Microsoft Edge");
        paths.push(base.join("Default/History"));
    } else {
        let base = home.join(".config/microsoft-edge");
        paths.push(base.join("Default/History"));
    }

    paths.into_iter().filter(|p| p.exists()).collect()
}

pub fn safari_history_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let p = home_dir().join("Library/Safari/History.db");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    } else {
        None
    }
}

pub fn firefox_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Firefox/Profiles")
    } else {
        home.join(".mozilla/firefox")
    };

    let pattern = base.join("*/places.sqlite");
    glob::glob(pattern.to_string_lossy().as_ref())
        .map(|paths| paths.filter_map(|p| p.ok()).collect())
        .unwrap_or_default()
}

// --- Dia ---

pub fn dia_history_paths() -> Vec<PathBuf> {
    let home = home_dir();
    let mut paths = Vec::new();

    if cfg!(target_os = "macos") {
        let base = home.join("Library/Application Support/Dia/User Data");
        paths.push(base.join("Default/History"));
    }

    paths.into_iter().filter(|p| p.exists()).collect()
}

// --- Claude Desktop ---

pub fn claude_desktop_sessions_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let p = home_dir().join("Library/Application Support/Claude/local-agent-mode-sessions");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

// --- ChatGPT Desktop ---

pub fn chatgpt_desktop_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let p = home_dir().join("Library/Application Support/com.openai.chat");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

// --- Cursor ---

pub fn cursor_state_db() -> Option<PathBuf> {
    let home = home_dir();
    let p = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cursor/User/globalStorage/state.vscdb")
    } else {
        home.join(".config/Cursor/User/globalStorage/state.vscdb")
    };
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Warp ---

pub fn warp_sqlite_path() -> Option<PathBuf> {
    let home = home_dir();
    if cfg!(target_os = "macos") {
        // Check multiple Warp variants
        for dir_name in &["dev.warp.Warp-Stable", "dev.warp.Warp"] {
            let p = home
                .join("Library/Application Support")
                .join(dir_name)
                .join("warp.sqlite");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

// --- Gemini CLI ---

pub fn gemini_dir() -> PathBuf {
    home_dir().join(".gemini")
}

// --- VS Code Copilot ---

pub fn vscode_workspace_storage_dir() -> Option<PathBuf> {
    let home = home_dir();
    let p = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Code/User/workspaceStorage")
    } else {
        home.join(".config/Code/User/workspaceStorage")
    };
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Shell History ---

pub fn zsh_history_path() -> Option<PathBuf> {
    let p = home_dir().join(".zsh_history");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub fn bash_history_path() -> Option<PathBuf> {
    let p = home_dir().join(".bash_history");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub fn fish_history_path() -> Option<PathBuf> {
    let p = home_dir().join(".local/share/fish/fish_history");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Aider ---

pub fn aider_history_path() -> Option<PathBuf> {
    let p = home_dir().join(".aider.history");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Continue ---

pub fn continue_dir() -> Option<PathBuf> {
    let p = home_dir().join(".continue");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Ollama ---

pub fn ollama_models_dir() -> Option<PathBuf> {
    let p = home_dir().join(".ollama/models");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

// --- Detection paths ---

pub fn codeium_dir() -> Option<PathBuf> {
    let p = home_dir().join(".codeium");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub fn tabnine_dir() -> Option<PathBuf> {
    let p = home_dir().join(".tabnine");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub fn amazon_q_dir() -> Option<PathBuf> {
    let p = home_dir().join(".aws/amazonq");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}
