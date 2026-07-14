//! LLM Plugin — hook points for request/response interception and stream event
//! processing.
//!
//! [`ModelPlugin`] provides three hook surfaces:
//! - [`before_request`](ModelPlugin::before_request) – called before a request is
//!   sent to the provider;
//! - [`after_response`](ModelPlugin::after_response) – called after the provider
//!   response has been normalised by an interpreter;
//! - [`on_stream_event`](ModelPlugin::on_stream_event) – called for every streaming
//!   event, and may return a modified or suppressed event.
//!
//! [`PluginPipeline`] sequences zero or more plugins. Execution is **sequential**
//! and **short-circuiting**: once a hook indicates the pipeline should stop
//! processing (as defined by each hook's return semantics), remaining plugins are
//! not invoked.

use crate::types::{InternalRequest, StreamEvent, UnifiedResponse};

/// A single plugin that may intercept and mutate the request / response flow.
///
/// All implementations must be `Send + Sync` so the pipeline can be shared across
/// concurrent calls.
pub trait ModelPlugin: Send + Sync {
    /// Returns the plugin's identifier.
    fn name(&self) -> &str;

    /// Called immediately before the `InternalRequest` is passed to the protocol
    /// layer for request building.
    ///
    /// The default implementation is a no-op.
    fn before_request(&self, _request: &mut InternalRequest) {}

    /// Called after the normalised [`UnifiedResponse`] is available, before it is
    /// returned to the caller.
    ///
    /// The default implementation is a no-op.
    fn after_response(&self, _response: &mut UnifiedResponse) {}

    /// Called for each streaming [`StreamEvent`] as it arrives from the provider.
    ///
    /// Return `Some(event)` to forward the (possibly modified) event downstream;
    /// return `None` to suppress the event entirely.
    ///
    /// The default implementation forwards all events unchanged.
    fn on_stream_event(&self, event: &StreamEvent) -> Option<StreamEvent> {
        Some(event.clone())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PluginPipeline
// ─────────────────────────────────────────────────────────────────────────────

/// An ordered sequence of [`ModelPlugin`] instances.
///
/// Plugins are invoked in registration order. The pipeline short-circuits when
/// [`on_stream_event`](ModelPlugin::on_stream_event) returns `None` – no further
/// plugins receive that event. `before_request` and `after_response` always run
/// all plugins regardless of earlier results, because they do not have a
/// short-circuit signal.
///
/// # Example
/// ```
/// # use closeclaw_llm::plugin::{ModelPlugin, PluginPipeline};
/// # use closeclaw_llm::types::{InternalRequest, StreamEvent, UnifiedResponse};
/// struct EchoPlugin;
/// impl ModelPlugin for EchoPlugin {
///     fn name(&self) -> &str { "echo" }
/// }
///
/// let pipeline = PluginPipeline::new();
/// assert!(pipeline.is_empty());
/// ```
#[derive(Default)]
pub struct PluginPipeline {
    plugins: Vec<Box<dyn ModelPlugin>>,
}

impl std::fmt::Debug for PluginPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginPipeline")
            .field("len", &self.plugins.len())
            .finish()
    }
}

impl PluginPipeline {
    /// Creates an empty pipeline.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Appends a plugin to the end of the pipeline.
    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, plugin: Box<dyn ModelPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Returns the number of plugins currently registered.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Returns `true` if the pipeline contains no plugins.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Runs `before_request` on every registered plugin, in order.
    pub fn before_request(&self, request: &mut InternalRequest) {
        for plugin in &self.plugins {
            plugin.before_request(request);
        }
    }

    /// Runs `after_response` on every registered plugin, in order.
    pub fn after_response(&self, response: &mut UnifiedResponse) {
        for plugin in &self.plugins {
            plugin.after_response(response);
        }
    }

    /// Runs `on_stream_event` on every registered plugin in order, short-
    /// circuiting when a plugin returns `None`.
    ///
    /// Returns `Some(event)` if at least one plugin forwarded the event,
    /// `None` if a plugin suppressed it.
    pub fn on_stream_event(&self, event: &StreamEvent) -> Option<StreamEvent> {
        let mut result: Option<StreamEvent> = Some(event.clone());
        for plugin in &self.plugins {
            if let Some(ref e) = result {
                result = plugin.on_stream_event(e);
            }
        }
        result
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentBlock, ContentBlockType, UnifiedUsage};
    use closeclaw_session::persistence::ReasoningLevel;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Shared counters for a recording plugin, accessible after the plugin
    /// is moved into the pipeline.
    #[derive(Default)]
    struct CallCounts {
        before: AtomicUsize,
        after: AtomicUsize,
        stream: AtomicUsize,
    }

