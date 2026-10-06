//! SessionManager `ActiveSessionQuery` impl.
//! Segregated in its own submodule so the super module stays within the
//! 1000-line window (see CONTRIBUTING.md).

use super::SessionManager;
use crate::sweeper::ActiveSessionQuery;
use async_trait::async_trait;
use closeclaw_common::SessionActivityDimensions;

#[async_trait]
impl ActiveSessionQuery for SessionManager {
    /// Return the four-dimensional activity state of the session.
    ///
    /// Delegates to [`SessionManager::activity_dimensions`].
    async fn activity_dimensions(&self, session_id: &str) -> SessionActivityDimensions {
        self.activity_dimensions(session_id).await
    }
}
