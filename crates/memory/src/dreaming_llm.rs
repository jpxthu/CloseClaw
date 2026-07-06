//! LLM integration for the dreaming pipeline.
//!
//! Provides lesson consolidation via a mockable [`DreamingLlmCaller`] trait.

use async_trait::async_trait;

/// Errors from the dreaming LLM caller.
#[derive(Debug, thiserror::Error)]
pub enum DreamingLlmError {
    /// The LLM call failed.
    #[error("llm error: {0}")]
    Llm(String),
}

/// Abstract LLM caller for lesson consolidation.
///
/// In production, wrap a real [`Provider`][closeclaw_llm::Provider];
/// in tests, provide a mock that returns fixed strings.
#[async_trait]
pub trait DreamingLlmCaller: Send + Sync {
    /// Consolidate multiple lessons into a single behavioral rule.
    ///
    /// `lessons` are the lesson strings from entity-associated events.
    /// `entity_name` is the human-readable entity name for context.
    /// Returns the consolidated rule text.
    async fn consolidate_lessons(
        &self,
        lessons: &[String],
        entity_name: &str,
    ) -> Result<String, DreamingLlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock LLM caller that returns a fixed response.
    struct MockLlmCaller;

    #[async_trait]
    impl DreamingLlmCaller for MockLlmCaller {
        async fn consolidate_lessons(
            &self,
            lessons: &[String],
            _entity_name: &str,
        ) -> Result<String, DreamingLlmError> {
            if lessons.is_empty() {
                return Err(DreamingLlmError::Llm("no lessons".into()));
            }
            Ok(format!("consolidated: {}", lessons.join(", ")))
        }
    }

    /// Failing LLM caller for testing degradation.
    struct FailingLlmCaller;

    #[async_trait]
    impl DreamingLlmCaller for FailingLlmCaller {
        async fn consolidate_lessons(
            &self,
            _lessons: &[String],
            _entity_name: &str,
        ) -> Result<String, DreamingLlmError> {
            Err(DreamingLlmError::Llm("simulated failure".into()))
        }
    }

    #[tokio::test]
    async fn test_mock_llm_returns_consolidated() {
        let llm = MockLlmCaller;
        let result = llm
            .consolidate_lessons(&["rule a".to_string(), "rule b".to_string()], "test-entity")
            .await
            .unwrap();
        assert!(result.contains("rule a"));
        assert!(result.contains("rule b"));
    }

    #[tokio::test]
    async fn test_failing_llm_returns_error() {
        let llm = FailingLlmCaller;
        let result = llm
            .consolidate_lessons(&["rule".to_string()], "entity")
            .await;
        assert!(result.is_err());
    }
}
