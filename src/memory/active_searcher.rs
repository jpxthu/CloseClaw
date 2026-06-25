//! Active Searcher — per-message entity search and memory injection.
//!
//! On every message, extracts query concepts via LLM, matches them against
//! the SQLite `entities` table (agent-isolated), resolves associated events,
//! deduplicates, and produces a text summary for the `memory_injection` slot.

use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::time::timeout;

use super::active_searcher_llm::{
    extract_concepts_llm, should_trigger_role, summarize_events_llm, LlmCaller,
};
use crate::llm::session::{InjectionPosition, MemoryInjection};

// ── Errors ───────────────────────────────────────────────────────────────

/// Errors specific to the active-searcher module.
#[derive(Debug, Error)]
pub enum ActiveSearcherError {
    /// SQLite storage error.
    #[error("sqlite error: {0}")]
    Sqlite(String),
    /// LLM caller error.
    #[error("llm error: {0}")]
    Llm(String),
}

// ── Configuration ────────────────────────────────────────────────────────

/// Search parameters for the active-searcher.
///
/// Typically loaded from the memory section of the agent config file.
/// All fields have sensible defaults.
#[derive(Debug, Clone)]
pub struct ActiveSearcherConfig {
    /// Timeout in milliseconds for the entire search pipeline.
    pub timeout_ms: u64,
    /// Maximum character count for the condensed summary text.
    pub max_summary_chars: usize,
    /// Minimum number of entity hits required to include an event.
    pub min_entity_hits: u32,
    /// Maximum number of events returned per search.
    pub top_k_events: usize,
    /// Number of recent conversation turns to include as context.
    pub context_turns: usize,
    /// LLM model used for concept extraction and summarisation.
    pub model: String,
}

impl Default for ActiveSearcherConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            max_summary_chars: 2000,
            min_entity_hits: 1,
            top_k_events: 10,
            context_turns: 3,
            model: "gpt-4o-mini".to_string(),
        }
    }
}

// ── Domain types ─────────────────────────────────────────────────────────

/// A row from the `entities` table, enriched with the matching type weight.
#[derive(Debug, Clone)]
pub struct MatchedEntity {
    /// Primary key of the matched entity.
    pub id: i64,
    /// Agent that owns this entity.
    pub agent_id: String,
    /// Entity type key (e.g. "person", "subject").
    pub entity_type: String,
    /// Human-readable entity name.
    pub name: String,
    /// Normalised name used for exact matching.
    pub normalized_name: String,
    /// Weight of the entity type (from `entity_types` table).
    pub weight: f64,
}

/// A row from the `events` table.
#[derive(Debug, Clone)]
pub struct EventRecord {
    /// Primary key.
    pub id: i64,
    /// Human-readable event content / summary.
    pub content: String,
    /// When the event was recorded (Unix timestamp).
    pub timestamp: i64,
    /// Source session that produced this event.
    pub source_session_id: String,
}

// ── Core searcher ────────────────────────────────────────────────────────

/// Stateless searcher that opens a fresh SQLite connection on each call.
///
/// Holding only a `PathBuf` and a config snapshot keeps the struct
/// `Send + Sync` and avoids holding a long-lived connection open.
#[derive(Debug, Clone)]
pub struct ActiveSearcher {
    /// Path to the SQLite database file.
    db_path: PathBuf,
    /// Search parameters.
    config: ActiveSearcherConfig,
}

