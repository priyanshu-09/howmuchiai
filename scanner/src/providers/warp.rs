use crate::platform;
use crate::providers::Provider;
use crate::sqlite_util::SafeSqlite;
use crate::types::{ProviderResult, ScanError};
use std::collections::HashMap;

pub struct WarpProvider;

impl Provider for WarpProvider {
    fn name(&self) -> &'static str {
        "warp"
    }

    fn display_name(&self) -> &'static str {
        "Warp Terminal AI"
    }

    fn is_available(&self) -> bool {
        platform::warp_sqlite_path().is_some()
    }

    fn scan(&self) -> Result<ProviderResult, ScanError> {
        let db_path = platform::warp_sqlite_path()
            .ok_or_else(|| ScanError::NotFound("Warp database not found".into()))?;

        let db = SafeSqlite::open(&db_path)?;
        let conn = db.conn();

        let mut stmt = conn.prepare(
            "SELECT conversation_id, start_ts, model_id FROM ai_queries ORDER BY start_ts",
        )?;

        struct QueryRow {
            conversation_id: String,
            start_ts: String,
            model_id: Option<String>,
        }

        let rows: Vec<QueryRow> = stmt
            .query_map([], |row| {
                Ok(QueryRow {
                    conversation_id: row.get(0)?,
                    start_ts: row.get(1)?,
                    model_id: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            let mut result = ProviderResult::new("Warp Terminal AI");
            result.sessions = Some(0);
            result.invocations = Some(0);
            return Ok(result);
        }

        // Group by conversation_id for session counting
        let mut sessions_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut model_counts: HashMap<String, u64> = HashMap::new();
        let mut timestamps: Vec<i64> = Vec::new();
        let total_invocations = rows.len() as u64;

        for row in &rows {
            sessions_set.insert(row.conversation_id.clone());

            if let Some(ref model) = row.model_id {
                *model_counts.entry(model.clone()).or_insert(0) += 1;
            }

            // Parse timestamp: "2024-01-15 14:30:00.123"
            if let Ok(dt) =
                chrono::NaiveDateTime::parse_from_str(&row.start_ts, "%Y-%m-%d %H:%M:%S%.f")
            {
                timestamps.push(dt.and_utc().timestamp());
            } else if let Ok(dt) =
                chrono::NaiveDateTime::parse_from_str(&row.start_ts, "%Y-%m-%d %H:%M:%S")
            {
                timestamps.push(dt.and_utc().timestamp());
            }
        }

        timestamps.sort_unstable();

        let hours = crate::time_util::active_hours_from_timestamps(&timestamps, 1800);

        let first_seen = timestamps.first().copied();
        let last_seen = timestamps.last().copied();

        // Build metadata with per-model counts
        let mut metadata = HashMap::new();
        let model_map: serde_json::Value = model_counts
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::from(v)))
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into();
        metadata.insert("models".to_string(), model_map);

        let mut result = ProviderResult::new("Warp Terminal AI");
        result.sessions = Some(sessions_set.len() as u64);
        result.invocations = Some(total_invocations);
        result.hours = Some(hours);
        result.first_seen = first_seen;
        result.last_seen = last_seen;
        result.metadata = Some(metadata);

        Ok(result)
    }
}
