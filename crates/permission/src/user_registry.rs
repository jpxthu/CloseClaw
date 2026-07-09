//! User registration management.
//!
//! Tracks users who have been approved by the Owner and provides
//! conversion of [`InitialPermissionSet`] into concrete [`Rule`] entries.

use closeclaw_common::permission_op::{InitialPermissionSet, UserRegistration};

use crate::engine::{Action, Effect, MatchType, Rule, RuleSet, Subject};

/// Manages the list of registered (approved) users.
///
/// Each registered user carries metadata about their IM channel and
/// the initial permission sets that were granted at registration time.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UserRegistry {
    /// All registered users.
    users: Vec<UserRegistration>,
}

impl UserRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new user with the given initial permission sets.
    ///
    /// Returns the generated [`Rule`] list that should be persisted
    /// into the agent's `permissions.json`.
    ///
    /// # Errors
    /// Returns `Err` if the user is already registered.
    pub fn register_user(
        &mut self,
        user_id: &str,
        channel: &str,
        initial_permissions: &[InitialPermissionSet],
    ) -> Result<RuleSet, RegistryError> {
        if self.is_registered(user_id) {
            return Err(RegistryError::AlreadyRegistered(user_id.to_string()));
        }
        let registration = UserRegistration {
            user_id: user_id.to_string(),
            im_channel: channel.to_string(),
            initial_permissions: initial_permissions.to_vec(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let rules: Vec<Rule> = initial_permissions
            .iter()
            .flat_map(|perm| perm.to_rules(user_id))
            .collect();
        self.users.push(registration);
        Ok(RuleSet {
            rules,
            ..Default::default()
        })
    }

    /// Check whether a user is already registered.
    pub fn is_registered(&self, user_id: &str) -> bool {
        self.users.iter().any(|u| u.user_id == user_id)
    }

    /// Return a slice of all registered users.
    pub fn list_users(&self) -> &[UserRegistration] {
        &self.users
    }

    /// Consume the registry and return the inner user list.
    pub fn into_users(self) -> Vec<UserRegistration> {
        self.users
    }
}

/// Errors that can occur during user registration.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("user already registered: {0}")]
    AlreadyRegistered(String),
}

/// Extension trait to convert [`InitialPermissionSet`] into concrete [`Rule`]s.
///
/// Implemented here in `closeclaw-permission` (not in `closeclaw-common`)
/// to respect the dependency direction: common defines the types, permission
/// provides the conversion logic.
pub trait InitialPermissionSetExt {
    /// Convert this permission set into a list of [`Rule`]s for the given user.
    ///
    /// The rules use `UserAndAgent` subjects with glob matching on the agent
    /// field so they apply across all agents for this user.
    fn to_rules(&self, user_id: &str) -> Vec<Rule>;
}

impl InitialPermissionSetExt for InitialPermissionSet {
    fn to_rules(&self, user_id: &str) -> Vec<Rule> {
        match self {
            InitialPermissionSet::BasicMessaging => {
                vec![
                    Rule {
                        name: format!("user-{}-chat-send", user_id),
                        subject: Subject::UserAndAgent {
                            user_id: user_id.to_string(),
                            agent: "*".to_string(),
                            user_match: MatchType::Exact,
                            agent_match: MatchType::Glob,
                        },
                        effect: Effect::Allow,
                        actions: vec![Action::ToolCall {
                            skill: "chat".to_string(),
                            methods: vec!["send".to_string()],
                        }],
                        template: None,
                        priority: 10,
                    },
                    Rule {
                        name: format!("user-{}-workspace-read", user_id),
                        subject: Subject::UserAndAgent {
                            user_id: user_id.to_string(),
                            agent: "*".to_string(),
                            user_match: MatchType::Exact,
                            agent_match: MatchType::Glob,
                        },
                        effect: Effect::Allow,
                        actions: vec![Action::File {
                            operation: "read".to_string(),
                            paths: vec!["workspace/**".to_string()],
                        }],
                        template: None,
                        priority: 10,
                    },
                ]
            }
        }
    }
}