    /// A plugin that records how many times each hook is called.
    struct RecordingPlugin {
        name: String,
        counts: Arc<CallCounts>,
    }

    impl RecordingPlugin {
        fn new(name: &str) -> (Self, Arc<CallCounts>) {
            let counts = Arc::new(CallCounts::default());
            (
                Self {
                    name: name.to_string(),
                    counts: Arc::clone(&counts),
                },
                counts,
            )
        }
    }

    impl ModelPlugin for RecordingPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn before_request(&self, _request: &mut InternalRequest) {
            self.counts.before.fetch_add(1, Ordering::Relaxed);
        }

        fn after_response(&self, _response: &mut UnifiedResponse) {
            self.counts.after.fetch_add(1, Ordering::Relaxed);
        }

        fn on_stream_event(&self, event: &StreamEvent) -> Option<StreamEvent> {
            self.counts.stream.fetch_add(1, Ordering::Relaxed);
            Some(event.clone())
        }
    }

    /// A plugin that short-circuits stream events (always returns `None`).
    struct ShortCircuitPlugin(&'static str);

    impl ModelPlugin for ShortCircuitPlugin {
        fn name(&self) -> &str {
            self.0
        }

        fn on_stream_event(&self, _event: &StreamEvent) -> Option<StreamEvent> {
            None
        }
    }

    fn make_request() -> InternalRequest {
        InternalRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: Some(256),
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        }
    }

    fn make_response() -> UnifiedResponse {
        UnifiedResponse {
            content_blocks: vec![ContentBlock::Text("hello".into())],
            usage: UnifiedUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: Some(15),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".to_string()),
            retry_attempts: 0,
        }
    }

    fn make_stream_event() -> StreamEvent {
        StreamEvent::BlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
        }
    }

    // ── empty pipeline ─────────────────────────────────────────────────────────

    #[test]
    fn test_empty_pipeline_before_request() {
        let pipeline = PluginPipeline::new();
        let mut req = make_request();
        pipeline.before_request(&mut req); // must not panic
    }

    #[test]
    fn test_empty_pipeline_after_response() {
        let pipeline = PluginPipeline::new();
        let mut resp = make_response();
        pipeline.after_response(&mut resp); // must not panic
    }

    #[test]
    fn test_empty_pipeline_on_stream_event() {
        let pipeline = PluginPipeline::new();
        let event = make_stream_event();
        let result = pipeline.on_stream_event(&event);
        // empty pipeline always forwards
        assert!(result.is_some());
        assert_eq!(result.unwrap(), event);
    }

    // ── sequential execution ───────────────────────────────────────────────────

    #[test]
    fn test_before_request_sequential() {
        let (p1, c1) = RecordingPlugin::new("p1");
        let (p2, c2) = RecordingPlugin::new("p2");
        let pipeline = PluginPipeline::new().add(Box::new(p1)).add(Box::new(p2));

        let mut req = make_request();
        pipeline.before_request(&mut req);

        assert_eq!(c1.before.load(Ordering::Relaxed), 1);
        assert_eq!(c2.before.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_after_response_sequential() {
        let (p1, c1) = RecordingPlugin::new("p1");
        let (p2, c2) = RecordingPlugin::new("p2");
        let pipeline = PluginPipeline::new().add(Box::new(p1)).add(Box::new(p2));

        let mut resp = make_response();
        pipeline.after_response(&mut resp);

        assert_eq!(c1.after.load(Ordering::Relaxed), 1);
        assert_eq!(c2.after.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_on_stream_event_sequential() {
        let (p1, c1) = RecordingPlugin::new("p1");
        let (p2, c2) = RecordingPlugin::new("p2");
        let pipeline = PluginPipeline::new().add(Box::new(p1)).add(Box::new(p2));

        let event = make_stream_event();
        let result = pipeline.on_stream_event(&event);

        assert!(result.is_some());
        assert_eq!(c1.stream.load(Ordering::Relaxed), 1);
        assert_eq!(c2.stream.load(Ordering::Relaxed), 1);
    }

    // ── short-circuit ───────────────────────────────────────────────────────────

    #[test]
    fn test_on_stream_event_short_circuits_when_none() {
        let (recording, counts) = RecordingPlugin::new("recorder");
        let pipeline = PluginPipeline::new()
            .add(Box::new(ShortCircuitPlugin("short")))
            .add(Box::new(recording));

        let event = make_stream_event();
        let result = pipeline.on_stream_event(&event);

        assert!(result.is_none());
        assert_eq!(
            counts.stream.load(Ordering::Relaxed),
            0,
            "plugin after short-circuit should not be called"
        );
    }
}
