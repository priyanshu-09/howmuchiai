//! Stable per-machine identity used to aggregate scans across devices.
//!
//! A UUID v4 is generated on first scan and written to the OS-appropriate
//! config directory:
//!
//! | OS      | Path                                                          |
//! |---------|---------------------------------------------------------------|
//! | macOS   | `~/Library/Application Support/howmuchiai/device_id`          |
//! | Linux   | `$XDG_CONFIG_HOME/howmuchiai/device_id` (default `~/.config/`) |
//! | Windows | `%APPDATA%\howmuchiai\device_id`                              |
//!
//! Regenerating the file produces a fresh device; deleting it resets identity.
//! On unix the file is chmod 0600 — it's not a secret, but it's per-user
//! state and we treat it like one.

use std::path::PathBuf;

const APP_NAME: &str = "howmuchiai";
const FILE_NAME: &str = "device_id";

/// Resolve the per-OS path where the device id is persisted.
fn device_id_path() -> Option<PathBuf> {
    // macOS:   ~/Library/Application Support/howmuchiai/device_id
    // Windows: %APPDATA%\howmuchiai\device_id
    // Linux:   $XDG_CONFIG_HOME/howmuchiai/device_id  (fallback ~/.config/...)
    let base = if cfg!(target_os = "macos") {
        dirs::data_dir()?
    } else if cfg!(target_os = "windows") {
        dirs::config_dir()?
    } else {
        // Linux + others: prefer XDG config dir
        dirs::config_dir()?
    };
    Some(base.join(APP_NAME).join(FILE_NAME))
}

/// Read an existing device id, or create + persist a fresh UUID v4.
///
/// Failures are non-fatal: if we can't read or write the file, a one-shot UUID
/// is returned so the scan still works (it just won't aggregate across runs).
pub fn load_or_create() -> String {
    if let Some(path) = device_id_path() {
        // Try to read first.
        if let Ok(existing) = std::fs::read_to_string(&path) {
            let trimmed = existing.trim();
            if is_valid_uuid_v4(trimmed) {
                return trimmed.to_string();
            }
        }

        // Generate fresh, write, return.
        let fresh = uuid::Uuid::new_v4().to_string();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&path, &fresh).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
        return fresh;
    }
    // No config dir resolvable — fall back to ephemeral id.
    uuid::Uuid::new_v4().to_string()
}

/// Best-effort hostname for a human-readable device label.
/// Returns `None` if the OS lookup fails or the result is empty / non-UTF-8.
///
/// Strips common mDNS / search-domain suffixes (`.local`, `.lan`, `.home`,
/// `.localdomain`) so a hostname like `example-host.local` uploads as
/// `example-host` — the suffix is noise from the user's POV and a
/// marginally more identifiable string in aggregate. The hostname itself
/// can still leak the user's real name; callers that want to opt out
/// should pass `--no-hostname` (TODO).
pub fn hostname_label() -> Option<String> {
    let raw = hostname::get().ok()?.into_string().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let stripped = strip_mdns_suffix(trimmed);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

fn strip_mdns_suffix(host: &str) -> &str {
    let lower = host.to_lowercase();
    for suffix in [".localdomain", ".local", ".lan", ".home"] {
        if lower.ends_with(suffix) {
            return &host[..host.len() - suffix.len()];
        }
    }
    host
}

/// Cheap UUID v4 sanity check — guards against accidentally re-using a
/// corrupted file from a previous bug or a manual edit gone wrong.
fn is_valid_uuid_v4(s: &str) -> bool {
    uuid::Uuid::parse_str(s)
        .map(|u| u.get_version_num() == 4)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_uuid_v4() {
        let v4 = uuid::Uuid::new_v4().to_string();
        assert!(is_valid_uuid_v4(&v4));
    }

    #[test]
    fn rejects_garbage() {
        assert!(!is_valid_uuid_v4(""));
        assert!(!is_valid_uuid_v4("not-a-uuid"));
        assert!(!is_valid_uuid_v4("xxxx"));
    }

    #[test]
    fn strips_mdns_suffix() {
        assert_eq!(strip_mdns_suffix("example-host.local"), "example-host");
        assert_eq!(strip_mdns_suffix("workstation.lan"), "workstation");
        assert_eq!(strip_mdns_suffix("router.home"), "router");
        assert_eq!(strip_mdns_suffix("server.localdomain"), "server");
        assert_eq!(strip_mdns_suffix("plain"), "plain");
        // Case-insensitive on the suffix
        assert_eq!(strip_mdns_suffix("foo.LOCAL"), "foo");
    }
}
