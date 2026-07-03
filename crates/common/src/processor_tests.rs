//! Unit tests for DslInstruction, DslParseResult, and ProcessedMessage.

use std::collections::HashMap;

use crate::processor::{
    ContentBlock, ContentBlockType, DslInstruction, DslParseResult, ProcessedMessage,
};

// ---------------------------------------------------------------------------
// DslInstruction
// ---------------------------------------------------------------------------

#[test]
fn test_dsl_instruction_roundtrip() {
    let mut params = HashMap::new();
    params.insert("label".into(), "Submit".into());
    params.insert("action".into(), "submit_form".into());
    params.insert("value".into(), "42".into());

    let inst = DslInstruction {
        instruction_type: "button".into(),
        params,
    };

    let json = serde_json::to_string(&inst).unwrap();
    let de: DslInstruction = serde_json::from_str(&json).unwrap();
    assert_eq!(de.instruction_type, "button");
    assert_eq!(de.params["label"], "Submit");
    assert_eq!(de.params["action"], "submit_form");
    assert_eq!(de.params["value"], "42");
}

#[test]
fn test_dsl_instruction_empty_params() {
    let inst = DslInstruction {
        instruction_type: "divider".into(),
        params: HashMap::new(),
    };

    let json = serde_json::to_string(&inst).unwrap();
    let de: DslInstruction = serde_json::from_str(&json).unwrap();
    assert_eq!(de.instruction_type, "divider");
    assert!(de.params.is_empty());
}

#[test]
fn test_dsl_instruction_special_chars_in_params() {
    let mut params = HashMap::new();
    params.insert("label".into(), "Line 1\nLine 2".into());
    params.insert("url".into(), "https://example.com?a=1&b=2".into());

    let inst = DslInstruction {
        instruction_type: "button".into(),
        params,
    };

    let json = serde_json::to_string(&inst).unwrap();
    let de: DslInstruction = serde_json::from_str(&json).unwrap();
    assert_eq!(de.params["label"], "Line 1\nLine 2");
    assert_eq!(de.params["url"], "https://example.com?a=1&b=2");
}

#[test]
fn test_dsl_instruction_from_json_literal() {
    let json = r#"{
        "instruction_type": "selector",
        "params": {
            "label": "Pick one",
            "options": "a,b,c",
            "action": "select"
        }
    }"#;
    let inst: DslInstruction = serde_json::from_str(json).unwrap();
    assert_eq!(inst.instruction_type, "selector");
    assert_eq!(inst.params["label"], "Pick one");
    assert_eq!(inst.params["options"], "a,b,c");
}

// ---------------------------------------------------------------------------
// DslParseResult
// ---------------------------------------------------------------------------

#[test]
fn test_dsl_parse_result_empty() {
    let result = DslParseResult {
        instructions: vec![],
    };
    let json = serde_json::to_string(&result).unwrap();
    let de: DslParseResult = serde_json::from_str(&json).unwrap();
    assert!(de.instructions.is_empty());
}

#[test]
fn test_dsl_parse_result_single() {
    let result = DslParseResult {
        instructions: vec![DslInstruction {
            instruction_type: "button".into(),
            params: HashMap::from([("label".into(), "OK".into())]),
        }],
    };
    let json = serde_json::to_string(&result).unwrap();
    let de: DslParseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(de.instructions.len(), 1);
    assert_eq!(de.instructions[0].instruction_type, "button");
}

#[test]
fn test_dsl_parse_result_multiple() {
    let result = DslParseResult {
        instructions: vec![
            DslInstruction {
                instruction_type: "button".into(),
                params: HashMap::from([("label".into(), "A".into())]),
            },
            DslInstruction {
                instruction_type: "selector".into(),
                params: HashMap::from([("label".into(), "B".into())]),
            },
            DslInstruction {
                instruction_type: "button".into(),
                params: HashMap::from([("label".into(), "C".into())]),
            },
        ],
    };
    let json = serde_json::to_string(&result).unwrap();
    let de: DslParseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(de.instructions.len(), 3);
    assert_eq!(de.instructions[1].instruction_type, "selector");
}

