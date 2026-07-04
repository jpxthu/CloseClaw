pub use closeclaw_common::{FragmentContext, PromptFragmentProvider};

/// Section type classification for a prompt fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionType {
    /// Bootstrap files (agent profile, workspace rules, etc.)
    Bootstrap,
    /// Tool registry / tool listings
    Tools,
    /// Skill listings
    Skills,
    /// Long-term memory (MEMORY.md)
    Memory,
}

/// A single prompt fragment produced by a [`PromptFragmentProvider`].
#[derive(Debug, Clone)]
pub struct PromptFragment {
    /// Section title, e.g. `"## AGENTS.md"` or `"## Available Skills"`.
    pub section_title: String,
    /// Classification of the section content.
    pub section_type: SectionType,
    /// Rendered text content of the section.
    pub content: String,
}
