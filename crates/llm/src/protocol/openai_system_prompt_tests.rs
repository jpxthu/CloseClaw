//! Tests for static/dynamic system prompt injection into the OpenAI request
//! body — the OpenAI protocol must carry the static layer outbound
//! (`docs/design/system_prompt/static-layer.md` 数据流第 7 步, downstream
//! "每次 API 请求时取出使用"; prefix order per `docs/design/llm/README.md`
//! 前缀稳定性原则: static → dynamic → history).

use super::{ChatProtocol, InternalMessage, InternalRequest, OpenAiProtocol};
use closeclaw_session::persistence::ReasoningLevel;

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "gpt-4".to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

fn messages_of(body: &serde_json::Value) -> &Vec<serde_json::Value> {
    body["messages"]
        .as_array()
        .expect("body must contain a messages array")
}

// ── Normal path: both areas non-empty ────────────────────────────────────────

#[test]
fn test_build_request_injects_static_then_dynamic_then_history() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.system_static = Some("STATIC_AREA".to_string());
    request.system_dynamic = Some("DYNAMIC_AREA".to_string());

    let body = proto.build_request(&request).unwrap();
    let messages = messages_of(&body);

    assert_eq!(
        messages.len(),
        3,
        "expect 2 system messages + original history"
    );
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "STATIC_AREA");
    assert_eq!(messages[1]["role"], "system");
    assert_eq!(messages[1]["content"], "DYNAMIC_AREA");
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"], "Hello");
}

#[test]
fn test_build_request_system_prompt_content_passed_verbatim() {
    // Multi-section static content must pass through unchanged — no split,
    // no trim: rewriting the prefix would break prefix stability.
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.system_static = Some("Section A\n\n  Section B  \n".to_string());
    request.system_dynamic = Some("channel ctx: #general".to_string());

    let body = proto.build_request(&request).unwrap();
    let messages = messages_of(&body);

    assert_eq!(messages[0]["content"], "Section A\n\n  Section B  \n");
    assert_eq!(messages[1]["content"], "channel ctx: #general");
}

// ── Boundary: empty areas ────────────────────────────────────────────────────

#[test]
fn test_build_request_no_injection_when_both_areas_empty() {
    let proto = OpenAiProtocol::new();
    let empties: [(Option<String>, Option<String>); 4] = [
        (None, None),
        (None, Some(String::new())),
        (Some(String::new()), None),
        (Some(String::new()), Some(String::new())),
    ];

    for (static_area, dynamic_area) in empties {
        let mut request = make_request();
        request.system_static = static_area;
        request.system_dynamic = dynamic_area;

        let body = proto.build_request(&request).unwrap();

        // Byte-equivalent to the pre-change body shape: original messages
        // only, no injected system message, no extra keys.
        let expected = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hello"}],
            "temperature": request.temperature,
            "stream": false,
            "max_tokens": 256,
        });
        assert_eq!(
            body, expected,
            "empty/absent areas must not inject anything"
        );
    }
}

#[test]
fn test_build_request_injects_only_static_when_dynamic_absent() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.system_static = Some("STATIC_ONLY".to_string());
    request.system_dynamic = None;

    let body = proto.build_request(&request).unwrap();
    let messages = messages_of(&body);

    assert_eq!(
        messages.len(),
        2,
        "only the non-empty static area is injected"
    );
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "STATIC_ONLY");
    assert_eq!(messages[1]["role"], "user");
}

#[test]
fn test_build_request_injects_only_dynamic_when_static_empty() {
    let proto = OpenAiProtocol::new();
    let mut request = make_request();
    request.system_static = Some(String::new());
    request.system_dynamic = Some("DYNAMIC_ONLY".to_string());

    let body = proto.build_request(&request).unwrap();
    let messages = messages_of(&body);

    assert_eq!(
        messages.len(),
        2,
        "only the non-empty dynamic area is injected"
    );
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "DYNAMIC_ONLY");
    assert_eq!(messages[1]["role"], "user");
}

// ── State transition: no cross-request pollution ─────────────────────────────

#[test]
fn test_build_request_injection_not_polluting_same_protocol_instance() {
    let proto = OpenAiProtocol::new();

    let mut injected = make_request();
    injected.system_static = Some("STATIC".to_string());
    injected.system_dynamic = Some("DYNAMIC".to_string());
    let with_prompt = proto.build_request(&injected).unwrap();
    assert_eq!(messages_of(&with_prompt).len(), 3);

    // A subsequent plain request on the same instance must not inherit it.
    let plain = make_request();
    let without_prompt = proto.build_request(&plain).unwrap();
    let messages = messages_of(&without_prompt);
    assert_eq!(messages.len(), 1, "plain request must stay uninjected");
    assert!(
        messages.iter().all(|m| m["role"] != "system"),
        "no injected system message may leak across requests"
    );

    // Re-serializing the injected request yields the identical body again.
    let again = proto.build_request(&injected).unwrap();
    assert_eq!(again, with_prompt, "injection must be deterministic");
}

// ── No regression: original fields serialize unchanged ───────────────────────

#[test]
fn test_build_request_injection_preserves_other_field_serialization() {
    let proto = OpenAiProtocol::new();
    let mut plain = make_request();
    plain.stream = true;
    plain
        .extra_body
        .insert("custom_key".to_string(), serde_json::json!("custom_value"));

    let mut injected = plain.clone();
    injected.system_static = Some("STATIC".to_string());
    injected.system_dynamic = Some("DYNAMIC".to_string());

    let plain_body = proto.build_request(&plain).unwrap();
    let injected_body = proto.build_request(&injected).unwrap();

    for key in [
        "model",
        "temperature",
        "stream",
        "max_tokens",
        "stream_options",
        "custom_key",
    ] {
        assert_eq!(
            injected_body[key], plain_body[key],
            "field `{key}` must serialize identically with injection"
        );
    }

    let plain_messages = messages_of(&plain_body);
    let injected_messages = messages_of(&injected_body);
    assert_eq!(injected_messages.len(), plain_messages.len() + 2);
    assert_eq!(injected_messages[0]["role"], "system");
    assert_eq!(injected_messages[1]["role"], "system");
    assert_eq!(
        &injected_messages[2..],
        plain_messages.as_slice(),
        "original history must follow the injected prefix unchanged"
    );
}
