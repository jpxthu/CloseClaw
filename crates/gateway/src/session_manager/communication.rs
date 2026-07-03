//! Agent communication permission checks.

pub use closeclaw_agent::communication::{
    check_communication_allowed, CommunicationCheckResult, CommunicationConfig, CommunicationError,
};

/// Resolved communication configs and agent IDs for a session pair.
struct SessionPairConfig {
    source_config: CommunicationConfig,
    source_agent_id: String,
    target_config: CommunicationConfig,
    target_agent_id: String,
}

/// Evaluate whether communication is allowed based on resolved configs.
/// Returns `Ok(())` if allowed, or `Err` with a descriptive reason.
fn evaluate_communication(pair: &SessionPairConfig) -> Result<(), CommunicationError> {
    match check_communication_allowed(
        &pair.source_config,
        &pair.source_agent_id,
        &pair.target_config,
        &pair.target_agent_id,
    ) {
        CommunicationCheckResult::Allowed => Ok(()),
        CommunicationCheckResult::TargetNotInSourceOutbound => Err(CommunicationError::Denied {
            reason: format!(
                "agent '{}' outbound list does not include '{}'",
                pair.source_agent_id, pair.target_agent_id,
            ),
        }),
        CommunicationCheckResult::SourceNotInTargetInbound => Err(CommunicationError::Denied {
            reason: format!(
                "agent '{}' inbound list does not include '{}'",
                pair.target_agent_id, pair.source_agent_id,
            ),
        }),
    }
}

impl super::SessionManager {
    /// Resolve communication configs and agent IDs for two sessions.
    async fn resolve_session_configs(
        &self,
        source_session_id: &str,
        target_session_id: &str,
    ) -> Result<SessionPairConfig, CommunicationError> {
        let (source_config_opt, target_config_opt) = {
            let conv = self.conversation_sessions.read().await;
            let source_cs = conv.get(source_session_id).ok_or_else(|| {
                CommunicationError::SessionNotFound(source_session_id.to_string())
            })?;
            let target_cs = conv.get(target_session_id).ok_or_else(|| {
                CommunicationError::SessionNotFound(target_session_id.to_string())
            })?;
            let source_cs = source_cs.read().await;
            let target_cs = target_cs.read().await;
            (
                source_cs.communication_config().cloned(),
                target_cs.communication_config().cloned(),
            )
        };
        let (source_agent_id, target_agent_id) = {
            let sessions = self.sessions.read().await;
            let source_session = sessions.get(source_session_id).ok_or_else(|| {
                CommunicationError::SessionNotFound(source_session_id.to_string())
            })?;
            let target_session = sessions.get(target_session_id).ok_or_else(|| {
                CommunicationError::SessionNotFound(target_session_id.to_string())
            })?;
            (
                source_session.agent_id.clone(),
                target_session.agent_id.clone(),
            )
        };
        let allow_all = CommunicationConfig {
            outbound: vec!["*".to_string()],
            inbound: vec!["*".to_string()],
        };
        Ok(SessionPairConfig {
            source_config: source_config_opt.unwrap_or(allow_all.clone()),
            source_agent_id,
            target_config: target_config_opt.unwrap_or(allow_all),
            target_agent_id,
        })
    }

    /// Check if communication from `source_session_id` to `target_session_id`
    /// is allowed by their respective `CommunicationConfig`.
    pub(crate) async fn check_session_communication(
        &self,
        source_session_id: &str,
        target_session_id: &str,
    ) -> Result<(), CommunicationError> {
        let pair = self
            .resolve_session_configs(source_session_id, target_session_id)
            .await?;
        evaluate_communication(&pair)
    }
}
