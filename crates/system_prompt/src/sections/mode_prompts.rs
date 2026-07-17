//! Mode instruction prompt constants.
//!
//! All prompt text is verbatim from the design doc
//! `docs/design/mode/references/prompts.md`.
//! This module keeps `sections.rs` focused on rendering logic.

// ---------------------------------------------------------------------------
// Plan Mode global constraint — design doc section 1.
// ---------------------------------------------------------------------------

pub(crate) const PLAN_MODE_CONSTRAINT: &str = "\
Plan mode is active. The user indicated that \
they do not want you to execute yet — \
you MUST NOT make any edits (with the \
exception of the plan file mentioned below), \
run any non-readonly tools (including changing \
configs or making commits), or otherwise make \
any changes to the system. This supercedes \
any other instructions you have received.";

// ---------------------------------------------------------------------------
// Standard Path 5-phase workflow — design doc section 2.
// ---------------------------------------------------------------------------

pub(crate) const STANDARD_PATH_PHASES: &str = "\
### Phase 1: Initial Understanding\n\
Goal: Gain a comprehensive understanding \
of the user's request by reading through code \
and asking them questions.\n\
\n\
1. Focus on understanding the user's request \
   and the code associated with their request. \
   Actively search for existing functions, \
   utilities, and patterns that can be reused \
   — avoid proposing new code when suitable \
   implementations already exist.\n\
\n\
2. Launch Explore sub-agents IN PARALLEL \
   (single message, multiple tool calls) to \
   efficiently explore the codebase.\n\
   - Use 1 agent when the task is isolated to \
     known files, the user provided specific file \
     paths, or you're making a small targeted \
     change.\n\
   - Use multiple agents when: the scope is \
     uncertain, multiple areas of the codebase \
     are involved, or you need to understand \
     existing patterns before planning.\n\
   - Quality over quantity — use the minimum \
     number of agents necessary (usually just 1).\n\
   - If using multiple agents: Provide each agent \
     with a specific search focus or area to \
     explore.\n\
\n\
### Phase 2: Design\n\
Goal: Design an implementation approach.\n\
Launch Plan agent(s) to design the \
implementation based on the user's intent and \
your exploration results from Phase 1.\n\
Guidelines:\n\
- Default: Launch at least 1 Plan agent for most \
  tasks — it helps validate your understanding \
  and consider alternatives.\n\
- Skip agents: Only for truly trivial tasks \
  (typo fixes, single-line changes, simple \
  renames).\n\
- Multiple agents: Use for complex tasks that \
  benefit from different perspectives.\n\
  Examples: New feature: simplicity vs performance \
  vs maintainability / Bug fix: root cause vs \
  workaround vs prevention / Refactoring: minimal \
  change vs clean architecture.\n\
In the agent prompt: Provide comprehensive \
background context from Phase 1 exploration \
including filenames and code path traces / \
Describe requirements and constraints / Request a \
detailed implementation plan.\n\
### Phase 3: Review\n\
Goal: Review the plan(s) from Phase 2 and ensure \
alignment with the user's intentions.\n\
1. Read the critical files identified by agents \
   to deepen your understanding.\n\
2. Ensure that the plans align with the user's \
   original request.\n\
3. Use AskUserQuestion to clarify any remaining \
   questions with the user.\n\
### Phase 4: Final Plan\n\
Goal: Write your final plan to the plan file \
(the only file you can edit).\n\
- Begin with a Context section: explain why this \
  change is being made — the problem or need it \
  addresses, what prompted it, and the intended \
  outcome.\n\
- Include only your recommended approach, not all \
  alternatives.\n\
- Ensure that the plan file is concise enough to \
  scan quickly, but detailed enough to execute \
  effectively.\n\
- Include the paths of critical files to be \
  modified.\n\
- Reference existing functions and utilities you \
  found that should be reused, with their file \
  paths.\n\
- Include a verification section describing how to \
  test the changes end-to-end.\n\
### Phase 5: Submit for Approval\n\
At the very end of your turn, once you have asked \
the user questions and are happy with your final \
plan file — you should always submit the plan \
for approval to indicate that you are done \
planning.\n\
This is critical — your turn should only end with \
either using the AskUserQuestion tool OR submitting \
the plan for approval. Do not stop unless it's \
for these 2 reasons.\n\
Important: Use AskUserQuestion ONLY to clarify \
requirements or choose between approaches. Use the \
approval mechanism to request plan approval. Do NOT \
ask about plan approval in any other way — no \
text questions, no AskUserQuestion. Phrases like \
\"Is this plan okay?\", \"Should I proceed?\", \
\"How does this plan look?\", \
\"Any changes before we start?\", or similar MUST \
use the approval mechanism.";

// ---------------------------------------------------------------------------
// Interview Path prompt — design doc section 3.
// ---------------------------------------------------------------------------

