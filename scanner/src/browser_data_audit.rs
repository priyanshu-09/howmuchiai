//! Deterministic local-data exhaustion audit for browser web-tool token estimates.
//!
//! Probes every on-disk source the scanner can read, records tier candidates,
//! and selects the max() winner with machine-readable justification.

use crate::browser_local_storage::{chrome_local_storage_paths, collect_session_signals};
use crate::browser_transcripts::{
    chrome_idb_folder, chrome_indexeddb_paths, collect_transcript_texts_for_folder,
    transcript_quality_failure, HISTORY_ONLY_REASON, MIN_TRANSCRIPT_CHARS,
    TOKENS_PER_CHATGPT_CONVERSATION, TOKENS_PER_NOTEBOOKLM_NOTEBOOK, TOKENS_PER_PERPLEXITY_THREAD,
};
use crate::platform;
use crate::token_estimate::count_tokens;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Tier-3: conservative active-hours benchmarks (tokens per browser-active hour).
pub fn tokens_per_hour_benchmark(display_name: &str) -> u64 {
    match display_name {
        "ChatGPT" => 1_200_000,
        "Gemini" => 900_000,
        "Perplexity" => 700_000,
        "NotebookLM" => 500_000,
        _ => 600_000,
    }
}

fn is_priority_web_domain(display_name: &str) -> bool {
    matches!(
        display_name,
        "ChatGPT" | "Gemini" | "Perplexity" | "NotebookLM"
    )
}

