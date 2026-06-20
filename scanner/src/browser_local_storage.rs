//! Chrome Local Storage (LevelDB) session metadata for web AI tools.
//!
//! Chromium stores Local Storage values as UTF-8 or UTF-16LE JSON blobs (often
//! with embedded control bytes). ChatGPT caches sidebar / conversation-history
//! lists under keys like `conversation-history-without-projects` per profile.

use crate::platform;
use rusty_leveldb::{LdbIterator, Options, DB};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Session/thread/notebook counts and optional snippet text from Local Storage.
#[derive(Debug, Clone, Default)]
pub struct LocalSessionSignals {
    pub conversations: u64,
    pub threads: u64,
    pub notebooks: u64,
    /// Decoded snippet / content strings suitable for tokenization.
    pub transcript_texts: Vec<String>,
}

/// Domain-specific origin needles (substring match on decoded record text).
fn ls_origin_needles(display_name: &str) -> &'static [&'static str] {
    match display_name {
        "ChatGPT" => &["chatgpt.com", "chat.openai.com"],
        "Gemini" => &["gemini.google.com", "bard.google.com"],
        "Perplexity" => &["perplexity.ai"],
        "NotebookLM" => &["notebooklm.google.com", "$notebooklm"],
        _ => &[],
    }
}

/// Cache-key substrings that indicate session-list payloads (even if origin is binary-split).
fn ls_cache_key_needles(display_name: &str) -> &'static [&'static str] {
    match display_name {
        "ChatGPT" => &[
            "conversation-history",
            "starred-conversations",
            "client-bootstrap",
        ],
        "Perplexity" => &["threads-v2", "threadId", "pplx"],
        "NotebookLM" => &["notebooklm", "notebooks"],
        "Gemini" => &["gemini.google", "bard.google"],
        _ => &[],
    }
}

/// Collect Chrome `Local Storage/leveldb` paths for Default + Profile * dirs.
pub fn chrome_local_storage_paths() -> Vec<PathBuf> {
    let home = platform::home_dir();
    let mut paths = Vec::new();

    let chrome_base = home.join("Library/Application Support/Google/Chrome");
    if chrome_base.exists() {
        collect_profile_ls(&chrome_base, &mut paths);
    }

    for browser in [
        "Arc/User Data",
        "BraveSoftware/Brave-Browser",
        "Microsoft Edge",
        "Dia/User Data",
    ] {
        let base = home.join("Library/Application Support").join(browser);
        if base.exists() {
            collect_profile_ls(&base, &mut paths);
        }
    }

    paths
}

fn collect_profile_ls(base: &Path, out: &mut Vec<PathBuf>) {
    let default = base.join("Default/Local Storage/leveldb");
    if default.is_dir() {
        out.push(default);
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("Profile ") {
                let p = entry.path().join("Local Storage/leveldb");
                if p.is_dir() {
                    out.push(p);
                }
            }
        }
    }
}

/// Scan all Chromium Local Storage DBs; dedupe session IDs across every profile/browser.
pub fn collect_session_signals(display_name: &str) -> LocalSessionSignals {
    let needles = ls_origin_needles(display_name);
    if needles.is_empty() {
        return LocalSessionSignals::default();
    }

    let mut seen_conversation_ids: HashSet<String> = HashSet::new();
    let mut seen_thread_ids: HashSet<String> = HashSet::new();
    let mut seen_notebook_ids: HashSet<String> = HashSet::new();
    let mut transcript_seen: HashSet<String> = HashSet::new();
    let mut transcript_texts: Vec<String> = Vec::new();
    let mut max_threads = 0u64;
    let cache_keys = ls_cache_key_needles(display_name);

    for path in chrome_local_storage_paths() {
        let mut ctx = LsExtractCtx {
            display_name,
            needles,
            cache_keys,
            seen_conversation_ids: &mut seen_conversation_ids,
            seen_thread_ids: &mut seen_thread_ids,
            seen_notebook_ids: &mut seen_notebook_ids,
            max_threads: &mut max_threads,
            transcript_texts: &mut transcript_texts,
            transcript_seen: &mut transcript_seen,
        };
        extract_signals_from_ls_db(&path, &mut ctx);
    }

    LocalSessionSignals {
        conversations: seen_conversation_ids.len() as u64,
        threads: seen_thread_ids.len().max(max_threads as usize) as u64,
        notebooks: seen_notebook_ids.len() as u64,
        transcript_texts,
    }
}

