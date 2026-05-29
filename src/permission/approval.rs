//! Approval Queue - In-memory pending approval management
//!
//! Provides a queue for operations that are denied and require owner approval.
//!
//! # Architecture
//! - `ApprovalQueue` manages pending approvals keyed by `RequestId`.
//! - Deduplication is based on `OperationKey` (SHA256 of caller + `PermissionRequestBody` JSON).
//! - Callbacks are triggered on approve/deny/clear operations.
//!
//! # Example
//! ```
//! use closeclaw::permission::approval::{ApprovalQueue, ApproveOrDeny};
//! use closeclaw::permission::engine::engine_types::{Caller, PermissionRequestBody};
//! use closeclaw::permission::engine::engine_risk::RiskLevel;
//!
//! let mut queue = ApprovalQueue::new();
//! let request = PermissionRequestBody::ToolCall {
//!     agent: "test".to_string(),
//!     skill: "test_skill".to_string(),
//!     method: "test_method".to_string(),
//! };
//! let caller = Caller {
//!     user_id: "user_123".to_string(),
//!     agent: "agent_001".to_string(),
//!     creator_id: "creator_001".to_string(),
//! };
//! let request_id = queue
//!     .enqueue(
//!         request,
//!         caller,
//!         "Test operation".to_string(),
//!         RiskLevel::Medium,
//!         "session_resume".to_string(),
//!         Box::new(|result| {
//!             println!("Result: {:?}", result);
//!         }),
//!     )
//!     .expect("enqueue succeeded");
//! ```

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

use super::engine::engine_risk::RiskLevel;
use super::engine::engine_types::{Caller, PermissionRequestBody};

/// Unique identifier for a pending approval request.
pub type RequestId = String;

/// SHA256 hex of `PermissionRequestBody` JSON — used for deduplication.
pub type OperationKey = String;

/// Result of an approval decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApproveOrDeny {
    Approve,
    Deny,
}

/// Reason why a request was rejected at enqueue time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Duplicate,
}

/// Callback invoked when an approval decision is made.
pub type Callback = Box<dyn FnOnce(ApproveOrDeny) + Send>;

/// A pending approval entry.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    /// Unique request identifier.
    pub request_id: RequestId,
    /// Caller that initiated the operation.
    pub caller: Caller,
    /// SHA256 key for deduplication.
    pub operation_key: OperationKey,
    /// Human-readable operation description.
    pub operation_desc: String,
    /// Risk level of the operation.
    pub risk_level: RiskLevel,
    /// Session resume handle (opaque token for session continuation).
    pub session_resume: String,
    /// When this entry was created.
    pub created_at: DateTime<Utc>,
}

/// In-memory queue for pending approval requests.
pub struct ApprovalQueue {
    pending: HashMap<RequestId, PendingApproval>,
    callbacks: HashMap<RequestId, Callback>,
}

// Manual Debug impl because Callback is not Debug.
impl std::fmt::Debug for ApprovalQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalQueue")
            .field("pending", &self.pending)
            .finish()
    }
}

impl Default for ApprovalQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalQueue {
    /// Create a new empty approval queue.
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            callbacks: HashMap::new(),
        }
    }

    /// Compute the operation key (SHA256 hex) for a permission request.
    ///
    /// Used for deduplication: same caller + same body produce identical keys.
    /// Different callers with the same body produce different keys.
    pub fn compute_operation_key(caller: &Caller, body: &PermissionRequestBody) -> OperationKey {
        let json = serde_json::to_string(body).expect("PermissionRequestBody is serializable");
        let input = format!("{}:{}:{}", caller.user_id, caller.agent, json);
        let hash = Sha256::digest(input.as_bytes());
        hex::encode(hash)
    }

    /// Enqueue a new pending approval request.
    ///
    /// Returns the new `RequestId` on success, or `RejectReason::Duplicate`
    /// if an equivalent request (same body → same operation_key) is already pending.
    pub fn enqueue(
        &mut self,
        request: PermissionRequestBody,
        caller: Caller,
        operation_desc: String,
        risk_level: RiskLevel,
        session_resume: String,
        callback: Callback,
    ) -> Result<RequestId, RejectReason> {
        let operation_key = Self::compute_operation_key(&caller, &request);

        // Check for duplicate by scanning all pending entries.
        let is_duplicate = self
            .pending
            .values()
            .any(|p| p.operation_key == operation_key);
        if is_duplicate {
            return Err(RejectReason::Duplicate);
        }

        let request_id = Uuid::new_v4().to_string();
        let pending = PendingApproval {
            request_id: request_id.clone(),
            caller,
            operation_key,
            operation_desc,
            risk_level,
            session_resume,
            created_at: Utc::now(),
        };

        self.callbacks.insert(request_id.clone(), callback);
        self.pending.insert(request_id.clone(), pending);
        Ok(request_id)
    }

    /// Approve a pending request by its ID.
    ///
    /// Triggers the callback with `ApproveOrDeny::Approve` and removes the entry.
    /// Returns `true` if the request existed and was approved, `false` otherwise.
    pub fn approve(&mut self, request_id: &str) -> bool {
        self.do_resolve(request_id, ApproveOrDeny::Approve)
    }

    /// Deny a pending request by its ID.
    ///
    /// Triggers the callback with `ApproveOrDeny::Deny` and removes the entry.
    /// Returns `true` if the request existed and was denied, `false` otherwise.
    pub fn deny(&mut self, request_id: &str) -> bool {
        self.do_resolve(request_id, ApproveOrDeny::Deny)
    }

    /// Internal helper to resolve (approve or deny) a pending request.
    fn do_resolve(&mut self, request_id: &str, result: ApproveOrDeny) -> bool {
        if let Some(callback) = self.callbacks.remove(request_id) {
            callback(result);
            self.pending.remove(request_id);
            true
        } else {
            false
        }
    }

    /// Clear all pending approvals.
    ///
    /// Triggers a `Deny` callback for every pending entry, then empties the queue.
    pub fn clear(&mut self) {
        for request_id in self.pending.keys().cloned().collect::<Vec<_>>() {
            self.deny(&request_id);
        }
    }

    /// Get a pending approval entry by its request ID.
    pub fn get_pending(&self, request_id: &str) -> Option<&PendingApproval> {
        self.pending.get(request_id)
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
