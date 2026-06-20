//! Per-domain browser token estimation (tiered, always labeled `tokens_estimated`).
//!
//! Web AI apps store conversations server-side. Local signals:
//!   1. IndexedDB message JSON (rare) → `cl100k_base`
//!   2. Chrome Local Storage session lists (sidebar threads, notebooks)
//!   3. Active browser hours × domain benchmark
//!
//! Every estimate includes [`crate::browser_data_audit::LocalDataAudit`] evidence
//! proving which local sources were exhausted (schema v6).

use crate::browser_data_audit::{probe_and_select, LocalDataAudit};
use crate::platform;
use crate::types::TokenUsage;
use rusty_leveldb::{LdbIterator, Options, DB};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Cache LevelDB transcript text per folder for the duration of one process scan.
static TRANSCRIPT_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn cache_get(folder: &str) -> Option<String> {
    let guard = TRANSCRIPT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref()?.get(folder).cloned()
}

fn cache_set(folder: &str, text: String) {
    let mut guard = TRANSCRIPT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    if let Some(map) = guard.as_mut() {
        map.insert(folder.to_string(), text);
    }
}

#[cfg(test)]
pub fn clear_transcript_cache_for_tests() {
    let mut guard = TRANSCRIPT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(HashMap::new());
}

/// Outcome of attempting to estimate tokens for one AI web-app display name.
#[derive(Debug, Clone)]
pub struct DomainTokenResult {
    pub tokens: Option<TokenUsage>,
    pub tokens_estimated: bool,
    pub tokens_estimate_method: Option<String>,
    pub tokens_unavailable: bool,
    pub tokens_unavailable_reason: Option<String>,
    pub tokens_estimate_evidence: Option<LocalDataAudit>,
}

impl DomainTokenResult {
    pub fn unavailable(reason: impl Into<String>, evidence: Option<LocalDataAudit>) -> Self {
        Self {
            tokens: None,
            tokens_estimated: false,
            tokens_estimate_method: None,
            tokens_unavailable: true,
            tokens_unavailable_reason: Some(reason.into()),
            tokens_estimate_evidence: evidence,
        }
    }

    pub fn estimated(
        tokens: TokenUsage,
        method: impl Into<String>,
        evidence: LocalDataAudit,
    ) -> Self {
        Self {
            tokens: Some(tokens),
            tokens_estimated: true,
            tokens_estimate_method: Some(method.into()),
            tokens_unavailable: false,
            tokens_unavailable_reason: None,
            tokens_estimate_evidence: Some(evidence),
        }
    }
}

pub(crate) const HISTORY_ONLY_REASON: &str =
    "Only visit timestamps found in browser history; no local chat transcript storage detected";

/// Minimum total message characters before we trust an IndexedDB scrape.
pub(crate) const MIN_TRANSCRIPT_CHARS: usize = 200;

/// Tier-2: median tokens per locally listed conversation/thread/notebook.
pub(crate) const TOKENS_PER_CHATGPT_CONVERSATION: u64 = 35_000;
pub(crate) const TOKENS_PER_PERPLEXITY_THREAD: u64 = 12_000;
pub(crate) const TOKENS_PER_NOTEBOOKLM_NOTEBOOK: u64 = 80_000;

fn usage_from_total(total: u64) -> TokenUsage {
    let mut usage = TokenUsage {
        input_tokens: 0,
        output_tokens: total,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total: 0,
    };
    usage.compute_total();
    usage
}

/// Display names from [`super::providers::browser::AI_DOMAINS`] that the dashboard
/// surfaces as separate tools with hours but (today) zero tokens.
pub const DASHBOARD_HOURS_ONLY_TOOLS: &[&str] = &[
    "ChatGPT",
    "NotebookLM",
    "Gemini",
    "Perplexity",
    "Grok",
    "Lovable",
    "AI Studio",
];

/// Map dashboard display name → Chrome IndexedDB folder suffix (under `IndexedDB/`).
pub(crate) fn chrome_idb_folder(display_name: &str) -> Option<&'static str> {
    match display_name {
        "ChatGPT" => Some("https_chatgpt.com_0.indexeddb.leveldb"),
        "Lovable" => Some("https_lovable.dev_0.indexeddb.leveldb"),
        "Grok" => Some("https_x.com_0.indexeddb.leveldb"),
        _ => None,
    }
}

