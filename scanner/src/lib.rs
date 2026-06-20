pub mod browser_data_audit;
pub mod browser_local_storage;
pub mod browser_transcripts;
pub mod device_id;
pub mod platform;
pub mod providers;
pub mod sqlite_util;
pub mod time_util;
pub mod token_estimate;
pub mod types;

use providers::{all_providers, detection};
use rayon::prelude::*;
use std::collections::HashMap;
use types::{ScanResult, Totals, SCHEMA_VERSION};

/// Run all providers in parallel, collect results, compute totals.
pub fn run_scan() -> ScanResult {
    let start = std::time::Instant::now();
    let providers = all_providers();

    // Scan all available providers in parallel
    let results: Vec<_> = providers
        .par_iter()
        .filter(|p| p.is_available())
        .map(|p| {
            let name = p.name().to_string();
            let display = p.display_name();
            match p.scan() {
                Ok(ref result) => {
                    let stats = format_provider_stats(result);
                    if stats.is_empty() {
                        eprintln!("  \x1b[32m✓\x1b[0m {}", display);
                    } else {
                        eprintln!("  \x1b[32m✓\x1b[0m {} \x1b[90m— {}\x1b[0m", display, stats);
                    }
                    (name, Some(result.clone()))
                }
                Err(e) => {
                    eprintln!("  \x1b[31m✗\x1b[0m {} \x1b[90m— {}\x1b[0m", display, e);
                    (name, None)
                }
            }
        })
        .collect();

    // Print skipped providers
    for p in &providers {
        if !p.is_available() {
            eprintln!("  \x1b[90m- {} (not found)\x1b[0m", p.display_name());
        }
    }

    let mut sources = HashMap::new();
    for (name, result) in results {
        if let Some(r) = result {
            sources.insert(name, r);
        }
    }

    // Detect tools that are installed but lack detailed usage data
    let detected_tools = detection::detect_tools();

    // Compute totals
    let totals = compute_totals(&sources);

    let elapsed = start.elapsed();

    // Load (or first-run create) the per-machine UUID + best-effort hostname
    // label. Both are stamped onto the scan so the web dashboard can
    // aggregate multiple devices under one account.
    let device_id = device_id::load_or_create();
    let device_label = device_id::hostname_label();

    ScanResult {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        schema_version: SCHEMA_VERSION,
        platform: platform::detect_platform().to_string(),
        scan_duration_ms: elapsed.as_millis() as u64,
        device_id,
        device_label,
        sources,
        totals,
        detected_tools,
    }
}

fn format_provider_stats(result: &types::ProviderResult) -> String {
    let mut parts = Vec::new();
    if let Some(h) = result.hours {
        if h >= 0.1 {
            parts.push(format!("{:.1}h", h));
        }
    }
    if let Some(ref t) = result.tokens {
        if t.total > 0 {
            let label = if result.tokens_estimated {
                format!("~{} est. tokens", format_compact(t.total))
            } else {
                format!("{} tokens", format_compact(t.total))
            };
            parts.push(label);
        }
    }
    if let Some(s) = result.sessions {
        if s > 0 {
            parts.push(format!("{} sessions", s));
        }
    }
    if let Some(v) = result.visits {
        if v > 0 {
            parts.push(format!("{} visits", v));
        }
    }
    if let Some(i) = result.invocations {
        if i > 0 {
            parts.push(format!("{} invocations", i));
        }
    }
    parts.join(", ")
}

