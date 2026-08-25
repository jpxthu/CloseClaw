//! Handler registry with interior mutability for shared registration.

use std::collections::HashMap;
use std::sync::Arc;

use crate::handler::SlashHandler;

/// Registry that maps command names to their handlers.
///
/// Uses `std::sync::RwLock` internally so handlers can be registered
/// through `Arc<HandlerRegistry>` without requiring `async`.
pub struct HandlerRegistry {
    handlers: std::sync::RwLock<HashMap<String, Arc<dyn SlashHandler>>>,
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            handlers: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a handler. Inserts one entry per command returned by
    /// [`SlashHandler::commands`]. If multiple handlers share a command
    /// name, the last one registered wins.
    pub fn register(&self, handler: Arc<dyn SlashHandler>) {
        let mut handlers = self.handlers.write().expect("registry lock poisoned");
        for cmd in handler.commands() {
            handlers.insert((*cmd).to_owned(), Arc::clone(&handler));
        }
    }

    /// Register a handler under a specific command name.
    ///
    /// This allows handlers whose [`SlashHandler::commands`] returns an
    /// empty slice (like [`crate::skill_handler::SkillSlashHandler`]) to
    /// dynamically claim command names at registration time.
    pub fn register_named(&self, command: &str, handler: Arc<dyn SlashHandler>) {
        let mut handlers = self.handlers.write().expect("registry lock poisoned");
        handlers.insert(command.to_owned(), handler);
    }

    /// Look up a handler by command name (without the leading `/`).
    ///
    /// Returns a cloned `Box<dyn SlashHandler>` via the trait's `clone_box`
    /// method, enabling the `SlashRouter` trait's `get_handler` signature.
    pub fn get(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .get(command)
            .map(|h| h.clone_box())
    }

    /// Look up a handler by command name, returning an Arc.
    pub fn get_arc(&self, command: &str) -> Option<Arc<dyn SlashHandler>> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .get(command)
            .cloned()
    }

    /// Iterate over (command, handler) pairs, returning boxed clones.
    pub fn iter(&self) -> Vec<(String, Box<dyn SlashHandler>)> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_box()))
            .collect()
    }

    /// Iterate over (command, handler) pairs, returning Arc clones.
    ///
    /// This avoids the Arc→Box heap allocation that [`iter`] requires,
    /// making it the preferred path for callers that only need `Arc`.
    pub fn iter_arc(&self) -> Vec<(String, Arc<dyn SlashHandler>)> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Return a list of all registered command names (unordered).
    pub fn all_commands(&self) -> Vec<String> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::SlashContext;
    use crate::handler::SlashHandler;
    use async_trait::async_trait;
    use closeclaw_common::slash_router::SlashResult;

    #[derive(Clone)]
    struct MockHandler {
        desc: String,
    }

    #[async_trait]
    impl SlashHandler for MockHandler {
        fn commands(&self) -> &[&str] {
            &[]
        }
        fn description(&self) -> &str {
            &self.desc
        }
        fn immediate(&self, _cmd: &str) -> bool {
            false
        }
        fn clone_box(&self) -> Box<dyn SlashHandler> {
            Box::new(self.clone())
        }
        async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
            SlashResult::Reply(format!("handled by {}", self.desc))
        }
    }

    #[tokio::test]
    async fn test_register_named_and_get() {
        let registry = HandlerRegistry::new();
        let handler: Arc<dyn SlashHandler> = Arc::new(MockHandler {
            desc: "skill-h".into(),
        });
        registry.register_named("my-skill", Arc::clone(&handler));

        let found = registry.get("my-skill");
        assert!(found.is_some());
        let h = found.unwrap();
        assert_eq!(h.description(), "skill-h");
    }

    #[tokio::test]
    async fn test_register_named_overwrites_existing() {
        let registry = HandlerRegistry::new();
        let h1: Arc<dyn SlashHandler> = Arc::new(MockHandler {
            desc: "first".into(),
        });
        let h2: Arc<dyn SlashHandler> = Arc::new(MockHandler {
            desc: "second".into(),
        });
        registry.register_named("dup", h1);
        registry.register_named("dup", h2);

        let found = registry.get("dup").unwrap();
        assert_eq!(found.description(), "second");
    }

    #[tokio::test]
    async fn test_register_named_in_all_commands() {
        let registry = HandlerRegistry::new();
        let handler: Arc<dyn SlashHandler> = Arc::new(MockHandler {
            desc: "test".into(),
        });
        registry.register_named("alpha", Arc::clone(&handler));
        registry.register_named("beta", Arc::clone(&handler));

        let mut cmds = registry.all_commands();
        cmds.sort();
        assert_eq!(cmds, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn test_register_named_not_in_regular_register() {
        let registry = HandlerRegistry::new();
        let handler: Arc<dyn SlashHandler> = Arc::new(MockHandler {
            desc: "named".into(),
        });
        registry.register_named("only-named", handler);

        // register() uses commands() which returns [] for MockHandler
        // so register_named should be the only way to add it
        assert!(registry.get("only-named").is_some());
        assert_eq!(registry.all_commands().len(), 1);
    }
}
