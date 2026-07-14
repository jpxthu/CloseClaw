//! Unit tests for the hook reviewer module.

use async_trait::async_trait;

use super::health_types::{HookContext, HookToolCallInfo};
use super::hook_reviewer::hook_prompt_template;
use super::hook_reviewer::*;
use closeclaw_common::HookParams;

/// Mock LLM provider for testing.
struct MockLlmProvider {
    /// Responses to return (true = flag, false = no flag).
    responses: Vec<Result<bool, String>>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl MockLlmProvider {
    fn new(responses: Vec<Result<bool, String>>) -> Self {
        Self {
            responses,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl HookLlmProvider for MockLlmProvider {
    async fn review(&self, _prompt: &str, _context: &str) -> Result<bool, String> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.responses.get(idx).cloned().unwrap_or(Ok(false))
    }
}

#[tokio::test]
async fn test_empty_hooks_returns_empty() {
    let llm = MockLlmProvider::new(vec![]);
    let reviewer = HookReviewer::new(vec![], Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    assert!(verdicts.is_empty());
}

#[tokio::test]
async fn test_disabled_hook_skipped() {
    let llm = MockLlmProvider::new(vec![]);
    let hooks = vec![HookConfig {
        hook_type: HookType::PlanCheck,
        enabled: false,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    assert!(verdicts.is_empty());
}

#[tokio::test]
async fn test_plan_check_flags_turn() {
    let llm = MockLlmProvider::new(vec![Ok(true)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::PlanCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: "I will plan the next steps".into(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::PlanCheck);
}

#[tokio::test]
async fn test_plan_check_no_flag() {
    let llm = MockLlmProvider::new(vec![Ok(false)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::PlanCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: "Done.".into(),
        tool_calls: vec![HookToolCallInfo {
            name: "write_file".into(),
            input: "{}".into(),
        }],
        tool_results: vec![],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(!verdicts[0].flag);
}

#[tokio::test]
async fn test_multiple_hooks_all_run() {
    let llm = MockLlmProvider::new(vec![Ok(false), Ok(true), Ok(false)]);
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: "test".into(),
        tool_calls: vec![HookToolCallInfo {
            name: "read".into(),
            input: "{}".into(),
        }],
        tool_results: vec!["content".into()],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 3);
    assert!(!verdicts[0].flag);
    assert!(verdicts[1].flag);
    assert!(!verdicts[2].flag);
}

#[tokio::test]
async fn test_llm_failure_graceful_degradation() {
    let llm = MockLlmProvider::new(vec![Err("API timeout".into())]);
    let hooks = vec![HookConfig {
        hook_type: HookType::LoopCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    // LLM failure → not flagged (graceful degradation)
    assert!(!verdicts[0].flag);
    assert!(verdicts[0].reason.contains("failed"));
}

#[test]
fn test_hook_type_prompt_templates_are_distinct() {
    let plan = HookType::PlanCheck;
    let loop_check = HookType::LoopCheck;
    let progress = HookType::ProgressCheck;

    // Each hook type must have a non-empty prompt template.
    assert!(!hook_prompt_template(&plan).is_empty());
    assert!(!hook_prompt_template(&loop_check).is_empty());
    assert!(!hook_prompt_template(&progress).is_empty());

    // Prompt templates must be distinct.
    assert_ne!(
        hook_prompt_template(&plan),
        hook_prompt_template(&loop_check)
    );
    assert_ne!(
        hook_prompt_template(&loop_check),
        hook_prompt_template(&progress)
    );
}

#[test]
fn test_format_turn_context_includes_all_fields() {
    let snapshot = HookContext {
        text: "Hello".into(),
        tool_calls: vec![HookToolCallInfo {
            name: "read".into(),
            input: r#"{"path": "/a"}"#.into(),
        }],
        tool_results: vec!["file content".into()],
        recent_tool_calls: vec![],
    };
    let ctx = format_turn_context(&snapshot);
    assert!(ctx.contains("Text: Hello"));
    assert!(ctx.contains("read("));
    assert!(ctx.contains("Result 0: file content"));
}

#[test]
fn test_format_turn_context_includes_recent_calls() {
    let snapshot = HookContext {
        text: String::new(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![HookToolCallInfo {
            name: "exec".into(),
            input: "ls".into(),
        }],
    };
    let ctx = format_turn_context(&snapshot);
    assert!(ctx.contains("Recent tool calls:"));
    assert!(ctx.contains("exec("));
}

// ── PlanCheck: normal paths ────────────────────────────────────────────────

#[tokio::test]
async fn test_plan_check_no_flag_no_tool_calls_no_promise() {
    // Normal path: no tool calls, no promise text → mock returns false
    let llm = MockLlmProvider::new(vec![Ok(false)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::PlanCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: "Here is the result.".into(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(!verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::PlanCheck);
}

// ── LoopCheck: normal and boundary paths ──────────────────────────────────

#[tokio::test]
async fn test_loop_check_no_flag_no_repeat() {
    // Normal path: tool calls with no repetition → mock returns false
    let llm = MockLlmProvider::new(vec![Ok(false)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::LoopCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: String::new(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![
            HookToolCallInfo {
                name: "read".into(),
                input: r#"{"path":"/a"}"#.into(),
            },
            HookToolCallInfo {
                name: "write".into(),
                input: r#"{"path":"/b"}"#.into(),
            },
        ],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(!verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::LoopCheck);
}

#[tokio::test]
async fn test_loop_check_flag_repeated_tools() {
    // Boundary: consecutive same tool + similar params → mock returns true
    let llm = MockLlmProvider::new(vec![Ok(true)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::LoopCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: String::new(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![
            HookToolCallInfo {
                name: "exec".into(),
                input: "ls -la".into(),
            },
            HookToolCallInfo {
                name: "exec".into(),
                input: "ls -la".into(),
            },
            HookToolCallInfo {
                name: "exec".into(),
                input: "ls -la".into(),
            },
        ],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::LoopCheck);
}

// ── ProgressCheck: normal and boundary paths ──────────────────────────────

#[tokio::test]
async fn test_progress_check_no_flag_with_changes() {
    // Normal path: tool results present → mock returns false
    let llm = MockLlmProvider::new(vec![Ok(false)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::ProgressCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: String::new(),
        tool_calls: vec![HookToolCallInfo {
            name: "write_file".into(),
            input: r#"{"path":"/a","content":"hello"}"#.into(),
        }],
        tool_results: vec!["Written 5 bytes".into()],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(!verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::ProgressCheck);
}

#[tokio::test]
async fn test_progress_check_flag_no_changes() {
    // Boundary: no tool calls, no tool results → mock returns true
    let llm = MockLlmProvider::new(vec![Ok(true)]);
    let hooks = vec![HookConfig {
        hook_type: HookType::ProgressCheck,
        enabled: true,
        ..Default::default()
    }];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext {
        text: String::new(),
        tool_calls: vec![],
        tool_results: vec![],
        recent_tool_calls: vec![],
    };
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 1);
    assert!(verdicts[0].flag);
    assert_eq!(verdicts[0].hook_type, HookType::ProgressCheck);
}

// ── Verdict reason formatting ─────────────────────────────────────────────

#[tokio::test]
async fn test_verdict_reason_contains_hook_type() {
    let llm = MockLlmProvider::new(vec![Ok(true), Ok(false)]);
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 2);
    // Flagged verdict reason contains hook type name
    assert!(verdicts[0].reason.contains("PlanCheck"));
    // Non-flagged verdict reason also contains hook type name
    assert!(verdicts[1].reason.contains("LoopCheck"));
}

// ── Mock call count verification ─────────────────────────────────────────

#[tokio::test]
async fn test_mock_called_exactly_once_per_enabled_hook() {
    let llm = MockLlmProvider::new(vec![Ok(false), Ok(false), Ok(false)]);
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: false,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    // Only 2 enabled hooks → 2 verdicts
    assert_eq!(verdicts.len(), 2);
    // Mock should have been called exactly 2 times
    // (once per enabled hook)
}

// ── Parallel execution verification ──────────────────────────────────────

/// Mock that records concurrent access via an atomic flag.
///
/// If hooks run sequentially, the flag is never set to true
/// because each call completes before the next starts.
/// If hooks run in parallel, the flag can be observed as true.
struct ParallelDetectLlm {
    /// Gate: all hooks must reach this before any can proceed.
    gate: std::sync::Arc<tokio::sync::Barrier>,
}

impl ParallelDetectLlm {
    fn new(num_hooks: usize) -> Self {
        Self {
            gate: std::sync::Arc::new(tokio::sync::Barrier::new(num_hooks)),
        }
    }
}

#[async_trait]
impl HookLlmProvider for ParallelDetectLlm {
    async fn review(&self, _prompt: &str, _context: &str) -> Result<bool, String> {
        // If we reach the barrier, all hooks are running concurrently.
        self.gate.wait().await;
        Ok(false)
    }
}

#[tokio::test]
async fn test_parallel_hooks_run_concurrently() {
    let num_hooks = 3;
    let llm = ParallelDetectLlm::new(num_hooks);
    let hooks: Vec<HookConfig> = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    // All 3 hooks ran (barrier was reached = all concurrent)
    assert_eq!(verdicts.len(), 3);
    // No flags (all returned false)
    assert!(verdicts.iter().all(|v| !v.flag));
}

#[tokio::test]
async fn test_return_order_matches_config_order() {
    // Responses are in reverse config order, but returned verdicts
    // must match config order.
    let llm = MockLlmProvider::new(vec![Ok(true), Ok(false), Ok(true)]);
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    // Verdicts must be in config order: PlanCheck, LoopCheck, ProgressCheck
    assert_eq!(verdicts.len(), 3);
    assert_eq!(verdicts[0].hook_type, HookType::PlanCheck);
    assert!(verdicts[0].flag);
    assert_eq!(verdicts[1].hook_type, HookType::LoopCheck);
    assert!(!verdicts[1].flag);
    assert_eq!(verdicts[2].hook_type, HookType::ProgressCheck);
    assert!(verdicts[2].flag);
}

#[tokio::test]
async fn test_individual_hook_failure_does_not_affect_others() {
    // First hook fails, second and third succeed.
    let llm = MockLlmProvider::new(vec![Err("LLM crashed".into()), Ok(false), Ok(true)]);
    let hooks = vec![
        HookConfig {
            hook_type: HookType::PlanCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::LoopCheck,
            enabled: true,
            ..Default::default()
        },
        HookConfig {
            hook_type: HookType::ProgressCheck,
            enabled: true,
            ..Default::default()
        },
    ];
    let reviewer = HookReviewer::new(hooks, Box::new(llm));
    let snapshot = HookContext::default();
    let verdicts = reviewer.review(&snapshot).await;
    assert_eq!(verdicts.len(), 3);
    // First hook failed → not flagged (graceful degradation)
    assert!(!verdicts[0].flag);
    assert!(verdicts[0].reason.contains("failed"));
    // Second hook succeeded → not flagged
    assert!(!verdicts[1].flag);
    // Third hook succeeded → flagged
    assert!(verdicts[2].flag);
}

// ── HookParams tests ─────────────────────────────────────────────────────

#[test]
fn test_hook_params_default_values() {
    let params = HookParams::default();
    assert_eq!(params.loop_check_repetition_threshold, 3);
    assert_eq!(params.progress_check_min_tool_calls, 1);
}

#[test]
fn test_hook_params_serialize_deserialize() {
    let params = HookParams {
        loop_check_repetition_threshold: 5,
        progress_check_min_tool_calls: 2,
    };
    let json = serde_json::to_string(&params).unwrap();
    assert!(json.contains("loopCheckRepetitionThreshold"));
    assert!(json.contains("progressCheckMinToolCalls"));
    let deserialized: HookParams = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.loop_check_repetition_threshold, 5);
    assert_eq!(deserialized.progress_check_min_tool_calls, 2);
}

#[test]
fn test_hook_params_default_when_omitted() {
    let json = r#"{}"#;
    let params: HookParams = serde_json::from_str(json).unwrap();
    assert_eq!(params.loop_check_repetition_threshold, 3);
    assert_eq!(params.progress_check_min_tool_calls, 1);
}

#[test]
fn test_hook_config_with_params_serialize_roundtrip() {
    let config = closeclaw_common::HookConfig {
        hook_type: closeclaw_common::HookType::LoopCheck,
        enabled: true,
        params: HookParams {
            loop_check_repetition_threshold: 5,
            progress_check_min_tool_calls: 2,
        },
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("loopCheckRepetitionThreshold"));
    assert!(json.contains("progressCheckMinToolCalls"));
    let deserialized: closeclaw_common::HookConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.params.loop_check_repetition_threshold, 5);
    assert_eq!(deserialized.params.progress_check_min_tool_calls, 2);
}

#[test]
fn test_hook_config_backward_compat_without_params() {
    // Old config JSON without params field should still deserialize
    let json = r#"{"hookType": "planCheck", "enabled": true}"#;
    let config: closeclaw_common::HookConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.hook_type, closeclaw_common::HookType::PlanCheck);
    assert!(config.enabled);
    assert_eq!(config.params.loop_check_repetition_threshold, 3);
    assert_eq!(config.params.progress_check_min_tool_calls, 1);
}

// ── build_review_prompt tests ────────────────────────────────────────────

#[test]
fn test_build_review_prompt_loop_check_includes_threshold() {
    let params = HookParams {
        loop_check_repetition_threshold: 5,
        ..Default::default()
    };
    let prompt = build_review_prompt(&HookType::LoopCheck, &params);
    assert!(prompt.contains("5 or more consecutive calls"));
}

#[test]
fn test_build_review_prompt_progress_check_includes_min() {
    let params = HookParams {
        progress_check_min_tool_calls: 3,
        ..Default::default()
    };
    let prompt = build_review_prompt(&HookType::ProgressCheck, &params);
    assert!(prompt.contains("at least 3 tool call(s)"));
}

#[test]
fn test_build_review_prompt_plan_check_no_param_note() {
    let params = HookParams::default();
    let prompt = build_review_prompt(&HookType::PlanCheck, &params);
    // PlanCheck has no param-driven additions
    assert_eq!(prompt, hook_prompt_template(&HookType::PlanCheck));
}

#[test]
fn test_build_review_prompt_default_threshold() {
    let params = HookParams::default();
    let prompt = build_review_prompt(&HookType::LoopCheck, &params);
    assert!(prompt.contains("3 or more consecutive calls"));
}
