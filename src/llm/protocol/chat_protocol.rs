//! LLM ChatProtocol abstraction — request building, response parsing, SSE parsing.
//!
//! This module defines the [`ChatProtocol`] trait, which is responsible for
//! **protocol-level** transformations between the internal unified types and
//! the wire format of a specific LLM API protocol (OpenAI, Anthropic, GLM, etc.).
//!
//! Specifically, a `ChatProtocol` implementation handles:
//! - Serialising an [`InternalRequest`][crate::llm::InternalRequest] into a
//!   protocol-specific HTTP request body (`serde_json::Value`).
//! - Deserialising a protocol-specific HTTP response body into an
//!   [`InternalResponse`][crate::llm::InternalResponse].
//! - Decorating an HTTP request with protocol-specific headers.
//! - Parsing raw SSE event streams into structured [`StreamEvent`][crate::llm::StreamEvent] values.
//!
//! One `ChatProtocol` exists per protocol variant (e.g. OpenAI, Anthropic).
//! The appropriate protocol is selected at runtime by matching the
//! [`ProtocolId`][crate::llm::ProtocolId] returned by a [`Provider`][crate::llm::Provider].

use async_trait::async_trait;
use futures::Stream;
use reqwest::header::HeaderMap;
use std::pin::Pin;

use crate::llm::types::{
    InternalRequest, InternalResponse, ProtocolId, RawSseChunk, SseStateMachine, StreamEvent,
};

/// Errors that can occur during protocol-level operations.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// Request serialization failed.
    #[error("failed to serialise request: {0}")]
    RequestBuild(#[from] serde_json::Error),
    /// Response parsing failed.
    #[error("failed to parse response: {0}")]
    ResponseParse(String),
    /// Header decoration failed.
    #[error("failed to decorate headers: {0}")]
    HeaderDecorate(String),
    /// SSE stream parsing failed.
    #[error("SSE stream parsing error: {0}")]
    SseParse(String),
}

/// Result type alias for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

/// Incoming stream type — a [`Stream`] of [`RawSseChunk`] emitted by a provider.
pub type IncomingSseStream = Pin<Box<dyn Stream<Item = RawSseChunk> + Send>>;

/// Outgoing stream type — a [`Stream`] of parsed [`StreamEvent`] values.
pub type OutgoingEventStream =
    Pin<Box<dyn Stream<Item = std::result::Result<StreamEvent, ProtocolError>> + Send>>;

/// LLM ChatProtocol trait — request/response protocol conversion.
///
/// Implementors translate between the internal unified types
/// (`InternalRequest`, `InternalResponse`, `StreamEvent`) and the
/// wire format of a specific LLM API protocol.
///
/// # Design contract
///
/// - All identifier/path accessors are **synchronous**.
/// - `build_request`, `parse_response`, and `decorate_headers` are **synchronous**
///   because they only perform serialisation / header manipulation with no I/O.
/// - `create_sse_machine` is a **factory** that returns a fresh state machine.
/// - `parse_sse_stream` is **asynchronous** and streaming; it consumes an
///   [`IncomingSseStream`] (from a [`Provider`][crate::llm::Provider]'s
///   [`send_streaming`][crate::llm::Provider::send_streaming]) and produces a
///   stream of parsed [`StreamEvent`][crate::llm::StreamEvent] values.
#[async_trait]
pub trait ChatProtocol: Send + Sync {
    // ── Identity ────────────────────────────────────────────────────────────

    /// Returns the protocol identifier for this implementation.
    ///
    /// The returned value must match one of the [`ProtocolId`][crate::llm::ProtocolId]
    /// values advertised by the [`Provider`][crate::llm::Provider] that hosts
    /// this protocol.
    fn protocol_id(&self) -> &ProtocolId;

    /// Returns the API endpoint path for chat completions.
    ///
    /// This path is appended to the provider's `base_url` to form the full URL.
    /// Example: `"/chat/completions"`.
    fn path(&self) -> &str;

    // ── Request / Response ──────────────────────────────────────────────────

    /// Builds a protocol-specific HTTP request body from an internal request.
    ///
    /// The caller is responsible for calling
    /// [`Provider::supported_protocols`][crate::llm::Provider::supported_protocols]
    /// to obtain the protocol ID, then selecting the matching `ChatProtocol`
    /// from the registry before invoking this method.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::RequestBuild`] if `request` cannot be serialised.
    fn build_request(&self, request: &InternalRequest) -> Result<serde_json::Value>;

    /// Parses a protocol-specific HTTP response body into an internal response.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::ResponseParse`] if the response body cannot be
    /// parsed into an [`InternalResponse`][crate::llm::InternalResponse].
    fn parse_response(&self, body: serde_json::Value) -> Result<InternalResponse>;

    // ── Header decoration ───────────────────────────────────────────────────

    /// Decorates an HTTP request header map with protocol-specific headers.
    ///
    /// This method is called by the framework before sending any request.
    /// Implementors may add protocol-specific headers (e.g. `Content-Type`,
    /// custom vendor headers) to the provided map.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::HeaderDecorate`] if a header cannot be added.
    fn decorate_headers(&self, headers: &mut HeaderMap) -> Result<()>;

    // ── SSE parsing ────────────────────────────────────────────────────────

    /// Creates a fresh SSE state machine for parsing a streaming response.
    ///
    /// A new state machine is required for each streaming request to ensure
    /// no residual state leaks between unrelated streams.
    fn create_sse_machine(&self) -> SseStateMachine;

    /// Parses a stream of raw SSE chunks into a stream of structured events.
    ///
    /// This method consumes an [`IncomingSseStream`] (typically obtained from
    /// [`Provider::send_streaming`][crate::llm::Provider::send_streaming]) and
    /// returns an [`OutgoingEventStream`] that yields one [`StreamEvent`] per
    /// parsed SSE event.  Internally it uses an [`SseStateMachine`] (created
    /// via [`create_sse_machine`][ChatProtocol::create_sse_machine]) to
    /// track incremental state.
    ///
    /// The stream terminates when the incoming SSE stream is exhausted.
    async fn parse_sse_stream(
        &self,
        incoming: IncomingSseStream,
        machine: SseStateMachine,
    ) -> OutgoingEventStream;
}
