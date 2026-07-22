use crate::miner_transcript::clean_transcript;
use closeclaw_config::agents::TranscriptCleanRules;

/// Lenient rules: 1 turn, 1 owner message, md format.
fn lenient_rules() -> TranscriptCleanRules {
    TranscriptCleanRules {
        min_turns: Some(1),
        min_owner_msgs: Some(1),
        format: Some("md".to_string()),
    }
}

fn make_transcript(n_owner: usize, n_agent: usize) -> String {
    let mut lines = Vec::new();
    for i in 0..n_owner {
        lines.push(format!("Owner: owner message {i}"));
        if i < n_agent {
            lines.push(format!("Agent: agent response {i}"));
        }
    }
    lines.join("\n")
}

#[test]
fn test_transcript_clean_removes_thinking_blocks() {
    let raw = "Owner: hello\n<thinking>\nSome thought\n</thinking>\nAgent: hi";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("thinking"));
    assert!(!cleaned.contains("Some thought"));
    assert!(cleaned.contains("hello"));
    assert!(cleaned.contains("hi"));
}

#[test]
fn test_transcript_clean_removes_tool_call_xml() {
    let raw = "Owner: go\n<tool_call>{\"name\":\"exec\"}</tool_call>\nAgent: done";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("tool_call"));
    assert!(cleaned.contains("done"));
}

#[test]
fn test_transcript_clean_removes_context_markers() {
    let raw = "Owner: test\n[context: system prompt]\nAgent: ok";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("[context:"));
}

#[test]
fn test_transcript_clean_removes_media_no_reply() {
    let raw = "Owner: msg\nMEDIA\nNO_REPLY\nAgent: response";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("MEDIA"));
    assert!(!cleaned.contains("NO_REPLY"));
}

#[test]
fn test_transcript_clean_collapses_blank_lines() {
    let raw = "Owner: a\n\n\n\nAgent: b";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(!cleaned.contains("\n\n\n"));
}

#[test]
fn test_transcript_clean_skips_short_transcripts() {
    let raw = "Owner: hi";
    let rules = TranscriptCleanRules {
        min_turns: Some(5),
        min_owner_msgs: Some(5),
        format: Some("md".to_string()),
    };
    let cleaned = clean_transcript(raw, &rules);
    assert!(cleaned.is_empty());
}

#[test]
fn test_transcript_clean_skips_few_owner_messages() {
    let raw = make_transcript(2, 5);
    let rules = TranscriptCleanRules {
        min_owner_msgs: Some(10),
        ..Default::default()
    };
    let cleaned = clean_transcript(&raw, &rules);
    assert!(cleaned.is_empty());
}

#[test]
fn test_transcript_clean_normalizes_owner_prefixes() {
    let raw = "Owner: a\nuser: b\nowner: c";
    let rules = lenient_rules();
    let cleaned = clean_transcript(raw, &rules);
    assert!(cleaned.contains("a"));
    assert!(cleaned.contains("b"));
    assert!(cleaned.contains("c"));
}
