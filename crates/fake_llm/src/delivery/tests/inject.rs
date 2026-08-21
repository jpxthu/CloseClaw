use crate::delivery::inject::*;
use crate::delivery::sse::SseEvent;
use crate::scenario::types::{HttpError, ResponseBlock, UsageResponse};

use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable};

fn noop_waker() -> std::task::Waker {
    static RAW_WAKER_VTABLE: RawWakerVTable = {
        unsafe fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &RAW_WAKER_VTABLE)
        }
        unsafe fn noop(_: *const ()) {}
        RawWakerVTable::new(clone, noop, noop, noop)
    };
    let raw = RawWaker::new(std::ptr::null(), &RAW_WAKER_VTABLE);
    unsafe { std::task::Waker::from_raw(raw) }
}

fn text_block(content: &str) -> ResponseBlock {
    ResponseBlock {
        block_type: "text".to_string(),
        content: Some(content.to_string()),
        tool_name: None,
        tool_arguments: None,
        reasoning: None,
        signature: None,
    }
}

fn default_usage() -> UsageResponse {
    UsageResponse {
        prompt_tokens: Some(10),
        completion_tokens: Some(20),
        reasoning_tokens: None,
        cache_hit_tokens: None,
        cache_write_tokens: None,
        cache_fields_missing: false,
    }
}

// ------------------------------------------------------------------
// deliver — streaming delay injection
// ------------------------------------------------------------------

#[tokio::test]
async fn deliver_streaming_first_token_delay() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hi")],
        http_error: None,
        delay: None,
        first_token_delay: Some(100),
        segment_delay: None,
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let start = std::time::Instant::now();
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    let elapsed = start.elapsed().as_millis();
    assert!(elapsed >= 80, "expected >= 80ms, got {}ms", elapsed);
    assert!(matches!(result, DeliveryResult::SseStreamWithConfig { .. }));
}

