//! Tests for MiniMax streaming implementation.

#[cfg(test)]
mod tests {
    use super::*;

    use crate::llm::{
        minimax::minimax_stream::{
            extract_message_text, parse_sse_line, parse_stream_chunk, process_buffer,
            process_chunk, ChatStreamChunk, MiniMaxStreamChunk, MiniMaxStreamMessage,
        },
        LLMError, MiniMaxProvider,
    };

    // --- SSE line parsing ---

    #[test]
    fn test_parse_sse_line_valid_json() {
        let line = "data: {\"id\":\"abc\"}";
        let result = parse_sse_line(line);
        assert_eq!(result, Some("{\"id\":\"abc\"}"));
    }

    #[test]
    fn test_parse_sse_line_done() {
        let line = "data: [DONE]";
        let result = parse_sse_line(line);
        assert_eq!(result, Some("[DONE]"));
    }

    #[test]
    fn test_parse_sse_line_empty() {
        assert!(parse_sse_line("").is_none());
    }

    #[test]
    fn test_parse_sse_line_not_sse() {
        assert!(parse_sse_line("hello world").is_none());
    }

    #[test]
    fn test_parse_sse_line_with_whitespace() {
        let line = "  data: {\"key\":\"val\"}  ";
        let result = parse_sse_line(line);
        assert_eq!(result, Some("{\"key\":\"val\"}"));
    }

    #[test]
    fn test_parse_sse_line_prefix_not_at_start() {
        let line = "prefix data: {}";
        let result = parse_sse_line(line);
        assert_eq!(result, None);
    }

    // --- Stream chunk deserialization ---

