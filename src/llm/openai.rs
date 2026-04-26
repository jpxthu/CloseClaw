//! OpenAI LLM Provider

use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider, Usage};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    messages: &'a [crate::llm::Message],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    #[allow(dead_code)]
    finish_reason: String,
    message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    #[allow(dead_code)]
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[allow(dead_code)]
pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }

    pub fn new_with_base_url(api_key: String, base_url: &str) -> Self {
        Self {
            api_key,
            base_url: base_url.to_string(),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Map HTTP status code to the appropriate LLM error.
    fn map_status_error(status: reqwest::StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            401 | 403 => LLMError::AuthFailed(format!("OpenAI API auth failed: {}", status)),
            429 => LLMError::RateLimitExceeded,
            _ => LLMError::ApiError(format!("OpenAI API error {}: {}", status, body)),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn models(&self) -> Vec<&str> {
        vec!["gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"]
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError> {
        let url = self.chat_url();

        let body = OpenAIRequest {
            model: &request.model,
            messages: &request.messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LLMError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            if status.as_u16() == 429 {
                return Err(LLMError::RateLimitExceeded);
            }
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let openai_resp: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| LLMError::ApiError(format!("failed to parse OpenAI response: {}", e)))?;

        let choice = openai_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LLMError::ApiError("no choices in OpenAI response".to_string()))?;

        Ok(ChatResponse {
            content: choice.message.content,
            model: openai_resp.model,
            usage: Usage {
                prompt_tokens: openai_resp.usage.prompt_tokens,
                completion_tokens: openai_resp.usage.completion_tokens,
                total_tokens: openai_resp.usage.total_tokens,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_chat_network_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/chat/completions")
            .with_status(401)
            .with_body(r#"{"error": {"message": "bad API key"}}"#)
            .create_async()
            .await;

        let provider = OpenAIProvider::new_with_base_url("bad-key".to_string(), &server.url());
        let request = ChatRequest {
            model: "gpt-4".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
        };
        let result = provider.chat(request).await;
        mock.assert_async().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            LLMError::AuthFailed(_) => {}
            _ => panic!("Expected AuthFailed"),
        }
    }

    #[test]
    fn test_openai_provider_new() {
        let provider = OpenAIProvider::new("test-key".to_string());
        assert_eq!(provider.api_key, "test-key");
        assert_eq!(provider.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn test_openai_provider_name() {
        let provider = OpenAIProvider::new("key".to_string());
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_openai_provider_models() {
        let provider = OpenAIProvider::new("key".to_string());
        let models = provider.models();
        assert_eq!(models, vec!["gpt-4", "gpt-4-turbo", "gpt-3.5-turbo"]);
    }

    #[test]
    fn test_chat_url() {
        let provider = OpenAIProvider::new("key".to_string());
        assert_eq!(
            provider.chat_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_map_status_error_401() {
        let err = OpenAIProvider::map_status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "bad key".to_string(),
        );
        match err {
            LLMError::AuthFailed(msg) => assert!(msg.contains("401")),
            _ => panic!("Expected AuthFailed"),
        }
    }

    #[test]
    fn test_map_status_error_403() {
        let err = OpenAIProvider::map_status_error(
            reqwest::StatusCode::FORBIDDEN,
            "forbidden".to_string(),
        );
        assert!(matches!(err, LLMError::AuthFailed(_)));
    }

    #[test]
    fn test_map_status_error_429() {
        let err = OpenAIProvider::map_status_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "rate limited".to_string(),
        );
        assert!(matches!(err, LLMError::RateLimitExceeded));
    }

    #[test]
    fn test_map_status_error_500() {
        let err = OpenAIProvider::map_status_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "server error".to_string(),
        );
        match err {
            LLMError::ApiError(msg) => assert!(msg.contains("500")),
            _ => panic!("Expected ApiError"),
        }
    }

    #[test]
    fn test_openai_request_serialization() {
        let msg = crate::llm::Message {
            role: "user".to_string(),
            content: "hello".to_string(),
        };
        let req = OpenAIRequest {
            model: "gpt-4",
            messages: &[msg],
            temperature: 0.7,
            max_tokens: Some(100),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("gpt-4"));
        assert!(json.contains("hello"));
        assert!(json.contains("100"));
    }

    #[test]
    fn test_openai_request_no_max_tokens() {
        let msg = crate::llm::Message {
            role: "user".to_string(),
            content: "hi".to_string(),
        };
        let req = OpenAIRequest {
            model: "gpt-4",
            messages: &[msg],
            temperature: 0.0,
            max_tokens: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("max_tokens"));
    }
}
