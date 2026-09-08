//! Built-in meta tool — ToolSearch.
//!
//! Allows the LLM to dynamically retrieve second-level tool detail
//! by weighted keyword matching or exact name match.

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::tool_registry::{ToolDescriptor, ToolRegistryQuery};
use serde_json::{json, Value};

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

// ---------------------------------------------------------------------------
// Scoring constants
// ---------------------------------------------------------------------------

/// Score for an exact tool name match (case-insensitive).
const SCORE_NAME_EXACT: u32 = 10;
/// Score per matching keyword.
const SCORE_KEYWORD: u32 = 5;
/// Score for a description/summary substring match.
const SCORE_SUBSTRING: u32 = 1;
/// Maximum number of results returned in keyword mode.
const MAX_RESULTS: usize = 10;

// ---------------------------------------------------------------------------
// ToolSearchTool
// ---------------------------------------------------------------------------

/// Dynamic tool detail lookup by weighted keyword or exact name.
///
/// # What it does
/// When the LLM needs to understand a tool's full input schema or detail
/// description, it calls `ToolSearch` with a `query` string.
///
/// - **exact mode** (query matches a tool name verbatim): returns that tool's
///   full `detail()` + `input_schema()`.
/// - **keyword mode** (query does not match any tool name): scores all tools
///   by name exactness, keyword hits, and description substring matches.
///   Returns top-scoring tools sorted by score descending.
pub struct ToolSearchTool {
    registry: Arc<dyn ToolRegistryQuery>,
}