#[tokio::test]
async fn deliver_streaming_segment_delay() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hello world")],
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: Some(50),
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    match result {
        DeliveryResult::SseStreamWithConfig {
            segment_delay_ms, ..
        } => {
            assert_eq!(segment_delay_ms, 50);
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

#[tokio::test]
async fn deliver_streaming_combined_delays() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hi")],
        http_error: None,
        delay: None,
        first_token_delay: Some(80),
        segment_delay: Some(30),
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let start = std::time::Instant::now();
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    let elapsed = start.elapsed().as_millis();
    assert!(elapsed >= 60, "expected >= 60ms, got {}ms", elapsed);
    match result {
        DeliveryResult::SseStreamWithConfig {
            segment_delay_ms, ..
        } => {
            assert_eq!(segment_delay_ms, 30);
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

// ------------------------------------------------------------------
// deliver — stream interrupt
// ------------------------------------------------------------------

#[tokio::test]
async fn deliver_streaming_interrupt_mid() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hello")],
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: Some(2),
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: Some(StreamInterrupt { after_event: 2 }),
    };
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    match result {
        DeliveryResult::SseStreamWithConfig { max_events, .. } => {
            assert_eq!(max_events, Some(2));
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

#[tokio::test]
async fn deliver_streaming_interrupt_zero() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hi")],
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: Some(0),
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: Some(StreamInterrupt { after_event: 0 }),
    };
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    match result {
        DeliveryResult::SseStreamWithConfig { max_events, .. } => {
            assert_eq!(max_events, Some(0));
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

#[tokio::test]
async fn deliver_streaming_interrupt_consumable() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("hi")],
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: Some(1),
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: Some(StreamInterrupt { after_event: 1 }),
    };
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    match result {
        DeliveryResult::SseStreamWithConfig {
            events,
            segment_delay_ms: _,
            max_events,
        } => {
            assert_eq!(max_events, Some(1));
            let mut stream =
                crate::delivery::sse::SseEventStream::new(events).with_max_events(max_events);
            use futures_core::Stream;
            let waker = noop_waker();
            let mut cx = Context::from_waker(&waker);
            let mut count = 0;
            loop {
                match Pin::new(&mut stream).poll_next(&mut cx) {
                    Poll::Ready(Some(_)) => count += 1,
                    _ => break,
                }
            }
            assert_eq!(count, 1);
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

// ------------------------------------------------------------------
// deliver — Anthropic streaming full sequence
// ------------------------------------------------------------------

#[tokio::test]
async fn deliver_anthropic_streaming_full_sequence() {
    let decision = crate::types::ScenarioDecision {
        model: "claude-3".to_string(),
        scenario: "test".to_string(),
        stream: true,
        response_blocks: vec![text_block("Hello, world!")],
        http_error: None,
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: Some(default_usage()),
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let result = deliver(&decision, Protocol::Anthropic, &config).await;
    match result {
        DeliveryResult::SseStreamWithConfig { events, .. } => {
            assert_eq!(events.len(), 7);
            // All Anthropic SSE events use event_type="message";
            // the actual type is in the JSON data's "type" field.
            // Anthropic protocol: all SSE events use event_type="message";
            // the JSON "type" field distinguishes: message, content_block_start,
            // ping, content_block_delta, content_block_stop, message_delta,
            // message_stop.
            let data_types: Vec<String> = events
                .iter()
                .map(|e| {
                    let v: serde_json::Value = serde_json::from_str(&e.data).unwrap();
                    v["type"].as_str().unwrap().to_string()
                })
                .collect();
            assert_eq!(data_types[0], "message");
            assert_eq!(data_types[1], "content_block_start");
            assert_eq!(data_types[2], "ping");
            assert_eq!(data_types[3], "content_block_delta");
            assert_eq!(data_types[4], "content_block_stop");
            assert_eq!(data_types[5], "message_delta");
            assert_eq!(data_types[6], "message_stop");
        }
        _ => panic!("expected SseStreamWithConfig"),
    }
}

// ------------------------------------------------------------------
// deliver — non-streaming delay then error
// ------------------------------------------------------------------

#[tokio::test]
async fn deliver_non_streaming_delay_then_error() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: false,
        response_blocks: vec![],
        http_error: Some(HttpError {
            status: 429,
            message: "rate limited".to_string(),
            retry_after: Some(60),
        }),
        delay: Some(50),
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: None,
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let start = std::time::Instant::now();
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    let elapsed = start.elapsed().as_millis();
    assert!(elapsed >= 30, "expected >= 30ms, got {}ms", elapsed);
    match result {
        DeliveryResult::HttpError {
            status,
            message,
            retry_after,
        } => {
            assert_eq!(status, 429);
            assert_eq!(message, "rate limited");
            assert_eq!(retry_after, Some(60));
        }
        _ => panic!("expected HttpError"),
    }
}

#[tokio::test]
async fn deliver_non_streaming_delay_then_error_500() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: false,
        response_blocks: vec![],
        http_error: Some(HttpError {
            status: 500,
            message: "server error".to_string(),
            retry_after: None,
        }),
        delay: Some(50),
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: None,
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let start = std::time::Instant::now();
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    let elapsed = start.elapsed().as_millis();
    assert!(elapsed >= 30, "expected >= 30ms, got {}ms", elapsed);
    match result {
        DeliveryResult::HttpError {
            status, message, ..
        } => {
            assert_eq!(status, 500);
            assert_eq!(message, "server error");
        }
        _ => panic!("expected HttpError"),
    }
}

#[tokio::test]
async fn deliver_non_streaming_delay_no_error() {
    let decision = crate::types::ScenarioDecision {
        model: "gpt-4".to_string(),
        scenario: "test".to_string(),
        stream: false,
        response_blocks: vec![text_block("ok")],
        http_error: None,
        delay: Some(50),
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        segment_granularity: None,
        usage: None,
    };
    let config = DeliveryConfig {
        segment_granularity: 0,
        include_usage: false,
        stream_interrupt: None,
    };
    let start = std::time::Instant::now();
    let result = deliver(&decision, Protocol::OpenAi, &config).await;
    let elapsed = start.elapsed().as_millis();
    assert!(elapsed >= 30, "expected >= 30ms, got {}ms", elapsed);
    match result {
        DeliveryResult::JsonResponse(json) => {
            assert_eq!(json["object"], "chat.completion");
        }
        _ => panic!("expected JsonResponse"),
    }
}

// ------------------------------------------------------------------
// SseEventStream — max_events edge cases
// ------------------------------------------------------------------

#[tokio::test]
async fn sse_event_stream_max_events_zero() {
    let events = vec![
        SseEvent {
            event_type: "message".into(),
            data: "a".into(),
        },
        SseEvent {
            event_type: "message".into(),
            data: "b".into(),
        },
    ];
    let mut stream = crate::delivery::sse::SseEventStream::new(events).with_max_events(Some(0));
    use futures_core::Stream;
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut count = 0;
    loop {
        match Pin::new(&mut stream).poll_next(&mut cx) {
            Poll::Ready(Some(_)) => count += 1,
            _ => break,
        }
    }
    assert_eq!(count, 0);
}
