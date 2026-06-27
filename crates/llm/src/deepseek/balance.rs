//! DeepSeek balance / credit API.

use serde::Deserialize;

use crate::provider::ProviderError;

use super::DeepSeekProvider;

/// Response from GET /user/balance.
#[derive(Debug, Deserialize)]
pub struct DeepSeekBalanceResponse {
    /// Whether the account has sufficient balance for API calls.
    pub is_available: bool,
    /// Breakdown of balance by currency.
    #[serde(default)]
    pub balance_infos: Vec<DeepSeekBalanceInfo>,
}

/// A single balance entry (per currency).
#[derive(Debug, Deserialize)]
pub struct DeepSeekBalanceInfo {
    /// Currency code (e.g. "USD").
    pub currency: String,
    /// Total balance available.
    pub total_balance: f64,
    /// Balance granted by promotions / credits.
    pub granted_balance: f64,
    /// Balance topped up (paid) by the user.
    pub topped_up_balance: f64,
}

impl DeepSeekProvider {
    /// Fetch account balance info from the DeepSeek API.
    ///
    /// Sends `GET {base_url}/user/balance` with Bearer token
    /// authentication and returns the parsed balance response.
    pub async fn fetch_balance(
        &self,
        base_url: &str,
    ) -> Result<DeepSeekBalanceResponse, ProviderError> {
        let url = format!("{}/user/balance", base_url.trim_end_matches('/'));

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Self::map_status_error(status, body));
        }

        let balance: DeepSeekBalanceResponse =
            response.json().await.map_err(ProviderError::Reqwest)?;

        Ok(balance)
    }
}
