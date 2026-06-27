//! GLM usage / quota API.

use crate::provider::ProviderError;

use super::types::GlmQuotaResponse;
use super::GlmProvider;

impl GlmProvider {
    /// Fetch GLM account quota / usage info from the
    /// Usage API.
    ///
    /// The `base_url` should be the GLM API base
    /// (e.g. `https://open.bigmodel.cn/api`);
    /// this method appends `/paas/quota` internally.
    pub async fn fetch_usage(
        &self,
        base_url: &str,
    ) -> std::result::Result<GlmQuotaResponse, ProviderError> {
        let url = format!("{}/paas/quota", base_url.trim_end_matches('/'));

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let quota: GlmQuotaResponse = response.json().await.map_err(ProviderError::Reqwest)?;

        if !quota.success {
            return Err(ProviderError::Legacy(format!(
                "GLM quota API error {}: {}",
                quota.code, quota.msg
            )));
        }

        Ok(quota)
    }
}
