//! Feishu lark-cli payload types — webhook event and CLI response structs.
use serde::Deserialize;

// Webhook event types

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuEvent {
    #[allow(dead_code)]
    pub(crate) schema: String,
    pub(crate) header: FeishuHeader,
    pub(crate) event: FeishuMessageEvent,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuHeader {
    #[allow(dead_code)]
    pub(crate) event_id: String,
    #[allow(dead_code)]
    pub(crate) event_type: String,
    #[allow(dead_code)]
    pub(crate) create_time: String,
    #[allow(dead_code)]
    pub(crate) token: String,
    pub(crate) app_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuMessageEvent {
    #[serde(default)]
    pub(crate) message_id: Option<String>,
    pub(crate) sender: FeishuSender,
    pub(crate) content: String,
    pub(crate) chat_id: String,
    #[serde(default)]
    pub(crate) chat_type: Option<String>,
    pub(crate) message_type: String,
    #[serde(default)]
    pub(crate) thread_id: Option<String>,
    #[serde(default)]
    pub(crate) root_id: Option<String>,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuSender {
    pub(crate) sender_id: FeishuSenderId,
    #[allow(dead_code)]
    pub(crate) sender_type: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FeishuSenderId {
    pub(crate) open_id: String,
}

/// Card action event payload (`card.action.trigger`).
///
/// Preserved for future use when card.action.trigger is enabled
/// (设计文档「暂缓能力」— card.action.trigger).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardActionEvent {
    pub(crate) operator: FeishuCardOperator,
    pub(crate) token: String,
    pub(crate) action: FeishuCardAction,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardOperator {
    pub(crate) open_id: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct FeishuCardAction {
    pub(crate) value: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub(crate) tag: Option<String>,
}

// lark-cli response types for message retrieval

#[derive(Deserialize)]
pub(crate) struct FeishuMsgBody {
    pub(crate) content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FeishuMsgItem {
    pub(crate) msg_type: Option<String>,
    pub(crate) body: Option<FeishuMsgBody>,
}

#[derive(Deserialize)]
pub(crate) struct FeishuGetMessageResponse {
    pub(crate) code: i32,
    pub(crate) msg: String,
    pub(crate) items: Option<Vec<FeishuMsgItem>>,
}

// lark-cli response type for chat info

#[derive(Deserialize)]
pub(crate) struct FeishuChatResponse {
    pub(crate) code: i32,
    pub(crate) msg: String,
    pub(crate) data: Option<FeishuChatData>,
}

#[derive(Deserialize)]
pub(crate) struct FeishuChatData {
    pub(crate) name: Option<String>,
}

// lark-cli response type for media download URL

#[derive(Deserialize)]
pub(crate) struct ResourceResp {
    pub(crate) code: i32,
    pub(crate) msg: String,
    pub(crate) data: Option<serde_json::Value>,
}
