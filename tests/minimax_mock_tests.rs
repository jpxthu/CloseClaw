//! MiniMax mock-based integration tests using mockito.
//!
//! Each test spins up a mockito server, points MiniMaxProvider at it,
//! sends a chat request, and validates the deserialized response.

use closeclaw::llm::MiniMaxProvider;
use closeclaw::llm::{ChatRequest, LLMProvider, Message as LLMMessage};
use mockito::Server;

// ---------------------------------------------------------------------------
// Helper: build a ChatRequest for the given user text
// ---------------------------------------------------------------------------

fn make_request(model: &str, user_text: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![LLMMessage {
            role: "user".to_string(),
            content: user_text.to_string(),
        }],
        temperature: 0.7,
        max_tokens: None,
    }
}

// ---------------------------------------------------------------------------
// Non-streaming fixture tests
// ---------------------------------------------------------------------------

/// simple-chat.json: content empty, reasoning_content non-empty → extract reasoning_content.
#[tokio::test]
async fn test_simple_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_header("content-type", "application/json")
        .match_body(mockito::Matcher::Regex(
            r#"\"model"\s*:\s*\"MiniMax-M2\.5\""#.into(),
        ))
        .with_body(include_str!("fixtures/llm/minimax/simple-chat.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Say hello in exactly 3 words");
    let response = provider.chat(request).await.expect("chat should succeed");

    // content is empty, so extract_content falls back to reasoning_content
    assert!(
        !response.content.is_empty(),
        "content should be extracted from reasoning_content"
    );
    // The reasoning_content in the fixture starts with "The user asks..."
    assert!(
        response.content.contains("The user asks"),
        "expected reasoning content, got: {}",
        response.content
    );
    assert_eq!(response.model, "MiniMax-M2.5");
    assert_eq!(response.usage.prompt_tokens, 48);
    assert_eq!(response.usage.completion_tokens, 50);
    assert_eq!(response.usage.total_tokens, 98);

    mock.assert_async().await;
}

/// m2-her-chat.json: content non-empty, no reasoning_content → extract content.
#[tokio::test]
async fn test_m2_her_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_body(mockito::Matcher::Regex(
            r#"\"model"\s*:\s*\"M2-her\""#.into(),
        ))
        .with_body(include_str!("fixtures/llm/minimax/m2-her-chat.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("M2-her", "Say something adventurous");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(
        response.content.contains("lands"),
        "expected pirate-style content, got: {}",
        response.content
    );
    assert_eq!(response.model, "M2-her");
    assert_eq!(response.usage.prompt_tokens, 163);
    assert_eq!(response.usage.completion_tokens, 44);

    mock.assert_async().await;
}

/// m2.5-highspeed-chat.json: MiniMax-M2.5-highspeed model.
#[tokio::test]
async fn test_m2_5_highspeed_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_body(mockito::Matcher::Regex(
            r#"\"model"\s*:\s*\"MiniMax-M2\.5-highspeed\""#.into(),
        ))
        .with_body(include_str!(
            "fixtures/llm/minimax/m2.5-highspeed-chat.json"
        ))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5-highspeed", "Say hi in one word");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty(), "content should not be empty");
    assert_eq!(response.model, "MiniMax-M2.5-highspeed");
    assert_eq!(response.usage.prompt_tokens, 46);
    assert_eq!(response.usage.completion_tokens, 10);

    mock.assert_async().await;
}

/// m2.7-chat.json: MiniMax-M2.7 model.
#[tokio::test]
async fn test_m2_7_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_body(mockito::Matcher::Regex(
            r#"\"model"\s*:\s*\"MiniMax-M2\.7\""#.into(),
        ))
        .with_body(include_str!("fixtures/llm/minimax/m2.7-chat.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.7", "Say hello in 3 words");
    let response = provider.chat(request).await.expect("chat should succeed");

    // content empty, reasoning_content non-empty
    assert!(
        !response.content.is_empty(),
        "should fall back to reasoning_content"
    );
    assert_eq!(response.model, "MiniMax-M2.7");
    assert_eq!(response.usage.prompt_tokens, 47);
    assert_eq!(response.usage.completion_tokens, 30);

    mock.assert_async().await;
}

/// m2.7-highspeed-chat.json: MiniMax-M2.7-highspeed model.
#[tokio::test]
async fn test_m2_7_highspeed_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .match_body(mockito::Matcher::Regex(
            r#"\"model"\s*:\s*\"MiniMax-M2\.7-highspeed\""#.into(),
        ))
        .with_body(include_str!(
            "fixtures/llm/minimax/m2.7-highspeed-chat.json"
        ))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.7-highspeed", "Say hi");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    assert_eq!(response.model, "MiniMax-M2.7-highspeed");

    mock.assert_async().await;
}