/// Attempt token estimation for a browser-history domain row (tiered + audited).
pub fn estimate_domain_tokens(display_name: &str, hours: f64) -> DomainTokenResult {
    let selection = probe_and_select(display_name, hours);

    if selection.unavailable {
        return DomainTokenResult::unavailable(
            selection
                .unavailable_reason
                .unwrap_or_else(|| HISTORY_ONLY_REASON.to_string()),
            Some(selection.audit),
        );
    }

    DomainTokenResult::estimated(
        usage_from_total(selection.audit.winner_tokens),
        selection.method.unwrap_or_default(),
        selection.audit,
    )
}

/// Collect Chrome IndexedDB paths for every installed Chromium profile.
pub fn chrome_indexeddb_paths(folder: &str) -> Vec<PathBuf> {
    let home = platform::home_dir();
    let mut paths = Vec::new();

    let chrome_base = home.join("Library/Application Support/Google/Chrome");
    if chrome_base.exists() {
        collect_profile_idb(&chrome_base, folder, &mut paths);
    }

    for browser in [
        "Arc/User Data",
        "BraveSoftware/Brave-Browser",
        "Microsoft Edge",
        "Dia/User Data",
    ] {
        let base = home.join("Library/Application Support").join(browser);
        if base.exists() {
            collect_profile_idb(&base, folder, &mut paths);
        }
    }

    paths
}

fn collect_profile_idb(base: &Path, folder: &str, out: &mut Vec<PathBuf>) {
    let default = base.join("Default/IndexedDB").join(folder);
    if default.is_dir() {
        out.push(default);
    }
    if let Ok(entries) = std::fs::read_dir(base.join("IndexedDB")) {
        let _ = entries;
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("Profile ") {
                let p = entry.path().join("IndexedDB").join(folder);
                if p.is_dir() {
                    out.push(p);
                }
            }
        }
    }
}

/// Read all LevelDB keys/values from every matching Chrome IndexedDB folder and
/// pull chat text only from JSON message fields. Results are cached per folder.
pub(crate) fn collect_transcript_texts_for_folder(folder: &str) -> Vec<String> {
    if let Some(cached) = cache_get(folder) {
        if cached.is_empty() {
            return Vec::new();
        }
        return cached.split('\n').map(str::to_string).collect();
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut texts: Vec<String> = Vec::new();

    for path in chrome_indexeddb_paths(folder) {
        extract_texts_from_leveldb(&path, &mut seen, &mut texts);
    }

    let joined = texts.join("\n");
    cache_set(folder, joined);
    texts
}

fn extract_texts_from_leveldb(path: &Path, seen: &mut HashSet<String>, texts: &mut Vec<String>) {
    let opts = Options {
        create_if_missing: false,
        ..Options::default()
    };

    let Ok(mut db) = DB::open(path, opts) else {
        return;
    };

    let Ok(mut iter) = db.new_iter() else {
        return;
    };

    while let Some((key, value)) = iter.next() {
        collect_text_from_bytes(&key, seen, texts);
        collect_text_from_bytes(&value, seen, texts);
    }
}

fn collect_text_from_bytes(bytes: &[u8], seen: &mut HashSet<String>, texts: &mut Vec<String>) {
    let raw = String::from_utf8_lossy(bytes);
    for json_str in find_json_objects(&raw) {
        if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
            collect_json_text(&value, false, seen, texts);
        }
    }
}

/// Reject PEM/base64 blobs and non-message noise that previously inflated counts.
fn is_plausible_message_text(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 12 {
        return false;
    }
    if t.contains("BEGIN CERTIFICATE") || t.contains("BEGIN PRIVATE KEY") {
        return false;
    }
    if t.contains("MIGHAg") || t.contains("MIIBF") {
        return false;
    }
    if t.chars().filter(|c| *c == '\u{FFFD}').count() > 2 {
        return false;
    }

    let alnum = t.chars().filter(|c| c.is_alphanumeric()).count();
    let base64ish = t
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .count();
    if t.len() >= 40 && base64ish as f64 / t.len() as f64 > 0.92 && alnum > 30 {
        return false;
    }

    let letters = t.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 8 {
        return false;
    }
    if !t.contains(' ') && letters < 24 {
        return false;
    }

    true
}

fn is_message_container_key(key: &str) -> bool {
    matches!(
        key,
        "messages"
            | "message"
            | "conversation"
            | "conversations"
            | "chat"
            | "mapping"
            | "items"
            | "history"
            | "thread"
            | "turns"
    )
}

