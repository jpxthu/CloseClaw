//! LLM integration for the memory miner.
//!
//! Provides a mockable [`MinerLlmCaller`] trait for Miner 1 (event extraction)
//! and Miner 2 (entity assignment). In production, wrap a real LLM provider;
//! in tests, use [`MockMinerLlmCaller`].

use async_trait::async_trait;

use crate::miner::{MiningEntity, MiningEvent};

/// Errors from the mining LLM caller.
#[derive(Debug, thiserror::Error)]
pub enum MinerLlmError {
    /// The LLM call failed.
    #[error("llm error: {0}")]
    Llm(String),

    /// The LLM response could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),
}

/// Abstract LLM caller for the two-stage mining pipeline.
///
/// Miner 1 calls [`extract_events`](Self::extract_events) to extract
/// structured events from a cleaned transcript. Miner 2 calls
/// [`assign_entities`](Self::assign_entities) to assign entities to
/// each event.
#[async_trait]
pub trait MinerLlmCaller: Send + Sync {
    /// Extract mining events from a cleaned transcript.
    ///
    /// `transcript` is the cleaned session transcript in markdown format.
    /// `existing_events` is a summary of recent events for dedup context.
    /// `existing_memory` is the current MEMORY.md content for dedup.
    async fn extract_events(
        &self,
        transcript: &str,
        existing_events: &str,
        existing_memory: &str,
    ) -> Result<Vec<MiningEvent>, MinerLlmError>;

    /// Assign entities to a list of mining events.
    ///
    /// `events` are the events from Miner 1 (title + summary + body).
    /// `entity_catalog` is the formatted entity/type directory text.
    async fn assign_entities(
        &self,
        events: &[MiningEvent],
        entity_catalog: &str,
    ) -> Result<Vec<Vec<MiningEntity>>, MinerLlmError>;
}

/// Mock LLM caller for unit tests.
///
/// Returns canned responses for `extract_events` and `assign_entities`.
/// Configure via the builder methods or use [`MockMinerLlmCaller::default`].
#[derive(Default)]
pub struct MockMinerLlmCaller {
    /// Events to return from `extract_events`.
    pub events_response: Vec<MiningEvent>,
    /// Entities to return from `assign_entities` (one inner Vec per event).
    pub entities_response: Vec<Vec<MiningEntity>>,
    /// If true, `extract_events` returns an error.
    pub fail_extract: bool,
    /// If true, `assign_entities` returns an error.
    pub fail_assign: bool,
}

#[async_trait]
impl MinerLlmCaller for MockMinerLlmCaller {
    async fn extract_events(
        &self,
        _transcript: &str,
        _existing_events: &str,
        _existing_memory: &str,
    ) -> Result<Vec<MiningEvent>, MinerLlmError> {
        if self.fail_extract {
            return Err(MinerLlmError::Llm("mock failure".into()));
        }
        Ok(self.events_response.clone())
    }

    async fn assign_entities(
        &self,
        events: &[MiningEvent],
        _entity_catalog: &str,
    ) -> Result<Vec<Vec<MiningEntity>>, MinerLlmError> {
        if self.fail_assign {
            return Err(MinerLlmError::Llm("mock failure".into()));
        }
        if self.entities_response.is_empty() {
            // Return one empty entity vec per event by default.
            return Ok(events.iter().map(|_| Vec::new()).collect());
        }
        Ok(self.entities_response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::miner::MiningEventCategory;

    fn make_event(title: &str, category: MiningEventCategory) -> MiningEvent {
        MiningEvent {
            title: title.to_string(),
            summary: format!("Summary of {title}"),
            body: format!("Body of {title}"),
            category,
            lesson: None,
        }
    }

    #[tokio::test]
    async fn test_mock_extract_returns_configured_events() {
        let mock = MockMinerLlmCaller {
            events_response: vec![make_event("test event", MiningEventCategory::Error)],
            ..Default::default()
        };
        let events = mock.extract_events("transcript", "", "").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "test event");
    }

    #[tokio::test]
    async fn test_mock_extract_failure() {
        let mock = MockMinerLlmCaller {
            fail_extract: true,
            ..Default::default()
        };
        let result = mock.extract_events("t", "", "").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_assign_returns_empty_per_event() {
        let mock = MockMinerLlmCaller::default();
        let events = vec![
            make_event("a", MiningEventCategory::Decision),
            make_event("b", MiningEventCategory::Anger),
        ];
        let entities = mock.assign_entities(&events, "catalog").await.unwrap();
        assert_eq!(entities.len(), 2);
        assert!(entities[0].is_empty());
        assert!(entities[1].is_empty());
    }

    #[tokio::test]
    async fn test_mock_assign_failure() {
        let mock = MockMinerLlmCaller {
            fail_assign: true,
            ..Default::default()
        };
        let events = vec![make_event("a", MiningEventCategory::Error)];
        let result = mock.assign_entities(&events, "catalog").await;
        assert!(result.is_err());
    }
}
