//! Unit tests for SectionType and PromptFragment (migrated from common).

use super::{PromptFragment, SectionType};

// =========================================================================
// SectionType
// =========================================================================

#[test]
fn test_section_type_variants() {
    let bootstrap = SectionType::Bootstrap;
    let tools = SectionType::Tools;
    let skills = SectionType::Skills;
    let memory = SectionType::Memory;

    // Verify all variants are constructible and distinct.
    assert_ne!(bootstrap, tools);
    assert_ne!(tools, skills);
    assert_ne!(skills, memory);
    assert_ne!(bootstrap, memory);
}

#[test]
fn test_section_type_debug() {
    let variants = [
        SectionType::Bootstrap,
        SectionType::Tools,
        SectionType::Skills,
        SectionType::Memory,
    ];
    for v in variants {
        let debug = format!("{:?}", v);
        assert!(!debug.is_empty());
    }
}

#[test]
fn test_section_type_clone() {
    let original = SectionType::Tools;
    let cloned = original;
    assert_eq!(original, cloned);
}

#[test]
fn test_section_type_hash() {
    use std::collections::HashMap;

    let mut map = HashMap::new();
    map.insert(SectionType::Bootstrap, "bootstrap");
    map.insert(SectionType::Tools, "tools");
    map.insert(SectionType::Skills, "skills");
    map.insert(SectionType::Memory, "memory");

    assert_eq!(map.len(), 4);
    assert_eq!(map.get(&SectionType::Bootstrap), Some(&"bootstrap"));
    assert_eq!(map.get(&SectionType::Tools), Some(&"tools"));
}

// =========================================================================
// PromptFragment
// =========================================================================

#[test]
fn test_prompt_fragment_fields() {
    let frag = PromptFragment {
        section_title: "## Tools".to_string(),
        section_type: SectionType::Tools,
        content: "tool listing".to_string(),
    };
    assert_eq!(frag.section_title, "## Tools");
    assert_eq!(frag.section_type, SectionType::Tools);
    assert_eq!(frag.content, "tool listing");
}

#[test]
fn test_prompt_fragment_clone() {
    let frag = PromptFragment {
        section_title: "## Memory".to_string(),
        section_type: SectionType::Memory,
        content: "memory content".to_string(),
    };
    let cloned = frag.clone();
    assert_eq!(cloned.section_title, frag.section_title);
    assert_eq!(cloned.section_type, frag.section_type);
    assert_eq!(cloned.content, frag.content);
}

#[test]
fn test_prompt_fragment_debug() {
    let frag = PromptFragment {
        section_title: "## Skills".to_string(),
        section_type: SectionType::Skills,
        content: "skills listing".to_string(),
    };
    let debug = format!("{:?}", frag);
    assert!(debug.contains("PromptFragment"));
    assert!(debug.contains("## Skills"));
}