impl ActiveSearcher {
    /// Create a new searcher with the given database path and config.
    pub fn new(db_path: impl AsRef<Path>, config: ActiveSearcherConfig) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            config,
        }
    }

    /// Returns a reference to the search configuration.
    pub fn config(&self) -> &ActiveSearcherConfig {
        &self.config
    }

    // ── Entity search ────────────────────────────────────────────────

    /// Match a list of query concepts against the `entities` table.
    ///
    /// For each concept, performs:
    /// - **Exact match**: `normalized_name = concept`
    /// - **Fuzzy match**: `normalized_name LIKE '%concept%'`
    ///
    /// Results are de-duplicated by entity ID and sorted by entity type
    /// weight descending (higher-weight types surface first).
    pub fn search_entities(
        &self,
        agent_id: &str,
        concepts: &[String],
    ) -> Result<Vec<MatchedEntity>, ActiveSearcherError> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT e.id, e.agent_id, e.type, e.name, e.normalized_name,
                        t.weight
                 FROM entities e
                 JOIN entity_types t ON e.type = t.type
                 WHERE e.agent_id = ?1
                   AND (e.normalized_name = ?2
                        OR e.normalized_name LIKE '%' || ?2 || '%')
                 ORDER BY t.weight DESC",
            )
            .map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))?;

        let mut seen: HashSet<i64> = HashSet::new();
        let mut results = Vec::new();

        for concept in concepts {
            let rows = stmt
                .query_map(params![agent_id, concept], |row| {
                    Ok(MatchedEntity {
                        id: row.get(0)?,
                        agent_id: row.get(1)?,
                        entity_type: row.get(2)?,
                        name: row.get(3)?,
                        normalized_name: row.get(4)?,
                        weight: row.get(5)?,
                    })
                })
                .map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))?;

            for row in rows {
                let entity = row.map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))?;
                if seen.insert(entity.id) {
                    results.push(entity);
                }
            }
        }

        Ok(results)
    }

    // ── Event resolution ─────────────────────────────────────────────

    /// Resolve associated events for a set of entity IDs.
    ///
    /// Joins `event_entities` with `events`, counts distinct entity hits
    /// per event, filters by `min_entity_hits`, and returns at most
    /// `top_k_events` results ordered by hit count descending.
    pub fn find_events(&self, entity_ids: &[i64]) -> Result<Vec<EventRecord>, ActiveSearcherError> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.open()?;
        let placeholders: Vec<String> = entity_ids.iter().map(|_| "?".to_string()).collect();
        let in_clause = placeholders.join(", ");

        let sql = format!(
            "SELECT ev.id, ev.content, ev.timestamp, ev.source_session_id,
                    COUNT(DISTINCT ee.entity_id) AS hit_count
             FROM events ev
             JOIN event_entities ee ON ev.id = ee.event_id
             WHERE ee.entity_id IN ({in_clause})
             GROUP BY ev.id
             HAVING hit_count >= ?{param_idx}
             ORDER BY hit_count DESC
             LIMIT ?{limit_idx}",
            in_clause = in_clause,
            param_idx = entity_ids.len() + 1,
            limit_idx = entity_ids.len() + 2,
        );

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(self.config.min_entity_hits));
        param_values.push(Box::new(self.config.top_k_events as i64));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))?;

        let events = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(EventRecord {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    timestamp: row.get(2)?,
                    source_session_id: row.get(3)?,
                })
            })
            .map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(events)
    }

    // ── Dedup ────────────────────────────────────────────────────────

    /// Exclude events whose IDs are already in `injected_event_ids`.
    pub fn dedup_events(
        &self,
        events: Vec<EventRecord>,
        injected_event_ids: &HashSet<i64>,
    ) -> Vec<EventRecord> {
        events
            .into_iter()
            .filter(|e| !injected_event_ids.contains(&e.id))
            .collect()
    }

    // ── Summarise ────────────────────────────────────────────────────

    /// Condense a list of events into a plain-text summary.
    ///
    /// Each event is rendered as `[timestamp] content` on its own line.
    /// The result is truncated to `max_summary_chars` if necessary.
    /// This is a pure-text implementation; LLM-based summarisation
    /// will be added in a later iteration.
    pub fn summarize_events(&self, events: &[EventRecord]) -> String {
        if events.is_empty() {
            return String::new();
        }

        let max = self.config.max_summary_chars;
        let mut out = String::with_capacity(max.min(events.len() * 80));

        for (i, ev) in events.iter().enumerate() {
            let line = if i == 0 {
                format!("[{}] {}", ev.timestamp, ev.content)
            } else {
                format!("\n[{}] {}", ev.timestamp, ev.content)
            };

            if out.len() + line.len() > max {
                let remaining = max.saturating_sub(out.len());
                if remaining > 0 {
                    out.push_str(&line[..remaining.min(line.len())]);
                }
                break;
            }
            out.push_str(&line);
        }

        out
    }

    // ── Full pipeline ─────────────────────────────────────────────

    /// Run the complete active-searcher pipeline.
    ///
    /// 1. Extract concepts via LLM
    /// 2. Search entities in SQLite
    /// 3. Find associated events
    /// 4. Deduplicate against already-injected events
    /// 5. Summarise via LLM
    ///
    /// Returns `None` if the pipeline times out or produces no results.
    pub async fn run(
        &self,
        agent_id: &str,
        session_role: &str,
        current_message: &str,
        context_messages: &[crate::llm::session::SessionMessage],
        injected_event_ids: &HashSet<i64>,
        llm: &dyn LlmCaller,
    ) -> Option<MemoryInjection> {
        if !should_trigger_role(session_role) {
            return None;
        }

        let timeout_duration = std::time::Duration::from_millis(self.config.timeout_ms);

        let result: Result<Result<Option<MemoryInjection>, ActiveSearcherError>, _> =
            timeout(timeout_duration, async {
                // 1. Extract concepts
                let concepts = extract_concepts_llm(llm, context_messages, current_message).await?;
                if concepts.is_empty() {
                    return Ok(None);
                }

                // 2. Search entities
                let entities = self.search_entities(agent_id, &concepts)?;
                if entities.is_empty() {
                    return Ok(None);
                }

                // 3. Find events
                let entity_ids: Vec<i64> = entities.iter().map(|e| e.id).collect();
                let events = self.find_events(&entity_ids)?;
                if events.is_empty() {
                    return Ok(None);
                }

                // 4. Deduplicate
                let events = self.dedup_events(events, injected_event_ids);
                if events.is_empty() {
                    return Ok(None);
                }

                // 5. Summarise via LLM
                let events_text = self.summarize_events(&events);
                let summary =
                    summarize_events_llm(llm, &events_text, self.config.max_summary_chars).await?;

                // 6. Determine position mode based on session role
                let position = if session_role == "user" {
                    InjectionPosition::AfterCurrent
                } else {
                    InjectionPosition::BeforeNext
                };

                // 7. Build injection with event IDs for dedup
                let mut injection = MemoryInjection::new(summary, position);
                for ev in &events {
                    injection.add_injected_event_id(ev.id);
                }

                Ok(Some(injection))
            })
            .await;

        match result {
            Ok(Ok(Some(injection))) => Some(injection),
            _ => None,
        }
    }

    /// Returns `true` if the given session role should trigger active-searcher.
    pub fn should_trigger(session_role: &str) -> bool {
        should_trigger_role(session_role)
    }

    // ── Internal ─────────────────────────────────────────────────────

    /// Open a new SQLite connection to the database.
    fn open(&self) -> Result<Connection, ActiveSearcherError> {
        Connection::open(&self.db_path).map_err(|e| ActiveSearcherError::Sqlite(e.to_string()))
    }
}
