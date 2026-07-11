//! Shared stream utilities for the LLM crate.

use crate::provider::SseStream;
use crate::types::RawSseChunk;

/// Bridges [`tokio::sync::mpsc::Receiver`] → [`futures::Stream`].
///
/// [`tokio::sync::mpsc::Receiver`] does not implement [`futures::Stream`]
/// directly.  This wrapper allows an [`SseStream`] to be used as an
/// [`IncomingSseStream`](crate::protocol::IncomingSseStream).
pub(crate) struct ReceiverStream {
    rx: Option<SseStream>,
}

impl ReceiverStream {
    pub(crate) fn new(rx: SseStream) -> Self {
        Self { rx: Some(rx) }
    }
}

impl futures::Stream for ReceiverStream {
    type Item = RawSseChunk;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.rx.as_mut() {
            Some(rx) => rx.poll_recv(cx),
            None => std::task::Poll::Ready(None),
        }
    }
}
