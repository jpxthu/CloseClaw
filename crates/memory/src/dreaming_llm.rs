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

/// Information about a promoted entity group for diary generation.
#[derive(Debug, Clone)]
pub struct PromotedGroupInfo {
    /// Human-readable entity name.
    pub entity_name: String,
    /// Entity type (e.g. "subject", "person").
    pub entity_type: String,
    /// Consolidated lessons / rules for this entity.
    pub lessons: Vec<String>,
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
        entity_type: &str,
        frequency: usize,
    ) -> Result<String, DreamingLlmError>;

    /// Generate a Dream Diary narrative summarizing promoted groups.
    ///
    /// `promoted_groups` contains entity groups that were actually
    /// promoted to MEMORY.md. Returns coherent prose for the diary.
    async fn generate_diary_narrative(
        &self,
        promoted_groups: &[PromotedGroupInfo],
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
            _entity_type: &str,
            _frequency: usize,
        ) -> Result<String, DreamingLlmError> {
            if lessons.is_empty() {
                return Err(DreamingLlmError::Llm("no lessons".into()));
            }
            Ok(format!("consolidated: {}", lessons.join(", ")))
        }

        async fn generate_diary_narrative(
            &self,
            promoted_groups: &[PromotedGroupInfo],
        ) -> Result<String, DreamingLlmError> {
            let names: Vec<&str> = promoted_groups
                .iter()
                .map(|g| g.entity_name.as_str())
                .collect();
            Ok(format!("diary about {}", names.join(", ")))
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
            _entity_type: &str,
            _frequency: usize,
        ) -> Result<String, DreamingLlmError> {
            Err(DreamingLlmError::Llm("simulated failure".into()))
        }

        async fn generate_diary_narrative(
            &self,
            _promoted_groups: &[PromotedGroupInfo],
        ) -> Result<String, DreamingLlmError> {
            Err(DreamingLlmError::Llm("simulated failure".into()))
        }
    }

    #[tokio::test]
    async fn test_mock_llm_returns_consolidated() {
        let llm = MockLlmCaller;
        let result = llm
            .consolidate_lessons(
                &["rule a".to_string(), "rule b".to_string()],
                "test-entity",
                "subject",
                3,
            )
            .await
            .unwrap();
        assert!(result.contains("rule a"));
        assert!(result.contains("rule b"));
    }

    #[tokio::test]
    async fn test_failing_llm_returns_error() {
        let llm = FailingLlmCaller;
        let result = llm
            .consolidate_lessons(&["rule".to_string()], "entity", "subject", 1)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_llm_diary_narrative() {
        let llm = MockLlmCaller;
        let groups = vec![
            PromotedGroupInfo {
                entity_name: "deploy".into(),
                entity_type: "subject".into(),
                lessons: vec!["always verify".into()],
            },
            PromotedGroupInfo {
                entity_name: "vim".into(),
                entity_type: "subject".into(),
                lessons: vec!["use vim".into()],
            },
        ];
        let result = llm.generate_diary_narrative(&groups).await.unwrap();
        assert!(result.contains("deploy"));
        assert!(result.contains("vim"));
    }

    #[tokio::test]
    async fn test_failing_llm_diary_narrative() {
        let llm = FailingLlmCaller;
        let result = llm
            .generate_diary_narrative(&[PromotedGroupInfo {
                entity_name: "x".into(),
                entity_type: "subject".into(),
                lessons: vec![],
            }])
            .await;
        assert!(result.is_err());
    }
}
