/// Chrome/Chromium: microseconds since Jan 1, 1601 → Unix timestamp
pub fn chrome_time_to_unix(chrome_time: i64) -> i64 {
    chrome_time / 1_000_000 - 11_644_473_600
}

/// Safari: seconds since Jan 1, 2001 (Mac absolute time) → Unix timestamp
pub fn safari_time_to_unix(safari_time: f64) -> i64 {
    (safari_time + 978_307_200.0) as i64
}

/// Firefox: microseconds since Unix epoch → Unix timestamp
pub fn firefox_time_to_unix(firefox_time: i64) -> i64 {
    firefox_time / 1_000_000
}

/// Chrome visit_duration: microseconds → seconds
pub fn chrome_duration_to_seconds(duration: i64) -> f64 {
    duration as f64 / 1_000_000.0
}

/// Calculate active hours from a sorted list of Unix timestamps.
/// Gaps greater than `threshold_seconds` start a new active segment.
/// Returns total active hours.
pub fn active_hours_from_timestamps(timestamps: &[i64], threshold_seconds: i64) -> f64 {
    if timestamps.len() < 2 {
        return if timestamps.is_empty() {
            0.0
        } else {
            1.0 / 60.0
        }; // 1 minute minimum
    }

    let mut total_seconds: f64 = 0.0;
    let mut segment_start = timestamps[0];
    let mut prev = timestamps[0];

    for &ts in &timestamps[1..] {
        if ts - prev > threshold_seconds {
            // End current segment, start new one
            total_seconds += (prev - segment_start) as f64;
            segment_start = ts;
        }
        prev = ts;
    }
    // Close final segment
    total_seconds += (prev - segment_start) as f64;

    // Minimum 1 minute per session
    let hours = total_seconds / 3600.0;
    if hours < 1.0 / 60.0 && !timestamps.is_empty() {
        1.0 / 60.0
    } else {
        hours
    }
}

/// Count distinct sessions from sorted timestamps using gap threshold.
/// Returns (session_count, Vec of session start/end timestamp pairs)
pub fn count_sessions(timestamps: &[i64], threshold_seconds: i64) -> (u64, Vec<(i64, i64)>) {
    if timestamps.is_empty() {
        return (0, vec![]);
    }
    if timestamps.len() == 1 {
        return (1, vec![(timestamps[0], timestamps[0])]);
    }

    let mut sessions = Vec::new();
    let mut segment_start = timestamps[0];
    let mut prev = timestamps[0];

    for &ts in &timestamps[1..] {
        if ts - prev > threshold_seconds {
            sessions.push((segment_start, prev));
            segment_start = ts;
        }
        prev = ts;
    }
    sessions.push((segment_start, prev));

    (sessions.len() as u64, sessions)
}

/// Parse ISO 8601 timestamp to Unix epoch seconds
pub fn iso8601_to_unix(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp())
        .or_else(|| {
            // Try without timezone (assume UTC)
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.fZ")
                .ok()
                .map(|dt| dt.and_utc().timestamp())
        })
}

/// Group unix-second timestamps by UTC ISO date ("YYYY-MM-DD").
pub fn group_by_day(timestamps: &[i64]) -> std::collections::HashMap<String, Vec<i64>> {
    let mut out: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();
    for &ts in timestamps {
        if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
            let day = dt.format("%Y-%m-%d").to_string();
            out.entry(day).or_default().push(ts);
        }
    }
    out
}

/// Minimum reasonable unix timestamp: 2000-01-01 UTC.
/// Events older than this are assumed to be garbage (epoch-0 / uninitialized clock).
const MIN_REASONABLE_TS: i64 = 946_684_800;

