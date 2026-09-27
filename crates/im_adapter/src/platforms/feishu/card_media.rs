//! Card media processing — upload card image/file elements to Feishu.
//!
//! Extracted from `plugin.rs` to keep impl blocks within the 100-line limit.

use tracing::warn;

use super::outbound_media::{prepare_outbound_local_media, upload_file, upload_image};
use super::FeishuPlugin;
use crate::error::AdapterError;

impl FeishuPlugin {
    /// Process media elements in a card payload, uploading files to Feishu
    /// and replacing local paths with Feishu image/file keys.
    pub(super) async fn process_card_media(
        &self,
        payload: &mut serde_json::Value,
    ) -> Result<(), AdapterError> {
        let elements = match payload
            .get_mut("card")
            .and_then(|c| c.get_mut("elements"))
            .and_then(|e| e.as_array_mut())
        {
            Some(e) => e,
            None => return Ok(()),
        };

        for element in elements.iter_mut() {
            let tag = element.get("tag").and_then(|t| t.as_str()).unwrap_or("");
            match tag {
                "img" => self.process_card_img(element).await,
                "media" | "audio" | "file" => self.process_card_media_file(element).await,
                _ => {}
            }
        }
        Ok(())
    }

    /// Process a single `img` element: validate, copy to outbound, upload.
    async fn process_card_img(&self, element: &mut serde_json::Value) {
        let img_key = match element.get("img_key").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => return,
        };
        let media_store = &self.adapter.media_store;
        let workspace_dir = self.adapter.workspace_dir.as_deref();
        let outbound = match prepare_outbound_local_media(img_key, workspace_dir, media_store).await
        {
            Some(p) => p,
            None => return,
        };
        match upload_image(&self.adapter, &outbound).await {
            Ok(key) => {
                if let Some(obj) = element.as_object_mut() {
                    obj.insert("img_key".to_string(), serde_json::Value::String(key));
                }
            }
            Err(e) => warn!(error = %e, "Failed to upload image to Feishu"),
        }
    }

    /// Process a single `media` element: validate, copy to outbound, upload.
    async fn process_card_media_file(&self, element: &mut serde_json::Value) {
        let file_token = match element.get("file_token").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => return,
        };
        let media_store = &self.adapter.media_store;
        let workspace_dir = self.adapter.workspace_dir.as_deref();
        let outbound =
            match prepare_outbound_local_media(file_token, workspace_dir, media_store).await {
                Some(p) => p,
                None => return,
            };
        let filename = outbound
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        match upload_file(&self.adapter, &outbound, &filename).await {
            Ok(key) => {
                if let Some(obj) = element.as_object_mut() {
                    obj.insert("file_token".to_string(), serde_json::Value::String(key));
                }
            }
            Err(e) => warn!(error = %e, "Failed to upload file to Feishu"),
        }
    }
}