pub(crate) fn transcript_quality_failure(
    hours: f64,
    texts: &[String],
    token_count: u64,
) -> Option<String> {
    let total_chars: usize = texts.iter().map(|s| s.len()).sum();
    if total_chars < MIN_TRANSCRIPT_CHARS {
        return Some(format!(
            "{HISTORY_ONLY_REASON} (only {total_chars} chars of local message text — likely metadata, not full transcripts)"
        ));
    }
    if hours >= 0.5 {
        let min_expected = ((hours * 1_000_f64) as u64).max(500);
        if token_count < min_expected {
            return Some(format!(
                "Local transcript cache too sparse (~{token_count} est. tokens vs {hours:.1}h active) — falling through to session/hours tiers"
            ));
        }
    }
    None
}

fn find_json_objects(s: &str) -> Vec<String> {
    let mut results = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0;
        let mut closed_at = None;
        for (offset, &byte) in bytes[i..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        closed_at = Some(i + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = closed_at {
            if let Ok(fragment) = std::str::from_utf8(&bytes[start..=end]) {
                if fragment.len() >= 20 {
                    results.push(fragment.to_string());
                }
            }
            i = end + 1;
        } else {
            break;
        }
    }
    results
}

fn collect_json_text(
    value: &Value,
    in_message_context: bool,
    seen: &mut HashSet<String>,
    texts: &mut Vec<String>,
) {
    match value {
        Value::String(s) => {
            if in_message_context && is_plausible_message_text(s) && seen.insert(s.to_string()) {
                texts.push(s.clone());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_json_text(item, in_message_context, seen, texts);
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                let child_context =
                    in_message_context || is_message_container_key(key) || is_message_field(key);
                if is_message_field(key) {
                    if let Some(s) = val.as_str() {
                        if is_plausible_message_text(s) && seen.insert(s.to_string()) {
                            texts.push(s.to_string());
                        }
                    } else {
                        collect_json_text(val, true, seen, texts);
                    }
                } else if child_context {
                    collect_json_text(val, true, seen, texts);
                }
            }
        }
        _ => {}
    }
}

fn is_message_field(key: &str) -> bool {
    matches!(
        key,
        "text"
            | "content"
            | "message"
            | "body"
            | "markdown"
            | "richText"
            | "parts"
            | "value"
            | "prompt"
    )
}

/// Merge token fields into a domain metadata JSON object.
pub fn enrich_domain_entry(
    mut entry: serde_json::Value,
    display_name: &str,
    hours: f64,
) -> serde_json::Value {
    let token_result = estimate_domain_tokens(display_name, hours);

    if let Some(obj) = entry.as_object_mut() {
        if let Some(tokens) = token_result.tokens {
            obj.insert(
                "tokens".to_string(),
                serde_json::to_value(tokens).unwrap_or(Value::Null),
            );
        }
        if token_result.tokens_estimated {
            obj.insert("tokens_estimated".to_string(), Value::Bool(true));
        }
        if let Some(method) = token_result.tokens_estimate_method {
            obj.insert("tokens_estimate_method".to_string(), Value::String(method));
        }
        if token_result.tokens_unavailable {
            obj.insert("tokens_unavailable".to_string(), Value::Bool(true));
        }
        if let Some(reason) = token_result.tokens_unavailable_reason {
            obj.insert(
                "tokens_unavailable_reason".to_string(),
                Value::String(reason),
            );
        }
        if let Some(evidence) = token_result.tokens_estimate_evidence {
            obj.insert(
                "tokens_estimate_evidence".to_string(),
                evidence.to_json_value(),
            );
        }
    }

    entry
}

/// Sum estimated tokens across all domain entries in a browser metadata map.
pub fn rollup_domain_estimates(
    domains: &std::collections::HashMap<String, Value>,
) -> Option<TokenUsage> {
    let mut total = TokenUsage::default();
    let mut any = false;

    for value in domains.values() {
        if value.get("tokens_estimated").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let Some(tokens_val) = value.get("tokens") else {
            continue;
        };
        if let Ok(partial) = serde_json::from_value::<TokenUsage>(tokens_val.clone()) {
            total.merge(&partial);
            any = true;
        }
    }

    if any {
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_web_domains_estimate_from_hours_tier() {
        for (name, hours, min_tokens) in [
            ("Gemini", 0.8, 700_000),
            ("Perplexity", 0.6, 400_000),
            ("NotebookLM", 1.7, 800_000),
        ] {
            let r = estimate_domain_tokens(name, hours);
            assert!(r.tokens_estimated, "{name} should be estimated");
            let total = r.tokens.as_ref().map(|t| t.total).unwrap_or(0);
            assert!(total >= min_tokens, "{name}: {total} < {min_tokens}");
            let evidence = r.tokens_estimate_evidence.expect("{name} needs evidence");
            assert!(!evidence.accurate_count_possible);
            assert!(evidence.winner_tier >= 2.0 || evidence.winner_tier == 1.5);
        }
        let ai_studio = estimate_domain_tokens("AI Studio", 0.1);
        assert!(ai_studio.tokens_unavailable);
    }

    #[test]
    fn enrich_domain_entry_adds_estimate_for_gemini() {
        let entry = serde_json::json!({ "visits": 10, "hours": 0.8 });
        let enriched = enrich_domain_entry(entry, "Gemini", 0.8);
        assert_eq!(
            enriched.get("tokens_estimated").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(enriched.get("tokens_unavailable").is_none());
        assert!(enriched.get("tokens_estimate_evidence").is_some());
    }

    #[test]
    fn collect_json_text_finds_message_content() {
        let mut seen = HashSet::new();
        let mut texts = Vec::new();
        let json: Value = serde_json::json!({
            "messages": [
                {"role": "user", "content": "This is a long enough user message for counting"},
                {"role": "assistant", "content": "This is a long enough assistant reply for counting"}
            ]
        });
        collect_json_text(&json, false, &mut seen, &mut texts);
        assert_eq!(texts.len(), 2);
    }

    #[test]
    fn pem_certificate_bytes_are_not_counted_as_messages() {
        let pem = "-----BEGIN CERTIFICATE-----\nMIIBFzCBvaADAgECAgkA3rb310L7ioAwCgYIKoZIzj0EAwIwETEPMA0GA1UEAwwG\nV2ViUlRDMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQ";
        assert!(!is_plausible_message_text(pem));

        let mut seen = HashSet::new();
        let mut texts = Vec::new();
        collect_text_from_bytes(pem.as_bytes(), &mut seen, &mut texts);
        assert!(
            texts.is_empty(),
            "PEM blobs must not produce transcript text"
        );
    }

    #[test]
    fn transcript_quality_gate_rejects_sparse_cache_for_many_hours() {
        let texts = vec!["short msg".to_string()];
        let reason = transcript_quality_failure(4.8, &texts, 100).expect("should fail gate");
        assert!(reason.contains("sparse") || reason.contains("metadata"));
    }

    #[test]
    fn chatgpt_web_estimates_from_tier2_or_tier3_not_pem_garbage() {
        clear_transcript_cache_for_tests();
        let r = estimate_domain_tokens("ChatGPT", 4.8);
        assert!(
            r.tokens_estimated,
            "ChatGPT web should produce tiered estimate"
        );
        let total = r.tokens.as_ref().map(|t| t.total).unwrap_or(0);
        assert!(total > 1_000_000, "ChatGPT estimate too low: {total}");
        let method = r.tokens_estimate_method.unwrap_or_default();
        assert!(!method.contains("BEGIN CERTIFICATE"));
        let evidence = r.tokens_estimate_evidence.expect("evidence");
        assert!(!evidence.provider_logged_usage_found);
    }

    /// Every browser-backed dashboard tool with hours must expose either an
    /// estimated token count or an explicit unavailable marker — never bare 0.
    #[test]
    fn all_seven_browser_dashboard_tools_have_honest_token_state() {
        clear_transcript_cache_for_tests();

        let sample_hours: &[(&str, f64)] = &[
            ("ChatGPT", 4.8),
            ("NotebookLM", 1.7),
            ("Gemini", 0.8),
            ("Perplexity", 0.6),
            ("Grok", 0.3),
            ("Lovable", 0.2),
            ("AI Studio", 0.1),
        ];

        for &(name, hours) in sample_hours {
            let enriched = enrich_domain_entry(
                serde_json::json!({ "visits": 10, "hours": hours }),
                name,
                hours,
            );
            assert_honest_token_fields(&enriched, name);
        }
    }

    fn assert_honest_token_fields(entry: &Value, label: &str) {
        let hours = entry.get("hours").and_then(|v| v.as_f64()).unwrap_or(0.0);
        assert!(hours > 0.0, "{label}: expected positive hours");

        let estimated = entry.get("tokens_estimated").and_then(|v| v.as_bool()) == Some(true);
        let unavailable = entry.get("tokens_unavailable").and_then(|v| v.as_bool()) == Some(true);
        assert!(
            estimated || unavailable,
            "{label}: hours>0 must have tokens_estimated or tokens_unavailable, got {entry}"
        );

        if estimated {
            let total = entry
                .get("tokens")
                .and_then(|t| t.get("total"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            assert!(total > 0, "{label}: estimated entry needs tokens.total > 0");
            assert!(
                entry
                    .get("tokens_estimate_method")
                    .and_then(|v| v.as_str())
                    .is_some(),
                "{label}: estimated entry needs tokens_estimate_method"
            );
            assert!(
                entry.get("tokens_estimate_evidence").is_some(),
                "{label}: estimated entry needs tokens_estimate_evidence"
            );
        }

        if unavailable {
            assert!(
                entry
                    .get("tokens_unavailable_reason")
                    .and_then(|v| v.as_str())
                    .is_some(),
                "{label}: unavailable entry needs tokens_unavailable_reason"
            );
            assert!(
                entry.get("tokens").is_none(),
                "{label}: unavailable must omit tokens"
            );
            assert!(
                entry.get("tokens_estimate_evidence").is_some(),
                "{label}: unavailable should still include audit evidence"
            );
        }
    }

    #[test]
    fn idb_probed_domains_never_emit_bare_zero_tokens() {
        clear_transcript_cache_for_tests();

        for name in ["ChatGPT", "Lovable", "Grok"] {
            let r = estimate_domain_tokens(name, 1.0);
            assert!(
                r.tokens_estimated || r.tokens_unavailable,
                "{name}: IDB-probed domain must be estimated or unavailable"
            );
            if r.tokens_unavailable {
                assert!(r.tokens.is_none());
            }
        }
    }

    #[test]
    fn rollup_sums_estimated_domain_tokens() {
        let mut domains = std::collections::HashMap::new();
        domains.insert(
            "ChatGPT".to_string(),
            serde_json::json!({
                "hours": 1.0,
                "tokens": { "input_tokens": 0, "output_tokens": 100, "cache_read_tokens": 0, "cache_creation_tokens": 0, "total": 100 },
                "tokens_estimated": true
            }),
        );
        domains.insert(
            "Gemini".to_string(),
            serde_json::json!({
                "hours": 0.5,
                "tokens_unavailable": true
            }),
        );
        let rolled = rollup_domain_estimates(&domains).expect("rollup");
        assert_eq!(rolled.total, 100);
    }

    /// Manual: `cargo test --lib print_live_web_domain_estimates -- --ignored --nocapture`
    #[test]
    #[ignore = "manual: print live web domain token estimates"]
    fn print_live_web_domain_estimates() {
        use crate::browser_local_storage::collect_session_signals;

        clear_transcript_cache_for_tests();
        let rows: &[(&str, f64)] = &[
            ("ChatGPT", 4.85),
            ("Gemini", 0.81),
            ("Perplexity", 0.60),
            ("NotebookLM", 1.68),
            ("Grok", 0.33),
            ("Lovable", 0.22),
        ];
        for &(name, hours) in rows {
            let signals = collect_session_signals(name);
            let r = estimate_domain_tokens(name, hours);
            eprintln!("--- {name} ({hours}h) ---");
            eprintln!(
                "  LS signals: conv={} threads={} notebooks={} snippets={}",
                signals.conversations,
                signals.threads,
                signals.notebooks,
                signals.transcript_texts.len()
            );
            if let Some(t) = &r.tokens {
                eprintln!("  tokens: ~{}", t.total);
            }
            if let Some(m) = &r.tokens_estimate_method {
                eprintln!("  method: {m}");
            }
            if let Some(e) = &r.tokens_estimate_evidence {
                eprintln!(
                    "  evidence: tier={} reason={} accurate={}",
                    e.winner_tier, e.winner_reason, e.accurate_count_possible
                );
            }
            if r.tokens_unavailable {
                eprintln!("  UNAVAILABLE: {:?}", r.tokens_unavailable_reason);
            }
        }
    }
}
