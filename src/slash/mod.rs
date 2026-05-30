pub mod context;
pub mod dispatcher;
pub mod handler;
pub mod registry;

pub use context::SlashContext;
pub use dispatcher::{parse_slash, SlashDispatcher};
pub use handler::{SlashHandler, SlashResult};

#[cfg(test)]
mod tests;
