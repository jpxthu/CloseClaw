//! Handler registry with interior mutability for shared registration.

use std::collections::HashMap;
use std::sync::Arc;

use crate::slash::handler::SlashHandler;

/// Registry that maps command names to their handlers.
///
/// Uses `std::sync::RwLock` internally so handlers can be registered
/// through `Arc<HandlerRegistry>` without requiring `async`.
pub struct HandlerRegistry {
    handlers: std::sync::RwLock<HashMap<String, Arc<dyn SlashHandler>>>,
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

    /// Look up a handler by command name (without the leading `/`).
    pub fn get(&self, command: &str) -> Option<Arc<dyn SlashHandler>> {
        self.handlers
            .read()
            .expect("registry lock poisoned")
            .get(command)
            .cloned()
    }

    /// Iterate over (command, handler) pairs.
    pub fn iter(&self) -> Vec<(String, Arc<dyn SlashHandler>)> {
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
