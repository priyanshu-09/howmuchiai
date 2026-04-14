use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::time_util;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;
use std::path::PathBuf;

const AI_DOMAINS: &[(&str, &str)] = &[
    ("claude.ai", "Claude.ai"),
    ("chatgpt.com", "ChatGPT"),
    ("chat.openai.com", "ChatGPT"),
    ("gemini.google.com", "Gemini"),
    ("perplexity.ai", "Perplexity"),
    ("poe.com", "Poe"),
    ("phind.com", "Phind"),
    ("copilot.microsoft.com", "Copilot"),
    ("deepseek.com", "DeepSeek"),
    ("groq.com", "Groq"),
    ("huggingface.co/chat", "HuggingFace"),
    ("aistudio.google.com", "AI Studio"),
    ("you.com", "You.com"),
    ("labs.google", "Google Labs"),
];

/// Build SQL CASE expression that returns the display name for matched domains.
/// This avoids selecting full URLs into Rust memory (security: URLs may contain tokens/PII).
fn domain_case_expr(url_col: &str) -> String {
    let cases: Vec<String> = AI_DOMAINS
        .iter()
        .map(|(domain, name)| format!("WHEN {} LIKE '%{}%' THEN '{}'", url_col, domain, name))
        .collect();
    format!("CASE {} END", cases.join(" "))
}

/// Build the SQL LIKE conditions for AI domains
fn domain_like_conditions(url_col: &str) -> String {
    AI_DOMAINS
        .iter()
        .map(|(domain, _)| format!("{} LIKE '%{}%'", url_col, domain))
        .collect::<Vec<_>>()
        .join(" OR ")
}

// ---------------------------------------------------------------------------
// Chromium-based browser scanning
// ---------------------------------------------------------------------------

