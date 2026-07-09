pub mod context;
pub mod dispatcher;
pub mod handler;
pub mod handlers;
pub mod handlers_mode;
pub mod handlers_permission;
pub mod handlers_session;
pub mod handlers_user;
pub mod registry;

pub use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
pub use context::SlashContext;
pub use dispatcher::{parse_slash, SlashDispatcher};
pub use handler::SlashHandler;
pub use handlers::{ClearHandler, CompactHandler, ExecHandler, HelpHandler};
pub use handlers_mode::{ExecuteHandler, ModeHandler, PauseHandler, PlanModeHandler};
pub use handlers_permission::PermissionSlashHandler;
pub use handlers_session::{NewSessionHandler, StatusHandler, StopHandler, VerboseHandler};
pub use handlers_user::UserSlashHandler;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod handlers_tests;

#[cfg(test)]
mod handlers_tests_new;

#[cfg(test)]
mod handlers_tests_legacy;

#[cfg(test)]
mod handlers_tests_system;

#[cfg(test)]
mod handlers_mode_tests;
#[cfg(test)]
mod handlers_permission_tests;
#[cfg(test)]
mod pause_handler_tests;

#[cfg(test)]
pub mod handlers_user_tests;
