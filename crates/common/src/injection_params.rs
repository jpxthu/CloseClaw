//! Injection chain parameter contract for system prompt building.
//!
//! The design doc (`docs/design/session/session-injection.md` §注入链路的参数契约)
//! defines four required inputs for system prompt injection:
//!
//! 1. `agent_id`
//! 2. ToolRegistry reference
//! 3. Session role (Main / Sub)
//! 4. Bootstrap mode (identity loading mode)
//!
//! Code already carries three additional parameters (`session_id`, `overrides`,
//! `activated_skills`) that live alongside the contract.  [`InjectionParams`]
//! bundles all seven into a single struct, avoiding excessive function
//! parameters while preserving every field needed by downstream consumers.

use std::sync::Arc;

use crate::bootstrap::BootstrapMode;
use crate::fragment::SessionRole;
use crate::system_prompt::PromptOverrides;
use crate::tool_registry::ToolRegistryQuery;

/// All parameters required by the system prompt injection chain.
///
/// Passed from [`SessionManager`][crate::session_lookup::SessionLookup] →
/// `ConversationSession` → `SystemPromptBuilder` on every injection call.
///
/// # Design rationale
///
/// The design doc (§注入链路的参数契约) lists four mandatory inputs.
/// Three additional parameters already exist in code (`session_id`,
/// `overrides`, `activated_skills`) and are included here so the
/// struct serves as the single parameter contract for the injection
/// chain.
#[derive(Clone)]
pub struct InjectionParams {
    /// Session identifier — passed through for logging and tracing.
    pub session_id: String,

    /// Agent identifier — determines which agent's bootstrap files
    /// and tool/skill visibility rules apply.
    pub agent_id: String,

    /// Optional prompt overrides (agent / custom / override).
    pub overrides: Option<PromptOverrides>,

    /// Identity loading mode (Full / Minimal) override.
    ///
    /// `None` means "use the default from agent configuration".
    pub bootstrap_mode_override: Option<BootstrapMode>,

    /// Names of conditionally-activated skills in this session.
    ///
    /// Passed through to [`SkillsFragmentProvider`][crate::fragment::PromptFragmentProvider]
    /// so that activated conditional skills appear in the listing.
    pub activated_skills: Vec<String>,

    /// Session role — determines identity-gated behaviour (long-term
    /// memory, BOOTSTRAP.md, etc.).
    pub session_role: SessionRole,

    /// Optional ToolRegistry reference for the injection chain.
    ///
    /// When `Some`, the builder forwards it to
    /// [`FragmentContext`][crate::fragment::FragmentContext]
    /// so that [`ToolsFragmentProvider`][crate::fragment::PromptFragmentProvider]
    /// can consume it at generation time.  `None` falls back to the
    /// provider-level default (e.g. closure-held reference).
    pub tool_registry: Option<Arc<dyn ToolRegistryQuery>>,
}

impl InjectionParams {
    /// Returns `InjectionParams` with empty defaults for every field.
    ///
    /// Intended for unit tests only.
    #[doc(hidden)]
    pub fn test_default() -> Self {
        Self {
            session_id: String::new(),
            agent_id: String::new(),
            overrides: None,
            bootstrap_mode_override: None,
            activated_skills: Vec::new(),
            session_role: SessionRole::Main,
            tool_registry: None,
        }
    }
}

impl std::fmt::Debug for InjectionParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InjectionParams")
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("overrides", &self.overrides)
            .field("bootstrap_mode_override", &self.bootstrap_mode_override)
            .field("activated_skills", &self.activated_skills)
            .field("session_role", &self.session_role)
            .field(
                "tool_registry",
                &self.tool_registry.as_ref().map(|_| "<ToolRegistryQuery>"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injection_params_test_default() {
        let p = InjectionParams::test_default();
        assert!(p.session_id.is_empty());
        assert!(p.agent_id.is_empty());
        assert!(p.overrides.is_none());
        assert!(p.bootstrap_mode_override.is_none());
        assert!(p.activated_skills.is_empty());
        assert_eq!(p.session_role, SessionRole::Main);
        assert!(p.tool_registry.is_none());
    }

    #[test]
    fn test_injection_params_debug() {
        let p = InjectionParams::test_default();
        let dbg = format!("{:?}", p);
        assert!(dbg.contains("InjectionParams"));
        // When tool_registry is None, the placeholder shows None
        assert!(dbg.contains("tool_registry: None"));
    }

    #[test]
    fn test_injection_params_clone() {
        let p = InjectionParams {
            session_id: "s1".into(),
            agent_id: "a1".into(),
            overrides: Some(PromptOverrides {
                override_prompt: Some("ov".into()),
                agent_prompt: None,
                custom_prompt: None,
            }),
            bootstrap_mode_override: Some(BootstrapMode::Minimal),
            activated_skills: vec!["skill-x".into()],
            session_role: SessionRole::Sub,
            tool_registry: None,
        };
        let cloned = p.clone();
        assert_eq!(cloned.session_id, "s1");
        assert_eq!(cloned.agent_id, "a1");
        assert_eq!(cloned.session_role, SessionRole::Sub);
        assert_eq!(cloned.activated_skills, vec!["skill-x"]);
    }
}
