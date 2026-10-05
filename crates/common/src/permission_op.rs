//! User registration types for permission management.
//!
//! Defines [`InitialPermissionSet`], [`UserRegistration`] and
//! [`UserCreationRequest`] — payloads shared between the approval flow,
//! the user registry and the `/perm` slash command handler.
//!
//! The `/perm` operation enum lives in `closeclaw-permission`
//! (its `permission_op` module).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// User registration types
// ---------------------------------------------------------------------------

/// Preset permission sets that an Owner can assign to a newly registered User.
///
/// Each variant maps to a concrete set of [`Rule`](crate) entries via
/// [`to_rules()`](InitialPermissionSet::to_rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialPermissionSet {
    /// Basic messaging: allows sending/receiving messages and reading the workspace.
    /// Translates to `ToolCall { skill: "chat", method: "send" }` + workspace read.
    BasicMessaging,
}

impl InitialPermissionSet {
    /// Human-readable name for display in confirmation messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::BasicMessaging => "BasicMessaging",
        }
    }
}

/// A registered user who has been approved by an Owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRegistration {
    /// Unique identifier for the user (e.g. Feishu `open_id`).
    pub user_id: String,
    /// IM channel through which the user interacts (e.g. "feishu").
    pub im_channel: String,
    /// Initial permission sets granted at registration time.
    pub initial_permissions: Vec<InitialPermissionSet>,
    /// ISO-8601 timestamp of when the user was registered.
    pub created_at: String,
}

/// Request payload carried through the [`ApprovalQueue`] for a new-user
/// registration that requires Owner approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreationRequest {
    /// The user who requested registration.
    pub user_id: String,
    /// IM channel the user will use.
    pub im_channel: String,
    /// Unique request identifier for tracking in the approval queue.
    pub request_id: String,
    /// Initial permission sets selected by the Owner at approval time.
    pub initial_permissions: Vec<InitialPermissionSet>,
}