// ---------------------------------------------------------------------------
// ProcessedMessage
// ---------------------------------------------------------------------------

#[test]
fn test_processed_message_from_raw_content() {
    let msg = ProcessedMessage::from_raw_content("hello world".into());
    assert_eq!(msg.content_blocks.len(), 1);
    assert_eq!(msg.text_content(), Some("hello world"));
    assert!(msg.metadata.is_empty());
}

#[test]
fn test_processed_message_text_content_none_for_non_text() {
    let msg = ProcessedMessage {
        content_blocks: vec![ContentBlock::Image {
            name: "img.png".into(),
            url: "https://example.com/img.png".into(),
        }],
        metadata: HashMap::new(),
    };
    assert!(msg.text_content().is_none());
}

#[test]
fn test_processed_message_text_content_first_text_wins() {
    let msg = ProcessedMessage {
        content_blocks: vec![
            ContentBlock::Image {
                name: "img.png".into(),
                url: "https://example.com/img.png".into(),
            },
            ContentBlock::Text("found it".into()),
        ],
        metadata: HashMap::new(),
    };
    assert_eq!(msg.text_content(), Some("found it"));
}

#[test]
fn test_processed_message_empty_content_blocks() {
    let msg = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    assert!(msg.text_content().is_none());
}

#[test]
fn test_processed_message_metadata_read_write() {
    let mut metadata = HashMap::new();
    metadata.insert("session_id".into(), "sess_123".into());
    metadata.insert("trace_id".into(), "tr_abc".into());

    let msg = ProcessedMessage {
        content_blocks: vec![],
        metadata,
    };

    assert_eq!(msg.metadata["session_id"], "sess_123");
    assert_eq!(msg.metadata["trace_id"], "tr_abc");
}

#[test]
fn test_processed_message_metadata_empty() {
    let msg = ProcessedMessage {
        content_blocks: vec![],
        metadata: HashMap::new(),
    };
    assert!(msg.metadata.is_empty());
}

#[test]
fn test_processed_message_serialization_roundtrip() {
    let msg = ProcessedMessage {
        content_blocks: vec![ContentBlock::Text("hi".into())],
        metadata: HashMap::from([("k".into(), "v".into())]),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let de: ProcessedMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(de.content_blocks.len(), 1);
    assert_eq!(de.metadata["k"], "v");
}

#[test]
fn test_processed_message_content_blocks_absent_deserializes_to_default() {
    let json = r#"{"metadata":{"a":"b"}}"#;
    let msg: ProcessedMessage = serde_json::from_str(json).unwrap();
    assert!(msg.content_blocks.is_empty());
    assert_eq!(msg.metadata["a"], "b");
}

// ---------------------------------------------------------------------------
// ContentBlockType serialization / deserialization
// ---------------------------------------------------------------------------

#[test]
fn test_content_block_type_text_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::Text).unwrap();
    assert_eq!(json, r#""text""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::Text);
}

#[test]
fn test_content_block_type_thinking_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::Thinking).unwrap();
    assert_eq!(json, r#""thinking""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::Thinking);
}

#[test]
fn test_content_block_type_tool_use_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::ToolUse).unwrap();
    assert_eq!(json, r#""tool_use""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::ToolUse);
}

#[test]
fn test_content_block_type_tool_result_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::ToolResult).unwrap();
    assert_eq!(json, r#""tool_result""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::ToolResult);
}

#[test]
fn test_content_block_type_image_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::Image).unwrap();
    assert_eq!(json, r#""image""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::Image);
}

#[test]
fn test_content_block_type_audio_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::Audio).unwrap();
    assert_eq!(json, r#""audio""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::Audio);
}

