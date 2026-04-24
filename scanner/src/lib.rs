pub mod platform;
pub mod providers;
pub mod sqlite_util;
pub mod time_util;
pub mod types;

use providers::{all_providers, detection};
use rayon::prelude::*;
use std::collections::HashMap;
use types::{ScanResult, Totals};

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

    ScanResult {
        scanned_at: chrono::Utc::now().to_rfc3339(),
        schema_version: 2,
        platform: platform::detect_platform().to_string(),
        scan_duration_ms: elapsed.as_millis() as u64,
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
            parts.push(format!("{} tokens", format_compact(t.total)));
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
    let mut sessions = 0_u64;
    let mut visits = 0_u64;
    let mut invocations = 0_u64;

    for result in sources.values() {
        if let Some(h) = result.hours {
            hours += h;
        }
        if let Some(ref t) = result.tokens {
            tokens = tokens.saturating_add(t.total);
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
        sessions,
        visits,
        invocations,
    }
}
