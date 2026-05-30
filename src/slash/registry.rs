use std::collections::HashMap;
use std::sync::Arc;

use crate::slash::handler::SlashHandler;

/// Registry that maps command names to their handlers.
pub(crate) struct HandlerRegistry {
    handlers: HashMap<String, Arc<dyn SlashHandler>>,
}

impl HandlerRegistry {
    /// Create an empty registry.
    pub(crate) fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler. Keyed by [`SlashHandler::name`].
    pub(crate) fn register(&mut self, handler: Arc<dyn SlashHandler>) {
        self.handlers.insert(handler.name().to_owned(), handler);
    }

    /// Look up a handler by command name (without the leading `/`).
    pub(crate) fn get(&self, command: &str) -> Option<Arc<dyn SlashHandler>> {
        self.handlers.get(command).cloned()
    }
}