    #[test]
    fn test_parse_delta_reasoning_content() {
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"The user"}}],"model":"MiniMax-M2.5","object":"chat.completion.chunk"}"#;
        let chunk = parse_stream_chunk(json).unwrap();
        let choice = chunk.choices.as_ref().and_then(|c| c.first()).unwrap();
        let delta = choice.delta.as_ref().unwrap();
        assert_eq!(delta.reasoning_content.as_deref(), Some("The user"));
        assert!(choice.message.is_none());
    }

    #[test]
    fn test_parse_delta_with_finish_reason() {
        let json = r#"{"id":"abc","choices":[{"finish_reason":"length","index":0,"delta":{"role":"assistant","reasoning_content":" wants me to count."}}],"model":"MiniMax-M2.5","object":"chat.completion.chunk"}"#;
        let chunk = parse_stream_chunk(json).unwrap();
        let choice = chunk.choices.as_ref().and_then(|c| c.first()).unwrap();
        assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn test_parse_final_chunk() {
        let json = r#"{"id":"abc","choices":[{"finish_reason":"length","index":0,"message":{"content":"","role":"assistant","reasoning_content":"Final content."}}],"created":0,"model":"MiniMax-M2.5","object":"chat.completion","usage":{"total_tokens":75,"prompt_tokens":45,"completion_tokens":30,"completion_tokens_details":{"reasoning_tokens":29}},"base_resp":{"status_code":0,"status_msg":""}}"#;
        let chunk = parse_stream_chunk(json).unwrap();
        let choice = chunk.choices.as_ref().and_then(|c| c.first()).unwrap();
        assert!(choice.message.is_some());
        assert!(choice.delta.is_none());
        assert_eq!(chunk.object, "chat.completion");
        let usage = chunk.usage.as_ref().unwrap();
        assert_eq!(usage.total_tokens, 75);
        assert_eq!(usage.prompt_tokens, 45);
    }

    #[test]
    fn test_parse_content_delta() {
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"}}]}"#;
        let chunk = parse_stream_chunk(json).unwrap();
        let choice = chunk.choices.as_ref().and_then(|c| c.first()).unwrap();
        let delta = choice.delta.as_ref().unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello"));
        assert!(delta.reasoning_content.is_none());
    }

    // --- process_chunk ---

    #[test]
    fn test_process_delta_reasoning_content_sends_text() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Hello"}}],"model":"MiniMax-M2.5"}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(should_continue);

        let result = rx.try_recv();
        match result {
            Ok(ChatStreamChunk::Text(text)) => assert_eq!(text, "Hello"),
            other => panic!("Expected Text('Hello'), got {:?}", other),
        }
    }

    #[test]
    fn test_process_content_delta_sends_text() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"}}],"model":"MiniMax-M2.5"}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(should_continue);
        let result = rx.try_recv();
        match result {
            Ok(ChatStreamChunk::Text(text)) => assert_eq!(text, "Hello"),
            other => panic!("Expected Text('Hello'), got {:?}", other),
        }
    }

    #[test]
    fn test_process_final_chunk_sends_done() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"finish_reason":"length","index":0,"message":{"content":"","role":"assistant","reasoning_content":"Final"}}],"model":"MiniMax-M2.5","usage":{"total_tokens":10,"prompt_tokens":5,"completion_tokens":5}}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(!should_continue);

        let first = rx.try_recv();
        match first {
            Ok(ChatStreamChunk::Text(t)) => assert_eq!(t, "Final"),
            other => panic!("Expected Text('Final') first, got {:?}", other),
        }
        let second = rx.try_recv();
        match second {
            Ok(ChatStreamChunk::Done { model, usage }) => {
                assert_eq!(model, "MiniMax-M2.5");
                assert_eq!(usage.total_tokens, 10);
            }
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_process_final_chunk_empty_message_no_content_sent() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"finish_reason":"length","index":0,"message":{"content":"","role":"assistant"}}],"model":"MiniMax-M2.5","usage":{"total_tokens":10,"prompt_tokens":5,"completion_tokens":5}}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(!should_continue);
        // No Text chunk for empty content, only Done
        let first = rx.try_recv();
        match first {
            Ok(ChatStreamChunk::Done { .. }) => {} // OK
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_process_error_chunk_returns_err() {
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"err","choices":[],"created":0,"model":"","object":"","usage":{},"base_resp":{"status_code":1004,"status_msg":"login fail"}}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let err = process_chunk(chunk, &tx).unwrap_err();
        assert!(matches!(err, LLMError::AuthFailed(_)));
    }

    #[test]
    fn test_process_chunk_no_choices() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","model":"MiniMax-M2.5"}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(should_continue);
        assert!(rx.try_recv().is_err());
    }

    // --- extract_message_text ---

    #[test]
    fn test_extract_message_text_prefers_content() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: Some("Hello world".to_string()),
            reasoning_content: Some("Should not use this".to_string()),
        };
        assert_eq!(extract_message_text(&msg), "Hello world");
    }

    #[test]
    fn test_extract_message_text_empty_content_falls_back() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: Some("".to_string()),
            reasoning_content: Some("Reasoning text".to_string()),
        };
        assert_eq!(extract_message_text(&msg), "Reasoning text");
    }

    #[test]
    fn test_extract_message_text_none_content_falls_back() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: Some("Reasoning text".to_string()),
        };
        assert_eq!(extract_message_text(&msg), "Reasoning text");
    }

    #[test]
    fn test_extract_message_text_all_empty() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
        };
        assert_eq!(extract_message_text(&msg), "");
    }

    #[test]
    fn test_extract_message_text_whitespace_only_content() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: Some("   \n\t  ".to_string()),
            reasoning_content: Some("reasoning".to_string()),
        };
        // whitespace-only content should fall back to reasoning_content
        assert_eq!(extract_message_text(&msg), "reasoning");
    }

    #[test]
    fn test_extract_message_text_whitespace_only_reasoning() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: Some("".to_string()),
            reasoning_content: Some("   \n\t  ".to_string()),
        };
        assert_eq!(extract_message_text(&msg), "");
    }

    #[test]
    fn test_extract_message_text_whitespace_trimmed() {
        let msg = MiniMaxStreamMessage {
            role: "assistant".to_string(),
            content: Some("  Hello  ".to_string()),
            reasoning_content: None,
        };
        assert_eq!(extract_message_text(&msg), "Hello");
    }

    #[test]
    fn test_process_chunk_content_delta_sends_text() {
        // Verify content delta (not reasoning_content) also sends text
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello world"}}],"model":"MiniMax-M2.5"}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(should_continue);
        let result = rx.try_recv();
        match result {
            Ok(ChatStreamChunk::Text(text)) => assert_eq!(text, "Hello world"),
            other => panic!("Expected Text('Hello world'), got {:?}", other),
        }
    }

    #[test]
    fn test_process_final_chunk_with_content() {
        // Final chunk with non-empty content field
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"finish_reason":"stop","index":0,"message":{"content":"Final answer","role":"assistant","reasoning_content":"Reasoning"}}],"model":"MiniMax-M2.5","usage":{"total_tokens":20,"prompt_tokens":10,"completion_tokens":10}}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(!should_continue);
        // Should send Text with content (not reasoning_content), then Done
        let first = rx.try_recv();
        match first {
            Ok(ChatStreamChunk::Text(t)) => assert_eq!(t, "Final answer"),
            other => panic!("Expected Text('Final answer') first, got {:?}", other),
        }
        let second = rx.try_recv();
        match second {
            Ok(ChatStreamChunk::Done { model, usage }) => {
                assert_eq!(model, "MiniMax-M2.5");
                assert_eq!(usage.total_tokens, 20);
            }
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_process_final_chunk_empty_content_no_reasoning() {
        // Final chunk with empty content and no reasoning_content → no Text, just Done
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"finish_reason":"stop","index":0,"message":{"content":"","role":"assistant"}}],"model":"MiniMax-M2.5","usage":{"total_tokens":5,"prompt_tokens":5,"completion_tokens":0}}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(!should_continue);
        // No Text chunk sent; only Done
        let first = rx.try_recv();
        match first {
            Ok(ChatStreamChunk::Done { .. }) => {}
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_process_chunk_delta_prefers_reasoning_content() {
        // When both content and reasoning_content are present in delta,
        // reasoning_content takes priority (matches actual SSE stream order)
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let json = r#"{"id":"abc","choices":[{"index":0,"delta":{"role":"assistant","content":"Visible","reasoning_content":"Hidden"}}],"model":"MiniMax-M2.5"}"#;
        let chunk: MiniMaxStreamChunk = serde_json::from_str(json).unwrap();
        let should_continue = process_chunk(chunk, &tx).unwrap();
        assert!(should_continue);
        let result = rx.try_recv();
        match result {
            Ok(ChatStreamChunk::Text(t)) => assert_eq!(t, "Hidden"),
            other => panic!("Expected Text('Hidden'), got {:?}", other),
        }
    }

    // --- Usage from fixture files ---

    #[test]
    fn test_streaming_file1_parse() {
        // Read streaming.txt and verify all chunks parse correctly
        let fixture = include_str!("../../tests/fixtures/llm/minimax/streaming.txt");
        for line in fixture.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(data) = parse_sse_line(line) {
                if data == "[DONE]" {
                    continue;
                }
                let chunk = parse_stream_chunk(data).unwrap();
                assert!(chunk.choices.is_some() || chunk.base_resp.is_some());
            }
        }
    }

    #[test]
    fn test_streaming_file2_parse() {
        let fixture = include_str!("../../tests/fixtures/llm/minimax/streaming-m2.7.txt");
        for line in fixture.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(data) = parse_sse_line(line) {
                if data == "[DONE]" {
                    continue;
                }
                let chunk = parse_stream_chunk(data).unwrap();
                assert!(chunk.choices.is_some() || chunk.base_resp.is_some());
            }
        }
    }

    #[test]
    fn test_streaming_integration_process_all_chunks() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let fixture = include_str!("../../tests/fixtures/llm/minimax/streaming.txt");
        let mut delta_count = 0;

        for line in fixture.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(data) = parse_sse_line(line) {
                if data == "[DONE]" {
                    break;
                }
                let chunk = parse_stream_chunk(data).unwrap();
                let should_continue = process_chunk(chunk, &tx).unwrap();
                if !should_continue {
                    break;
                }
                delta_count += 1;
            }
        }

        // Drain all Text chunks
        let mut text_chunks = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            if let ChatStreamChunk::Text(t) = chunk {
                text_chunks.push(t);
            }
        }

        // Should have received some text chunks
        assert!(
            !text_chunks.is_empty(),
            "Expected text chunks from streaming"
        );
        // The concatenated text should contain "The user"
        let combined = text_chunks.join("");
        assert!(
            combined.contains("The user"),
            "Combined text should contain 'The user', got: {}",
            combined
        );
    }

    #[test]
    fn test_process_buffer_with_multiple_lines() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let buffer = br#"data: {"id":"a","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Hello"}}],"model":"m"}