#[test]
fn test_content_block_type_file_roundtrip() {
    let json = serde_json::to_string(&ContentBlockType::File).unwrap();
    assert_eq!(json, r#""file""#);
    let de: ContentBlockType = serde_json::from_str(&json).unwrap();
    assert_eq!(de, ContentBlockType::File);
}

#[test]
fn test_content_block_type_invalid_value_fails() {
    assert!(serde_json::from_str::<ContentBlockType>(r#""unknown""#).is_err());
}

#[test]
fn test_content_block_type_all_seven_variants() {
    // Verify ContentBlockType has exactly 7 distinct variants.
    let all_types = [
        ContentBlockType::Text,
        ContentBlockType::Thinking,
        ContentBlockType::ToolUse,
        ContentBlockType::ToolResult,
        ContentBlockType::Image,
        ContentBlockType::Audio,
        ContentBlockType::File,
    ];
    // All serialize to distinct strings.
    let mut serialized: Vec<String> = all_types
        .iter()
        .map(|t| serde_json::to_string(t).unwrap())
        .collect();
    serialized.sort();
    serialized.dedup();
    assert_eq!(serialized.len(), 7, "expected 7 distinct variants");
}

// ---------------------------------------------------------------------------
// ContentBlock serialization / deserialization (new variants)
// ---------------------------------------------------------------------------

#[test]
fn test_content_block_tool_result_roundtrip() {
    let block = ContentBlock::ToolResult {
        tool_call_id: "tc_1".into(),
        content: "result data".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    match de {
        ContentBlock::ToolResult {
            tool_call_id,
            content,
        } => {
            assert_eq!(tool_call_id, "tc_1");
            assert_eq!(content, "result data");
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

#[test]
fn test_content_block_image_roundtrip() {
    let block = ContentBlock::Image {
        name: "photo.jpg".into(),
        url: "https://example.com/photo.jpg".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(
        de,
        ContentBlock::Image {
            name: "photo.jpg".into(),
            url: "https://example.com/photo.jpg".into(),
        }
    );
}

#[test]
fn test_content_block_audio_roundtrip() {
    let block = ContentBlock::Audio {
        name: "voice.mp3".into(),
        url: "https://example.com/voice.mp3".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(
        de,
        ContentBlock::Audio {
            name: "voice.mp3".into(),
            url: "https://example.com/voice.mp3".into(),
        }
    );
}

#[test]
fn test_content_block_file_roundtrip() {
    let block = ContentBlock::File {
        name: "report.pdf".into(),
        url: "https://example.com/report.pdf".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(
        de,
        ContentBlock::File {
            name: "report.pdf".into(),
            url: "https://example.com/report.pdf".into(),
        }
    );
}

#[test]
fn test_content_block_all_seven_variants_serde() {
    let blocks = [
        ContentBlock::Text("t".into()),
        ContentBlock::Thinking {
            thinking: "r".into(),
            signature: None,
        },
        ContentBlock::ToolUse {
            id: "1".into(),
            name: "n".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "1".into(),
            content: "ok".into(),
        },
        ContentBlock::Image {
            name: "img".into(),
            url: "https://example.com/img".into(),
        },
        ContentBlock::Audio {
            name: "aud".into(),
            url: "https://example.com/aud".into(),
        },
        ContentBlock::File {
            name: "f".into(),
            url: "https://example.com/f".into(),
        },
    ];
    for block in &blocks {
        let json = serde_json::to_string(block).unwrap();
        let de: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{:?}", block), format!("{:?}", de));
    }
}

// ---------------------------------------------------------------------------
// Image / Audio / File: exact serialization format & field correctness
// ---------------------------------------------------------------------------

#[test]
fn test_content_block_image_serialization_format() {
    let block = ContentBlock::Image {
        name: "photo.jpg".into(),
        url: "https://example.com/photo.jpg".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    assert_eq!(
        json,
        r#"{"type":"image","content":{"name":"photo.jpg","url":"https://example.com/photo.jpg"}}"#
    );
}

#[test]
fn test_content_block_audio_serialization_format() {
    let block = ContentBlock::Audio {
        name: "voice.mp3".into(),
        url: "https://example.com/voice.mp3".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    assert_eq!(
        json,
        r#"{"type":"audio","content":{"name":"voice.mp3","url":"https://example.com/voice.mp3"}}"#
    );
}

#[test]
fn test_content_block_file_serialization_format() {
    let block = ContentBlock::File {
        name: "report.pdf".into(),
        url: "https://example.com/report.pdf".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    assert_eq!(
        json,
        r#"{"type":"file","content":{"name":"report.pdf","url":"https://example.com/report.pdf"}}"#
    );
}

#[test]
fn test_content_block_image_deserialize_from_exact_json() {
    let json = r#"{"type":"image","content":{"name":"screenshot.png","url":"https://cdn.example.com/screenshot.png"}}"#;
    let block: ContentBlock = serde_json::from_str(json).unwrap();
    match block {
        ContentBlock::Image { name, url } => {
            assert_eq!(name, "screenshot.png");
            assert_eq!(url, "https://cdn.example.com/screenshot.png");
        }
        other => panic!("expected Image, got {:?}", other),
    }
}

#[test]
fn test_content_block_audio_deserialize_from_exact_json() {
    let json = r#"{"type":"audio","content":{"name":"recording.wav","url":"https://cdn.example.com/recording.wav"}}"#;
    let block: ContentBlock = serde_json::from_str(json).unwrap();
    match block {
        ContentBlock::Audio { name, url } => {
            assert_eq!(name, "recording.wav");
            assert_eq!(url, "https://cdn.example.com/recording.wav");
        }
        other => panic!("expected Audio, got {:?}", other),
    }
}

#[test]
fn test_content_block_file_deserialize_from_exact_json() {
    let json =
        r#"{"type":"file","content":{"name":"data.csv","url":"https://cdn.example.com/data.csv"}}"#;
    let block: ContentBlock = serde_json::from_str(json).unwrap();
    match block {
        ContentBlock::File { name, url } => {
            assert_eq!(name, "data.csv");
            assert_eq!(url, "https://cdn.example.com/data.csv");
        }
        other => panic!("expected File, got {:?}", other),
    }
}

#[test]
fn test_content_block_image_name_url_independent() {
    // name and url can differ: name is a local identifier, url is the remote access path
    let block = ContentBlock::Image {
        name: "local_key".into(),
        url: "https://storage.example.com/remote/path/img.jpg".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    match de {
        ContentBlock::Image { name, url } => {
            assert_eq!(name, "local_key");
            assert_eq!(url, "https://storage.example.com/remote/path/img.jpg");
        }
        other => panic!("expected Image, got {:?}", other),
    }
}

#[test]
fn test_content_block_audio_name_url_independent() {
    let block = ContentBlock::Audio {
        name: "memo_key".into(),
        url: "https://storage.example.com/remote/aud.mp3".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    match de {
        ContentBlock::Audio { name, url } => {
            assert_eq!(name, "memo_key");
            assert_eq!(url, "https://storage.example.com/remote/aud.mp3");
        }
        other => panic!("expected Audio, got {:?}", other),
    }
}

#[test]
fn test_content_block_file_name_url_independent() {
    let block = ContentBlock::File {
        name: "doc_ref".into(),
        url: "https://storage.example.com/remote/doc.pdf".into(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    match de {
        ContentBlock::File { name, url } => {
            assert_eq!(name, "doc_ref");
            assert_eq!(url, "https://storage.example.com/remote/doc.pdf");
        }
        other => panic!("expected File, got {:?}", other),
    }
}

#[test]
fn test_content_block_image_deserialize_missing_url_fails() {
    // url is required; missing it should fail
    let json = r#"{"type":"image","content":{"name":"x"}}"#;
    assert!(serde_json::from_str::<ContentBlock>(json).is_err());
}

#[test]
fn test_content_block_audio_deserialize_missing_name_fails() {
    // name is required; missing it should fail
    let json = r#"{"type":"audio","content":{"url":"https://x.com/a.mp3"}}"#;
    assert!(serde_json::from_str::<ContentBlock>(json).is_err());
}

#[test]
fn test_content_block_file_empty_fields() {
    // Empty name and url are still valid strings
    let block = ContentBlock::File {
        name: String::new(),
        url: String::new(),
    };
    let json = serde_json::to_string(&block).unwrap();
    let de: ContentBlock = serde_json::from_str(&json).unwrap();
    match de {
        ContentBlock::File { name, url } => {
            assert!(name.is_empty());
            assert!(url.is_empty());
        }
        other => panic!("expected File, got {:?}", other),
    }
}
