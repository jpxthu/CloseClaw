use super::*;
use mockito::Server;

/// Verify a GlmQuotaResponse parsed from a fixture has the expected limit structure.
fn assert_quota_limits(
    quota: &GlmQuotaResponse,
    expected_level: &str,
    expected_limit_types: &[&str],
) {
    assert_eq!(quota.code, 200);
    assert!(quota.success);
    assert_eq!(quota.data.level, expected_level);
    assert_eq!(
        quota.data.limits.len(),
        expected_limit_types.len(),
        "limit count mismatch"
    );
    for (limit, expected_type) in quota.data.limits.iter().zip(expected_limit_types.iter()) {
        assert_eq!(limit.limit_type, *expected_type);
    }
}

#[tokio::test]
async fn test_glm_usage_coding_plan_mock() {
    // usage-glm-coding-plan.json: coding plan quota with TIME_LIMIT and TOKENS_LIMIT entries
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/usage-glm-coding-plan.json");
    let m = server
        .mock("GET", "/paas/quota")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".into()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), server.url());
    let quota = provider.fetch_usage(&server.url()).await.unwrap();

    m.assert_async().await;
    assert_quota_limits(
        &quota,
        "pro",
        &["TIME_LIMIT", "TOKENS_LIMIT", "TOKENS_LIMIT"],
    );

    // Verify TIME_LIMIT details
    let time_limit = &quota.data.limits[0];
    assert_eq!(time_limit.limit_type, "TIME_LIMIT");
    assert_eq!(time_limit.unit, 5);
    assert_eq!(time_limit.number, Some(1));
    assert_eq!(time_limit.usage, Some(1000));
    assert_eq!(time_limit.remaining, Some(1000));
    assert_eq!(time_limit.percentage, Some(0));

    // Verify usageDetails for TIME_LIMIT
    let details = time_limit.usage_details.as_ref().unwrap();
    assert_eq!(details.len(), 3);
    assert_eq!(details[0].model_code, "search-prime");
    assert_eq!(details[0].usage, 0);
    assert_eq!(details[1].model_code, "web-reader");
    assert_eq!(details[2].model_code, "zread");

    // Verify first TOKENS_LIMIT
    let tokens_limit = &quota.data.limits[1];
    assert_eq!(tokens_limit.limit_type, "TOKENS_LIMIT");
    assert_eq!(tokens_limit.unit, 3);
    assert_eq!(tokens_limit.number, Some(5));
    assert_eq!(tokens_limit.percentage, Some(22));
    assert!(tokens_limit.next_reset_time.is_some());
}

#[tokio::test]
async fn test_glm_usage_global_mock() {
    // usage-glm-global.json: global endpoint quota response
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/usage-glm-global.json");
    let m = server
        .mock("GET", "/paas/quota")
        .match_header(
            "authorization",
            mockito::Matcher::Regex(r"Bearer .+".into()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), server.url());
    let quota = provider.fetch_usage(&server.url()).await.unwrap();

    m.assert_async().await;
    assert_quota_limits(
        &quota,
        "pro",
        &["TIME_LIMIT", "TOKENS_LIMIT", "TOKENS_LIMIT"],
    );

    // Verify global quota matches coding plan structure
    let time_limit = &quota.data.limits[0];
    assert_eq!(time_limit.usage, Some(1000));
    assert_eq!(time_limit.remaining, Some(1000));
    let tokens_limit = &quota.data.limits[1];
    assert_eq!(tokens_limit.percentage, Some(22));
}

#[tokio::test]
async fn test_glm_usage_trailing_slash_handled() {
    // fetch_usage should handle base_url with or without trailing slash
    let mut server = Server::new_async().await;
    let fixture = include_str!("../../../../tests/fixtures/llm/glm/usage-glm-coding-plan.json");
    let m = server
        .mock("GET", "/paas/quota")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(fixture)
        .create_async()
        .await;

    let provider = GlmProvider::with_base_url("fake-key".into(), server.url());
    // Pass URL with trailing slash — should still work
    let trailing_url = format!("{}/", server.url());
    let quota = provider.fetch_usage(&trailing_url).await.unwrap();

    m.assert_async().await;
    assert_eq!(quota.data.level, "pro");
}