impl ToolSearchTool {
    pub fn new(registry: Arc<dyn ToolRegistryQuery>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn group(&self) -> &str {
        "meta"
    }

    fn summary(&self) -> String {
        "Search for tool details by keyword or exact name".to_string()
    }

    fn detail(&self) -> String {
        "[keywords: search find discover tool lookup query] \
         Dynamically retrieve second-level tool detail by keyword or exact \
         name. When the LLM needs the full description or input schema of a \
         specific tool, call this tool with a `query` string.\n\n\
         **Exact mode**: if `query` exactly matches a registered tool name \
         (case-insensitive), returns that tool's full `detail()` text and \
         `input_schema` JSON object.\n\n\
         **Keyword mode**: if `query` does not exactly match any tool name, \
         scores all tools by name exactness, keyword hits, and description \
         substring matches. Returns a list of `{name, group, summary, score}` \
         for the top 10 results, sorted by score descending.\n\n\
         This is the only way to load a tool's second-level detail into the \
         context, since first-level index only shows group + tool name list."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query: exact tool name or keyword"
                }
            },
            "required": ["query"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            is_destructive: false,
            is_expensive: true,
            is_deferred_by_default: false,
        }
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolCallError::InvalidArgs("missing `query` parameter".into()))?;
        let query_lower = query.to_lowercase();

        // Batch-fetch descriptors once for both exact and keyword modes
        let descs = self.registry.get_tool_descriptors(None, None, None).await;

        // Exact mode: case-insensitive tool name match
        if let Some(result) = self.try_exact_match(&query_lower, &descs).await {
            return Ok(result);
        }

        // Keyword mode: weighted scoring across all tools
        let results = self.keyword_search(&query_lower, &descs);
        Ok(ToolResult {
            data: json!({ "tools": results }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

impl ToolSearchTool {
    /// Try to find a tool by exact name match (case-insensitive).
    /// Receives pre-fetched descriptors to avoid redundant registry calls.
    async fn try_exact_match(
        &self,
        query_lower: &str,
        descs: &[ToolDescriptor],
    ) -> Option<ToolResult> {
        let matched = descs
            .iter()
            .find(|d| d.name.to_lowercase() == query_lower)?;
        let schema = self.registry.get_tool_schema(&matched.name).await;
        Some(ToolResult {
            data: json!({
                "name": matched.name,
                "group": matched.group,
                "summary": matched.summary,
                "detail": matched.detail,
                "input_schema": schema,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }

    /// Score all tools against the query and return top results.
    /// Receives pre-fetched descriptors to avoid redundant registry calls.
    fn keyword_search(&self, query_lower: &str, descs: &[ToolDescriptor]) -> Vec<Value> {
        let mut scored: Vec<(String, String, String, u32)> = Vec::with_capacity(descs.len());

        for desc in descs {
            let score = score_tool(desc, query_lower);
            if score > 0 {
                scored.push((
                    desc.name.clone(),
                    desc.group.clone(),
                    desc.summary.clone(),
                    score,
                ));
            }
        }

        scored.sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(MAX_RESULTS);

        scored
            .into_iter()
            .map(|(name, group, summary, score)| {
                json!({ "name": name, "group": group, "summary": summary, "score": score })
            })
            .collect()
    }
}

/// Compute a relevance score for a tool against a lowered query.
///
/// Keyword matching uses word-level matching: the query is split by
/// whitespace into a word set, and a keyword matches only if it is a
/// complete word in that set. This prevents short keywords (e.g. "r",
/// "in") from causing false positives via substring matching.
fn score_tool(desc: &closeclaw_common::tool_registry::ToolDescriptor, query_lower: &str) -> u32 {
    let mut score: u32 = 0;

    // 1. Exact name match (case-insensitive)
    if desc.name.to_lowercase() == query_lower {
        score += SCORE_NAME_EXACT;
    }

    // 2. Keyword matches — word-level: keyword must be a complete word in the query
    let query_words: std::collections::HashSet<&str> = query_lower.split_whitespace().collect();
    for kw in &desc.keywords {
        if query_words.contains(kw.as_str()) {
            score += SCORE_KEYWORD;
        }
    }

    // 3. Substring fallback in summary + detail
    if score == 0 {
        let summary_lower = desc.summary.to_lowercase();
        let detail_lower = desc.detail.to_lowercase();
        let text_match = summary_lower.contains(query_lower)
            || detail_lower.contains(query_lower)
            || query_words
                .iter()
                .any(|w| summary_lower.contains(w) || detail_lower.contains(w));
        if text_match {
            score += SCORE_SUBSTRING;
        }
    }

    score
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::tool_registry::{ToolDescriptor, ToolFlags as CommonFlags};
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    /// A minimal mock registry for unit tests.
    struct MockRegistry {
        tools: RwLock<HashMap<String, ToolDescriptor>>,
    }

    impl MockRegistry {
        fn new() -> Self {
            Self {
                tools: RwLock::new(HashMap::new()),
            }
        }

        async fn insert(&self, desc: ToolDescriptor) {
            self.tools.write().await.insert(desc.name.clone(), desc);
        }
    }

    #[async_trait]
    impl ToolRegistryQuery for MockRegistry {
        async fn list_tool_names(&self) -> Vec<String> {
            self.tools.read().await.keys().cloned().collect()
        }

        async fn get_tool_descriptors(
            &self,
            _agent_id: Option<&str>,
            _agent_tools: Option<&[String]>,
            _agent_disallowed_tools: Option<&[String]>,
        ) -> Vec<ToolDescriptor> {
            self.tools.read().await.values().cloned().collect()
        }

        async fn has_tool(&self, name: &str) -> bool {
            self.tools.read().await.contains_key(name)
        }

        async fn get_tool_schema(&self, _name: &str) -> Option<Value> {
            None
        }

        async fn get_tool_detail(&self, name: &str) -> Option<ToolDescriptor> {
            self.tools.read().await.get(name).cloned()
        }

        async fn list_tool_names_by_group(&self, _group: &str) -> Vec<String> {
            vec![]
        }
    }

    fn make_desc(name: &str, group: &str, summary: &str, keywords: Vec<&str>) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            group: group.to_string(),
            summary: summary.to_string(),
            detail: format!(
                "[keywords: {}] some detail about {}",
                keywords.join(" "),
                name.to_lowercase()
            ),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: keywords.into_iter().map(String::from).collect(),
        }
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test".into(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        }
    }

    // --- Metadata tests ---

    #[test]
    fn test_toolsearch_name_group() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(reg);
        assert_eq!(tool.name(), "ToolSearch");
        assert_eq!(tool.group(), "meta");
    }

    #[test]
    fn test_toolsearch_summary_len() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(reg);
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_toolsearch_flags() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(reg);
        let flags = tool.flags();
        assert!(flags.is_concurrency_safe);
        assert!(!flags.is_deferred_by_default);
        assert!(flags.is_expensive);
        assert!(flags.is_read_only);
    }

    #[test]
    fn test_toolsearch_input_schema_has_query() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(reg);
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("query"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("query")));
    }

    #[test]
    fn test_toolsearch_detail_contains_modes() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(reg);
        let detail = tool.detail();
        assert!(detail.contains("Exact mode"));
        assert!(detail.contains("Keyword mode"));
    }

    // --- call() tests: exact mode ---

    #[tokio::test]
    async fn test_exact_mode_returns_detail_and_schema() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read file",
            vec!["read", "file"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "Read"}), &make_ctx())
            .await
            .unwrap();
        assert_eq!(result.data["name"], "Read");
        assert_eq!(result.data["group"], "file_ops");
        assert!(result.data["detail"]
            .as_str()
            .unwrap()
            .contains("some detail about"));
    }

    #[tokio::test]
    async fn test_exact_mode_case_insensitive() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc("Read", "file_ops", "Read file", vec!["read"]))
            .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "read"}), &make_ctx())
            .await
            .unwrap();
        assert_eq!(result.data["name"], "Read");
    }

    #[tokio::test]
    async fn test_exact_mode_nonexistent_returns_keyword_mode() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc("Read", "file_ops", "Read file", vec!["read"]))
            .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "nonexistent"}), &make_ctx())
            .await
            .unwrap();
        // Falls through to keyword mode, should be empty
        let tools = result.data["tools"].as_array().unwrap();
        assert!(tools.is_empty());
    }

    // --- call() tests: keyword mode ---

    #[tokio::test]
    async fn test_keyword_match_scores高于substring() {
        let reg = Arc::new(MockRegistry::new());
        // Tool with keyword "read" — should score 10 (2 keyword matches)
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read file",
            vec!["read", "file"],
        ))
        .await;
        // Tool with "read" only in summary — should score 1
        reg.insert(make_desc(
            "AuditLog",
            "meta",
            "Audit read operations",
            vec!["audit", "log"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        // Use a query that doesn't exactly match any tool name to test keyword mode
        let result = tool
            .call(json!({"query": "read file content"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        // Read (2 keyword matches: "read" + "file", score 10) should come first
        assert_eq!(tools[0]["name"], "Read");
        assert_eq!(tools[0]["score"], SCORE_KEYWORD * 2);
        // AuditLog (substring match: "read" in summary, score 1) should come second
        assert_eq!(tools[1]["name"], "AuditLog");
        assert_eq!(tools[1]["score"], SCORE_SUBSTRING);
    }

    #[tokio::test]
    async fn test_multiple_keyword_matches_accumulate() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read files",
            vec!["read", "file", "content"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "read file"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        // "read" matches keyword "read", "file" matches keyword "file" → 2 * 5 = 10
        assert_eq!(tools[0]["score"], SCORE_KEYWORD * 2);
    }

    #[tokio::test]
    async fn test_no_match_returns_empty() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read file",
            vec!["read", "file"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "zzz_unknown"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_empty_registry_returns_empty() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "anything"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn test_results_capped_at_10() {
        let reg = Arc::new(MockRegistry::new());
        for i in 0..15 {
            reg.insert(make_desc(
                &format!("Tool{}", i),
                "test",
                &format!("tool number {}", i),
                vec!["tool"],
            ))
            .await;
        }
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "tool"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert_eq!(tools.len(), MAX_RESULTS);
    }

    #[tokio::test]
    async fn test_invalid_args_returns_error() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool.call(json!({}), &make_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_stable_sort_by_name() {
        let reg = Arc::new(MockRegistry::new());
        reg.insert(make_desc("Banana", "a", "Banana tool", vec!["tool"]))
            .await;
        reg.insert(make_desc("Apple", "a", "Apple tool", vec!["tool"]))
            .await;
        reg.insert(make_desc("Cherry", "a", "Cherry tool", vec!["tool"]))
            .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "tool"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "Apple");
        assert_eq!(tools[1]["name"], "Banana");
        assert_eq!(tools[2]["name"], "Cherry");
    }

    #[tokio::test]
    async fn test_toolsearch_own_keywords_extracted() {
        let reg = Arc::new(MockRegistry::new());
        // Register a tool with keywords
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read file",
            vec!["read", "file", "content"],
        ))
        .await;
        // Register ToolSearchTool descriptor with its keywords
        reg.insert(make_desc(
            "ToolSearch",
            "meta",
            "Search tools",
            vec!["search", "find", "discover"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "search"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        // ToolSearch: name exact match ("search" == "toolsearch"? No)
        // "search" keyword matches ToolSearch (5) + Read has no match
        // ToolSearch scores: keyword "search" matches → 5
        assert!(!tools.is_empty());
        assert_eq!(tools[0]["name"], "ToolSearch");
    }

    #[tokio::test]
    async fn test_keyword_mode_ordering_by_score_desc() {
        let reg = Arc::new(MockRegistry::new());
        // Tool with 3 matching keywords
        reg.insert(make_desc(
            "Read",
            "file_ops",
            "Read file",
            vec!["read", "file", "content"],
        ))
        .await;
        // Tool with 1 matching keyword
        reg.insert(make_desc(
            "Write",
            "file_ops",
            "Write file",
            vec!["write", "file"],
        ))
        .await;
        // Tool with 0 matching keywords but substring in summary
        reg.insert(make_desc(
            "Grep",
            "search",
            "Search in read files",
            vec!["grep", "search"],
        ))
        .await;
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let result = tool
            .call(json!({"query": "read file"}), &make_ctx())
            .await
            .unwrap();
        let tools = result.data["tools"].as_array().unwrap();
        // Read: 2 keyword matches (read + file) → score 10
        // Write: 1 keyword match (file) → score 5
        // Grep: substring match ("read" in summary) → score 1
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0]["name"], "Read");
        assert_eq!(tools[0]["score"], SCORE_KEYWORD * 2);
        assert_eq!(tools[1]["name"], "Write");
        assert_eq!(tools[1]["score"], SCORE_KEYWORD);
        assert_eq!(tools[2]["name"], "Grep");
        assert_eq!(tools[2]["score"], SCORE_SUBSTRING);
    }

    #[tokio::test]
    async fn test_toolsearch_detail_has_keywords_prefix() {
        let reg = Arc::new(MockRegistry::new());
        let tool = ToolSearchTool::new(Arc::clone(&reg) as Arc<dyn ToolRegistryQuery>);
        let detail = tool.detail();
        assert!(
            detail.starts_with("[keywords:"),
            "ToolSearchTool detail should start with [keywords:], got: {}",
            &detail[..detail.len().min(50)]
        );
    }

    // =====================================================================
    // score_tool — direct unit tests
    // =====================================================================

    #[test]
    fn test_score_tool_exact_name_match_highest() {
        let desc = ToolDescriptor {
            name: "Read".into(),
            group: "file_ops".into(),
            summary: "Read file".into(),
            detail: "[keywords: read file] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        // Exact name match (10) + keyword "read" matches (5) = 15
        let score = score_tool(&desc, "read");
        assert_eq!(score, SCORE_NAME_EXACT + SCORE_KEYWORD);
    }

    #[test]
    fn test_score_tool_keyword_match_higher_than_substring() {
        let desc_with_kw = ToolDescriptor {
            name: "Read".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        let desc_no_kw = ToolDescriptor {
            name: "AuditLog".into(),
            group: "meta".into(),
            summary: "Audit read operations".into(),
            detail: "no keywords here".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        let kw_score = score_tool(&desc_with_kw, "read");
        let sub_score = score_tool(&desc_no_kw, "read");
        assert!(
            kw_score > sub_score,
            "keyword score {} should exceed substring score {}",
            kw_score,
            sub_score
        );
    }

    #[test]
    fn test_score_tool_multiple_keywords_accumulate() {
        let desc = ToolDescriptor {
            name: "Read".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file content] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into(), "content".into()],
        };
        // Query "read file" matches 2 keywords → 2 * SCORE_KEYWORD
        let score = score_tool(&desc, "read file");
        assert_eq!(score, SCORE_KEYWORD * 2);
    }

    #[test]
    fn test_score_tool_no_match_returns_zero() {
        let desc = ToolDescriptor {
            name: "Read".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        let score = score_tool(&desc, "zzz_unknown");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_score_tool_substring_fallback() {
        let desc = ToolDescriptor {
            name: "ToolA".into(),
            group: "file_ops".into(),
            summary: "Read files from disk".into(),
            detail: "no keywords here".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        // No keywords, no name match, "read" appears in summary → SCORE_SUBSTRING
        let score = score_tool(&desc, "read");
        assert_eq!(score, SCORE_SUBSTRING);
    }

    #[test]
    fn test_score_tool_substring_not_applied_when_keyword_matches() {
        let desc = ToolDescriptor {
            name: "ToolX".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file] detail with read in it".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        // No name match, keyword "read" matches → score = SCORE_KEYWORD = 5
        // Substring fallback should NOT add extra points
        let score = score_tool(&desc, "read");
        assert_eq!(score, SCORE_KEYWORD);
    }

    #[test]
    fn test_score_tool_name_exact_beats_keyword_only() {
        // Tool A: name exact match + no keyword match
        let desc_a = ToolDescriptor {
            name: "read".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        // Tool B: no name match, 2 keyword matches
        let desc_b = ToolDescriptor {
            name: "ToolB".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        let score_a = score_tool(&desc_a, "read");
        let score_b = score_tool(&desc_b, "read");
        // A: name exact (10) = 10
        // B: 1 keyword match ("read") = 5
        assert_eq!(score_a, SCORE_NAME_EXACT);
        assert_eq!(score_b, SCORE_KEYWORD);
        // Verify name exact + keyword combo
        let desc_c = ToolDescriptor {
            name: "read".into(),
            group: "file_ops".into(),
            summary: "Read files".into(),
            detail: "[keywords: read file] detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec!["read".into(), "file".into()],
        };
        let score_c = score_tool(&desc_c, "read");
        // C: name exact (10) + keyword (5) = 15 > both A and B
        assert_eq!(score_c, SCORE_NAME_EXACT + SCORE_KEYWORD);
    }

    #[test]
    fn test_score_tool_empty_keywords_no_panic() {
        let desc = ToolDescriptor {
            name: "Empty".into(),
            group: "test".into(),
            summary: "Empty keywords".into(),
            detail: "no keywords".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        let score = score_tool(&desc, "anything");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_score_tool_detail_substring_fallback() {
        let desc = ToolDescriptor {
            name: "ToolA".into(),
            group: "test".into(),
            summary: "summary".into(),
            detail: "This tool handles file operations".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        // "file" appears in detail but not in summary → still matches via detail
        let score = score_tool(&desc, "file");
        assert_eq!(score, SCORE_SUBSTRING);
    }

    #[test]
    fn test_score_tool_word_boundary_fallback() {
        let desc = ToolDescriptor {
            name: "ToolA".into(),
            group: "test".into(),
            summary: "summary".into(),
            detail: "detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        // Query "hello world" — neither word appears in summary or detail
        let score = score_tool(&desc, "hello world");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_score_tool_word_boundary_partial_match() {
        let desc = ToolDescriptor {
            name: "ToolA".into(),
            group: "test".into(),
            summary: "Search the web".into(),
            detail: "detail".into(),
            input_schema: json!({}),
            flags: CommonFlags::default(),
            keywords: vec![],
        };
        // Query "web search" — "web" matches in summary, "search" matches in summary
        // Since no keywords, fallback checks: summary.contains("web search") → false
        // Then word split: "web" in summary → true
        let score = score_tool(&desc, "web search");
        assert_eq!(score, SCORE_SUBSTRING);
    }
}
