//! Integration tests for `execute_compact` with mock LLM client.
//!
//! These tests verify the complete `execute_compact` flow including
//! token counting with precise stats vs. pure character estimation.

use crate::compaction::execute_compact;
use crate::fallback::{FallbackClient, ModelEntry};
use crate::provider::Provider;
use crate::types::{InternalResponse, ProtocolId, RawContentBlock, RawUsage};
use crate::Message;
use closeclaw_common::RunningStats;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock provider that returns a summary-tagged response
// ---------------------------------------------------------------------------

struct SummaryProvider {
    summary_text: String,
}

impl SummaryProvider {
    fn new(summary_text: impl Into<String>) -> Self {
        Self {
            summary_text: summary_text.into(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for SummaryProvider {
    fn id(&self) -> &str {
        "mock-summary"
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn api_key(&self) -> &str {
        ""
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }

    fn http_client(&self) -> &reqwest::Client {
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        CLIENT.get_or_init(reqwest::Client::new)
    }

    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        static HEADERS: std::sync::OnceLock<reqwest::header::HeaderMap> =
            std::sync::OnceLock::new();
        HEADERS.get_or_init(reqwest::header::HeaderMap::new)
    }

    async fn send(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(format!(
                "<summary>{}</summary>",
                self.summary_text
            ))],
            usage: RawUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: Some(150),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }

    async fn send_streaming(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        unimplemented!("streaming not needed in compaction tests")
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a `FallbackClient` backed by a mock provider that returns
/// the given summary text wrapped in `<summary>` tags.
async fn mock_llm(summary_text: &str) -> FallbackClient {
    let registry = Arc::new(crate::LLMRegistry::new());
    let provider: Arc<dyn Provider> = Arc::new(SummaryProvider::new(summary_text));
    registry
        .register("mock-summary".to_string(), provider)
        .await;
    FallbackClient::new(
        registry,
        vec![ModelEntry {
            provider: "mock-summary".to_string(),
            model: "test-model".to_string(),
        }],
    )
}

/// Build a small set of conversation messages for testing.
fn test_messages() -> Vec<Message> {
    vec![
        Message {
            role: "user".into(),
            content: "What is the capital of France?".into(),
        },
        Message {
            role: "assistant".into(),
            content: "The capital of France is Paris, a vibrant city known for art and culture."
                .into(),
        },
        Message {
            role: "user".into(),
            content: "Tell me more about the Eiffel Tower.".into(),
        },
        Message {
            role: "assistant".into(),
            content: "The Eiffel Tower was built in 1889 and stands 330 meters tall.".into(),
        },
    ]
}

/// Compute the expected `original_tokens` when stats cover the first
/// `stats.request_count` messages via precise usage, and the rest via
/// character estimation.
fn expected_with_stats(messages: &[Message], stats: &RunningStats, chars_per_token: f64) -> usize {
    use closeclaw_session::compaction::{estimate_tokens, CompactionMessage};

    let compaction_msgs: Vec<CompactionMessage> = messages
        .iter()
        .map(|m| CompactionMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    let precise = stats.total_tokens as usize;
    let start = (stats.request_count as usize).min(compaction_msgs.len());
    let remaining_tokens: usize = compaction_msgs[start..]
        .iter()
        .map(|m| estimate_tokens(&m.content, chars_per_token))
        .sum();
    precise + remaining_tokens
}

/// Compute the expected `original_tokens` using pure character estimation
/// (no stats).
fn expected_without_stats(messages: &[Message], chars_per_token: f64) -> usize {
    use closeclaw_session::compaction::{estimate_messages_tokens, CompactionMessage};

    let compaction_msgs: Vec<CompactionMessage> = messages
        .iter()
        .map(|m| CompactionMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    estimate_messages_tokens(&compaction_msgs, chars_per_token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When `stats` is provided with `request_count > 0`, `original_tokens`
/// equals `stats.total_tokens` + character-based estimation for messages
/// beyond the counted set.
#[tokio::test]
async fn test_execute_compact_with_stats_uses_precise_tokens() {
    let llm = mock_llm("Discussed French geography and Eiffel Tower.").await;
    let messages = test_messages();
    let chars_per_token = 0.25;

    let mut stats = RunningStats::new();
    stats.total_tokens = 5_000;
    stats.request_count = 2; // covers first 2 messages

    let result = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        Some(&stats),
    )
    .await
    .expect("execute_compact should succeed");

    let expected = expected_with_stats(&messages, &stats, chars_per_token);
    assert_eq!(
        result.original_tokens, expected,
        "with stats: original_tokens should be stats.total_tokens + remaining char estimation"
    );
    // original_tokens must exceed stats.total_tokens alone
    assert!(
        result.original_tokens > stats.total_tokens as usize,
        "original_tokens ({}) should exceed stats.total_tokens ({})",
        result.original_tokens,
        stats.total_tokens
    );
    assert_eq!(
        result.compacted_tokens, result.after_token_count,
        "compacted_tokens should equal after_token_count"
    );
}

/// When `stats` is `None`, `original_tokens` uses pure character estimation
/// for all messages (no stats contribution).
#[tokio::test]
async fn test_execute_compact_without_stats_uses_char_estimation() {
    let llm = mock_llm("Discussed French geography and Eiffel Tower.").await;
    let messages = test_messages();
    let chars_per_token = 0.25;

    let result = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        None,
    )
    .await
    .expect("execute_compact should succeed");

    let expected = expected_without_stats(&messages, chars_per_token);
    assert_eq!(
        result.original_tokens, expected,
        "without stats: original_tokens should be pure char estimation"
    );
    // Verify it is a reasonable number (non-zero, within order of magnitude)
    assert!(
        result.original_tokens > 0,
        "original_tokens should be non-zero"
    );
    assert!(
        result.original_tokens < 1000,
        "original_tokens ({}) should be reasonable for short messages",
        result.original_tokens
    );
    assert_eq!(
        result.compacted_tokens, result.after_token_count,
        "compacted_tokens should equal after_token_count"
    );
}

/// When `stats` covers ALL messages, `original_tokens` equals exactly
/// `stats.total_tokens` (no remaining messages to estimate).
#[tokio::test]
async fn test_execute_compact_stats_covers_all_messages() {
    let llm = mock_llm("All messages covered by stats.").await;
    let messages = test_messages();
    let chars_per_token = 0.25;

    let mut stats = RunningStats::new();
    stats.total_tokens = 12_000;
    stats.request_count = 4; // covers all 4 messages

    let result = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        Some(&stats),
    )
    .await
    .expect("execute_compact should succeed");

    assert_eq!(
        result.original_tokens, 12_000,
        "when stats covers all messages, original_tokens == stats.total_tokens"
    );
}

/// When `stats.request_count == 0`, execution falls back to pure character
/// estimation — identical to the `None` path.
#[tokio::test]
async fn test_execute_compact_stats_zero_request_count_falls_back() {
    let llm = mock_llm("Fallback case.").await;
    let messages = test_messages();
    let chars_per_token = 0.25;

    let mut stats = RunningStats::new();
    stats.total_tokens = 99_999; // irrelevant when request_count == 0
    stats.request_count = 0;

    let result_with_zero = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        Some(&stats),
    )
    .await
    .expect("execute_compact should succeed");

    let result_without = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        None,
    )
    .await
    .expect("execute_compact should succeed");

    let expected = expected_without_stats(&messages, chars_per_token);
    assert_eq!(
        result_with_zero.original_tokens, expected,
        "request_count=0 should fall back to char estimation"
    );
    assert_eq!(
        result_with_zero.original_tokens, result_without.original_tokens,
        "request_count=0 and None should produce identical results"
    );
}

/// `after_token_count` is consistent regardless of whether stats are
/// provided — only `original_tokens` differs between the two modes.
#[tokio::test]
async fn test_execute_compact_after_token_count_consistent() {
    let llm = mock_llm("Same summary for both paths.").await;
    let messages = test_messages();
    let chars_per_token = 0.25;

    let mut stats = RunningStats::new();
    stats.total_tokens = 5_000;
    stats.request_count = 2;

    let result_with = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        Some(&stats),
    )
    .await
    .expect("execute_compact should succeed");

    let result_without = execute_compact(
        &messages,
        &llm,
        "test-model",
        None,
        false,
        chars_per_token,
        None,
    )
    .await
    .expect("execute_compact should succeed");

    assert_eq!(
        result_with.after_token_count, result_without.after_token_count,
        "after_token_count should be identical for both modes"
    );
    // boundary_message contains a timestamp, so verify content structure
    assert!(result_with
        .boundary_message
        .contains("Same summary for both paths."));
    assert!(result_without
        .boundary_message
        .contains("Same summary for both paths."));
    assert!(result_with.boundary_message.contains("Session Compaction"));
    assert!(result_without
        .boundary_message
        .contains("Session Compaction"));
    assert_eq!(
        result_with.compacted_tokens, result_without.compacted_tokens,
        "compacted_tokens should be identical for both modes"
    );
}

/// Empty messages produce an error (no LLM call made).
#[tokio::test]
async fn test_execute_compact_empty_messages() {
    let llm = mock_llm("should not be called").await;
    let err = execute_compact(&[], &llm, "test-model", None, false, 0.25, None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            closeclaw_session::compaction::CompactionError::EmptyMessages
        ),
        "expected EmptyMessages, got: {err}"
    );
}