struct LsExtractCtx<'a> {
    display_name: &'a str,
    needles: &'a [&'static str],
    cache_keys: &'a [&'static str],
    seen_conversation_ids: &'a mut HashSet<String>,
    seen_thread_ids: &'a mut HashSet<String>,
    seen_notebook_ids: &'a mut HashSet<String>,
    max_threads: &'a mut u64,
    transcript_texts: &'a mut Vec<String>,
    transcript_seen: &'a mut HashSet<String>,
}

impl LsExtractCtx<'_> {
    fn text_matches_domain(&self, text: &str) -> bool {
        self.needles.iter().any(|n| text.contains(n))
            || self.cache_keys.iter().any(|n| text.contains(n))
    }

    fn absorb_text(&mut self, text: &str) {
        absorb_session_json_text(self, text);
    }
}

fn extract_signals_from_ls_db(path: &Path, ctx: &mut LsExtractCtx<'_>) {
    let opts = Options {
        create_if_missing: false,
        ..Options::default()
    };
    if let Ok(mut db) = DB::open(path, opts) {
        if let Ok(mut iter) = db.new_iter() {
            while let Some((key, value)) = iter.next() {
                let key_lossy = String::from_utf8_lossy(&key);
                let key_hit = ctx.needles.iter().any(|n| key_lossy.contains(n))
                    || ctx.cache_keys.iter().any(|n| key_lossy.contains(n));

                for text in decode_ls_text_candidates(&value) {
                    if key_hit || ctx.text_matches_domain(&text) {
                        ctx.absorb_text(&text);
                    }
                }
            }
        }
    }

    // Chromium Local Storage: LevelDB iterator often misses origin-split blobs;
    // scan raw .ldb/.log files (same approach as forensic strings probes).
    scan_ldb_files_on_disk(path, ctx);
}

fn scan_ldb_files_on_disk(path: &Path, ctx: &mut LsExtractCtx<'_>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".ldb") || name.ends_with(".log")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let raw = String::from_utf8_lossy(&bytes);
        if !ctx.needles.iter().any(|n| raw.contains(n))
            && !ctx.cache_keys.iter().any(|n| raw.contains(n))
        {
            continue;
        }
        for text in decode_ls_text_candidates(&bytes) {
            ctx.absorb_text(&text);
        }
    }
}

/// Decode UTF-8 and UTF-16LE JSON blobs from a Chromium Local Storage value.
fn decode_ls_text_candidates(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let utf8 = sanitize_storage_text(&String::from_utf8_lossy(bytes));
    if utf8.len() >= 20 {
        out.push(utf8);
    }

    let mut i = 0;
    while i + 12 < bytes.len() {
        // UTF-16LE `{"` → `{\x00"\x00`
        if bytes[i] == b'{' && bytes[i + 1] == 0 && bytes[i + 2] == b'"' && bytes[i + 3] == 0 {
            let end = (i + 400_000).min(bytes.len());
            let mut decoded = String::new();
            let chunk = &bytes[i..end];
            let mut j = 0;
            while j + 1 < chunk.len() {
                let unit = u16::from_le_bytes([chunk[j], chunk[j + 1]]);
                if let Some(ch) = char::from_u32(unit as u32) {
                    decoded.push(ch);
                } else {
                    decoded.push('\u{FFFD}');
                }
                j += 2;
            }
            let sanitized = sanitize_storage_text(&decoded);
            if sanitized.len() >= 20 {
                out.push(sanitized);
            }
            i += 2;
        } else {
            i += 1;
        }
    }

    out
}

fn sanitize_storage_text(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\t' || c == '\n' || c == '\r' || !c.is_control() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn absorb_session_json_text(ctx: &mut LsExtractCtx<'_>, text: &str) {
    for json_str in find_json_objects(text) {
        if let Ok(value) = serde_json::from_str::<Value>(&json_str) {
            absorb_session_json(ctx, &value);
        }
    }

    if ctx.display_name == "ChatGPT" {
        extract_chatgpt_ids_from_text(text, ctx.seen_conversation_ids);
    }
    if ctx.display_name == "Perplexity" {
        extract_thread_ids_from_text(text, ctx.seen_thread_ids, ctx.max_threads);
    }
    if ctx.display_name == "NotebookLM" {
        extract_notebook_ids_from_text(text, ctx.seen_notebook_ids);
    }

    for field in ["snippet", "content", "text", "message", "body", "title"] {
        extract_quoted_field_values(text, field, ctx.transcript_texts, ctx.transcript_seen);
    }
}