fn scan_chromium_history(
    paths: &[PathBuf],
    provider_name: &str,
) -> Result<ProviderResult, ScanError> {
    if paths.is_empty() {
        return Err(ScanError::NotFound(format!(
            "No {} history databases found",
            provider_name
        )));
    }

    let conditions = domain_like_conditions("u.url");
    let case_expr = domain_case_expr("u.url");
    // Select only the domain display name — never the full URL (may contain tokens/PII)
    let query = format!(
        "SELECT {}, v.visit_time, v.visit_duration \
         FROM visits v JOIN urls u ON v.url = u.id \
         WHERE ({}) \
         AND u.url NOT LIKE '%/api/%' \
         AND u.url NOT LIKE '%/auth/%' \
         AND u.url NOT LIKE '%/callback%' \
         AND u.url NOT LIKE '%/oauth%' \
         ORDER BY v.visit_time",
        case_expr, conditions
    );

    let mut all_timestamps: Vec<i64> = Vec::new();
    let mut domain_visits: HashMap<String, Vec<i64>> = HashMap::new();
    let mut first_seen: Option<i64> = None;
    let mut last_seen: Option<i64> = None;

    for path in paths {
        let db = match SafeSqlite::open(path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut stmt = match db.conn().prepare(&query) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rows = match stmt.query_map([], |row| {
            let domain_name: String = row.get(0)?;
            let visit_time: i64 = row.get(1)?;
            let _visit_duration: i64 = row.get(2)?;
            Ok((domain_name, visit_time))
        }) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for row in rows {
            let (domain_name, visit_time) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let unix_ts = time_util::chrome_time_to_unix(visit_time);

            // Skip timestamps before 2020 or in the far future (bad data)
            if unix_ts < 1_577_836_800 || unix_ts > 2_000_000_000 {
                continue;
            }

            all_timestamps.push(unix_ts);
            first_seen = Some(first_seen.map_or(unix_ts, |fs: i64| fs.min(unix_ts)));
            last_seen = Some(last_seen.map_or(unix_ts, |ls: i64| ls.max(unix_ts)));

            domain_visits.entry(domain_name).or_default().push(unix_ts);
        }
    }

    if all_timestamps.is_empty() {
        return Err(ScanError::NotFound(format!(
            "No AI site visits found in {}",
            provider_name
        )));
    }

    all_timestamps.sort_unstable();
    let total_visits = all_timestamps.len() as u64;
    let total_hours = time_util::active_hours_from_timestamps(&all_timestamps, 1800);

    // Build per-domain metadata
    let mut domain_breakdown: HashMap<String, serde_json::Value> = HashMap::new();
    for (domain_name, timestamps) in &domain_visits {
        let mut sorted = timestamps.clone();
        sorted.sort_unstable();
        let domain_hours = time_util::active_hours_from_timestamps(&sorted, 1800);
        let visit_count = sorted.len() as u64;

        domain_breakdown.insert(
            domain_name.clone(),
            serde_json::json!({
                "visits": visit_count,
                "hours": (domain_hours * 100.0).round() / 100.0,
            }),
        );
    }

    let mut result = ProviderResult::new(provider_name);
    result.hours = Some(total_hours);
    result.visits = Some(total_visits);
    result.first_seen = first_seen;
    result.last_seen = last_seen;

    let mut metadata = HashMap::new();
    metadata.insert(
        "domains".to_string(),
        serde_json::to_value(&domain_breakdown).unwrap_or_default(),
    );
    result.metadata = Some(metadata);

    Ok(result)
}

// ---------------------------------------------------------------------------
// Safari scanning
// ---------------------------------------------------------------------------

fn scan_safari_history(path: &std::path::Path) -> Result<ProviderResult, ScanError> {
    let conditions = domain_like_conditions("hi.url");
    let case_expr = domain_case_expr("hi.url");

    // Select only domain display name — never the full URL
    let query = format!(
        "SELECT {}, hv.visit_time \
         FROM history_visits hv \
         JOIN history_items hi ON hv.history_item = hi.id \
         WHERE ({}) \
         AND hi.url NOT LIKE '%/api/%' \
         AND hi.url NOT LIKE '%/auth/%' \
         AND hi.url NOT LIKE '%/callback%' \
         AND hi.url NOT LIKE '%/oauth%' \
         ORDER BY hv.visit_time",
        case_expr, conditions
    );

    let db = SafeSqlite::open(path)?;
    let mut stmt = db.conn().prepare(&query)?;

    let mut all_timestamps: Vec<i64> = Vec::new();
    let mut domain_visits: HashMap<String, Vec<i64>> = HashMap::new();
    let mut first_seen: Option<i64> = None;
    let mut last_seen: Option<i64> = None;

    let rows = stmt.query_map([], |row| {
        let domain_name: String = row.get(0)?;
        let visit_time: f64 = row.get(1)?;
        Ok((domain_name, visit_time))
    })?;

    for row in rows {
        let (domain_name, visit_time) = match row {
            Ok(r) => r,
            Err(_) => continue,
        };

        let unix_ts = time_util::safari_time_to_unix(visit_time);

        if unix_ts < 1_577_836_800 || unix_ts > 2_000_000_000 {
            continue;
        }

        all_timestamps.push(unix_ts);
        first_seen = Some(first_seen.map_or(unix_ts, |fs: i64| fs.min(unix_ts)));
        last_seen = Some(last_seen.map_or(unix_ts, |ls: i64| ls.max(unix_ts)));

        domain_visits.entry(domain_name).or_default().push(unix_ts);
    }

    if all_timestamps.is_empty() {
        return Err(ScanError::NotFound(
            "No AI site visits found in Safari".into(),
        ));
    }

    all_timestamps.sort_unstable();
    let total_visits = all_timestamps.len() as u64;
    let total_hours = time_util::active_hours_from_timestamps(&all_timestamps, 1800);

    let mut domain_breakdown: HashMap<String, serde_json::Value> = HashMap::new();
    for (domain_name, timestamps) in &domain_visits {
        let mut sorted = timestamps.clone();
        sorted.sort_unstable();
        let domain_hours = time_util::active_hours_from_timestamps(&sorted, 1800);
        let visit_count = sorted.len() as u64;

        domain_breakdown.insert(
            domain_name.clone(),
            serde_json::json!({
                "visits": visit_count,
                "hours": (domain_hours * 100.0).round() / 100.0,
            }),
        );
    }

    let mut result = ProviderResult::new("Safari");
    result.hours = Some(total_hours);
    result.visits = Some(total_visits);
    result.first_seen = first_seen;
    result.last_seen = last_seen;

    let mut metadata = HashMap::new();
    metadata.insert(
        "domains".to_string(),
        serde_json::to_value(&domain_breakdown).unwrap_or_default(),
    );
    result.metadata = Some(metadata);

    Ok(result)
}

// ---------------------------------------------------------------------------
// Firefox scanning
// ---------------------------------------------------------------------------

fn scan_firefox_history(paths: &[PathBuf]) -> Result<ProviderResult, ScanError> {
    if paths.is_empty() {
        return Err(ScanError::NotFound(
            "No Firefox history databases found".into(),
        ));
    }

    let conditions = domain_like_conditions("p.url");
    let case_expr = domain_case_expr("p.url");

    // Select only domain display name — never the full URL
    let query = format!(
        "SELECT {}, h.visit_date \
         FROM moz_historyvisits h \
         JOIN moz_places p ON h.place_id = p.id \
         WHERE ({}) \
         AND p.url NOT LIKE '%/api/%' \
         AND p.url NOT LIKE '%/auth/%' \
         AND p.url NOT LIKE '%/callback%' \
         AND p.url NOT LIKE '%/oauth%' \
         ORDER BY h.visit_date",
        case_expr, conditions
    );

    let mut all_timestamps: Vec<i64> = Vec::new();
    let mut domain_visits: HashMap<String, Vec<i64>> = HashMap::new();
    let mut first_seen: Option<i64> = None;
    let mut last_seen: Option<i64> = None;

    for path in paths {
        let db = match SafeSqlite::open(path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut stmt = match db.conn().prepare(&query) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let rows = match stmt.query_map([], |row| {
            let domain_name: String = row.get(0)?;
            let visit_date: i64 = row.get(1)?;
            Ok((domain_name, visit_date))
        }) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for row in rows {
            let (domain_name, visit_date) = match row {
                Ok(r) => r,
                Err(_) => continue,
            };

            let unix_ts = time_util::firefox_time_to_unix(visit_date);

            if unix_ts < 1_577_836_800 || unix_ts > 2_000_000_000 {
                continue;
            }

            all_timestamps.push(unix_ts);
            first_seen = Some(first_seen.map_or(unix_ts, |fs: i64| fs.min(unix_ts)));
            last_seen = Some(last_seen.map_or(unix_ts, |ls: i64| ls.max(unix_ts)));

            domain_visits.entry(domain_name).or_default().push(unix_ts);
        }
    }

    if all_timestamps.is_empty() {
        return Err(ScanError::NotFound(
            "No AI site visits found in Firefox".into(),
        ));
    }

    all_timestamps.sort_unstable();
    let total_visits = all_timestamps.len() as u64;
    let total_hours = time_util::active_hours_from_timestamps(&all_timestamps, 1800);

    let mut domain_breakdown: HashMap<String, serde_json::Value> = HashMap::new();
    for (domain_name, timestamps) in &domain_visits {
        let mut sorted = timestamps.clone();
        sorted.sort_unstable();
        let domain_hours = time_util::active_hours_from_timestamps(&sorted, 1800);
        let visit_count = sorted.len() as u64;

        domain_breakdown.insert(
            domain_name.clone(),
            serde_json::json!({
                "visits": visit_count,
                "hours": (domain_hours * 100.0).round() / 100.0,
            }),
        );
    }

    let mut result = ProviderResult::new("Firefox");
    result.hours = Some(total_hours);
    result.visits = Some(total_visits);
    result.first_seen = first_seen;
    result.last_seen = last_seen;

    let mut metadata = HashMap::new();
    metadata.insert(
        "domains".to_string(),
        serde_json::to_value(&domain_breakdown).unwrap_or_default(),
    );
    result.metadata = Some(metadata);

    Ok(result)
}

// ---------------------------------------------------------------------------
// Provider implementations
// ---------------------------------------------------------------------------

pub struct ChromeProvider;

impl Provider for ChromeProvider {
    fn name(&self) -> &'static str {
        "chrome_browser"
    }

    fn display_name(&self) -> &'static str {
        "Chrome Browser"
    }

    fn is_available(&self) -> bool {
        !platform::chrome_history_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let paths = platform::chrome_history_paths();
        scan_chromium_history(&paths, "Chrome")
    }
}

