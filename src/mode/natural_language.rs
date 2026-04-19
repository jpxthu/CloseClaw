//! Natural Language Intent Recognition
//!
//! Recognizes user intent from natural language input and maps it to
//! reasoning modes with confidence scores.

use serde::{Deserialize, Serialize};

pub use crate::session::persistence::ReasoningMode;

/// Intent recognition result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentResult {
    /// Recognized target mode
    pub mode: ReasoningMode,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    /// Matched keywords
    pub matched_keywords: Vec<String>,
}

impl IntentResult {
    /// Create a new IntentResult
    pub fn new(mode: ReasoningMode, confidence: f32, matched_keywords: Vec<String>) -> Self {
        Self {
            mode,
            confidence,
            matched_keywords,
        }
    }

    /// Check if confidence is above threshold
    pub fn is_confident(&self, threshold: f32) -> bool {
        self.confidence >= threshold
    }
}

/// Natural language patterns for intent recognition
#[derive(Clone)]
pub struct NaturalLanguagePatterns {
    /// Planning intent patterns
    plan_patterns: Vec<(&'static str, f32)>,
    /// Code intent patterns
    code_patterns: Vec<(&'static str, f32)>,
    /// Debug intent patterns
    debug_patterns: Vec<(&'static str, f32)>,
    /// Review intent patterns
    review_patterns: Vec<(&'static str, f32)>,
}

impl NaturalLanguagePatterns {
    /// Create with default Chinese patterns
    pub fn new() -> Self {
        Self {
            plan_patterns: vec![
                ("帮我规划", 1.0),
                ("怎么设计", 1.0),
                ("有什么方案", 1.0),
                ("设计一个", 1.0),
                ("规划一下", 1.0),
                ("请分析一下", 0.8),
                ("帮我分析", 0.8),
                ("方案是什么", 0.8),
                ("怎么实现", 0.6),
                ("如何设计", 0.8),
            ],
            code_patterns: vec![
                ("写代码", 1.0),
                ("如何实现", 1.0),
                ("代码示例", 1.0),
                ("写个函数", 1.0),
                ("写一个", 1.0),
                ("写一段", 0.9),
                ("实现一个", 0.9),
                ("帮我写", 1.0),
                ("生成代码", 1.0),
                ("排序函数", 0.8),
            ],
            debug_patterns: vec![
                ("为什么报错", 1.0),
                ("为什么报", 1.0),
                ("怎么修复", 1.0),
                ("问题排查", 1.0),
                ("调试", 1.0),
                ("报错", 0.9),
                ("出错了", 0.9),
                ("有问题", 0.5),
                ("不对", 0.3),
            ],
            review_patterns: vec![
                ("检查一下", 1.0),
                ("有什么问题", 1.0),
                ("代码审查", 1.0),
                ("review", 1.0),
                ("审视", 0.8),
                ("帮我看看", 0.7),
                ("看看代码", 0.8),
            ],
        }
    }

    /// Check input against a pattern list and return matched keywords with weights
    fn match_patterns(
        &self,
        input: &str,
        patterns: &[(&'static str, f32)],
    ) -> Vec<(&'static str, f32)> {
        let lower = input.to_lowercase();
        patterns
            .iter()
            .filter(|(keyword, _)| lower.contains(&keyword.to_lowercase()))
            .copied()
            .collect()
    }

    /// Build an IntentResult from matched patterns
    fn build_intent(&self, mode: ReasoningMode, matches: Vec<(&'static str, f32)>) -> IntentResult {
        if matches.is_empty() {
            return IntentResult::new(ReasoningMode::Direct, 0.0, vec![]);
        }
        let confidence = (matches.len() as f32 * 0.85).min(1.0);
        let keywords: Vec<String> = matches.iter().map(|(k, _)| (*k).to_string()).collect();
        IntentResult::new(mode, confidence, keywords)
    }
}

impl NaturalLanguagePatterns {
    /// Recognize planning intent
    pub fn recognize_plan(&self, input: &str) -> IntentResult {
        let matches = self.match_patterns(input, &self.plan_patterns);
        self.build_intent(ReasoningMode::Plan, matches)
    }

    /// Recognize code generation intent
    pub fn recognize_code(&self, input: &str) -> IntentResult {
        let matches = self.match_patterns(input, &self.code_patterns);
        self.build_intent(ReasoningMode::Stream, matches)
    }

    /// Recognize debug intent
    pub fn recognize_debug(&self, input: &str) -> IntentResult {
        let matches = self.match_patterns(input, &self.debug_patterns);
        self.build_intent(ReasoningMode::Stream, matches)
    }

    /// Recognize review intent
    pub fn recognize_review(&self, input: &str) -> IntentResult {
        let matches = self.match_patterns(input, &self.review_patterns);
        self.build_intent(ReasoningMode::Plan, matches)
    }
}

impl Default for NaturalLanguagePatterns {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse natural language input and return the best intent match
pub fn parse_natural_language_intent(input: &str) -> IntentResult {
    let patterns = NaturalLanguagePatterns::new();

    let plan_result = patterns.recognize_plan(input);
    let code_result = patterns.recognize_code(input);
    let debug_result = patterns.recognize_debug(input);
    let review_result = patterns.recognize_review(input);

    // Find the best match
    let mut best = IntentResult::new(ReasoningMode::Direct, 0.0, vec![]);

    for result in [plan_result, code_result, debug_result, review_result] {
        if result.confidence > best.confidence {
            best = result;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_intent_recognition() {
        let patterns = NaturalLanguagePatterns::new();

        let result = patterns.recognize_plan("帮我规划一下系统架构");
        assert!(result.confidence > 0.0);
        assert_eq!(result.mode, ReasoningMode::Plan);

        let result = patterns.recognize_plan("怎么设计一个缓存系统");
        assert!(result.confidence > 0.0);

        let result = patterns.recognize_plan("有什么方案吗");
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn test_code_intent_recognition() {
        let patterns = NaturalLanguagePatterns::new();

        let result = patterns.recognize_code("写一个排序函数");
        assert!(result.confidence > 0.0);
        assert_eq!(result.mode, ReasoningMode::Stream);

        let result = patterns.recognize_code("帮我写段代码");
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn test_debug_intent_recognition() {
        let patterns = NaturalLanguagePatterns::new();

        let result = patterns.recognize_debug("为什么报这个错");
        assert!(result.confidence > 0.0);
        assert_eq!(result.mode, ReasoningMode::Stream);

        let result = patterns.recognize_debug("怎么修复这个问题");
        assert!(result.confidence > 0.0);
    }

    #[test]
    fn test_review_intent_recognition() {
        let patterns = NaturalLanguagePatterns::new();

        let result = patterns.recognize_review("帮我检查一下这段代码");
        assert!(result.confidence > 0.0);
        assert_eq!(result.mode, ReasoningMode::Plan);
    }

    #[test]
    fn test_no_intent() {
        let patterns = NaturalLanguagePatterns::new();

        let result = patterns.recognize_plan("今天天气不错");
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_parse_natural_language_best_match() {
        let result = parse_natural_language_intent("帮我设计一个缓存系统");
        assert!(result.confidence > 0.0);
        // Should match plan patterns
        assert_eq!(result.mode, ReasoningMode::Plan);
    }

    #[test]
    fn test_multiple_keywords_increase_confidence() {
        let patterns = NaturalLanguagePatterns::new();

        // Single keyword
        let result1 = patterns.recognize_plan("帮我规划");
        // Multiple keywords
        let result2 = patterns.recognize_plan("帮我规划一下系统架构，有什么方案吗");

        assert!(result2.confidence >= result1.confidence);
    }

    #[test]
    fn test_intent_result_threshold() {
        let result = IntentResult::new(ReasoningMode::Plan, 0.5, vec![]);
        assert!(result.is_confident(0.4));
        assert!(!result.is_confident(0.6));
    }
}
