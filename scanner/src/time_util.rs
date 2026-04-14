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
