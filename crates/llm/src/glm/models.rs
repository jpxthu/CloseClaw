//! GLM model listing implementation.

use std::time::Duration;

use async_trait::async_trait;
use tokio::time::timeout;

use crate::provider::ProviderError;
use crate::{ModelInfo, ModelLister};

use super::types::{GlmModel, GlmModelsResponse};
use super::GlmProvider;

/// Build ModelInfo from a raw GLM model entry.
fn build_model_info(m: GlmModel) -> ModelInfo {
    use crate::InputType;
    let model_id = m.id.clone();
    let kb = crate::ProviderModelKnowledge::new();
    let params = kb.find("glm", &model_id);
    let (context_window, max_tokens, default_temperature, reasoning, input_types) = match params {
        Some(p) => (
            p.context_window,
            p.max_tokens,
            Some(p.default_temperature),
            p.reasoning,
            p.input_types,
        ),
        None => (128_000, 8_192, Some(0.7), false, vec![InputType::Text]),
    };
    let name = format!(
        "GLM {}",
        model_id
            .trim_start_matches("glm-")
            .trim_start_matches("GLM-")
    );
    ModelInfo {
        id: model_id,
        name,
        context_window,
        max_tokens,
        default_temperature,
        reasoning,
        input_types,
    }
}

impl GlmProvider {
    /// Shared model-listing logic used by both `Provider`
    /// and `ModelLister`.
    pub(crate) async fn fetch_model_list_impl(
        &self,
        bearer_token: &str,
    ) -> std::result::Result<Vec<ModelInfo>, ProviderError> {
        let models_url = self
            .base_url
            .replace("/coding/paas/v4/chat/completions", "/paas/v4/models")
            .replace("/chat/completions", "/models");

        let response = match timeout(
            Duration::from_secs(10),
            self.client
                .get(&models_url)
                .header("Authorization", format!("Bearer {}", bearer_token))
                .send(),
        )
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(ProviderError::Reqwest(e)),
            Err(_) => {
                return Err(ProviderError::Legacy(
                    "fetch_model_list timed out \
                     after 10s"
                        .to_string(),
                ));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let api_resp: GlmModelsResponse = response.json().await.map_err(ProviderError::Reqwest)?;

        let models: Vec<ModelInfo> = api_resp.data.into_iter().map(build_model_info).collect();

        Ok(models)
    }
}

#[async_trait]
impl ModelLister for GlmProvider {
    async fn fetch_model_list(
        &self,
        bearer_token: &str,
    ) -> std::result::Result<Vec<ModelInfo>, crate::LLMError> {
        self.fetch_model_list_impl(bearer_token)
            .await
            .map_err(|e| crate::LLMError::ApiError(e.to_string()))
    }
}
