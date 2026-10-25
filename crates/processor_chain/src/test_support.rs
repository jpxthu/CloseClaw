//! Shared test helpers for processor_chain unit tests.
//!
//! This module is compiled only under `cfg(test)` and provides test
//! scaffolding reused across the `*_tests.rs` files of this crate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::processor_chain::context::MessageContext;
use crate::processor_chain::error::ProcessError;
use crate::processor_chain::processor::{MessageProcessor, ProcessPhase};
use crate::ProcessedMessage;
use closeclaw_common::im_plugin::NormalizedMessage;
use closeclaw_common::processor::ContentBlock;

pub(crate) fn make_normalized(content: &str) -> NormalizedMessage {
    NormalizedMessage {
        platform: "test".to_string(),
        sender_id: "user_1".to_string(),
        peer_id: String::new(),
        content: content.to_string(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        message_type: Default::default(),
        media_refs: Vec::new(),
        reply_ref: None,
        unavailable_media: Vec::new(),
        thread_id: None,
        account_id: String::new(),
        ..Default::default()
    }
}

/// Test processor that sets its own name as content and records call count.
pub(crate) struct TestProc {
    pub(crate) name: String,
    pub(crate) phase: ProcessPhase,
    pub(crate) priority: u8,
    pub(crate) call_counter: Arc<AtomicUsize>,
    pub(crate) metadata_kv: Option<(String, String)>,
    pub(crate) suppress: bool,
    pub(crate) skip: bool,
}

impl TestProc {
    pub(crate) fn inbound(name: &str, priority: u8) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let proc = Self {
            name: name.to_string(),
            phase: ProcessPhase::Inbound,
            priority,
            call_counter: counter.clone(),
            metadata_kv: None,
            suppress: false,
            skip: false,
        };
        (Arc::new(proc), counter)
    }

    pub(crate) fn outbound(name: &str, priority: u8) -> (Arc<Self>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let proc = Self {
            name: name.to_string(),
            phase: ProcessPhase::Outbound,
            priority,
            call_counter: counter.clone(),
            metadata_kv: None,
            suppress: false,
            skip: false,
        };
        (Arc::new(proc), counter)
    }
}

#[async_trait]
impl MessageProcessor for TestProc {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> ProcessPhase {
        self.phase
    }
    fn priority(&self) -> u8 {
        self.priority
    }

    async fn process(
        &self,
        _ctx: &MessageContext,
    ) -> Result<Option<ProcessedMessage>, ProcessError> {
        self.call_counter.fetch_add(1, Ordering::SeqCst);
        if self.skip || self.suppress {
            return Ok(None);
        }
        let mut metadata = HashMap::new();
        if let Some((ref k, ref v)) = self.metadata_kv {
            metadata.insert(k.clone(), v.clone());
        }
        Ok(Some(ProcessedMessage {
            content_blocks: vec![ContentBlock::Text(self.name.clone())],
            metadata,
        }))
    }
}

pub(crate) struct FailingProc {
    pub(crate) name: String,
    pub(crate) phase: ProcessPhase,
    pub(crate) priority: u8,
}

impl FailingProc {
    pub(crate) fn inbound(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            phase: ProcessPhase::Inbound,
            priority: 1,
        })
    }
    pub(crate) fn outbound(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            phase: ProcessPhase::Outbound,
            priority: 1,
        })
    }
}

#[async_trait]
impl MessageProcessor for FailingProc {
    fn name(&self) -> &str {
        &self.name
    }
    fn phase(&self) -> ProcessPhase {
        self.phase
    }
    fn priority(&self) -> u8 {
        self.priority
    }

    async fn process(&self, _: &MessageContext) -> Result<Option<ProcessedMessage>, ProcessError> {
        Err(ProcessError::processor_failed(
            self.name(),
            "intentional failure",
        ))
    }
}
