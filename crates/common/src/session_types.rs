//! Shared session-related types.

use serde::{Deserialize, Serialize};

/// Reasoning Level — 推理深度控制等级
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    /// 低推理深度（最小推理 token 消耗）
    Low,
    /// 中等推理深度
    Medium,
    /// 高推理深度（默认）
    #[default]
    High,
    /// 最大推理深度（最大推理 token 消耗）
    Max,
}

impl std::fmt::Display for ReasoningLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningLevel::Low => write!(f, "low"),
            ReasoningLevel::Medium => write!(f, "medium"),
            ReasoningLevel::High => write!(f, "high"),
            ReasoningLevel::Max => write!(f, "max"),
        }
    }
}

/// Agent Role — 智能体角色枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRole {
    /// 主智能体
    MainAgent,
    /// 分身智能体
    SubAgent,
}
