use super::*;
use crate::llm::types::InternalMessage;
use mockito::Server;

fn provider_url(server: &Server) -> String {
    format!("{}/chat/completions", server.url())
}

fn internal_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Say hi".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        session_id: None,
        reasoning_level: crate::session::persistence::ReasoningLevel::default(),
        turn_count: None,
    }
}

fn chat_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "Say hi"}],
        "temperature": 0.0
    })
}

fn assert_first_text(resp: &InternalResponse, expected: &str) {
    assert!(!resp.content_blocks.is_empty());
    match &resp.content_blocks[0] {
        RawContentBlock::Text(s) => {
            assert!(s.contains(expected), "expected '{}', got: {}", expected, s)
        }
        RawContentBlock::Thinking(s) => {
            assert!(s.contains(expected), "expected '{}', got: {}", expected, s)
        }
        other => panic!("Expected Text/Thinking, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_glm_4_7_multi_turn_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-4.7-multi-turn.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(chat_body("GLM-4.7")))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let resp = provider
        .send(internal_request("GLM-4.7"), chat_body("GLM-4.7"))
        .await
        .unwrap();

    m.assert_async().await;
    assert_first_text(&resp, "name");
    let json: GlmResponse = serde_json::from_str(fixture).unwrap();
    assert_eq!(json.usage.as_ref().unwrap().prompt_tokens, 20);
    assert_eq!(json.usage.as_ref().unwrap().completion_tokens, 30);
}

#[tokio::test]
async fn test_glm_error_invalid_model_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-invalid-model.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(chat_body("glm-nonexistent")))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .send(
            internal_request("glm-nonexistent"),
            chat_body("glm-nonexistent"),
        )
        .await
        .unwrap_err();

    m.assert_async().await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("1211"),
                "error should contain 1211, got: {}",
                msg
            );
        }
        other => panic!("Expected Legacy error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_glm_error_empty_messages_mock() {
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/glm-error-empty-messages.json");
    let m = server
        .mock("POST", "/chat/completions")
        .match_body(mockito::Matcher::PartialJson(chat_body("glm-5.1")))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), provider_url(&server));
    let err = provider
        .send(internal_request("glm-5.1"), chat_body("glm-5.1"))
        .await
        .unwrap_err();

    m.assert_async().await;
    match err {
        ProviderError::Legacy(msg) => {
            assert!(
                msg.contains("1214"),
                "error should contain 1214, got: {}",
                msg
            );
        }
        other => panic!("Expected Legacy error, got: {:?}", other),
    }
}
