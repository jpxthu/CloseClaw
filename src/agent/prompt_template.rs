//! Prompt template definitions for sub-agent spawning.
//!
//! Provides predefined prompt prefixes that constrain sub-agent behavior modes
//! (read-only research vs. validation/audit). Injected into the sub-agent's
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
    /// Validation/audit mode: constrains the sub-agent to perform item-by-item
    /// verification and report differences in structured output.
    Validation,
}

/// Error returned when parsing an invalid prompt template string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPromptTemplate;

impl fmt::Display for InvalidPromptTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid prompt template: expected \"explore\" or \"validation\""
        )
    }
}

impl std::error::Error for InvalidPromptTemplate {}

impl FromStr for PromptTemplate {
    type Err = InvalidPromptTemplate;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "explore" => Ok(PromptTemplate::Explore),
            "validation" => Ok(PromptTemplate::Validation),
            _ => Err(InvalidPromptTemplate),
        }
    }
}

impl PromptTemplate {
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
            PromptTemplate::Validation => {
                "You are in VALIDATION/AUDIT mode. Your task is to perform \
                 item-by-item verification against the given criteria. For each \
                 item: state whether it passes or fails, provide the expected \
                 value, the actual value, and a brief explanation of any \
                 discrepancy. Output your findings as a structured checklist \
                 with clear PASS/FAIL status for every item."
            }
        }
    }
}
