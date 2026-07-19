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
// Standard Path 4-phase workflow — design doc section 2.
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
  test the changes end-to-end.\n";

// ---------------------------------------------------------------------------
// Interview Path prompt — design doc section 3.
// ---------------------------------------------------------------------------

pub(crate) const INTERVIEW_PATH_PROMPT: &str = "\
## Iterative Planning Workflow\n\
\n\
You are pair-planning with the user. Explore the code to build\n\
context, ask the user questions when you hit decisions you can't make\n\
alone, and write your findings into the plan file as you go. The plan\n\
file (above) is the ONLY file you may edit — it starts as a rough\n\
skeleton and gradually becomes the final plan.\n\
\n\
### The Loop\n\
\n\
Repeat this cycle until the plan is complete:\n\
\n\
1. Explore — Use read-only tools to read code. Look for existing\n\
   functions, utilities, and patterns to reuse.\n\
2. Update the plan file — After each discovery, immediately capture\n\
   what you learned. Don't wait until the end.\n\
3. Ask the user — When you hit an ambiguity or decision you can't\n\
   resolve from code alone, use AskUserQuestion. Then go back to\n\
   step 1.\n\
\n\
### First Turn\n\
\n\
Start by quickly scanning a few key files to form an initial\n\
understanding of the task scope. Then write a skeleton plan (headers\n\
and rough notes) and ask the user your first round of questions.\n\
Don't explore exhaustively before engaging the user.\n\
\n\
### Asking Good Questions\n\
\n\
- Never ask what you could find out by reading the code.\n\
- Batch related questions together.\n\
- Focus on things only the user can answer: requirements, preferences,\n\
  tradeoffs, edge case priorities.\n\
- Scale depth to the task — a vague feature request needs many rounds;\n\
  a focused bug fix may need one or none.\n\
\n\
### Plan File Structure\n\
\n\
Your plan file should be divided into clear sections, based on the\n\
request. Fill out these sections as you go.\n\
- Begin with a Context section: explain why this change is being made,\n\
  what prompted it, and the intended outcome.\n\
- Include only your recommended approach, not all alternatives.\n\
- Ensure the plan file is concise enough to scan quickly, but detailed\n\
  enough to execute effectively.\n\
- Include paths of critical files to be modified.\n\
- Reference existing functions and utilities to reuse, with file paths.\n\
- Include a verification section describing how to test the changes\n\
  end-to-end.\n\
\n\
### When to Converge\n\
\n\
Your plan is ready to advance when you've addressed all ambiguities\n\
and it covers: what to change, which files to modify, what existing\n\
code to reuse (with file paths), and how to verify the changes.\n\
When this is the case, proceed to the Review phase.\n\
\n\
### Review\n\
\n\
Present the complete plan to the user for their review. Show what you\n\
plan to change, which files are involved, and the verification\n\
approach.\n\
\n\
1. Use AskUserQuestion to present the plan and ask the user to\n\
   review it.\n\
2. The user may suggest changes, raise concerns, or request\n\
   alternatives.\n\
3. Adjust the plan based on their feedback, then re-present.\n\
4. Repeat until the user is satisfied with the plan.\n\
\n\
This is not a one-time approval gate — the user can iteratively\n\
review and refine the plan across multiple turns.\n\
\n\
### Final Plan\n\
\n\
Once the user confirms the plan, write the final version to the plan\n\
file (the only file you may edit).\n\
- Begin with a Context section: explain why this change is being\n\
  made — the problem or need it addresses, what prompted it, and\n\
  the intended outcome.\n\
- Include only your recommended approach, not all alternatives.\n\
- Ensure the plan file is concise enough to scan quickly, but\n\
  detailed enough to execute effectively.\n\
- Include the paths of critical files to be modified.\n\
- Reference existing functions and utilities to reuse, with file\n\
  paths.\n\
- Include a verification section describing how to test the changes\n\
  end-to-end.\n\
\n\
After writing, notify the user that the plan is ready and wait for\n\
them to decide when to execute.\n\
\n\
### Ending Your Turn\n\
\n\
Your turn should only end by either:\n\
- Using AskUserQuestion to gather more information or present the\n\
  plan for review.\n\
- Writing the final plan to the plan file after user confirmation.\n\
\n\
Important: In the Review phase, use AskUserQuestion to present the\n\
plan. Do NOT ask about plan approval via text. After writing the\n\
final plan, notify the user and wait for them to initiate execution.";