data: {"id":"a","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":" World"}}],"model":"m"}
data: [DONE]
"#;

        let consumed = process_buffer(buffer, &tx).unwrap();
        assert!(consumed > 0);

        let mut texts = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            if let ChatStreamChunk::Text(t) = chunk {
                texts.push(t);
            }
        }

        assert_eq!(texts, vec!["Hello", " World"]);
    }

    #[test]
    fn test_process_buffer_partial_line_at_end() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        // Note: the second line has no trailing newline
        let buffer = br#"data: {"id":"a","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Hello"}}],"model":"m"}
"#;
        let consumed = process_buffer(buffer, &tx).unwrap();
        assert!(consumed > 0);

        let texts: Vec<_> = std::iter::from_fn(|| match rx.try_recv() {
            Ok(ChatStreamChunk::Text(t)) => Some(t),
            _ => None,
        })
        .collect();
        assert_eq!(texts, vec!["Hello"]);
    }

    // --- Error mapping ---

    #[test]
    fn test_map_status_error_auth() {
        let err = MiniMaxProvider::map_status_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "unauthorized".to_string(),
        );
        assert!(matches!(err, LLMError::AuthFailed(_)));
    }

    #[test]
    fn test_map_base_resp_error_auth() {
        let err = MiniMaxProvider::map_base_resp_error(1004, "login fail");
        assert!(matches!(err, LLMError::AuthFailed(_)));
    }

    #[test]
    fn test_map_base_resp_error_model_not_found() {
        let err = MiniMaxProvider::map_base_resp_error(2013, "unknown model: MiniMax-M3");
        assert!(matches!(err, LLMError::ModelNotFound(_)));
    }

    #[test]
    fn test_map_base_resp_error_invalid_request() {
        let err = MiniMaxProvider::map_base_resp_error(2013, "messages is empty");
        assert!(matches!(err, LLMError::InvalidRequest(_)));
    }

    #[test]
    fn test_map_base_resp_error_other() {
        let err = MiniMaxProvider::map_base_resp_error(9999, "something wrong");
        assert!(matches!(err, LLMError::ApiError(_)));
    }
}
