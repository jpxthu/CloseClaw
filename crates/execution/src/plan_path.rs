//! Plan Path — plan 双路径选择
//!
//! 标准路径（需求明确）或 Interview 路径（需求模糊）。
//! 无显式指定时由系统自动判断。

use serde::{Deserialize, Serialize};

/// Plan Path — plan 双路径选择
///
/// 标准路径（需求明确）或 Interview 路径（需求模糊）。
/// 无显式指定时由系统自动判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanPath {
    /// 标准路径：需求明确，4 阶段工作流
    #[default]
    Standard,
    /// Interview 路径：需求模糊，循环探索
    Interview,
}

impl std::fmt::Display for PlanPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Standard => write!(f, "standard"),
            Self::Interview => write!(f, "interview"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_path_default_is_standard() {
        assert_eq!(PlanPath::default(), PlanPath::Standard);
    }

    #[test]
    fn test_plan_path_all_variants() {
        let variants = [PlanPath::Standard, PlanPath::Interview];
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn test_plan_path_serde_snake_case() {
        let cases = [
            (PlanPath::Standard, r#""standard""#),
            (PlanPath::Interview, r#""interview""#),
        ];
        for (path, expected_json) in cases {
            let json = serde_json::to_string(&path).unwrap();
            assert_eq!(
                json, expected_json,
                "path {:?} should serialize to {}",
                path, expected_json
            );
            let deserialized: PlanPath = serde_json::from_str(expected_json).unwrap();
            assert_eq!(deserialized, path);
        }
    }

    #[test]
    fn test_plan_path_display() {
        assert_eq!(PlanPath::Standard.to_string(), "standard");
        assert_eq!(PlanPath::Interview.to_string(), "interview");
    }
}