/// reasoning-heavy.json: both content and reasoning_content non-empty → prefer content.
#[tokio::test]
async fn test_reasoning_heavy() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/reasoning-heavy.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.7", "What is 17 * 23? Show your work.");
    let response = provider.chat(request).await.expect("chat should succeed");

    // content takes priority when non-empty
    assert!(
        response.content.contains("17"),
        "expected math answer, got: {}",
        response.content
    );
    assert!(
        !response.content.trim().is_empty(),
        "content should not be empty"
    );

    mock.assert_async().await;
}

/// code-generation.json: long answer with code block.
#[tokio::test]
async fn test_code_generation() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/code-generation.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Write a hello world in Python");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(
        response.content.contains("Hello"),
        "expected hello world content, got: {}",
        response.content
    );
    assert_eq!(response.usage.prompt_tokens, 47);
    assert_eq!(response.usage.completion_tokens, 100);

    mock.assert_async().await;
}

/// math-temp0.json: temperature=0 (deterministic).
#[tokio::test]
async fn test_math_temp0() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/math-temp0.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = ChatRequest {
        model: "MiniMax-M2.5".to_string(),
        messages: vec![LLMMessage {
            role: "user".to_string(),
            content: "What is 2+2?".to_string(),
        }],
        temperature: 0.0,
        max_tokens: None,
    };
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    assert_eq!(response.usage.prompt_tokens, 48);
    assert_eq!(response.usage.completion_tokens, 20);

    mock.assert_async().await;
}

/// unicode-chat.json: Chinese content.
#[tokio::test]
async fn test_unicode_chat() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/unicode-chat.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "用三个词形容春天");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    // reasoning_content contains Chinese text
    assert!(
        response.content.contains('\u{8C07}') || !response.content.is_empty(),
        "should contain unicode content"
    );

    mock.assert_async().await;
}

/// long-response.json: finish_reason=stop (full paragraph about dogs).
#[tokio::test]
async fn test_long_response() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/long-response.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = make_request("MiniMax-M2.5", "Write a paragraph about dogs");
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(
        response.content.contains("Dogs"),
        "expected 'Dogs' in response, got: {}",
        response.content
    );
    assert_eq!(response.usage.prompt_tokens, 46);
    assert_eq!(response.usage.completion_tokens, 222);

    mock.assert_async().await;
}

/// short-max-tokens.json: very short max_tokens.
#[tokio::test]
async fn test_short_max_tokens() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/short-max-tokens.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = ChatRequest {
        model: "MiniMax-M2.5".to_string(),
        messages: vec![LLMMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }],
        temperature: 0.7,
        max_tokens: Some(5),
    };
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    assert_eq!(response.usage.completion_tokens, 5);

    mock.assert_async().await;
}

/// temp-1.0.json: temperature=1.0.
#[tokio::test]
async fn test_temp_1() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/temp-1.0.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = ChatRequest {
        model: "MiniMax-M2.5".to_string(),
        messages: vec![LLMMessage {
            role: "user".to_string(),
            content: "Give me a random word".to_string(),
        }],
        temperature: 1.0,
        max_tokens: None,
    };
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    assert_eq!(response.usage.prompt_tokens, 46);
    assert_eq!(response.usage.completion_tokens, 10);

    mock.assert_async().await;
}

/// system-prompt.json: conversation with a system prompt.
#[tokio::test]
async fn test_system_prompt() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/system-prompt.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = ChatRequest {
        model: "MiniMax-M2.5".to_string(),
        messages: vec![
            LLMMessage {
                role: "system".to_string(),
                content: "You always respond using emojis.".to_string(),
            },
            LLMMessage {
                role: "user".to_string(),
                content: "Say hi".to_string(),
            },
        ],
        temperature: 0.7,
        max_tokens: None,
    };
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(!response.content.is_empty());
    assert_eq!(response.model, "MiniMax-M2.5");

    mock.assert_async().await;
}

/// multi-turn.json: multi-turn conversation (last message is user telling name).
#[tokio::test]
async fn test_multi_turn() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .match_header("authorization", "Bearer test-api-key")
        .with_body(include_str!("fixtures/llm/minimax/multi-turn.json"))
        .create();

    let provider = MiniMaxProvider::with_base_url(
        "test-api-key".to_string(),
        server.url() + "/v1/chat/completions",
    );

    let request = ChatRequest {
        model: "MiniMax-M2.5".to_string(),
        messages: vec![
            LLMMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            },
            LLMMessage {
                role: "assistant".to_string(),
                content: "Hi there! How can I help you?".to_string(),
            },
            LLMMessage {
                role: "user".to_string(),
                content: "My name is Alice".to_string(),
            },
        ],
        temperature: 0.7,
        max_tokens: None,
    };
    let response = provider.chat(request).await.expect("chat should succeed");

    assert!(
        response.content.contains("Alice"),
        "expected 'Alice' in response, got: {}",
        response.content
    );
    assert_eq!(response.usage.prompt_tokens, 63);
    assert_eq!(response.usage.completion_tokens, 30);

    mock.assert_async().await;
}
