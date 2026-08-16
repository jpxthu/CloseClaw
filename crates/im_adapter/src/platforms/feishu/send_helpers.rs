//! Feishu adapter send helpers — `send_msg` and `try_fallback_to_text`.
//!
//! Extracted from `adapter.rs` to keep the main adapter file under the
//! 1000-line hard limit.  Both functions are thin wrappers that live on
//! [`FeishuAdapter`] but are defined here for modularity.

use crate::error::AdapterError;

use super::adapter::{FeishuAdapter, SendResponse};

#[derive(serde::Serialize)]
struct SendRequest<'a> {
    receive_id: &'a str,
    msg_type: &'a str,
    content: &'a str,
}

impl FeishuAdapter {
    /// Low-level: POST a message to the Feishu send API.
    pub(crate) async fn send_msg(
        &self,
        token: &str,
        receive_id: &str,
        msg_type: &str,
        content: &str,
        root_id: Option<&str>,
    ) -> Result<SendResponse, AdapterError> {
        let payload = SendRequest {
            receive_id,
            msg_type,
            content,
        };
        let mut url = format!(
            "{}/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );
        if let Some(rid) = root_id {
            let enc: String =
                url::form_urlencoded::byte_serialize(rid.as_bytes()).collect();
            url = format!("{}&root_id={}", url, enc);
        }
        self.http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(
                    receive_id = %receive_id,
                    error = %e,
                    "Feishu send request failed"
                );
                AdapterError::SendFailed(e.to_string())
            })?
            .json()
            .await
            .map_err(|e| {
                tracing::warn!(
                    receive_id = %receive_id,
                    error = %e,
                    "Feishu send response parse failed"
                );
                AdapterError::SendFailed(e.to_string())
            })
    }

    /// Attempt to send the card's text content as a plain text message.
    ///
    /// Used when `send_card_json` fails with a capability error
    /// (e.g. unsupported `select_static` component). Extracts
    /// markdown/plain_text content from the card payload via
    /// `renderer::extract_card_plain_text` and sends it through the
    /// text message API.
    pub(crate) async fn try_fallback_to_text(
        &self,
        chat_id: &str,
        card_json: &str,
        token: &str,
        root_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let card_value: serde_json::Value =
            serde_json::from_str(card_json).unwrap_or(serde_json::Value::Null);
        let plain_text = super::renderer::extract_card_plain_text(&card_value);
        if plain_text.is_empty() {
            tracing::warn!(
                receive_id = %chat_id,
                "Capability fallback: no extractable text in card"
            );
            return Ok(());
        }
        let text_content = serde_json::json!({"text": &plain_text}).to_string();
        let resp = self
            .send_msg(token, chat_id, "text", &text_content, root_id)
            .await?;
        if resp.code != 0 {
            tracing::warn!(
                receive_id = %chat_id,
                code = resp.code,
                msg = %resp.msg,
                "Capability fallback: text send failed"
            );
            return Err(AdapterError::SendFailed(format!(
                "Feishu fallback text send error {}: {}",
                resp.code, resp.msg
            )));
        }
        Ok(())
    }
}