fn format_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn compute_totals(sources: &HashMap<String, types::ProviderResult>) -> Totals {
    let mut hours = 0.0_f64;
    let mut tokens = 0_u64;
    let mut estimated_tokens = 0_u64;
    let mut sessions = 0_u64;
    let mut visits = 0_u64;
    let mut invocations = 0_u64;

    for result in sources.values() {
        if let Some(h) = result.hours {
            hours += h;
        }
        if let Some(ref t) = result.tokens {
            if result.tokens_estimated {
                estimated_tokens = estimated_tokens.saturating_add(t.total);
            } else {
                tokens = tokens.saturating_add(t.total);
            }
        }
        if let Some(s) = result.sessions {
            sessions = sessions.saturating_add(s);
        }
        if let Some(v) = result.visits {
            visits = visits.saturating_add(v);
        }
        if let Some(i) = result.invocations {
            invocations = invocations.saturating_add(i);
        }
    }

    Totals {
        hours,
        tokens,
        estimated_tokens,
        sessions,
        visits,
        invocations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{ProviderResult, TokenUsage};

    #[test]
    fn compute_totals_excludes_estimated_provider_tokens() {
        let mut real = ProviderResult::new("Real");
        let mut real_tokens = TokenUsage::default();
        real_tokens.output_tokens = 100;
        real_tokens.compute_total();
        real.tokens = Some(real_tokens);

        let mut est = ProviderResult::new("Cursor");
        let mut est_tokens = TokenUsage::default();
        est_tokens.output_tokens = 50_000;
        est_tokens.compute_total();
        est.tokens = Some(est_tokens);
        est.tokens_estimated = true;

        let mut sources = HashMap::new();
        sources.insert("real".into(), real);
        sources.insert("cursor".into(), est);

        let totals = compute_totals(&sources);
        assert_eq!(totals.tokens, 100);
        assert_eq!(totals.estimated_tokens, 50_000);
    }

    /// Schema v6 JSON shape for all 8 dashboard tools that previously showed
    /// hours with bare 0 tokens (Cursor + 7 browser web apps).
    #[test]
    fn eight_dashboard_tools_schema_v6_json_roundtrip() {
        use browser_transcripts::{enrich_domain_entry, DASHBOARD_HOURS_ONLY_TOOLS};
        use serde_json::Value;

        let browser_domains: Vec<(String, Value)> = DASHBOARD_HOURS_ONLY_TOOLS
            .iter()
            .enumerate()
            .map(|(i, &name)| {
                let hours = (i as f64 + 1.0) * 0.5;
                let entry = enrich_domain_entry(
                    serde_json::json!({ "visits": 50, "hours": hours }),
                    name,
                    hours,
                );
                (name.to_string(), entry)
            })
            .collect();

        let mut domain_map = serde_json::Map::new();
        for (name, entry) in browser_domains {
            domain_map.insert(name, entry);
        }

        let mut browser = ProviderResult::new("Chrome");
        browser.hours = Some(8.5);
        browser.visits = Some(500);
        let mut est_tokens = TokenUsage::default();
        est_tokens.output_tokens = 42_000;
        est_tokens.compute_total();
        browser.tokens = Some(est_tokens);
        browser.tokens_estimated = true;
        browser.tokens_estimate_method =
            Some("cl100k_base over Chrome IndexedDB transcripts in metadata.domains".into());
        browser.metadata = Some(HashMap::from([(
            "domains".to_string(),
            Value::Object(domain_map),
        )]));

        let mut cursor = ProviderResult::new("Cursor");
        cursor.hours = Some(16.0);
        let mut cursor_tokens = TokenUsage::default();
        cursor_tokens.output_tokens = 82_000_000;
        cursor_tokens.compute_total();
        cursor.tokens = Some(cursor_tokens);
        cursor.tokens_estimated = true;
        cursor.tokens_estimate_method = Some("cl100k_base over Cursor bubble transcripts".into());

        let mut sources = HashMap::new();
        sources.insert("chrome_browser".into(), browser);
        sources.insert("cursor".into(), cursor);

        let totals = compute_totals(&sources);
        assert_eq!(totals.tokens, 0);
        assert!(totals.estimated_tokens > 0);

        let scan = ScanResult {
            scanned_at: "2026-06-20T00:00:00Z".into(),
            schema_version: SCHEMA_VERSION,
            platform: "macos".into(),
            scan_duration_ms: 1,
            device_id: "test-device".into(),
            device_label: Some("test".into()),
            sources,
            totals,
            detected_tools: vec![],
        };

        let json = serde_json::to_string(&scan).expect("serialize scan");
        let parsed: Value = serde_json::from_str(&json).expect("parse scan json");
        assert_eq!(
            parsed.get("schema_version").and_then(|v| v.as_u64()),
            Some(6)
        );

        let domains = parsed
            .pointer("/sources/chrome_browser/metadata/domains")
            .and_then(|v| v.as_object())
            .expect("browser domains");

        for tool in DASHBOARD_HOURS_ONLY_TOOLS {
            let entry = domains
                .get(*tool)
                .unwrap_or_else(|| panic!("missing {tool}"));
            let hours = entry.get("hours").and_then(|v| v.as_f64()).unwrap_or(0.0);
            assert!(hours > 0.0, "{tool} needs hours in fixture");
            let estimated = entry.get("tokens_estimated").and_then(|v| v.as_bool()) == Some(true);
            let unavailable =
                entry.get("tokens_unavailable").and_then(|v| v.as_bool()) == Some(true);
            assert!(
                estimated || unavailable,
                "{tool}: JSON must not imply bare 0 tokens when hours > 0"
            );
            if estimated {
                assert!(
                    entry.get("tokens_estimate_evidence").is_some(),
                    "{tool}: estimated domain needs tokens_estimate_evidence"
                );
            }
        }

        let cursor_src = parsed.pointer("/sources/cursor").expect("cursor source");
        assert_eq!(
            cursor_src.get("tokens_estimated").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(
            cursor_src
                .pointer("/tokens/total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                > 0
        );
    }

    #[test]
    fn compute_totals_includes_browser_estimated_tokens() {
        let mut browser = ProviderResult::new("Chrome");
        let mut est_tokens = TokenUsage::default();
        est_tokens.output_tokens = 1_200;
        est_tokens.compute_total();
        browser.tokens = Some(est_tokens);
        browser.tokens_estimated = true;

        let mut sources = HashMap::new();
        sources.insert("chrome_browser".into(), browser);

        let totals = compute_totals(&sources);
        assert_eq!(totals.tokens, 0);
        assert_eq!(totals.estimated_tokens, 1_200);
    }
}
