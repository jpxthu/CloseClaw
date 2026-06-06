//! Scenario definitions for `FakeProvider`.

use std::time::Duration;

use crate::llm::provider::ProviderError;
use crate::llm::types::RawUsage;
use crate::llm::{LLMError, Usage};

/// A scenario defines what the next `chat()` / `send()` call should return.
#[derive(Debug)]
pub enum Scenario {
    /// Respond with a successful response.
    Ok {
        content: String,
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Respond with an error, optionally with usage metrics.
    Err {
        error: ProviderError,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Sleep for the given duration then behave as the wrapped scenario.
    Delay {
        duration: Duration,
        inner: Box<Scenario>,
    },
}

impl Scenario {
    /// Shortcut: a successful scenario with default usage metrics.
    pub fn ok(content: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Ok {
            content: content.into(),
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: 10,
        }
    }

    /// Shortcut: an error scenario with default zero usage.
    pub fn err(error: ProviderError) -> Self {
        Self::Err {
            error,
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    /// Error scenario with custom usage metrics.
    pub fn err_with(error: ProviderError, prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self::Err {
            error,
            prompt_tokens,
            completion_tokens,
        }
    }

    /// Shortcut: a delayed scenario — sleeps for `duration` then resolves as `inner`.
    pub fn delay(duration: Duration, inner: Scenario) -> Self {
        Self::Delay {
            duration,
            inner: Box::new(inner),
        }
    }

    /// Returns usage as [`RawUsage`] (for the new Provider trait).
    pub(crate) fn raw_usage(&self) -> RawUsage {
        match self {
            Self::Ok {
                prompt_tokens,
                completion_tokens,
                ..
            } => RawUsage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: Some(*prompt_tokens + *completion_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            Self::Err {
                prompt_tokens,
                completion_tokens,
                ..
            } => RawUsage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: Some(*prompt_tokens + *completion_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            Self::Delay { inner, .. } => inner.raw_usage(),
        }
    }

    /// Returns usage as legacy [`Usage`] (for LLMProvider compat).
    pub(crate) fn usage(&self) -> Usage {
        match self {
            Self::Ok {
                prompt_tokens,
                completion_tokens,
                ..
            } => Usage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: *prompt_tokens + *completion_tokens,
            },
            Self::Err {
                prompt_tokens,
                completion_tokens,
                ..
            } => Usage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                total_tokens: *prompt_tokens + *completion_tokens,
            },
            Self::Delay { inner, .. } => inner.usage(),
        }
    }

    pub(crate) fn content(&self) -> String {
        match self {
            Self::Ok { content, .. } => content.clone(),
            Self::Err { .. } => String::new(),
            Self::Delay { inner, .. } => inner.content(),
        }
    }

    pub(crate) fn model(&self) -> String {
        match self {
            Self::Ok { model, .. } => model.clone(),
            Self::Err { .. } => String::new(),
            Self::Delay { inner, .. } => inner.model(),
        }
    }

    /// Convert Scenario error to legacy LLMError for LLMProvider compat.
    pub(crate) fn as_llm_error(&self) -> Option<LLMError> {
        match self {
            Self::Err { error, .. } => {
                // Convert ProviderError to LLMError without requiring Clone on ProviderError
                let err_str = format!("{}", error);
                Some(LLMError::ApiError(err_str))
            }
            _ => None,
        }
    }
}

impl Clone for Scenario {
    fn clone(&self) -> Self {
        match self {
            Self::Ok {
                content,
                model,
                prompt_tokens,
                completion_tokens,
            } => Self::Ok {
                content: content.clone(),
                model: model.clone(),
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
            },
            Self::Err {
                error,
                prompt_tokens,
                completion_tokens,
            } => {
                // ProviderError doesn't impl Clone; reconstruct from Display
                let err_str = format!("{}", error);
                Self::Err {
                    error: ProviderError::Legacy(err_str),
                    prompt_tokens: *prompt_tokens,
                    completion_tokens: *completion_tokens,
                }
            }
            Self::Delay { duration, inner } => Self::Delay {
                duration: *duration,
                inner: inner.clone(),
            },
        }
    }
}
