//! Late-bound proxy for `SessionManagerOps`.
//!
//! Enables deferred injection of the real `SessionManager` implementation.
//! During daemon startup, tools are registered (layer 3) *before* the
//! `SessionManager` is created (layer 4).  The proxy holds an `OnceLock` that
//! starts empty and is filled once the real manager is ready.
//!
//! Until `set()` is called, every method returns an explicit error so that
//! tool invocations during the brief startup window fail fast rather than
//! panicking.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::spawn::{ChildSessionInfo, SpawnMode};
use closeclaw_config::agents::ResolvedAgentConfig;

use super::SessionManagerOps;

const NOT_INITIALIZED: &str = "SessionManager not yet initialized";

/// A transparent proxy that wraps `Arc<dyn SessionManagerOps>` in an
/// `OnceLock`, allowing the inner value to be supplied after construction.
///
/// # Usage
///
/// ```text
/// let proxy = Arc::new(LateBoundSessionManagerOps::new());
/// // proxy.create_child_session(…) → Err("SessionManager not yet initialized")
///
/// proxy.set(real_manager);
/// // proxy.create_child_session(…) → delegates to real_manager
/// ```
pub struct LateBoundSessionManagerOps {
    inner: OnceLock<Arc<dyn SessionManagerOps>>,
}

impl LateBoundSessionManagerOps {
    /// Create a new, un-initialised proxy.
    pub fn new() -> Self {
        Self {
            inner: OnceLock::new(),
        }
    }

    /// Inject the real `SessionManagerOps` implementation.
    ///
    /// Returns `Err(manager)` if `set()` has already been called (the
    /// `OnceLock` does not allow a second write).
    pub fn set(
        &self,
        manager: Arc<dyn SessionManagerOps>,
    ) -> Result<(), Arc<dyn SessionManagerOps>> {
        self.inner.set(manager)
    }

    /// Returns `true` if `set()` has been called.
    pub fn is_ready(&self) -> bool {
        self.inner.get().is_some()
    }

    fn get_ref(&self) -> Result<&Arc<dyn SessionManagerOps>, String> {
        self.inner.get().ok_or_else(|| NOT_INITIALIZED.to_string())
    }
}

impl Default for LateBoundSessionManagerOps {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionManagerOps for LateBoundSessionManagerOps {
    async fn create_child_session(
        &self,
        config: &ResolvedAgentConfig,
        parent_session_id: &str,
        depth: u32,
        task: &str,
        light_context: bool,
        workspace: Option<&str>,
        mode: SpawnMode,
        fork: bool,
        allowed_tools: Option<Vec<String>>,
        model_override: Option<&str>,
        parent_subagents_model: Option<&str>,
        max_spawn_depth: u32,
        spawn_timeout: Option<u64>,
        label: Option<&str>,
        prompt_template_prefix: Option<&str>,
    ) -> Result<String, String> {
        self.get_ref()?
            .create_child_session(
                config,
                parent_session_id,
                depth,
                task,
                light_context,
                workspace,
                mode,
                fork,
                allowed_tools,
                model_override,
                parent_subagents_model,
                max_spawn_depth,
                spawn_timeout,
                label,
                prompt_template_prefix,
            )
            .await
    }

    async fn validate_child_ownership(
        &self,
        parent_id: &str,
        child_id: &str,
    ) -> Option<ChildSessionInfo> {
        match self.get_ref() {
            Ok(m) => m.validate_child_ownership(parent_id, child_id).await,
            Err(_) => None,
        }
    }

    async fn steer_child(&self, child_id: &str, task: &str) -> Result<(), String> {
        self.get_ref()?.steer_child(child_id, task).await
    }

    async fn kill_child(&self, parent_id: &str, child_id: &str) -> Result<(), String> {
        self.get_ref()?.kill_child(parent_id, child_id).await
    }

    async fn get_chat_id(&self, session_id: &str) -> Option<String> {
        match self.get_ref() {
            Ok(m) => m.get_chat_id(session_id).await,
            Err(_) => None,
        }
    }

    async fn get_session_depth(&self, session_id: &str) -> Option<u32> {
        match self.get_ref() {
            Ok(m) => m.get_session_depth(session_id).await,
            Err(_) => None,
        }
    }