fn absorb_session_json(ctx: &mut LsExtractCtx<'_>, value: &Value) {
    match value {
        Value::Array(arr) => {
            if ctx.display_name == "Perplexity" && looks_like_thread_list(arr) {
                for item in arr {
                    if let Some(id) = item.get("threadId").and_then(|v| v.as_str()) {
                        ctx.seen_thread_ids.insert(id.to_string());
                    }
                }
                *ctx.max_threads = (*ctx.max_threads).max(arr.len() as u64);
            }
            for item in arr {
                absorb_session_json(ctx, item);
            }
        }
        Value::Object(map) => {
            if ctx.display_name == "ChatGPT" {
                if let Some(items) = map
                    .get("value")
                    .and_then(|v| v.get("pages"))
                    .and_then(|p| p.as_array())
                    .and_then(|pages| pages.first())
                    .and_then(|page| page.get("items"))
                    .and_then(|i| i.as_array())
                {
                    for item in items {
                        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                            if looks_like_uuid(id) {
                                ctx.seen_conversation_ids.insert(id.to_string());
                            }
                        }
                        push_transcript_field(
                            item,
                            "snippet",
                            ctx.transcript_texts,
                            ctx.transcript_seen,
                        );
                        push_transcript_field(
                            item,
                            "title",
                            ctx.transcript_texts,
                            ctx.transcript_seen,
                        );
                    }
                }
                if let Some(items) = map.get("items").and_then(|i| i.as_array()) {
                    for item in items {
                        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                            if looks_like_uuid(id) {
                                ctx.seen_conversation_ids.insert(id.to_string());
                            }
                        }
                    }
                }
            }
            if ctx.display_name == "NotebookLM" {
                if let Some(notebooks) = map.get("notebooks").and_then(|n| n.as_array()) {
                    for nb in notebooks {
                        if let Some(id) = nb.get("id").and_then(|v| v.as_str()) {
                            ctx.seen_notebook_ids.insert(id.to_string());
                        }
                    }
                }
            }
            for (_, v) in map {
                absorb_session_json(ctx, v);
            }
        }
        _ => {}
    }
}

fn push_transcript_field(
    item: &Value,
    field: &str,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if let Some(s) = item.get(field).and_then(|v| v.as_str()) {
        let t = s.trim();
        if t.len() >= 12 && seen.insert(t.to_string()) {
            out.push(t.to_string());
        }
    }
}

fn looks_like_thread_list(arr: &[Value]) -> bool {
    !arr.is_empty()
        && arr.iter().any(|v| {
            v.get("threadId").is_some() || (v.get("name").is_some() && v.get("createdAt").is_some())
        })
}

fn looks_like_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn extract_chatgpt_ids_from_text(text: &str, seen: &mut HashSet<String>) {
    const NEEDLE: &[u8] = b"\"id\":\"";
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + NEEDLE.len() < bytes.len() {
        if bytes[i..].starts_with(NEEDLE) {
            let start = i + NEEDLE.len();
            if start + 36 <= bytes.len() {
                if let Ok(id) = std::str::from_utf8(&bytes[start..start + 36]) {
                    if looks_like_uuid(id) {
                        seen.insert(id.to_string());
                    }
                }
            }
            i = start.saturating_add(1);
        } else {
            i += 1;
        }
    }
}

fn extract_thread_ids_from_text(text: &str, seen: &mut HashSet<String>, max_threads: &mut u64) {
    const NEEDLE: &[u8] = b"\"threadId\":\"";
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut count = 0u64;
    while i + NEEDLE.len() < bytes.len() {
        if bytes[i..].starts_with(NEEDLE) {
            let start = i + NEEDLE.len();
            if let Ok(rest) = std::str::from_utf8(&bytes[start..]) {
                if let Some(end) = rest.find('"') {
                    let id = &rest[..end];
                    if !id.is_empty() {
                        seen.insert(id.to_string());
                        count += 1;
                    }
                }
            }
            i = start.saturating_add(1);
        } else {
            i += 1;
        }
    }
    if count > 0 {
        *max_threads = (*max_threads).max(count);
    }
}

fn extract_notebook_ids_from_text(text: &str, seen: &mut HashSet<String>) {
    if !text.contains("notebook") && !text.contains("NotebookLM") {
        return;
    }
    extract_chatgpt_ids_from_text(text, seen);
}

