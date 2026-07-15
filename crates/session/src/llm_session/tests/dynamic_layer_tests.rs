//! Unit tests for dynamic-layer per-request injection (Step 1.3 / 1.5).
//!
//! Covers:
//! - `build_llm_request` populates `system_static` / `system_dynamic`
//!   when a `DynamicPromptBuilder` is injected.
//! - Timestamps are generated per-request (not frozen at session creation).
//! - No system prompt → still generates dynamic layer.
//! - No builder → legacy `split_static_dynamic` fallback.
//! - `set_request_context` / `request_context` roundtrip.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::processor::{ContentBlock, UnifiedUsage};
use closeclaw_common::{
    DynamicPromptBuilder, DynamicPromptContext, InternalRequest, LLMError, LlmCaller,
    PromptOverrides, RequestContext, UnifiedResponse,
};

use super::super::ConversationSession;
use super::tmp_path;

// ── test doubles ──────────────────────────────────────────────────────────

/// A fake `DynamicPromptBuilder` that records each call's context and
/// returns configurable static/dynamic values.
struct FakeDynamicBuilder {
    static_val: Option<String>,
    dynamic_val: Option<String>,
    last_ctx: std::sync::Mutex<Option<CapturedContext>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CapturedContext {
    has_system_prompt: bool,
    sender_id: String,
    channel: String,
    timestamp: i64,
    workdir: String,
    appends_count: usize,
    session_mode: String,
    has_overrides: bool,
    user_input: Option<String>,
}

impl FakeDynamicBuilder {
    fn new(static_val: Option<String>, dynamic_val: Option<String>) -> Self {
        Self {
            static_val,
            dynamic_val,
            last_ctx: std::sync::Mutex::new(None),
        }
    }