pub struct ArcProvider;

impl Provider for ArcProvider {
    fn name(&self) -> &'static str {
        "arc_browser"
    }

    fn display_name(&self) -> &'static str {
        "Arc Browser"
    }

    fn is_available(&self) -> bool {
        !platform::arc_history_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let paths = platform::arc_history_paths();
        scan_chromium_history(&paths, "Arc")
    }
}

pub struct BraveProvider;

impl Provider for BraveProvider {
    fn name(&self) -> &'static str {
        "brave_browser"
    }

    fn display_name(&self) -> &'static str {
        "Brave Browser"
    }

    fn is_available(&self) -> bool {
        !platform::brave_history_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let paths = platform::brave_history_paths();
        scan_chromium_history(&paths, "Brave")
    }
}

pub struct EdgeProvider;

impl Provider for EdgeProvider {
    fn name(&self) -> &'static str {
        "edge_browser"
    }

    fn display_name(&self) -> &'static str {
        "Edge Browser"
    }

    fn is_available(&self) -> bool {
        !platform::edge_history_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let paths = platform::edge_history_paths();
        scan_chromium_history(&paths, "Edge")
    }
}

pub struct SafariProvider;

impl Provider for SafariProvider {
    fn name(&self) -> &'static str {
        "safari_browser"
    }

    fn display_name(&self) -> &'static str {
        "Safari Browser"
    }

    fn is_available(&self) -> bool {
        platform::safari_history_path().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let path = platform::safari_history_path()
            .ok_or_else(|| ScanError::NotFound("Safari history not found".into()))?;
        scan_safari_history(&path)
    }
}

pub struct FirefoxProvider;

impl Provider for FirefoxProvider {
    fn name(&self) -> &'static str {
        "firefox_browser"
    }

    fn display_name(&self) -> &'static str {
        "Firefox Browser"
    }

    fn is_available(&self) -> bool {
        !platform::firefox_history_paths().is_empty()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let paths = platform::firefox_history_paths();
        scan_firefox_history(&paths)
    }
}