fn origin_needles(display_name: &str) -> &'static [&'static str] {
    match display_name {
        "ChatGPT" => &["chatgpt.com", "chat.openai.com"],
        "Gemini" => &["gemini.google.com", "bard.google.com"],
        "Perplexity" => &["perplexity.ai"],
        "NotebookLM" => &["notebooklm.google.com", "notebooklm"],
        _ => &[],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WinnerReason {
    Tier1IdbTranscripts,
    Tier1_5SnippetText,
    Tier2SessionMetadata,
    Tier3ActiveHours,
    Tier3HoursFloorExceededTier2,
    Unavailable,
}

impl WinnerReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tier1IdbTranscripts => "tier1_idb_transcripts",
            Self::Tier1_5SnippetText => "tier1_5_snippet_text",
            Self::Tier2SessionMetadata => "tier2_session_metadata",
            Self::Tier3ActiveHours => "tier3_active_hours",
            Self::Tier3HoursFloorExceededTier2 => "tier3_hours_floor_exceeded_tier2",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn human_detail(&self, display_name: &str, hours: f64, tier2_detail: &str) -> String {
        match self {
            Self::Tier1IdbTranscripts => {
                format!("IndexedDB message JSON for {display_name}")
            }
            Self::Tier1_5SnippetText => {
                format!("Local Storage snippet/title text for {display_name}")
            }
            Self::Tier2SessionMetadata => tier2_detail.to_string(),
            Self::Tier3ActiveHours => format!(
                "{hours:.1}h active browser time × ~{} tokens/h ({display_name} stores chats server-side; not provider-logged)",
                tokens_per_hour_benchmark(display_name)
            ),
            Self::Tier3HoursFloorExceededTier2 => format!(
                "{tier2_detail} with Tier-3 active-hours floor ({hours:.1}h × ~{} tokens/h)",
                tokens_per_hour_benchmark(display_name)
            ),
            Self::Unavailable => HISTORY_ONLY_REASON.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdbProbe {
    pub paths_scanned: u32,
    pub folders_found: u32,
    pub message_chars: usize,
    pub token_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalStorageProbe {
    pub profiles_scanned: u32,
    pub conversations: u64,
    pub threads: u64,
    pub notebooks: u64,
    pub snippet_chars: usize,
    pub snippet_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tier1Candidate {
    pub tokens: u64,
    pub message_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tier1_5Candidate {
    pub tokens: u64,
    pub snippet_chars: usize,
    pub snippet_count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tier2Candidate {
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tier3Candidate {
    pub tokens: u64,
    pub hours: f64,
    pub benchmark_tph: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierCandidates {
    #[serde(rename = "1")]
    pub tier1: Tier1Candidate,
    #[serde(rename = "1.5")]
    pub tier1_5: Tier1_5Candidate,
    #[serde(rename = "2")]
    pub tier2: Tier2Candidate,
    #[serde(rename = "3")]
    pub tier3: Tier3Candidate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SafariWebStorageProbe {
    pub paths_scanned: u32,
    pub message_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalDataAudit {
    pub sources_checked: Vec<String>,
    pub idb: IdbProbe,
    pub local_storage: LocalStorageProbe,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safari_web_storage: Option<SafariWebStorageProbe>,
    pub tier_candidates: TierCandidates,
    pub winner_tier: f64,
    pub winner_reason: String,
    pub winner_reason_detail: String,
    pub provider_logged_usage_found: bool,
    pub accurate_count_possible: bool,
    pub winner_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct AuditSelection {
    pub audit: LocalDataAudit,
    pub estimated: bool,
    pub unavailable: bool,
    pub unavailable_reason: Option<String>,
    pub method: Option<String>,
}

pub fn probe_and_select(display_name: &str, hours: f64) -> AuditSelection {
    let mut sources_checked = vec!["chrome_history".to_string()];

    let idb = probe_indexeddb(display_name, hours);
    if idb.paths_scanned > 0 {
        sources_checked.push("chrome_indexeddb".to_string());
    }

    let ls_signals = collect_session_signals(display_name);
    let ls_profiles = chrome_local_storage_paths().len() as u32;
    if ls_profiles > 0 && is_priority_web_domain(display_name) {
        sources_checked.push("chrome_local_storage".to_string());
    }

    let safari = probe_safari_web_storage(display_name);
    if is_priority_web_domain(display_name) || safari.paths_scanned > 0 {
        sources_checked.push("safari_web_storage".to_string());
    }

    let snippet_chars: usize = ls_signals.transcript_texts.iter().map(|s| s.len()).sum();
    let tier1_5_tokens = if ls_signals.transcript_texts.is_empty() {
        0
    } else {
        count_tokens(&ls_signals.transcript_texts.join("\n"))
    };

    let (tier2_tokens, tier2_sessions, tier2_kind, tier2_detail) =
        tier2_from_signals(display_name, &ls_signals);

    let benchmark_tph = tokens_per_hour_benchmark(display_name);
    let tier3_tokens = if hours > 0.0 {
        (hours * benchmark_tph as f64).max(500.0) as u64
    } else {
        0
    };

    let tier_candidates = TierCandidates {
        tier1: Tier1Candidate {
            tokens: idb.token_count,
            message_chars: idb.message_chars,
            rejected: idb.rejection_reason.clone(),
        },
        tier1_5: Tier1_5Candidate {
            tokens: tier1_5_tokens,
            snippet_chars,
            snippet_count: ls_signals.transcript_texts.len() as u32,
        },
        tier2: Tier2Candidate {
            tokens: tier2_tokens,
            sessions: tier2_sessions,
            session_kind: tier2_kind,
        },
        tier3: Tier3Candidate {
            tokens: tier3_tokens,
            hours,
            benchmark_tph,
        },
    };

    let local_storage = LocalStorageProbe {
        profiles_scanned: ls_profiles,
        conversations: ls_signals.conversations,
        threads: ls_signals.threads,
        notebooks: ls_signals.notebooks,
        snippet_chars,
        snippet_tokens: tier1_5_tokens,
    };

    if hours <= 0.0 {
        return unavailable_selection(
            display_name,
            sources_checked,
            idb,
            local_storage,
            safari,
            tier_candidates,
            "No active hours recorded for this domain",
        );
    }

    // Tier 1 wins only when IDB passed quality gate with tokens.
    if idb.token_count > 0 && idb.rejection_reason.is_none() {
        let reason = WinnerReason::Tier1IdbTranscripts;
        let detail = reason.human_detail(display_name, hours, "");
        let winner_tokens = idb.token_count;
        return estimated_selection(
            LocalDataAudit {
                sources_checked,
                idb,
                local_storage,
                safari_web_storage: Some(safari),
                tier_candidates,
                winner_tier: 1.0,
                winner_reason: reason.as_str().to_string(),
                winner_reason_detail: detail.clone(),
                provider_logged_usage_found: false,
                accurate_count_possible: true,
                winner_tokens,
            },
            &format_method(display_name, &detail),
        );
    }

    if !is_priority_web_domain(display_name) {
        return unavailable_selection(
            display_name,
            sources_checked,
            idb,
            local_storage,
            safari,
            tier_candidates,
            HISTORY_ONLY_REASON,
        );
    }

    let winner_tokens = tier1_5_tokens.max(tier2_tokens).max(tier3_tokens);

    if winner_tokens == 0 {
        return unavailable_selection(
            display_name,
            sources_checked,
            idb,
            local_storage,
            safari,
            tier_candidates,
            HISTORY_ONLY_REASON,
        );
    }

    let (winner_tier, reason) = pick_winner(
        tier1_5_tokens,
        tier2_tokens,
        tier3_tokens,
        tier2_sessions.unwrap_or(0),
    );
    let detail = reason.human_detail(display_name, hours, &tier2_detail);

    estimated_selection(
        LocalDataAudit {
            sources_checked,
            idb,
            local_storage,
            safari_web_storage: Some(safari),
            tier_candidates,
            winner_tier,
            winner_reason: reason.as_str().to_string(),
            winner_reason_detail: detail.clone(),
            provider_logged_usage_found: false,
            accurate_count_possible: false,
            winner_tokens,
        },
        &format_method_tiered(display_name, &reason, &detail, hours),
    )
}

fn pick_winner(tier1_5: u64, tier2: u64, tier3: u64, tier2_sessions: u64) -> (f64, WinnerReason) {
    let max = tier1_5.max(tier2).max(tier3);
    if tier1_5 == max && tier1_5 > 0 {
        return (1.5, WinnerReason::Tier1_5SnippetText);
    }
    if tier2 == max && tier2_sessions > 0 {
        if tier3 > tier2 && tier3 == max {
            return (3.0, WinnerReason::Tier3HoursFloorExceededTier2);
        }
        return (2.0, WinnerReason::Tier2SessionMetadata);
    }
    if tier2 > 0 && tier3 == max && tier3 > tier2 {
        return (3.0, WinnerReason::Tier3HoursFloorExceededTier2);
    }
    (3.0, WinnerReason::Tier3ActiveHours)
}

fn tier2_from_signals(
    display_name: &str,
    signals: &crate::browser_local_storage::LocalSessionSignals,
) -> (u64, Option<u64>, Option<String>, String) {
    match display_name {
        "ChatGPT" if signals.conversations > 0 => {
            let tokens = signals
                .conversations
                .saturating_mul(TOKENS_PER_CHATGPT_CONVERSATION);
            let detail = format!(
                "{} sidebar conversations (deduped across Chrome profiles)",
                signals.conversations
            );
            (
                tokens,
                Some(signals.conversations),
                Some("conversations".to_string()),
                detail,
            )
        }
        "Perplexity" if signals.threads > 0 => {
            let tokens = signals.threads.saturating_mul(TOKENS_PER_PERPLEXITY_THREAD);
            let detail = format!(
                "{} local thread records (deduped across Chrome profiles)",
                signals.threads
            );
            (
                tokens,
                Some(signals.threads),
                Some("threads".to_string()),
                detail,
            )
        }
        "NotebookLM" if signals.notebooks > 0 => {
            let tokens = signals
                .notebooks
                .saturating_mul(TOKENS_PER_NOTEBOOKLM_NOTEBOOK);
            let detail = format!(
                "{} local notebook records (deduped across Chrome profiles)",
                signals.notebooks
            );
            (
                tokens,
                Some(signals.notebooks),
                Some("notebooks".to_string()),
                detail,
            )
        }
        _ => (0, None, None, String::new()),
    }
}

fn probe_indexeddb(display_name: &str, hours: f64) -> IdbProbe {
    let Some(folder) = chrome_idb_folder(display_name) else {
        return IdbProbe::default();
    };

    let paths = chrome_indexeddb_paths(folder);
    let folders_found = paths.len() as u32;
    if folders_found == 0 {
        return IdbProbe {
            paths_scanned: 0,
            folders_found: 0,
            ..Default::default()
        };
    }

    let texts = collect_transcript_texts_for_folder(folder);
    let message_chars: usize = texts.iter().map(|s| s.len()).sum();
    let token_count = if texts.is_empty() {
        0
    } else {
        count_tokens(&texts.join("\n"))
    };

    let rejection_reason = if texts.is_empty() {
        Some("no_message_text_in_indexeddb".to_string())
    } else if message_chars < MIN_TRANSCRIPT_CHARS {
        Some("sparse_cache".to_string())
    } else if token_count == 0 {
        Some("zero_tokens_after_tokenize".to_string())
    } else {
        transcript_quality_failure(hours, &texts, token_count).map(|r| {
            if r.contains("sparse") {
                "sparse_cache".to_string()
            } else {
                "quality_gate_failed".to_string()
            }
        })
    };

    IdbProbe {
        paths_scanned: folders_found,
        folders_found,
        message_chars,
        token_count: if rejection_reason.is_some() {
            0
        } else {
            token_count
        },
        rejection_reason,
    }
}

/// Read-only Safari WebKit WebsiteData listing; confirms no message bodies for origin.
fn probe_safari_web_storage(display_name: &str) -> SafariWebStorageProbe {
    let needles = origin_needles(display_name);
    if needles.is_empty() {
        return SafariWebStorageProbe::default();
    }

    let base = platform::home_dir().join("Library/WebKit/WebsiteData");
    if !base.is_dir() {
        return SafariWebStorageProbe::default();
    }

    let mut paths_scanned = 0u32;
    let mut message_chars = 0usize;

    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            paths_scanned += 1;
            message_chars += scan_dir_for_origin_text(&path, needles, 64 * 1024);
        }
    }

    SafariWebStorageProbe {
        paths_scanned,
        message_chars,
    }
}

fn scan_dir_for_origin_text(dir: &Path, needles: &[&str], max_bytes: usize) -> usize {
    let mut total = 0usize;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        if total >= max_bytes {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            total += scan_dir_for_origin_text(&path, needles, max_bytes.saturating_sub(total));
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 2_000_000 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let raw = String::from_utf8_lossy(&bytes);
        if needles.iter().any(|n| raw.contains(n)) {
            total += raw.len().min(max_bytes.saturating_sub(total));
        }
    }
    total
}

fn format_method(display_name: &str, detail: &str) -> String {
    format!("cl100k_base tokenizer over Chrome IndexedDB message JSON for {display_name}; {detail}")
}

fn format_method_tiered(
    display_name: &str,
    reason: &WinnerReason,
    detail: &str,
    _hours: f64,
) -> String {
    let prefix = "cl100k_base tokenizer";
    match reason {
        WinnerReason::Tier1_5SnippetText => {
            format!("{prefix}; Tier-1.5 Local Storage snippet text for {display_name} ({detail})")
        }
        WinnerReason::Tier2SessionMetadata => {
            format!("{prefix}; Tier-2 session metadata for {display_name} ({detail})")
        }
        WinnerReason::Tier3HoursFloorExceededTier2 => {
            format!("{prefix}; Tier-2 {detail}")
        }
        WinnerReason::Tier3ActiveHours => {
            format!("{prefix}; Tier-3 active-hours benchmark for {display_name} ({detail})")
        }
        _ => format!("{prefix}; {detail}"),
    }
}

fn estimated_selection(audit: LocalDataAudit, method: &str) -> AuditSelection {
    AuditSelection {
        audit,
        estimated: true,
        unavailable: false,
        unavailable_reason: None,
        method: Some(method.to_string()),
    }
}

fn unavailable_selection(
    _display_name: &str,
    sources_checked: Vec<String>,
    idb: IdbProbe,
    local_storage: LocalStorageProbe,
    safari: SafariWebStorageProbe,
    tier_candidates: TierCandidates,
    reason: &str,
) -> AuditSelection {
    let winner_reason = WinnerReason::Unavailable;
    AuditSelection {
        audit: LocalDataAudit {
            sources_checked,
            idb,
            local_storage,
            safari_web_storage: Some(safari),
            tier_candidates,
            winner_tier: 0.0,
            winner_reason: winner_reason.as_str().to_string(),
            winner_reason_detail: reason.to_string(),
            provider_logged_usage_found: false,
            accurate_count_possible: false,
            winner_tokens: 0,
        },
        estimated: false,
        unavailable: true,
        unavailable_reason: Some(reason.to_string()),
        method: None,
    }
}

impl LocalDataAudit {
    pub fn to_json_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_winner_prefers_tier3_floor_over_tier2() {
        let (tier, reason) = pick_winner(1200, 630_000, 5_820_000, 18);
        assert_eq!(tier, 3.0);
        assert_eq!(reason, WinnerReason::Tier3HoursFloorExceededTier2);
    }

    #[test]
    fn pick_winner_tier2_when_highest() {
        let (tier, reason) = pick_winner(100, 800_000, 400_000, 10);
        assert_eq!(tier, 2.0);
        assert_eq!(reason, WinnerReason::Tier2SessionMetadata);
    }

    #[test]
    fn pick_winner_tier1_5_when_highest() {
        let (tier, reason) = pick_winner(900_000, 100, 50_000, 0);
        assert_eq!(tier, 1.5);
        assert_eq!(reason, WinnerReason::Tier1_5SnippetText);
    }

    #[test]
    fn idb_probe_rejects_sparse_text() {
        clear_transcript_cache();
        // ChatGPT IDB on dev machines may have data; test structure via probe with Grok if no IDB
        let probe = probe_indexeddb("AI Studio", 0.1);
        assert_eq!(probe.paths_scanned, 0);
    }

    #[test]
    fn priority_domain_audit_not_accurate_when_tier3_wins() {
        let sel = probe_and_select("Gemini", 0.8);
        if sel.audit.winner_tier >= 2.0 {
            assert!(!sel.audit.accurate_count_possible);
            assert!(!sel.audit.provider_logged_usage_found);
        }
    }

    #[test]
    fn audit_json_has_required_fields() {
        let sel = probe_and_select("Gemini", 0.8);
        let v = sel.audit.to_json_value();
        assert!(v
            .get("sources_checked")
            .and_then(|x| x.as_array())
            .is_some());
        assert!(v.get("winner_tier").is_some());
        assert!(v.get("tier_candidates").is_some());
        assert_eq!(
            v.get("provider_logged_usage_found")
                .and_then(|x| x.as_bool()),
            Some(false)
        );
    }

    #[test]
    #[ignore = "requires local Chrome; run on dev machine"]
    fn live_browser_audit_exhaustion() {
        for (name, hours) in [
            ("ChatGPT", 4.85),
            ("Gemini", 0.81),
            ("Perplexity", 0.60),
            ("NotebookLM", 1.68),
        ] {
            let sel = probe_and_select(name, hours);
            let a = &sel.audit;
            assert!(!a.sources_checked.is_empty(), "{name}: sources_checked");
            assert!(!a.provider_logged_usage_found, "{name}: no provider logs");
            assert!(sel.estimated, "{name}: should be estimated");
            if a.winner_tier == 1.0 {
                assert!(a.accurate_count_possible, "{name}: tier1 must be accurate");
            } else {
                assert!(
                    !a.accurate_count_possible,
                    "{name}: tier {} must not be accurate",
                    a.winner_tier
                );
            }
            eprintln!(
                "{name}: tier={} reason={} tokens={} sources={:?}",
                a.winner_tier, a.winner_reason, a.winner_tokens, a.sources_checked
            );
        }
    }

    fn clear_transcript_cache() {
        crate::browser_transcripts::clear_transcript_cache_for_tests();
    }
}