    fn last_context(&self) -> Option<CapturedContext> {
        self.last_ctx.lock().unwrap().clone()
    }
}

impl DynamicPromptBuilder for FakeDynamicBuilder {
    fn build_prompt_parts(
        &self,
        context: &DynamicPromptContext,
    ) -> (Option<String>, Option<String>) {
        let captured = CapturedContext {
            has_system_prompt: context.system_prompt.is_some(),
            sender_id: context.ctx.sender_id.clone(),
            channel: context.ctx.channel.clone(),
            timestamp: context.ctx.timestamp,
            workdir: context.workdir.to_string_lossy().into_owned(),
            appends_count: context.system_appends.len(),
            session_mode: format!("{:?}", context.session_mode),
            has_overrides: context.overrides.is_some(),
            user_input: context.user_input.map(|s| s.to_string()),
        };
        *self.last_ctx.lock().unwrap() = Some(captured);
        (self.static_val.clone(), self.dynamic_val.clone())
    }
}

/// A fake `LlmCaller` that captures the last request.
struct FakeLlmCaller {
    last_request: std::sync::Mutex<Option<InternalRequest>>,
}

impl FakeLlmCaller {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            last_request: std::sync::Mutex::new(None),
        })
    }

    fn last_request(&self) -> Option<InternalRequest> {
        self.last_request.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmCaller for FakeLlmCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        *self.last_request.lock().unwrap() = Some(request);
        Ok(UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("ok".into())],
            usage: UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".into()),
            retry_attempts: 0,
        })
    }

    async fn call_streaming(
        &self,
        _request: InternalRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<
                        Item = Result<closeclaw_common::processor::StreamEvent, LLMError>,
                    > + Send,
            >,
        >,
        LLMError,
    > {
        Err(LLMError::ApiError("not tested".into()))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

/// With a `DynamicPromptBuilder` injected, `build_llm_request` must
/// populate `system_static` and `system_dynamic` from the builder.
#[tokio::test]
async fn test_build_llm_request_with_builder_populates_system_fields() {
    let mut session = ConversationSession::new("s_dl1".into(), "m".into(), tmp_path());
    session.replace_system_prompt("base prompt");
    session.set_dynamic_prompt_builder(Arc::new(FakeDynamicBuilder::new(
        Some("static_layer".into()),
        Some("dynamic_layer".into()),
    )));
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake.last_request().unwrap();
    assert_eq!(req.system_static.as_deref(), Some("static_layer"));
    assert_eq!(req.system_dynamic.as_deref(), Some("dynamic_layer"));
}

/// Multiple requests produce different timestamps, proving the dynamic
/// layer is generated per-request, not frozen at session creation.
#[tokio::test]
async fn test_dynamic_layer_timestamp_is_per_request() {
    let mut session = ConversationSession::new("s_dl2".into(), "m".into(), tmp_path());
    let builder = Arc::new(FakeDynamicBuilder::new(None, Some("dyn".into())));
    let builder_ref = builder.clone();
    session.set_dynamic_prompt_builder(builder);
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    // First request: timestamp = 1000
    session.set_request_context(RequestContext {
        sender_id: "u1".into(),
        channel: "feishu".into(),
        timestamp: 1000,
    });
    let _ = session.invoke_llm("first").await.unwrap();
    let ctx1 = builder_ref.last_context().unwrap();
    assert_eq!(ctx1.timestamp, 1000);

    // Second request: timestamp = 2000
    session.set_request_context(RequestContext {
        sender_id: "u1".into(),
        channel: "feishu".into(),
        timestamp: 2000,
    });
    let _ = session.invoke_llm("second").await.unwrap();
    let ctx2 = builder_ref.last_context().unwrap();
    assert_eq!(ctx2.timestamp, 2000);

    // Timestamps differ → dynamic layer is per-request
    assert_ne!(ctx1.timestamp, ctx2.timestamp);
}

/// When no system prompt is set, the dynamic layer is still generated
/// (the builder receives `system_prompt: None` and can still produce
/// dynamic content).
#[tokio::test]
async fn test_dynamic_layer_no_system_prompt_still_generated() {
    let mut session = ConversationSession::new("s_dl3".into(), "m".into(), tmp_path());
    // No system_prompt set
    assert!(session.system_prompt().is_none());
    let builder = Arc::new(FakeDynamicBuilder::new(None, Some("dyn_only".into())));
    let builder_ref = builder.clone();
    session.set_dynamic_prompt_builder(builder);
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hello").await.unwrap();
    let ctx = builder_ref.last_context().unwrap();
    assert!(
        !ctx.has_system_prompt,
        "builder should see None system_prompt"
    );

    let req = fake.last_request().unwrap();
    // system_static is None (no prompt to split), dynamic is populated
    assert!(req.system_static.is_none());
    assert_eq!(req.system_dynamic.as_deref(), Some("dyn_only"));
}

/// Without a `DynamicPromptBuilder`, the session falls back to
/// `split_static_dynamic` on the stored prompt.
#[tokio::test]
async fn test_build_llm_request_no_builder_fallback() {
    let mut session = ConversationSession::new("s_dl4".into(), "m".into(), tmp_path());
    session.replace_system_prompt("static part\n__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__\ndynamic part");
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake.last_request().unwrap();
    assert_eq!(req.system_static.as_deref(), Some("static part"));
    assert_eq!(req.system_dynamic.as_deref(), Some("dynamic part"));
}

/// Without a builder and no boundary marker, the full prompt becomes
/// `system_static` with no dynamic layer.
#[tokio::test]
async fn test_build_llm_request_no_builder_no_marker_all_static() {
    let mut session = ConversationSession::new("s_dl5".into(), "m".into(), tmp_path());
    session.replace_system_prompt("entire prompt without marker");
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake.last_request().unwrap();
    assert_eq!(
        req.system_static.as_deref(),
        Some("entire prompt without marker")
    );
    assert!(req.system_dynamic.is_none());
}

/// Without a builder and no system prompt, both fields are None.
#[tokio::test]
async fn test_build_llm_request_no_builder_no_prompt_both_none() {
    let mut session = ConversationSession::new("s_dl6".into(), "m".into(), tmp_path());
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hello").await.unwrap();
    let req = fake.last_request().unwrap();
    assert!(req.system_static.is_none());
    assert!(req.system_dynamic.is_none());
}

/// Builder receives correct context fields (sender_id, channel, workdir, appends).
#[tokio::test]
async fn test_dynamic_builder_receives_correct_context() {
    let mut session = ConversationSession::new("s_dl7".into(), "m".into(), tmp_path());
    session.add_system_append("note1".to_string());
    session.add_system_append("note2".to_string());
    let builder = Arc::new(FakeDynamicBuilder::new(None, None));
    let builder_ref = builder.clone();
    session.set_dynamic_prompt_builder(builder);
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    session.set_request_context(RequestContext {
        sender_id: "ou_sender".into(),
        channel: "telegram".into(),
        timestamp: 5555,
    });
    let _ = session.invoke_llm("test").await.unwrap();

    let ctx = builder_ref.last_context().unwrap();
    assert_eq!(ctx.sender_id, "ou_sender");
    assert_eq!(ctx.channel, "telegram");
    assert_eq!(ctx.timestamp, 5555);
    assert_eq!(ctx.appends_count, 2);
}

/// `set_request_context` / `request_context` roundtrip.
#[test]
fn test_request_context_set_get_roundtrip() {
    let session = ConversationSession::new("s_rc".into(), "m".into(), tmp_path());
    // Default
    let default = session.request_context();
    assert!(default.sender_id.is_empty());
    assert_eq!(default.timestamp, 0);

    // Set
    session.set_request_context(RequestContext {
        sender_id: "ou_new".into(),
        channel: "slack".into(),
        timestamp: 7777,
    });
    let got = session.request_context();
    assert_eq!(got.sender_id, "ou_new");
    assert_eq!(got.channel, "slack");
    assert_eq!(got.timestamp, 7777);
}

/// Builder receives user_input from the last user message.
#[tokio::test]
async fn test_dynamic_builder_receives_user_input() {
    let mut session = ConversationSession::new("s_dl8".into(), "m".into(), tmp_path());
    let builder = Arc::new(FakeDynamicBuilder::new(None, None));
    let builder_ref = builder.clone();
    session.set_dynamic_prompt_builder(builder);
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    session.set_request_context(RequestContext::default());
    let _ = session.invoke_llm("fix the compile error").await.unwrap();

    let ctx = builder_ref.last_context().unwrap();
    assert_eq!(ctx.user_input.as_deref(), Some("fix the compile error"));
}

/// Builder receives prompt_overrides when set on the session.
#[tokio::test]
async fn test_dynamic_builder_receives_overrides() {
    let mut session = ConversationSession::new("s_dl9".into(), "m".into(), tmp_path());
    session.set_prompt_overrides(Some(PromptOverrides {
        override_prompt: Some("custom".into()),
        agent_prompt: None,
        custom_prompt: None,
    }));
    let builder = Arc::new(FakeDynamicBuilder::new(None, None));
    let builder_ref = builder.clone();
    session.set_dynamic_prompt_builder(builder);
    let fake = FakeLlmCaller::new();
    let caller_ref: Arc<dyn LlmCaller> = fake.clone();
    session.set_llm_caller(caller_ref);

    let _ = session.invoke_llm("hi").await.unwrap();
    let ctx = builder_ref.last_context().unwrap();
    assert!(ctx.has_overrides);
}
