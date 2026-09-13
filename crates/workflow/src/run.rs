//! Workflow run state types.

use serde::{Deserialize, Serialize};

/// Hint attached to the next goal message, indicating whether this is a
/// normal first-time injection or a reexecute re-entry.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalHint {
    /// First-time goal injection (default).
    #[default]
    Normal,
    /// Reexecute re-entry; goal message should include a re-execution hint.
    Reexecute,
}

/// Execution phases of a workflow step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Agent is executing step content.
    Executing,
    /// Engine has injected verify checklist, waiting for Agent response.
    Verifying,
    /// Engine has injected jump questions, waiting for Agent answers.
    Jumping,
    /// Blocked waiting for owner intervention.
    Blocked,
    /// Workflow has completed.
    Complete,
}

/// Entry in the step history recording completed steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepHistoryEntry {
    /// The step index that was completed.
    pub step_id: usize,
    /// The step name at time of completion.
    pub step_name: String,
    /// ISO 8601 timestamp when the step was started (entered).
    #[serde(default)]
    pub entered_at: String,
    /// ISO 8601 timestamp when the step was completed.
    pub completed_at: String,
}

/// Persistent state tracking verify injection attempts.
///
/// Tracks the count of injections, the last injection timestamp,
/// and the maximum retry limit before the workflow transitions to Blocked.
///
/// Backward compatible with old checkpoints where `pending_verify` was
/// a bare `usize` — the old format deserializes to
/// `PendingVerify { count: old_value, ..Default::default() }`.
#[derive(Debug, Clone)]
pub struct PendingVerify {
    /// Number of verify attempts since last reset.
    pub count: usize,
    /// ISO 8601 timestamp of the last verify injection, or empty if never injected.
    pub last_inject_time: String,
    /// Maximum number of verify retries before transitioning to Blocked.
    /// Defaults to 3 for backward compatibility.
    pub max_retry_limit: usize,
}

impl Default for PendingVerify {
    fn default() -> Self {
        Self {
            count: 0,
            last_inject_time: String::new(),
            max_retry_limit: 3,
        }
    }
}

impl serde::Serialize for PendingVerify {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("count", &self.count)?;
        map.serialize_entry("last_inject_time", &self.last_inject_time)?;
        map.serialize_entry("max_retry_limit", &self.max_retry_limit)?;
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for PendingVerify {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct PendingVerifyVisitor;

        impl<'de> de::Visitor<'de> for PendingVerifyVisitor {
            type Value = PendingVerify;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a map or an integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                // Old checkpoint format: bare `usize` integer.
                Ok(PendingVerify {
                    count: v as usize,
                    last_inject_time: String::new(),
                    max_retry_limit: 3,
                })
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut count = None;
                let mut last_inject_time = None;
                let mut max_retry_limit = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "count" => count = Some(map.next_value()?),
                        "last_inject_time" => last_inject_time = Some(map.next_value()?),
                        "max_retry_limit" => max_retry_limit = Some(map.next_value()?),
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                Ok(PendingVerify {
                    count: count.unwrap_or(0),
                    last_inject_time: last_inject_time.unwrap_or_default(),
                    max_retry_limit: max_retry_limit.unwrap_or(3),
                })
            }
        }

        deserializer.deserialize_any(PendingVerifyVisitor)
    }
}

/// Runtime state of a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// ID of the workflow being executed.
    pub workflow_id: String,
    /// Name of the workflow definition (used for three-level file lookup).
    ///
    /// Required for post-compaction re-injection: after compaction clears
    /// system_injection_appends, the gateway uses this name to reload the definition
    /// and rebuild the workflow context.
    ///
    /// 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空字符串）。
    #[serde(default)]
    pub definition_name: String,
    /// Version of the workflow definition.
    pub definition_version: String,
    /// Index of the current step (0-based).
    pub current_step: usize,
    /// Current execution phase.
    pub phase: Phase,
    /// ISO 8601 timestamp when the current step was entered.
    #[serde(default)]
    pub current_step_entered_at: String,
    /// History of completed steps.
    pub step_history: Vec<StepHistoryEntry>,
    /// Cross-step shared data.
    #[serde(default)]
    pub step_data: serde_yaml::Value,
    /// Hint for the next goal injection (Normal vs Reexecute).
    #[serde(default)]
    pub pending_goal_hint: GoalHint,
    /// Persistent verify injection state (count, timestamps, retry limit).
    #[serde(default)]
    pub pending_verify: PendingVerify,
    /// Reason the workflow is paused while in `Blocked` phase.
    ///
    /// Only non-empty when `phase == Blocked`. Cleared on any transition
    /// out of `Blocked`. 用 `#[serde(default)]` 兼容旧 checkpoint JSON（无此字段时反序列化为空字符串）。
    #[serde(default)]
    pub paused_reason: String,
}
