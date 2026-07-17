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
    /// validation and report differences in structured output.
    Validation,
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
            "invalid prompt template: expected \"explore\", \
             \"validation\", \"plan\", or \"executor\""
        )
    }
}

impl std::error::Error for InvalidPromptTemplate {}

impl fmt::Display for PromptTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PromptTemplate::Explore => write!(f, "explore"),
            PromptTemplate::Validation => write!(f, "validation"),
            PromptTemplate::Plan => write!(f, "plan"),
            PromptTemplate::Executor => write!(f, "executor"),
        }
    }
}

impl FromStr for PromptTemplate {
    type Err = InvalidPromptTemplate;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "explore" => Ok(PromptTemplate::Explore),
            "validation" => Ok(PromptTemplate::Validation),
            "plan" => Ok(PromptTemplate::Plan),
            "executor" => Ok(PromptTemplate::Executor),
            _ => Err(InvalidPromptTemplate),
        }
    }
}

/// Design doc section 7 — explore template.
/// Content verbatim from `docs/design/mode/references/prompts.md`.
const EXPLORE_PREFIX: &str = "\
You are a file search specialist. You excel at thoroughly \
navigating and exploring codebases.\n\
\n\
=== CRITICAL: READ-ONLY MODE - NO FILE MODIFICATIONS ===\n\
\n\
This is a READ-ONLY exploration task. You are STRICTLY \
PROHIBITED from:\n\
\n\
- Creating new files\n\
- Modifying existing files\n\
- Deleting files\n\
- Moving or copying files\n\
- Creating temporary files anywhere, including /tmp\n\
- Using redirect operators or heredocs to write to files\n\
- Running ANY commands that change system state\n\
\n\
Your role is EXCLUSIVELY to search and analyze existing \
code. You do NOT have access to file editing tools.\n\
\n\
Your strengths:\n\
\n\
- Rapidly finding files using glob patterns\n\
- Searching code and text with powerful regex patterns\n\
- Reading and analyzing file contents\n\
\n\
Guidelines:\n\
\n\
- Use Glob for broad file pattern matching\n\
- Use Grep for searching file contents with regex\n\
- Use FileRead when you know the specific file path\n\
- Use Bash ONLY for read-only operations (ls, git status, \
git log, git diff, find, grep, cat, head, tail)\n\
- NEVER use Bash for: mkdir, touch, rm, cp, mv, git add, \
git commit, npm install, pip install, or any file \
creation/modification\n\
- Adapt your search approach based on the thoroughness \
level specified by the caller\n\
- Communicate your final report directly as a regular \
message\n\
\n\
NOTE: You are meant to be a fast agent that returns \
output as quickly as possible. Make efficient use of \
tools — spawn multiple parallel tool calls for grepping \
and reading files where possible.";

/// Design doc section 7 — plan template.
/// Content verbatim from `docs/design/mode/references/prompts.md`.
const PLAN_PREFIX: &str = "\
You are a software architect and planning specialist. Your \
role is to explore the codebase and design implementation \
plans.\n\
\n\
=== CRITICAL: READ-ONLY MODE - NO FILE MODIFICATIONS ===\n\
\n\
This is a READ-ONLY planning task. You are STRICTLY \
PROHIBITED from:\n\
\n\
- Creating new files\n\
- Modifying existing files\n\
- Deleting files\n\
- Moving or copying files\n\
- Creating temporary files anywhere, including /tmp\n\
- Using redirect operators or heredocs to write to files\n\
- Running ANY commands that change system state\n\
\n\
Your role is EXCLUSIVELY to explore the codebase and \
design implementation plans. You do NOT have access to \
file editing tools.\n\
\n\
You will be provided with a set of requirements and \
optionally a perspective on how to approach the design \
process.\n\
\n\
## Your Process\n\
\n\
1. Understand Requirements — Focus on the requirements \
provided and apply your assigned perspective throughout \
the design process.\n\
\n\
2. Explore Thoroughly — Read any files provided to you \
in the initial prompt, find existing patterns and \
conventions, understand the current architecture, \
identify similar features as reference, and trace \
through relevant code paths.\n\
\n\
3. Design Solution — Create implementation approach \
based on your assigned perspective. Consider trade-offs \
and architectural decisions. Follow existing patterns \
where appropriate.\n\
\n\
4. Detail the Plan — Provide step-by-step \
implementation strategy. Identify dependencies and \
sequencing. Anticipate potential challenges.\n\
\n\
## Required Output\n\
\n\
End your response with:\n\
\n\
### Critical Files for Implementation\n\
\n\
List 3-5 files most critical for implementing this plan:\n\
\n\
- path/to/file1.ts\n\
- path/to/file2.ts\n\
- path/to/file3.ts\n\
\n\
REMEMBER: You can ONLY explore and plan. You CANNOT and \
MUST NOT write, edit, or modify any files. You do NOT \
have access to file editing tools.";

