//! Model fetching logic with retry and fallback
use crate::llm::retry::backoff_delay;
use crate::llm::{ErrorKind, LLMProvider, ModelInfo, ProviderModelKnowledge};
use std::sync::Arc;
use std::time::Duration;

const FETCH_MAX_RETRIES: u32 = 3;

/// Fetch model list with retry and exponential backoff for transient errors.
///
/// - Transient errors (429, 5xx, timeout, network) → retry up to 3 times with backoff
/// - Auth / Billing / InvalidRequest → immediately fallback to knowledge base
/// - After exhausting retries → fallback to knowledge base
pub async fn fetch_models_with_retry(
    provider: &Arc<dyn LLMProvider>,
    credential: &str,
) -> Vec<ModelInfo> {
    // Show spinner while fetching
    print!("Fetching models from provider...");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    for attempt in 1..=FETCH_MAX_RETRIES {
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            provider.fetch_model_list(credential),
        )
        .await;

        match result {
            Ok(Ok(model_infos)) => {
                println!(" done");
                return model_infos;
            }
            Ok(Err(err)) => {
                let kind = err.kind();
                match kind {
                    ErrorKind::Transient | ErrorKind::Unknown => {
                        if attempt < FETCH_MAX_RETRIES {
                            let delay = backoff_delay(
                                attempt,
                                Duration::from_secs(1),
                                Duration::from_secs(10),
                            );
                            println!(
                                "\n[Retry {}] {} ({}), retrying in {:?}...",
                                attempt,
                                kind_str(&kind),
                                err,
                                delay
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        println!(
                            "\n[Warning] API fetch failed after {} attempts ({}), falling back to knowledge base",
                            FETCH_MAX_RETRIES, err
                        );
                        return knowledge_fallback(provider.name()).await;
                    }
                    ErrorKind::Auth | ErrorKind::Billing | ErrorKind::InvalidRequest => {
                        println!(
                            "\n[Warning] API fetch failed ({}), falling back to knowledge base",
                            err
                        );
                        return knowledge_fallback(provider.name()).await;
                    }
                }
            }
            Err(_) => {
                // Timeout is treated as transient error
                if attempt < FETCH_MAX_RETRIES {
                    let delay = backoff_delay(
                        attempt,
                        Duration::from_secs(1),
                        Duration::from_secs(10),
                    );
                    println!(
                        "\n[Retry {}] API fetch timed out (10s), retrying in {:?}...",
                        attempt, delay
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                println!("\n[Warning] API fetch timed out after {} attempts, falling back to knowledge base", FETCH_MAX_RETRIES);
                return knowledge_fallback(provider.name()).await;
            }
        }
    }

    // Unreachable because we return on the last attempt
    unreachable!()
}

/// Helper to convert ErrorKind to user-friendly string
fn kind_str(kind: &ErrorKind) -> &str {
    match kind {
        ErrorKind::Transient => "Transient error",
        ErrorKind::Auth => "Authentication error",
        ErrorKind::Billing => "Billing error",
        ErrorKind::InvalidRequest => "Invalid request",
        ErrorKind::Unknown => "Unknown error",
    }
}

/// Fetch model list with a 10-second timeout.
/// On timeout or error, falls back to `ProviderModelKnowledge`.
pub async fn fetch_models_with_fallback(
    provider: &Arc<dyn LLMProvider>,
    credential: &str,
) -> Vec<ModelInfo> {
    // Show spinner while fetching
    print!("Fetching models from provider...");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        provider.fetch_model_list(credential),
    )
    .await;

    match result {
        Ok(Ok(model_infos)) => {
            println!(" done");
            model_infos
        }
        Ok(Err(err)) => {
            println!(
                "\n[Warning] API fetch failed ({}), falling back to knowledge base",
                err
            );
            knowledge_fallback(provider.name()).await
        }
        Err(_) => {
            println!("\n[Warning] API fetch timed out (10s), falling back to knowledge base");
            knowledge_fallback(provider.name()).await
        }
    }
}

/// Return model list from `ProviderModelKnowledge` for the given provider name.
pub async fn knowledge_fallback(provider_name: &str) -> Vec<ModelInfo> {
    let kb = ProviderModelKnowledge::new();
    let model_ids = kb.all_models(provider_name);
    model_ids
        .into_iter()
        .map(|id| {
            let params = kb.find(provider_name, id).unwrap();
            ModelInfo {
                id: id.to_string(),
                name: id.to_string(),
                context_window: params.context_window,
                max_tokens: params.max_tokens,
                default_temperature: Some(params.default_temperature),
                reasoning: params.reasoning,
                input_types: params.input_types,
            }
        })
        .collect()
}
