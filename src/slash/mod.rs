pub mod context;
pub mod dispatcher;
pub mod handler;
pub mod handlers;
pub mod registry;

pub use context::SlashContext;
pub use dispatcher::{parse_slash, SlashDispatcher};
pub use handler::{SlashHandler, SlashResult};
pub use handlers::{ClearHandler, CompactHandler, ExecHandler, HelpHandler};

#[cfg(test)]
mod handlers_tests;

#[cfg(test)]
mod handlers_tests_legacy;

#[cfg(test)]
mod handlers_tests_system;

#[cfg(test)]
mod tests;