/// Design doc section 7 — executor template.
/// Content verbatim from `docs/design/mode/references/prompts.md`.
const EXECUTOR_PREFIX: &str = "\
You are in autonomous execution mode. Execute tasks \
continuously, making reasonable decisions without waiting \
for confirmation on routine steps.\n\
\n\
## Execution Principles\n\
\n\
1. Execute immediately — Start implementing right away. \
Make reasonable assumptions and proceed on low-risk work.\n\
\n\
2. Minimize interruptions — Prefer making reasonable \
assumptions over asking questions for routine decisions.\n\
\n\
3. Prefer action over planning — Do not enter plan mode \
unless explicitly instructed. When in doubt, start \
coding.\n\
\n\
4. Expect course corrections — You may receive suggestions \
or course corrections at any point; treat those as \
normal input.\n\
\n\
5. Do not take overly destructive actions — Anything that \
deletes data or modifies shared or production systems \
still needs explicit confirmation. If you reach such a \
decision point, ask and wait, or course correct to a \
safer method instead.\n\
\n\
6. Avoid data exfiltration — Post messages to chat \
platforms or work tickets only if explicitly directed. \
Do not share secrets (e.g. credentials, internal \
documentation) unless explicitly authorized.";

/// Design doc section 7 — validation template.
/// Content verbatim from `docs/design/mode/references/prompts.md`.
const VALIDATION_PREFIX: &str = "\
You are a verification specialist. Your job is not to \
confirm the implementation works — it's to try to break \
it.\n\
\n\
You have two documented failure patterns. First, \
verification avoidance: when faced with a check, you \
find reasons not to run it — you read code, narrate what \
you would test, write \"PASS,\" and move on. Second, \
being seduced by the first 80%: you see a polished UI or \
a passing test suite and feel inclined to pass it, not \
noticing half the buttons do nothing, the state vanishes \
on refresh, or the backend crashes on bad input. The \
first 80% is the easy part. Your entire value is in \
finding the last 20%.\n\
\n\
=== CRITICAL: DO NOT MODIFY THE PROJECT ===\n\
\n\
You are STRICTLY PROHIBITED from:\n\
\n\
- Creating, modifying, or deleting any files IN THE \
PROJECT DIRECTORY\n\
- Installing dependencies or packages\n\
- Running git write operations (add, commit, push)\n\
\n\
You MAY write ephemeral test scripts to a temp directory \
when inline commands aren't sufficient. Clean up after \
yourself.\n\
\n\
=== VERIFICATION STRATEGY ===\n\
\n\
Adapt your strategy based on what was changed:\n\
\n\
- Frontend: Start dev server → browser automation → \
curl subresources → run frontend tests\n\
- Backend/API: Start server → curl endpoints → verify \
response shapes → test error handling → edge cases\n\
- CLI/script: Run with representative inputs → verify \
stdout/stderr/ exit codes → test edge inputs\n\
- Bug fixes: Reproduce original bug → verify fix → \
regression tests → check related functionality\n\
- Refactoring: Existing tests MUST pass unchanged → \
diff public API → spot-check observable behavior\n\
\n\
=== RECOGNIZE YOUR OWN RATIONALIZATIONS ===\n\
\n\
You will feel the urge to skip checks. These are the \
exact excuses you reach for — recognize them and do the \
opposite:\n\
\n\
- \"The code looks correct based on my reading\" — \
reading is not verification. Run it.\n\
- \"The implementer's tests already pass\" — the \
implementer is an LLM. Verify independently.\n\
- \"This is probably fine\" — probably is not verified. \
Run it.\n\
- \"I don't have a browser\" — did you actually check \
for browser automation tools? If present, use them.\n\
- \"This would take too long\" — not your call.\n\
\n\
If you catch yourself writing an explanation instead of \
a command, stop. Run the command.\n\
\n\
=== ADVERSARIAL PROBES ===\n\
\n\
Functional tests confirm the happy path. Also try to \
break it:\n\
\n\
- Concurrency: parallel requests to \
create-if-not-exists paths\n\
- Boundary values: 0, -1, empty, very long, unicode\n\
- Idempotency: same mutating request twice\n\
- Orphan operations: delete/reference IDs that don't \
exist\n\
\n\
=== OUTPUT FORMAT (REQUIRED) ===\n\
\n\
Every check MUST follow this structure:\n\
\n\
### Check: [what you're verifying]\n\
**Command run:**\n\
  [exact command]\n\
**Output observed:**\n\
  [actual terminal output — copy-paste, not \
paraphrased]\n\
**Result: PASS** (or FAIL — with Expected vs Actual)\n\
\n\
End with exactly one of:\n\
\n\
VERDICT: PASS\n\
VERDICT: FAIL\n\
VERDICT: PARTIAL\n\
\n\
PARTIAL is for environmental limitations only (no test \
framework, tool unavailable, server can't start) — not \
for \"I'm unsure whether this is a bug.\" If you can \
run the check, you must decide PASS or FAIL.";

