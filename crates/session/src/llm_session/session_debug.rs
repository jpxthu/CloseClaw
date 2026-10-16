//! `Debug` implementation for [`ConversationSession`].
//!
//! Split from `mod.rs` for file-size governance (CONTRIBUTING.md 1000-line
//! cap); pure move, no logic change.

use std::sync::atomic::Ordering;

use super::ConversationSession;

impl std::fmt::Debug for ConversationSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationSession")
            .field("session_id", &self.session_id)
            .field("messages", &self.messages)
            .field("system_prompt", &self.system_prompt)
            .field("turn_counter", &self.turn_counter)
            .field("model", &self.model)
            .field("compaction_state", &self.compaction_state)
            .field("is_compacted", &self.is_compacted)
            .field("is_sub_agent", &self.is_sub_agent)
            .field("unified_queue", &self.unified_queue)
            .field("reasoning_level", &self.reasoning_level)
            .field("effective_reasoning_level", &self.effective_reasoning_level)
            .field("workdir", &self.workdir)
            .field("created_at", &self.created_at)
            .field("stats", &self.stats)
            .field(
                "streaming_sink",
                &self.streaming_sink.as_ref().map(|_| "<StreamingSink>"),
            )
            .field("stream_enabled", &self.stream_enabled)
            .field(
                "llm_state",
                &*self.llm_state.read().expect("llm_state lock poisoned"),
            )
            .field(
                "tool_states",
                &*self.tool_states.read().expect("tool_states lock poisoned"),
            )
            .field(
                "child_states",
                &*self
                    .child_states
                    .read()
                    .expect("child_states lock poisoned"),
            )
            .field(
                "tool_handles",
                &self
                    .tool_handles
                    .read()
                    .expect("tool_handles lock poisoned")
                    .len(),
            )
            .field(
                "child_handles",
                &self
                    .child_handles
                    .read()
                    .expect("child_handles lock poisoned")
                    .len(),
            )
            .field("cancel_token", &"<CancelToken>")
            .field("stopped", &self.stopped.load(Ordering::SeqCst))
            .field("is_yielding", &self.is_yielding.load(Ordering::SeqCst))
            .field("communication_config", &self.communication_config)
            .field("bootstrap_mode", &self.bootstrap_mode)
            .field("no_bootstrap", &self.no_bootstrap)
            .field("verbosity_level", &self.verbosity_level)
            .field(
                "session_mode",
                &*self
                    .session_mode
                    .lock()
                    .expect("session_mode lock poisoned"),
            )
            .field(
                "skill_listing_provider",
                &self
                    .skill_listing_provider
                    .as_ref()
                    .map(|_| "<SkillListingProvider>"),
            )
            .field("skill_listing_snapshot", &self.skill_listing_snapshot)
            .field(
                "pending_compaction_listing_reset",
                &self.pending_compaction_listing_reset,
            )
            .field(
                "activated_conditional_skills",
                &self.activated_conditional_skills,
            )
            .field(
                "tool_registry",
                &self.tool_registry.as_ref().map(|_| "<TR>"),
            )
            .field("agent_skills", &self.agent_skills)
            .field(
                "injected_task_ids",
                &*self
                    .injected_task_ids
                    .lock()
                    .expect("injected_task_ids lock poisoned"),
            )
            .field(
                "memory_injection",
                &*self
                    .memory_injection
                    .lock()
                    .expect("memory_injection lock poisoned"),
            )
            .field(
                "health_checker",
                &self.health_checker.as_ref().map(|_| "<HC>"),
            )
            .field("manual_background_signal", &"<Notify>")
            .field(
                "file_mtimes",
                &self.file_mtimes.read().ok().map(|m| m.len()),
            )
            .field(
                "file_read_ranges",
                &self.file_read_ranges.read().ok().map(|m| m.len()),
            )
            .finish()
    }
}