    async fn start_yield_timeout(
        self: Arc<Self>,
        session_id: &str,
        agent_id: &str,
        timeout_secs: Option<u64>,
    ) {
        if let Ok(m) = self.get_ref() {
            let m = Arc::clone(m);
            m.start_yield_timeout(session_id, agent_id, timeout_secs)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::SpawnMode;
    use closeclaw_config::agents::ResolvedAgentConfig;

    /// A minimal mock that always succeeds.
    struct MockSessionManagerOps;

    #[async_trait]
    impl SessionManagerOps for MockSessionManagerOps {
        async fn create_child_session(
            &self,
            _config: &ResolvedAgentConfig,
            _parent_session_id: &str,
            _depth: u32,
            _task: &str,
            _light_context: bool,
            _workspace: Option<&str>,
            _mode: SpawnMode,
            _fork: bool,
            _allowed_tools: Option<Vec<String>>,
            _model_override: Option<&str>,
            _parent_subagents_model: Option<&str>,
            _max_spawn_depth: u32,
            _spawn_timeout: Option<u64>,
            _label: Option<&str>,
            _prompt_template_prefix: Option<&str>,
        ) -> Result<String, String> {
            Ok("child-session-id".into())
        }

        async fn validate_child_ownership(
            &self,
            _parent_id: &str,
            _child_id: &str,
        ) -> Option<ChildSessionInfo> {
            None
        }

        async fn steer_child(&self, _child_id: &str, _task: &str) -> Result<(), String> {
            Ok(())
        }

        async fn kill_child(&self, _parent_id: &str, _child_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
            Some("agent-1".into())
        }

        async fn get_session_depth(&self, _session_id: &str) -> Option<u32> {
            Some(0)
        }

        async fn start_yield_timeout(
            self: Arc<Self>,
            _session_id: &str,
            _agent_id: &str,
            _timeout_secs: Option<u64>,
        ) {
        }
    }

    #[tokio::test]
    async fn uninitialised_returns_error() {
        let proxy = LateBoundSessionManagerOps::new();
        assert!(!proxy.is_ready());

        let config = test_config();
        let result = proxy
            .create_child_session(
                &config,
                "parent-1",
                0,
                "do something",
                false,
                None,
                SpawnMode::Run,
                false,
                None,
                None,
                None,
                3,
                None,
                None,
                None,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), NOT_INITIALIZED);
    }

    /// Build a minimal valid config for tests.
    fn test_config() -> ResolvedAgentConfig {
        use closeclaw_config::agents::{AgentConfig, ConfigSource};
        let mut config = AgentConfig::default();
        config.id = "test-agent".to_string();
        ResolvedAgentConfig::from_single(config, ConfigSource::User, "test", None)
            .expect("test config should be valid")
    }

    #[tokio::test]
    async fn uninitialised_option_methods_return_none() {
        let proxy = LateBoundSessionManagerOps::new();

        assert!(proxy.validate_child_ownership("p", "c").await.is_none());
        assert!(proxy.get_chat_id("s").await.is_none());
        assert!(proxy.get_session_depth("s").await.is_none());
    }

    #[tokio::test]
    async fn uninitialised_result_methods_return_error() {
        let proxy = LateBoundSessionManagerOps::new();

        assert!(proxy.steer_child("c", "t").await.is_err());
        assert!(proxy.kill_child("p", "c").await.is_err());
    }

    #[tokio::test]
    async fn set_then_delegate() {
        let proxy = Arc::new(LateBoundSessionManagerOps::new());
        let mock = Arc::new(MockSessionManagerOps);

        proxy.set(mock).ok().expect("set should succeed");
        assert!(proxy.is_ready());

        let config = test_config();
        let result = proxy
            .create_child_session(
                &config,
                "parent-1",
                0,
                "do something",
                false,
                None,
                SpawnMode::Run,
                false,
                None,
                None,
                None,
                3,
                None,
                None,
                None,
            )
            .await;
        assert_eq!(result.unwrap(), "child-session-id");
    }

    #[tokio::test]
    async fn set_then_option_methods_delegate() {
        let proxy = Arc::new(LateBoundSessionManagerOps::new());
        let mock = Arc::new(MockSessionManagerOps);
        proxy.set(mock).ok().unwrap();

        assert_eq!(proxy.get_chat_id("s").await, Some("agent-1".into()));
        assert_eq!(proxy.get_session_depth("s").await, Some(0));
    }

    #[test]
    fn set_twice_fails() {
        let proxy = LateBoundSessionManagerOps::new();
        let mock1 = Arc::new(MockSessionManagerOps);
        let mock2 = Arc::new(MockSessionManagerOps);

        proxy.set(mock1).ok().unwrap();
        let err = proxy.set(mock2);
        assert!(err.is_err());
    }
}
