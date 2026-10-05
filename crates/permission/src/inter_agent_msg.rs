//! Inter-agent message payload for permission evaluation.

/// Simplified inter-agent message body for permission evaluation.
///
/// Distinct from [`crate::PermissionRequestBody::InterAgentMsg`], which is
/// the engine request-body variant carrying the same routing fields.
#[derive(Debug, Clone)]
pub struct InterAgentMsg {
    pub from: String,
    pub to: String,
}