fn extract_quoted_field_values(
    text: &str,
    field: &str,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let needle = format!("\"{field}\":\"");
    let needle_bytes = needle.as_bytes();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + needle_bytes.len() < bytes.len() {
        if bytes[i..].starts_with(needle_bytes) {
            let start = i + needle_bytes.len();
            let Some(rest) = std::str::from_utf8(&bytes[start..]).ok() else {
                i = start.saturating_add(1);
                continue;
            };
            let mut escaped = false;
            let mut end = 0;
            for (j, ch) in rest.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                if ch == '"' {
                    end = j;
                    break;
                }
            }
            if end > 12 {
                let slice = &rest[..end];
                if slice.chars().any(|c| c.is_alphabetic()) && seen.insert(slice.to_string()) {
                    out.push(slice.to_string());
                }
            }
            i = start + end + 1;
        } else {
            i += 1;
        }
    }
}

fn find_json_objects(s: &str) -> Vec<String> {
    let mut results = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' && bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let open = bytes[i];
        let close = if open == b'{' { b'}' } else { b']' };
        let mut depth = 0;
        let mut closed_at = None;
        for (offset, &byte) in bytes[i..].iter().enumerate() {
            if byte == open {
                depth += 1;
            } else if byte == close {
                depth -= 1;
                if depth == 0 {
                    closed_at = Some(i + offset);
                    break;
                }
            }
        }
        if let Some(end) = closed_at {
            if let Ok(fragment) = std::str::from_utf8(&bytes[start..=end]) {
                if fragment.len() >= 30 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absorb_chatgpt_sidebar_json() {
        let json: Value = serde_json::json!({
            "value": {"pages": [{"items": [
                {"id": "6a180a39-8ea8-83a8-9c9b-72aceb15a113", "title": "One"},
                {"id": "6a17f2b4-8ea8-83a8-9c9b-72aceb15a114", "title": "Two"}
            ]}]}
        });
        let mut conv = HashSet::new();
        let mut thr = HashSet::new();
        let mut nb = HashSet::new();
        let mut max_t = 0;
        let mut texts = Vec::new();
        let mut seen_t = HashSet::new();
        let mut ctx = LsExtractCtx {
            display_name: "ChatGPT",
            needles: &["chatgpt.com"],
            cache_keys: &["conversation-history"],
            seen_conversation_ids: &mut conv,
            seen_thread_ids: &mut thr,
            seen_notebook_ids: &mut nb,
            max_threads: &mut max_t,
            transcript_texts: &mut texts,
            transcript_seen: &mut seen_t,
        };
        absorb_session_json(&mut ctx, &json);
        assert_eq!(conv.len(), 2);
    }

    #[test]
    fn extract_chatgpt_ids_from_corrupted_text() {
        let mut seen = HashSet::new();
        let text = r#"garbage {"id":"6a180a39-8ea8-83a8-9c9b-72aceb15a113","title":"Hi"}"#;
        extract_chatgpt_ids_from_text(text, &mut seen);
        assert_eq!(seen.len(), 1);
    }

    #[test]
    fn decode_utf16le_value() {
        let utf16: Vec<u16> = " {\"value\":{\"pages\":[{\"items\":[{\"id\":\"x\"}]}]}}"
            .encode_utf16()
            .collect();
        let mut bytes = vec![0u8; 4];
        for unit in utf16 {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let decoded = decode_ls_text_candidates(&bytes);
        assert!(decoded.iter().any(|s| s.contains("value")));
    }

    #[test]
    fn absorb_perplexity_thread_list() {
        let json: Value = serde_json::json!([
            {"threadId": "t1", "name": "A", "createdAt": 1},
            {"threadId": "t2", "name": "B", "createdAt": 2}
        ]);
        let mut conv = HashSet::new();
        let mut thr = HashSet::new();
        let mut nb = HashSet::new();
        let mut max_t = 0;
        let mut texts = Vec::new();
        let mut seen_t = HashSet::new();
        let mut ctx = LsExtractCtx {
            display_name: "Perplexity",
            needles: &["perplexity.ai"],
            cache_keys: &["threads-v2"],
            seen_conversation_ids: &mut conv,
            seen_thread_ids: &mut thr,
            seen_notebook_ids: &mut nb,
            max_threads: &mut max_t,
            transcript_texts: &mut texts,
            transcript_seen: &mut seen_t,
        };
        absorb_session_json(&mut ctx, &json);
        assert_eq!(thr.len(), 2);
    }

    #[test]
    #[ignore = "requires local Chrome Local Storage; run on dev machine"]
    fn live_chrome_ls_finds_chatgpt_conversations() {
        let signals = collect_session_signals("ChatGPT");
        assert!(
            signals.conversations > 0,
            "expected ChatGPT conversations from Local Storage on dev machine"
        );
    }
}
