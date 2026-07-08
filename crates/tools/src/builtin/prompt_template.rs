//! Prompt template definitions for sub-agent spawning.
//!
//! Provides predefined prompt prefixes that constrain sub-agent behavior modes
//! (read-only research vs. verification/audit). Injected into the sub-agent's
//! first message when spawning via `sessions_spawn`.

use std::fmt;
use std::str::FromStr;

/// Built-in prompt templates for sub-agent behavior modes.
///
/// Each variant carries a pre-defined prompt prefix that is prepended to
/// the task description when spawning a sub-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptTemplate {
    /// Read-only research mode: constrains the sub-agent to investigation
    /// and analysis without modifying any files.
    Explore,
    /// Verification/audit mode: constrains the sub-agent to perform item-by-item
    /// verification and report differences in structured output.
    Verification,
    /// Design-phase mode: read-only tools (no write or approval tools),
    /// architect perspective for generating implementation plans.
    Plan,
    /// Auto Mode execution: full toolset, dangerous operations subject
    /// to review, executes plan tasks step by step.
    Executor,
}

/// Error returned when parsing an invalid prompt template string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPromptTemplate;

impl fmt::Display for InvalidPromptTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid prompt template: expected \"explore\", \"verification\", \"plan\", or \"executor\""
        )
    }
}

impl std::error::Error for InvalidPromptTemplate {}

impl FromStr for PromptTemplate {
    type Err = InvalidPromptTemplate;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "explore" => Ok(PromptTemplate::Explore),
            "verification" => Ok(PromptTemplate::Verification),
            "plan" => Ok(PromptTemplate::Plan),
            "executor" => Ok(PromptTemplate::Executor),
            _ => Err(InvalidPromptTemplate),
        }
    }
}

impl PromptTemplate {
    /// Returns the default tool whitelist for this template.
    ///
    /// When `sessions_spawn` does not receive an explicit `allowedTools`
    /// parameter but specifies a prompt template, this list is applied
    /// automatically so the child agent only has access to the appropriate
    /// tool subset.
    ///
    /// Returns `None` for templates that should inherit the full toolset
    /// (i.e. no override).
    pub fn default_allowed_tools(&self) -> Option<Vec<&'static str>> {
        match self {
            // Read-only research: investigation and analysis tools only.
            PromptTemplate::Explore => Some(vec![
                "Read",
                "Grep",
                "Ls",
                "ToolSearch",
                "GitStatus",
                "GitLog",
                "PermissionQuery",
                "Progress",
            ]),
            // Plan/design: same read-only set, no write or approval tools.
            PromptTemplate::Plan => Some(vec![
                "Read",
                "Grep",
                "Ls",
                "ToolSearch",
                "GitStatus",
                "GitLog",
                "PermissionQuery",
                "Progress",
            ]),
            // Verification/audit: read-only plus Bash for running test scripts.
            PromptTemplate::Verification => Some(vec![
                "Read",
                "Grep",
                "Ls",
                "ToolSearch",
                "Bash",
                "GitStatus",
                "GitLog",
                "PermissionQuery",
                "Progress",
            ]),
            // Auto-mode execution: full toolset, no override.
            PromptTemplate::Executor => None,
        }
    }

    /// Returns the prompt prefix text for this template.
    ///
    /// The prefix is prepended to the task description when spawning a sub-agent
    /// with this template.
    pub fn prefix(&self) -> &'static str {
        match self {
            PromptTemplate::Explore => {
                "You are in READ-ONLY RESEARCH mode. You MUST NOT modify any \
                 files, create new files, delete files, or execute commands that \
                 alter system state. Your sole purpose is to investigate, analyze, \
                 and report findings. Provide your analysis in a structured report \
                 format."
            }
            PromptTemplate::Verification => {
                "You are in VERIFICATION/AUDIT mode. Your task is to perform \
                 item-by-item verification against the given criteria. For each \
                 item: state whether it passes or fails, provide the expected \
                 value, the actual value, and a brief explanation of any \
                 discrepancy. Output your findings as a structured checklist \
                 with clear PASS/FAIL status for every item."
            }
            PromptTemplate::Plan => {
                "You are in PLAN/DESIGN mode. You have READ-ONLY access — \
                 write tools and approval tools are NOT available. Your role is \
                 that of an architect: analyze requirements, explore the codebase, \
                 and produce a structured implementation plan. Output key files \
                 affected, design decisions, and ordered task steps. Do not \
                 modify any files."
            }
            PromptTemplate::Executor => {
                "You are in AUTO MODE EXECUTION. You have the full toolset. \
                 Dangerous operations (file deletion, production config changes, \
                 database mutations) require explicit user approval before \
                 execution. Execute plan tasks step by step, updating each \
                 task checkbox upon completion."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_explore() {
        let template = PromptTemplate::from_str("explore").unwrap();
        assert_eq!(template, PromptTemplate::Explore);
    }

    #[test]
    fn test_parse_verification() {
        let template = PromptTemplate::from_str("verification").unwrap();
        assert_eq!(template, PromptTemplate::Verification);
    }

    #[test]
    fn test_parse_invalid() {
        let result = PromptTemplate::from_str("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_explore_prefix_non_empty() {
        let template = PromptTemplate::Explore;
        let prefix = template.prefix();
        assert!(!prefix.is_empty());
    }

    #[test]
    fn test_verification_prefix_non_empty() {
        let template = PromptTemplate::Verification;
        let prefix = template.prefix();
        assert!(!prefix.is_empty());
    }

    #[test]
    fn test_prefixes_differ() {
        let explore_prefix = PromptTemplate::Explore.prefix();
        let verification_prefix = PromptTemplate::Verification.prefix();
        assert_ne!(explore_prefix, verification_prefix);
    }

    #[test]
    fn test_parse_plan() {
        let template = PromptTemplate::from_str("plan").unwrap();
        assert_eq!(template, PromptTemplate::Plan);
    }

    #[test]
    fn test_parse_executor() {
        let template = PromptTemplate::from_str("executor").unwrap();
        assert_eq!(template, PromptTemplate::Executor);
    }

    #[test]
    fn test_plan_prefix_read_only() {
        let prefix = PromptTemplate::Plan.prefix();
        assert!(prefix.contains("READ-ONLY"));
        assert!(prefix.contains("architect"));
    }

    #[test]
    fn test_executor_prefix_full_toolset() {
        let prefix = PromptTemplate::Executor.prefix();
        assert!(prefix.contains("full toolset"));
        assert!(prefix.to_lowercase().contains("dangerous"));
    }

    #[test]
    fn test_all_prefixes_non_empty() {
        let templates = [
            PromptTemplate::Explore,
            PromptTemplate::Verification,
            PromptTemplate::Plan,
            PromptTemplate::Executor,
        ];
        for tpl in &templates {
            assert!(
                !tpl.prefix().is_empty(),
                "template {:?} has empty prefix",
                tpl
            );
        }
    }
}