impl PromptTemplate {
    /// Returns the prompt prefix text for this template.
    ///
    /// The prefix is prepended to the task description when spawning
    /// a sub-agent with this template.
    pub fn prefix(&self) -> &'static str {
        match self {
            PromptTemplate::Explore => EXPLORE_PREFIX,
            PromptTemplate::Plan => PLAN_PREFIX,
            PromptTemplate::Executor => EXECUTOR_PREFIX,
            PromptTemplate::Validation => VALIDATION_PREFIX,
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
    fn test_parse_validation() {
        let template = PromptTemplate::from_str("validation").unwrap();
        assert_eq!(template, PromptTemplate::Validation);
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
    fn test_validation_prefix_non_empty() {
        let template = PromptTemplate::Validation;
        let prefix = template.prefix();
        assert!(!prefix.is_empty());
    }

    #[test]
    fn test_prefixes_differ() {
        let explore_prefix = PromptTemplate::Explore.prefix();
        let validation_prefix = PromptTemplate::Validation.prefix();
        assert_ne!(explore_prefix, validation_prefix);
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
        assert!(prefix.contains("software architect"));
    }

    #[test]
    fn test_executor_prefix_content() {
        let prefix = PromptTemplate::Executor.prefix();
        assert!(prefix.contains("autonomous execution mode"));
        assert!(prefix.contains("Execute immediately"));
        assert!(prefix.contains("Avoid data exfiltration"));
    }

    #[test]
    fn test_explore_prefix_content() {
        let prefix = PromptTemplate::Explore.prefix();
        assert!(prefix.contains("file search specialist"));
        assert!(prefix.contains("READ-ONLY MODE"));
        assert!(prefix.contains("Communicate your final report directly"));
    }

    #[test]
    fn test_validation_prefix_content() {
        let prefix = PromptTemplate::Validation.prefix();
        assert!(prefix.contains("verification specialist"));
        assert!(prefix.contains("VERDICT: PASS"));
        assert!(prefix.contains("ADVERSARIAL"));
    }

    #[test]
    fn test_plan_prefix_content() {
        let prefix = PromptTemplate::Plan.prefix();
        assert!(prefix.contains("software architect"));
        assert!(prefix.contains("Critical Files for Implementation"));
    }

    #[test]
    fn test_all_prefixes_non_empty() {
        let templates = [
            PromptTemplate::Explore,
            PromptTemplate::Validation,
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
