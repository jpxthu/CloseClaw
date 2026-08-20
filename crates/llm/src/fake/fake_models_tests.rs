use super::super::provider::ProviderError;
use super::fake_scenario::DeliveryConfig;
use super::*;
use std::time::{Duration, Instant};

// ── send_models tests (Step 1.6) ────────────────────────────────────────────

#[tokio::test]
async fn test_send_models_normal() {
    let p = FakeProvider::builder()
        .then_models(vec!["gpt-4".into(), "gpt-3.5".into()], "gpt-4")
        .build();
    assert_eq!(p.send_models().await.unwrap(), vec!["gpt-4", "gpt-3.5"]);
}

#[tokio::test]
async fn test_send_models_http_errors() {
    for (code, retry) in [(401, None), (429, Some(30)), (500, None)] {
        let p = FakeProvider::builder()
            .then_models_error("m", code, retry)
            .build();
        match p.send_models().await.unwrap_err() {
            ProviderError::Http {
                status_code,
                retry_after,
                ..
            } => {
                assert_eq!(status_code, code);
                assert_eq!(retry_after, retry);
            }
            other => panic!("Expected Http({code}), got: {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_send_models_overall_delay() {
    let p = FakeProvider {
        inner: Arc::new(Mutex::new(super::SharedState {
            scenarios: std::collections::VecDeque::from([Scenario::Models {
                models: vec!["d".into()],
                model: "d".into(),
                delivery: DeliveryConfig {
                    overall_delay: Some(Duration::from_millis(200)),
                    ..Default::default()
                },
            }]),
            ..Default::default()
        })),
    };
    let t = Instant::now();
    assert_eq!(p.send_models().await.unwrap(), vec!["d"]);
    assert!(t.elapsed().as_millis() >= 180);
}

#[tokio::test]
async fn test_send_models_exhaustion_fallback() {
    assert!(FakeProvider::builder()
        .or_else("x")
        .build()
        .send_models()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_send_models_sequential_and_passthrough() {
    let p = FakeProvider::builder()
        .then_models(vec!["gpt-4".into()], "gpt-4")
        .then_models(vec!["c".into()], "c")
        .build();
    assert_eq!(p.send_models().await.unwrap(), vec!["gpt-4"]);
    assert_eq!(p.send_models().await.unwrap(), vec!["c"]);
    // Err passthrough
    let p2 = FakeProvider::builder()
        .then_err(ProviderError::Legacy("a".into()))
        .build();
    assert!(matches!(p2.send_models().await.unwrap_err(), ProviderError::Legacy(s) if s == "a"));
    // Ok passthrough returns empty
    assert!(FakeProvider::builder()
        .then_ok("t", "m")
        .build()
        .send_models()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[should_panic(expected = "scenarios exhausted")]
async fn test_send_models_exhaust_panics() {
    let p = FakeProvider::builder()
        .then_models(vec!["g".into()], "g")
        .build();
    p.send_models().await.unwrap();
    p.send_models().await.unwrap();
}

#[tokio::test]
async fn test_send_models_delay_wrapping() {
    let p = FakeProvider {
        inner: Arc::new(Mutex::new(super::SharedState {
            scenarios: std::collections::VecDeque::from([Scenario::delay(
                Duration::from_millis(100),
                Scenario::Models {
                    models: vec!["d".into()],
                    model: "d".into(),
                    delivery: DeliveryConfig::default(),
                },
            )]),
            ..Default::default()
        })),
    };
    let t = Instant::now();
    assert_eq!(p.send_models().await.unwrap(), vec!["d"]);
    assert!(t.elapsed().as_millis() >= 80);
}