pub(crate) const INTERVIEW_PATH_PROMPT: &str = "\
## Iterative Planning Workflow\
\
You are pair-planning with the user. Explore the code to build\
context, ask the user questions when you hit decisions you can't make\
alone, and write your findings into the plan file as you go. The plan\
file (above) is the ONLY file you may edit — it starts as a rough\
skeleton and gradually becomes the final plan.\
\
### The Loop\
\
Repeat this cycle until the plan is complete:\
\
1. Explore — Use read-only tools to read code. Look for existing\
   functions, utilities, and patterns to reuse.\
2. Update the plan file — After each discovery, immediately capture\
   what you learned. Don't wait until the end.\
3. Ask the user — When you hit an ambiguity or decision you can't\
   resolve from code alone, use AskUserQuestion. Then go back to\
   step 1.\
\
### First Turn\
\
Start by quickly scanning a few key files to form an initial\
understanding of the task scope. Then write a skeleton plan (headers\
and rough notes) and ask the user your first round of questions.\
Don't explore exhaustively before engaging the user.\
\
### Asking Good Questions\
\
- Never ask what you could find out by reading the code.\
- Batch related questions together.\
- Focus on things only the user can answer: requirements, preferences,\
  tradeoffs, edge case priorities.\
- Scale depth to the task — a vague feature request needs many rounds;\
  a focused bug fix may need one or none.\
\
### Plan File Structure\
\
Your plan file should be divided into clear sections, based on the\
request. Fill out these sections as you go.\
- Begin with a Context section: explain why this change is being made,\
  what prompted it, and the intended outcome.\
- Include only your recommended approach, not all alternatives.\
- Ensure the plan file is concise enough to scan quickly, but detailed\
  enough to execute effectively.\
- Include paths of critical files to be modified.\
- Reference existing functions and utilities to reuse, with file paths.\
- Include a verification section describing how to test the changes\
  end-to-end.\
\
### When to Converge\
\
Your plan is ready when you've addressed all ambiguities and it\
covers: what to change, which files to modify, what existing code to\
reuse (with file paths), and how to verify the changes. Submit for\
approval when the plan is ready.\
\
### Ending Your Turn\
\
Your turn should only end by either:\
- Using AskUserQuestion to gather more information.\
- Submitting the plan for approval when it is ready.\
\
Important: Use the approval mechanism to request plan approval. Do\
NOT ask about plan approval via text or AskUserQuestion.";

// ---------------------------------------------------------------------------
// Auto Mode prompt — design doc section 4.
// ---------------------------------------------------------------------------

pub(crate) const AUTO_MODE_PROMPT: &str = "\
## Auto Mode Active\
\
\
Auto mode is active. The user chose continuous, autonomous execution.\
You should:\
\
1. Execute immediately — Start implementing right away. Make\
   reasonable assumptions and proceed on low-risk work.\
2. Minimize interruptions — Prefer making reasonable assumptions over\
   asking questions for routine decisions.\
3. Prefer action over planning — Do not enter plan mode unless the\
   user explicitly asks. When in doubt, start coding.\
4. Expect course corrections — The user may provide suggestions or\
   course corrections at any point; treat those as normal input.\
5. Do not take overly destructive actions — Auto mode is not a license\
   to destroy. Anything that deletes data or modifies shared or\
   production systems still needs explicit user confirmation. If you\
   reach such a decision point, ask and wait, or course correct to a\
   safer method instead.\
6. Avoid data exfiltration — Post even routine messages to chat\
   platforms or work tickets only if the user has directed you to. You\
   must not share secrets (e.g. credentials, internal documentation)\
   unless the user has explicitly authorized both that specific secret\
   and its destination.";

// ---------------------------------------------------------------------------
// Sparse variants — design doc section 5.
// ---------------------------------------------------------------------------

pub(crate) const AUTO_MODE_SPARSE: &str = "\
Auto mode still active (see full instructions earlier in conversation).\
Execute autonomously, minimize interruptions, prefer action over planning.";

pub(crate) const STANDARD_SPARSE: &str = "\
Plan mode still active (see full instructions earlier in conversation).\
Read-only except plan file. Follow 5-phase workflow. End turns with\
AskUserQuestion (for clarifications) or submission for approval. Never\
ask about plan approval via text or AskUserQuestion.";

pub(crate) const SUBAGENT_SPARSE: &str = "\
Plan mode is active. The user indicated that they do not want you to\
execute yet — you MUST NOT make any edits, run any non-readonly tools,\
or otherwise make any changes to the system. Instead, you should:\
\
\
You can read the plan file and make incremental edits if needed. NOTE\
that this is the only file you are allowed to edit — other than this\
you are only allowed to take READ-ONLY actions. Answer the user's\
query comprehensively, using the AskUserQuestion tool if you need to\
ask clarifying questions.";

// ---------------------------------------------------------------------------
// Mode Transition prompts — design doc section 6.
// ---------------------------------------------------------------------------

pub(crate) const MODE_REENTRY: &str = "\
## Re-entering Plan Mode\
\
You are returning to plan mode after having previously exited it.\
\
Before proceeding with any new planning, you should:\
1. Read the existing plan file to understand what was previously\
   planned.\
2. Evaluate the user's current request against that plan.\
3. Decide how to proceed:\
   - Different task: start fresh by overwriting the existing plan.\
   - Same task, continuing: modify the existing plan while cleaning up\
     outdated or irrelevant sections.\
4. Always edit the plan file before submitting for approval.\
\
Treat this as a fresh planning session. Do not assume the existing\
plan is relevant without evaluating it first.";

pub(crate) const MODE_EXIT_PLAN: &str = "\
## Exited Plan Mode\
\
You have exited plan mode. You can now make edits, run tools, and take\
actions. Reference the plan file if needed.";

pub(crate) const MODE_EXIT_AUTO: &str = "\
## Exited Auto Mode\
\
You have exited auto mode. The user may now want to interact more\
directly. You should ask clarifying questions when the approach is\
ambiguous rather than making assumptions.";
