//! GLM internal types and helper methods.

use serde::Deserialize;

use crate::provider::{ProviderError, Result};
use crate::types::{InternalResponse, RawContentBlock, RawUsage};

/// Minimum trimmed length for `reasoning_content` to be treated as a
/// genuine reasoning block. Values at or below this threshold are
/// demoted to plain Text (per design doc: "极短的 reasoning_content
/// 不视为推理块").
const MIN_REASONING_LENGTH: usize = 2;

use super::GlmProvider;

// ── Internal response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct GlmResponse {
    #[serde(default)]
    pub(crate) choices: Option<Vec<GlmChoice>>,
    #[serde(default)]
    pub(crate) usage: Option<GlmUsage>,
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) model: String,
    /// Top-level GLM error (e.g. code="1211", "1214")
    #[serde(default)]
    pub(crate) error: Option<GlmErrorBody>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmChoice {
    pub(crate) message: GlmMessage,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmMessage {
    #[allow(dead_code)]
    pub(crate) role: String,
    pub(crate) content: String,
    /// GLM reasoning content for reasoning models
    /// (glm-5.1, glm-4.7, etc.).
    /// When content is empty, the visible reply is
    /// in this field.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct GlmUsage {
    #[serde(default)]
    pub(crate) prompt_tokens: u32,
    #[serde(default)]
    pub(crate) completion_tokens: u32,
    #[serde(default)]
    pub(crate) total_tokens: u32,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<GlmCompletionTokensDetails>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<GlmPromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct GlmCompletionTokensDetails {
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct GlmPromptTokensDetails {
    #[serde(default)]
    pub(crate) cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlmErrorBody {
    pub(crate) code: String,
    pub(crate) message: String,
}

// ── GLM /models API types ────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct GlmModelsResponse {
    pub(crate) data: Vec<GlmModel>,
    #[serde(default)]
    pub(crate) object: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct GlmModel {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) object: Option<String>,
    #[serde(default)]
    pub(crate) created: Option<u64>,
    #[serde(default)]
    pub(crate) owned_by: Option<String>,
}

// ── GLM Usage / Quota API types ──────────────────────────────────────────────

/// GLM quota API response wrapper.
#[derive(Debug, Deserialize)]
pub struct GlmQuotaResponse {
    pub code: u16,
    pub msg: String,
    pub data: GlmQuotaData,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct GlmQuotaData {
    pub limits: Vec<GlmLimit>,
    #[serde(default)]
    pub level: String,
}

/// A single quota limit entry.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GlmLimit {
    #[serde(rename = "type")]
    pub(crate) limit_type: String,
    pub(crate) unit: u32,
    #[serde(default)]
    pub(crate) number: Option<u32>,
    #[serde(default)]
    pub(crate) usage: Option<u64>,
    #[serde(default)]
    pub(crate) remaining: Option<u64>,
    #[serde(default)]
    pub(crate) percentage: Option<u32>,
    #[serde(rename = "nextResetTime", default)]
    pub(crate) next_reset_time: Option<u64>,
    #[serde(rename = "usageDetails", default)]
    pub(crate) usage_details: Option<Vec<GlmUsageDetail>>,
}

/// Per-model usage breakdown.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GlmUsageDetail {
    #[serde(rename = "modelCode")]
    pub(crate) model_code: String,
    pub(crate) usage: u64,
}

// ── Error mapping & content extraction ─────────────────────────────────────

impl GlmProvider {
    /// Map GLM HTTP status error to ProviderError.
    pub(crate) fn map_status_error(status: reqwest::StatusCode, body: String) -> ProviderError {
        ProviderError::Legacy(format!("GLM API error {}: {}", status, body))
    }

    /// Map GLM error code to ProviderError.
    ///
    /// All GLM error codes map to `ProviderError::Legacy`
    /// with a descriptive message.
    pub(crate) fn map_glm_error(code: &str, message: &str) -> ProviderError {
        ProviderError::Legacy(format!("GLM API error {}: {}", code, message))
    }

    /// Extract visible content from a GLM message.
    ///
    /// Prefer `content`; if it's empty or pure whitespace,
    /// fall back to `reasoning_content`.
    ///
    /// Used in unit tests to verify extraction logic independently.
    #[allow(dead_code)]
    pub(crate) fn extract_content(msg: &GlmMessage) -> String {
        if !msg.content.trim().is_empty() {
            msg.content.trim().to_string()
        } else {
            msg.reasoning_content
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default()
        }
    }
}

// ── Response parsing ─────────────────────────────────────────────────────────

impl GlmProvider {
    /// Parse a GLM chat response into InternalResponse.
    pub(crate) fn parse_chat_response(api_resp: GlmResponse) -> Result<InternalResponse> {
        if let Some(ref err) = api_resp.error {
            return Err(Self::map_glm_error(&err.code, &err.message));
        }

        let msg = api_resp
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .map(|c| &c.message)
            .ok_or_else(|| ProviderError::Legacy("no choices in GLM response".to_string()))?;

        // ── Build content blocks per protocol-mapping spec ──
        //
        // 1. content non-empty + reasoning non-empty & long enough
        //    → Text(content) + Thinking(reasoning)
        // 2. content non-empty + reasoning empty/short
        //    → Text(content)
        // 3. content empty + reasoning non-empty
        //    → Text(reasoning)  (degrade — no Thinking block)
        // 4. both empty → empty Text block
        let content_is_empty = msg.content.trim().is_empty();
        let trimmed_reasoning = msg
            .reasoning_content
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        let mut content_blocks = Vec::new();
        if !content_is_empty {
            content_blocks.push(RawContentBlock::Text(msg.content.trim().to_string()));
        }
        if let Some(rc) = trimmed_reasoning {
            if content_is_empty {
                // Degraded: reasoning_content becomes visible text
                content_blocks.push(RawContentBlock::Text(rc.to_string()));
            } else if rc.len() > MIN_REASONING_LENGTH {
                // Both content and reasoning present and substantive
                content_blocks.push(RawContentBlock::Thinking {
                    thinking: rc.to_string(),
                    signature: None,
                });
            }
            // else: content present but reasoning too short — skip it
        }
        if content_blocks.is_empty() {
            content_blocks.push(RawContentBlock::Text(String::new()));
        }

        let usage = api_resp.usage.as_ref();
        Ok(InternalResponse {
            content_blocks,
            usage: RawUsage {
                prompt_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
                total_tokens: usage.map(|u| u.total_tokens),
                cache_read_tokens: usage
                    .and_then(|u| u.prompt_tokens_details.as_ref())
                    .and_then(|d| d.cached_tokens),
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
}