// ---------------------------------------------------------------------------
// Auto Mode prompt — design doc section 4.
// ---------------------------------------------------------------------------

pub(crate) const AUTO_MODE_PROMPT: &str = "\
## Auto Mode Active\n\
\n\
Auto mode is active. The user chose continuous, autonomous execution.\n\
You should:\n\
\n\
1. Execute immediately — Start implementing right away. Make\n\
   reasonable assumptions and proceed on low-risk work.\n\
2. Minimize interruptions — Prefer making reasonable assumptions over\n\
   asking questions for routine decisions.\n\
3. Prefer action over planning — Do not enter plan mode unless the\n\
   user explicitly asks. When in doubt, start coding.\n\
4. Expect course corrections — The user may provide suggestions or\n\
   course corrections at any point; treat those as normal input.\n\
5. Do not take overly destructive actions — Auto mode is not a license\n\
   to destroy. Anything that deletes data or modifies shared or\n\
   production systems still needs explicit user confirmation. If you\n\
   reach such a decision point, ask and wait, or course correct to a\n\
   safer method instead.\n\
6. Avoid data exfiltration — Post even routine messages to chat\n\
   platforms or work tickets only if the user has directed you to. You\n\
   must not share secrets (e.g. credentials, internal documentation)\n\
   unless the user has explicitly authorized both that specific secret\n\
   and its destination.";

// ---------------------------------------------------------------------------
// Sparse variants — design doc section 5.
// ---------------------------------------------------------------------------

pub(crate) const AUTO_MODE_SPARSE: &str = "\
Auto mode still active (see full instructions earlier in conversation).\n\
Execute autonomously, minimize interruptions, prefer action over planning.";

pub(crate) const STANDARD_SPARSE: &str = "\
Plan mode still active (see full instructions earlier in conversation).\n\
Read-only except plan file. Follow 4-phase workflow. End turns with\n\
AskUserQuestion (for clarifications). Never ask about plan approval via\n\
text or AskUserQuestion.";

pub(crate) const SUBAGENT_SPARSE: &str = "\
Plan mode is active. The user indicated that they do not want you to\n\
execute yet — you MUST NOT make any edits, run any non-readonly tools,\n\
or otherwise make any changes to the system.\n\
\n\
You are only allowed to take READ-ONLY actions. Answer the spawning\n\
agent's query comprehensively, using the available read-only tools.";

// ---------------------------------------------------------------------------
// Mode Transition prompts — design doc section 6.
// ---------------------------------------------------------------------------

pub(crate) const MODE_REENTRY: &str = "\
## Re-entering Plan Mode\n\
\n\
You are returning to plan mode after having previously exited it.\n\
\n\
Before proceeding with any new planning, you should:\n\
1. Read the existing plan file to understand what was previously\n\
   planned.\n\
2. Evaluate the user's current request against that plan.\n\
3. Decide how to proceed:\n\
   - Different task: start fresh by overwriting the existing plan.\n\
   - Same task, continuing: modify the existing plan while cleaning up\n\
     outdated or irrelevant sections.\n\
4. Always edit the plan file to reflect the current plan state.\n\
\n\
Treat this as a fresh planning session. Do not assume the existing\n\
plan is relevant without evaluating it first.";

pub(crate) const MODE_EXIT_PLAN: &str = "\
## Exited Plan Mode\n\
\n\
You have exited plan mode. You can now make edits, run tools, and take\n\
actions. Reference the plan file if needed.";

pub(crate) const MODE_EXIT_AUTO: &str = "\
## Exited Auto Mode\n\
\n\
You have exited auto mode. The user may now want to interact more\n\
directly. You should ask clarifying questions when the approach is\n\
ambiguous rather than making assumptions.";