/// Build per-day usage buckets from a flat list of events.
///
/// Each event is `(unix_ts_seconds, tokens_for_event, session_id_opt)`.
/// - Events grouped by UTC calendar day (`YYYY-MM-DD`).
/// - `tokens` = sum of event tokens for the day.
/// - `sessions` = count of distinct `Some(session_id)` per day.
/// - `hours` = `active_hours_from_timestamps(day_ts, 1800)` — per-day active time
///   using the same 30-minute gap threshold as total-hours calculation.
/// - Timestamps below `MIN_REASONABLE_TS` (2000-01-01 UTC) are filtered out.
/// - `invocations` is left `None` — CLI providers don't track per-event invocations;
///   shell_history builds its own buckets with invocations populated.
///
/// Returns empty `HashMap` (not `None`) — callers decide whether to wrap in `Some`.
pub fn build_daily_buckets(
    events: &[(i64, u64, Option<String>)],
) -> std::collections::HashMap<String, crate::types::DailyBucket> {
    use crate::types::DailyBucket;
    use std::collections::{HashMap, HashSet};

    let mut day_tokens: HashMap<String, u64> = HashMap::new();
    let mut day_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    let mut day_timestamps: HashMap<String, Vec<i64>> = HashMap::new();

    for (ts, tokens, session_id) in events {
        if *ts < MIN_REASONABLE_TS {
            continue;
        }
        let Some(dt) = chrono::DateTime::from_timestamp(*ts, 0) else {
            continue;
        };
        let day = dt.format("%Y-%m-%d").to_string();

        let entry = day_tokens.entry(day.clone()).or_insert(0);
        *entry = entry.saturating_add(*tokens);
        day_timestamps.entry(day.clone()).or_default().push(*ts);
        if let Some(sid) = session_id {
            day_sessions.entry(day).or_default().insert(sid.clone());
        }
    }

    let mut buckets: HashMap<String, DailyBucket> = HashMap::new();
    for (day, mut ts_list) in day_timestamps {
        ts_list.sort_unstable();
        let hours = active_hours_from_timestamps(&ts_list, 1800);
        let tokens = day_tokens.get(&day).copied().unwrap_or(0);
        let sessions = day_sessions.get(&day).map(|s| s.len() as u64).unwrap_or(0);
        buckets.insert(
            day,
            DailyBucket {
                hours,
                tokens,
                sessions,
                invocations: None,
            },
        );
    }

    buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-01-15 00:00:00 UTC
    const DAY_EPOCH: i64 = 1_705_276_800;

    #[test]
    fn build_daily_buckets_aggregates_tokens_and_sessions_per_day() {
        let events = vec![
            // Day 1: two sessions with some multi-minute activity so hours > 0.
            (DAY_EPOCH + 60, 100, Some("sess-a".to_string())),
            (DAY_EPOCH + 120, 200, Some("sess-a".to_string())),
            (DAY_EPOCH + 180, 50, Some("sess-b".to_string())),
            // Day 2: single event, short session.
            (DAY_EPOCH + 86_400 + 30, 10, Some("sess-c".to_string())),
            // Garbage timestamp that should be filtered.
            (0, 999, Some("garbage".to_string())),
        ];

        let buckets = build_daily_buckets(&events);
        assert_eq!(buckets.len(), 2, "expected two UTC days");

        let day1 = buckets.get("2024-01-15").expect("day 1 bucket");
        assert_eq!(day1.tokens, 350);
        assert_eq!(day1.sessions, 2, "two distinct session ids on day 1");
        assert!(
            day1.hours > 0.0,
            "multi-minute activity must yield non-zero hours (was {})",
            day1.hours
        );
        assert!(day1.invocations.is_none());

        let day2 = buckets.get("2024-01-16").expect("day 2 bucket");
        assert_eq!(day2.tokens, 10);
        assert_eq!(day2.sessions, 1);
        assert!(
            day2.hours > 0.0,
            "single event still yields min-session hours"
        );
    }

    #[test]
    fn build_daily_buckets_handles_none_session_id() {
        let events = vec![(DAY_EPOCH + 60, 0, None), (DAY_EPOCH + 120, 0, None)];
        let buckets = build_daily_buckets(&events);
        let day = buckets.get("2024-01-15").expect("bucket exists");
        assert_eq!(day.sessions, 0, "None session ids must not be counted");
        assert_eq!(day.tokens, 0);
        assert!(day.hours > 0.0);
    }

    #[test]
    fn build_daily_buckets_empty_input_returns_empty_map() {
        let events: Vec<(i64, u64, Option<String>)> = Vec::new();
        let buckets = build_daily_buckets(&events);
        assert!(buckets.is_empty());
    }
}
