//! Slash command executor types and traits.
//!
//! Re-exports executor types from `closeclaw-common`. The canonical definitions
//! live in `closeclaw-common::executor` because the `closeclaw-gateway` crate
//! cannot depend on `closeclaw-slash` (cycle: gateway → slash → tools → gateway).

pub use closeclaw_common::executor::{
    CompactionError, CompactionResult, ReplyAction, SideEffectContext, SlashEffectExecutor,
    SlashResultExecutor,
};
