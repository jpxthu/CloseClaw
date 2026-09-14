use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Minimal mock for `SkillListingProvider`.
struct MockListingProvider {
    output: String,
    rescan_called: Arc<AtomicBool>,
}

impl SkillListingProvider for MockListingProvider {
    fn rescan(&self) {
        self.rescan_called.store(true, Ordering::SeqCst);
    }

    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

#[test]
fn test_name_and_priority() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    assert_eq!(provider.name(), "skills");
    assert_eq!(provider.priority(), 3);
}

#[tokio::test]
async fn test_cache_key_includes_agent_id() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "agent-xyz".to_string();
    // Empty activated skills → trailing colon + empty string + fingerprint
    let key = provider.cache_key(&ctx).await.unwrap();
    assert!(
        key.starts_with("skill_listing:agent-xyz:"),
        "key should start with agent prefix, got: {key}"
    );
}

#[tokio::test]
async fn test_cache_key_varies_with_agent_id() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));

    let mut ctx_a = FragmentContext::test_default();
    ctx_a.agent_id = "agent-a".to_string();
    let mut ctx_b = FragmentContext::test_default();
    ctx_b.agent_id = "agent-b".to_string();

    assert_ne!(
        provider.cache_key(&ctx_a).await,
        provider.cache_key(&ctx_b).await
    );
}

#[tokio::test]
async fn test_cache_key_varies_with_activated_skills() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));

    let mut ctx_empty = FragmentContext::test_default();
    ctx_empty.agent_id = "agent-1".to_string();

    let mut ctx_activated = FragmentContext::test_default();
    ctx_activated.agent_id = "agent-1".to_string();
    ctx_activated.activated_skills = vec!["skill-a".to_string(), "skill-b".to_string()];

    assert_ne!(
        provider.cache_key(&ctx_empty).await,
        provider.cache_key(&ctx_activated).await,
        "different activation sets must produce different cache keys"
    );
}

#[tokio::test]
async fn test_cache_key_sorts_activated_skills() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));

    let mut ctx_a = FragmentContext::test_default();
    ctx_a.agent_id = "agent-1".to_string();
    ctx_a.activated_skills = vec!["b".to_string(), "a".to_string()];

    let mut ctx_b = FragmentContext::test_default();
    ctx_b.agent_id = "agent-1".to_string();
    ctx_b.activated_skills = vec!["a".to_string(), "b".to_string()];

    assert_eq!(
        provider.cache_key(&ctx_a).await,
        provider.cache_key(&ctx_b).await,
        "same activation set in different order must produce same cache key"
    );
}

#[tokio::test]
async fn test_generate_with_listing() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: "- **foo**: A skill\n- **bar**: Another skill".to_string(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let ctx = FragmentContext::test_default();
    let fragment = provider.generate(&ctx).await;
    let frag = fragment.expect("expected a fragment");
    assert_eq!(frag.section_title, "## Skills");
    assert_eq!(frag.section_type, SectionType::Skills);
    assert!(frag.content.contains("foo"));
    assert!(frag.content.contains("bar"));
}

#[tokio::test]
async fn test_generate_empty_returns_none() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let ctx = FragmentContext::test_default();
    assert!(provider.generate(&ctx).await.is_none());
}

#[tokio::test]
async fn test_generate_triggers_rescan() {
    let rescan_flag = Arc::new(AtomicBool::new(false));
    let mock = MockListingProvider {
        output: "- **test_skill**: A skill".to_string(),
        rescan_called: rescan_flag.clone(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let ctx = FragmentContext::test_default();

    let fragment = provider.generate(&ctx).await;
    assert!(fragment.is_some(), "expected a fragment");

    // Verify rescan was called during generate()
    assert!(
        rescan_flag.load(Ordering::SeqCst),
        "rescan() should have been called during generate()"
    );
}

/// A mock that returns different output based on the activated set,
/// simulating real generate_listing_with_activated behavior.
struct ActivatedMockProvider {
    base_listing: String,
    activated_listing: String,
}

impl SkillListingProvider for ActivatedMockProvider {
    fn rescan(&self) {}

    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.activated_listing.clone()
    }

    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.base_listing.clone()
    }

    fn generate_listing_with_activated(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
        activated: &[String],
    ) -> String {
        if activated.is_empty() {
            self.base_listing.clone()
        } else {
            self.activated_listing.clone()
        }
    }

    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

// ------------------------------------------------------------------
// Dimension: Normal path — activated conditional skills in output
// ------------------------------------------------------------------

#[tokio::test]
async fn test_generate_includes_activated_conditional_skills() {
    let mock = ActivatedMockProvider {
        base_listing: "- **base_skill**: A base skill".to_string(),
        activated_listing:
            "- **base_skill**: A base skill\n- **cond_skill**: ⚡ A conditional skill".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let mut ctx = FragmentContext::test_default();
    ctx.activated_skills = vec!["cond_skill".to_string()];

    let frag = provider.generate(&ctx).await.expect("expected fragment");
    assert!(
        frag.content.contains("cond_skill"),
        "activated conditional skill should appear"
    );
    assert!(
        frag.content.contains("base_skill"),
        "base skill should still appear"
    );
}

#[tokio::test]
async fn test_generate_excludes_unactivated_conditional_skills() {
    // Mock returns only base when no activated conditional skills match.
    let mock = ActivatedMockProvider {
        base_listing: "- **base_skill**: A base skill".to_string(),
        activated_listing: "- **base_skill**: A base skill".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let mut ctx = FragmentContext::test_default();
    ctx.activated_skills = vec!["nonexistent_skill".to_string()];

    let frag = provider.generate(&ctx).await.expect("expected fragment");
    assert!(
        frag.content.contains("base_skill"),
        "base skill should appear"
    );
    assert!(
        !frag.content.contains("nonexistent_skill"),
        "nonexistent skill must not appear"
    );
}

// ------------------------------------------------------------------
// Dimension: Empty activation set — regresses to base listing
// ------------------------------------------------------------------

#[tokio::test]
async fn test_generate_empty_activated_set_matches_excluding_conditional() {
    let base = "- **alpha**: desc alpha\n- **beta**: desc beta".to_string();
    let mock = ActivatedMockProvider {
        base_listing: base.clone(),
        activated_listing: base.clone(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let ctx = FragmentContext::test_default(); // activated_skills is empty

    let frag = provider.generate(&ctx).await.expect("expected fragment");
    assert_eq!(
        frag.content, base,
        "empty activated set must produce base listing"
    );
}

#[tokio::test]
async fn test_generate_empty_activated_set_no_conditional_leak() {
    let mock = ActivatedMockProvider {
        base_listing: "- **plain**: A plain skill".to_string(),
        activated_listing: "- **plain**: A plain skill\n- **cond**: ⚡ conditional".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let ctx = FragmentContext::test_default();

    let frag = provider.generate(&ctx).await.expect("expected fragment");
    assert!(
        !frag.content.contains("cond"),
        "conditional skill must not leak with empty activated set"
    );
}

// ------------------------------------------------------------------
// Dimension: Error/boundary — nonexistent skill in activated set
// ------------------------------------------------------------------

#[tokio::test]
async fn test_generate_nonexistent_activated_skill_no_panic() {
    let mock = ActivatedMockProvider {
        base_listing: "- **real_skill**: exists".to_string(),
        activated_listing: "- **real_skill**: exists".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let mut ctx = FragmentContext::test_default();
    ctx.activated_skills = vec!["deleted_skill".to_string()];

    // Should not panic; provider passes activated set to listing which silently ignores unknowns.
    let result = provider.generate(&ctx).await;
    assert!(
        result.is_some(),
        "should still produce fragment for base skills"
    );
}

// ------------------------------------------------------------------
// Dimension: Cache key includes activation fingerprint
// ------------------------------------------------------------------

#[tokio::test]
async fn test_cache_key_distinct_for_different_activated_sets() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));

    let mut ctx_empty = FragmentContext::test_default();
    ctx_empty.agent_id = "agent-1".to_string();

    let mut ctx_a = FragmentContext::test_default();
    ctx_a.agent_id = "agent-1".to_string();
    ctx_a.activated_skills = vec!["skill-x".to_string()];

    let mut ctx_b = FragmentContext::test_default();
    ctx_b.agent_id = "agent-1".to_string();
    ctx_b.activated_skills = vec!["skill-x".to_string(), "skill-y".to_string()];

    assert_ne!(
        provider.cache_key(&ctx_empty).await,
        provider.cache_key(&ctx_a).await,
        "empty vs single activation must differ"
    );
    assert_ne!(
        provider.cache_key(&ctx_a).await,
        provider.cache_key(&ctx_b).await,
        "different activation sets must differ"
    );
}

#[tokio::test]
async fn test_cache_key_order_independent() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));

    let mut ctx_1 = FragmentContext::test_default();
    ctx_1.agent_id = "agent-1".to_string();
    ctx_1.activated_skills = vec!["b".to_string(), "a".to_string()];

    let mut ctx_2 = FragmentContext::test_default();
    ctx_2.agent_id = "agent-1".to_string();
    ctx_2.activated_skills = vec!["a".to_string(), "b".to_string()];

    assert_eq!(
        provider.cache_key(&ctx_1).await,
        provider.cache_key(&ctx_2).await,
        "same set in different order must produce same cache key"
    );
}

// ------------------------------------------------------------------
// Dimension: Fragment section metadata is correct
// ------------------------------------------------------------------

#[tokio::test]
async fn test_generate_activated_fragment_has_skills_section_type() {
    let mock = ActivatedMockProvider {
        base_listing: "- **skill1**: desc".to_string(),
        activated_listing: "- **skill1**: desc\n- **cond1**: ⚡ cond".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let mut ctx = FragmentContext::test_default();
    ctx.activated_skills = vec!["cond1".to_string()];

    let frag = provider.generate(&ctx).await.expect("expected fragment");
    assert_eq!(frag.section_title, "## Skills");
    assert_eq!(frag.section_type, SectionType::Skills);
}

// ------------------------------------------------------------------
// Dimension: Fingerprint — default mock returns "0"
// ------------------------------------------------------------------

#[test]
fn test_mock_default_fingerprint_is_zero() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    assert_eq!(provider.listing.fingerprint(), "0");
}

// ------------------------------------------------------------------
// Dimension: Cache key includes fingerprint
// ------------------------------------------------------------------

#[tokio::test]
async fn test_cache_key_includes_fingerprint() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "agent-1".to_string();
    let key = provider.cache_key(&ctx).await.unwrap();
    assert!(
        key.ends_with(":0"),
        "cache key should end with fingerprint ':0', got: {key}"
    );
}

// ------------------------------------------------------------------
// Dimension: Cache key varies with fingerprint
// ------------------------------------------------------------------

/// Mock that returns a configurable fingerprint.
struct FingerprintMockProvider {
    output: String,
    fp: String,
}

impl SkillListingProvider for FingerprintMockProvider {
    fn rescan(&self) {}
    fn fingerprint(&self) -> String {
        self.fp.clone()
    }
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }
    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.output.clone()
    }
    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

#[tokio::test]
async fn test_cache_key_varies_with_fingerprint() {
    let provider_a = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
        output: String::new(),
        fp: "fp_a".to_string(),
    }));
    let provider_b = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
        output: String::new(),
        fp: "fp_b".to_string(),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "agent-1".to_string();
    assert_ne!(
        provider_a.cache_key(&ctx).await,
        provider_b.cache_key(&ctx).await,
        "different fingerprints must produce different cache keys"
    );
}

#[tokio::test]
async fn test_cache_key_same_fingerprint_same_key() {
    let provider_a = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
        output: String::new(),
        fp: "same_fp".to_string(),
    }));
    let provider_b = SkillsFragmentProvider::new(Arc::new(FingerprintMockProvider {
        output: String::new(),
        fp: "same_fp".to_string(),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "agent-1".to_string();
    assert_eq!(
        provider_a.cache_key(&ctx).await,
        provider_b.cache_key(&ctx).await,
        "same fingerprints must produce same cache keys"
    );
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — within limit preserves byte-identical output
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_within_limit_unchanged() {
    let listing = "- **alpha**: desc alpha\n- **beta**: desc beta";
    let result = truncate_listing(listing, 4000);
    assert_eq!(result, listing);
}

#[test]
fn test_truncate_listing_exact_limit_unchanged() {
    let listing = "- **alpha**: desc alpha";
    assert_eq!(listing.len(), 23);
    let result = truncate_listing(listing, 23);
    assert_eq!(result, listing);
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — drops whole entries, no half entries
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_drops_whole_entry() {
    let listing =
        "- **alpha**: short\n- **beta**: a much longer description that takes many characters";
    let result = truncate_listing(listing, 50);
    assert_eq!(result, "- **alpha**: short");
}

#[test]
fn test_truncate_listing_never_produces_half_entry() {
    let listing = "- **a**: short\n- **b**: medium length\n- **c**: another entry";
    let result = truncate_listing(listing, 30);
    for line in result.lines() {
        assert!(
            line.starts_with("- **"),
            "truncated entry must be whole: {line}"
        );
    }
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — at least 1 entry preserved
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_at_least_one_entry() {
    let long_entry = format!("- **mega**: {}", "x".repeat(500));
    let result = truncate_listing(&long_entry, 100);
    assert_eq!(
        result, long_entry,
        "must keep the single entry even if it exceeds the limit"
    );
}

#[test]
fn test_truncate_listing_at_least_one_with_multiple_entries() {
    let listing = "- **a**: very long description\n- **b**: second entry";
    let result = truncate_listing(listing, 10);
    assert_eq!(result.lines().count(), 1);
    assert!(result.starts_with("- **a"));
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — exactly at limit boundary
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_boundary_exactly_fits_two() {
    let listing = "- **a**: 1234\n- **b**: 5678";
    let result = truncate_listing(listing, 27);
    assert_eq!(result, listing);
}

#[test]
fn test_truncate_listing_boundary_one_over() {
    let listing = "- **a**: 1234\n- **b**: 5678";
    // Total = 27 bytes. max_len = 26 forces truncation after first entry.
    let result = truncate_listing(listing, 26);
    assert_eq!(result, "- **a**: 1234");
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — multi-byte UTF-8 entries
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_multibyte_utf8_within_limit() {
    let listing = "- **skill_cn**: 描述一个中文技能\n- **skill_en**: Another skill";
    assert!(!listing.is_empty());
    let result = truncate_listing(listing, 4000);
    assert_eq!(result, listing);
}

#[test]
fn test_truncate_listing_multibyte_utf8_truncates_at_byte_boundary() {
    // 描述 = 6 bytes (3 each), 技能 = 6 bytes (3 each)
    let listing = "- **skill_cn**: 描述一个中文技能\n- **skill_en**: Another skill";
    // Choose a limit that cuts within the first entry but not at a char boundary
    let limit = "- **skill_cn**: 描述一中".len();
    let result = truncate_listing(listing, limit);
    // Should keep at least the first entry (even if it exceeds limit)
    assert!(
        result.starts_with("- **skill_cn"),
        "must keep first entry, got: {result}"
    );
    // Result must be valid UTF-8
    assert!(
        std::str::from_utf8(result.as_bytes()).is_ok(),
        "result must be valid UTF-8"
    );
    // Should not contain the second entry
    assert!(
        !result.contains("skill_en"),
        "second entry must not appear when truncated"
    );
}

#[test]
fn test_truncate_listing_multibyte_utf8_preserves_whole_entries() {
    // Two entries with multi-byte chars; limit fits first but not second
    let entry1 = "- **cn_skill**: 中文描述";
    let entry2 = "- **en_skill**: English description";
    let listing = format!("{}\n{}", entry1, entry2);
    let limit = entry1.len() + 5; // fits entry1 + separator but not entry2
    let result = truncate_listing(&listing, limit);
    assert_eq!(result, entry1, "should keep only the first whole entry");
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — empty input
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_empty_string() {
    assert_eq!(truncate_listing("", 4000), "");
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — constant value check
// ------------------------------------------------------------------

#[test]
fn test_skills_section_max_len_value() {
    assert_eq!(SKILLS_SECTION_MAX_LEN, 4000);
}

// ------------------------------------------------------------------
// Dimension: truncate_listing — multiple entries partial truncation
// ------------------------------------------------------------------

#[test]
fn test_truncate_listing_multiple_entries_partial() {
    // Each entry = 14 bytes. 3 entries with separators = 14+1+14+1+14 = 44.
    // max_len = 43: first 2 entries (29 bytes) fit, adding 3rd (44) > 43, so 2 kept.
    let listing = "- **a**: short\n- **b**: short\n- **c**: short\n- **d\": short";
    let result = truncate_listing(listing, 43);
    assert!(result.starts_with("- **a"));
    assert!(result.contains("- **b"));
    assert!(!result.contains("- **c"));
}

// ------------------------------------------------------------------
// Dimension: Cache invalidation — long chain: change → assemble →
//   change → assemble → both rebuilds reflect latest skill set
// ------------------------------------------------------------------

use std::sync::atomic::AtomicUsize;

/// Mock backed by a shared generation counter.
/// When the generation changes, the fingerprint and output change.
struct GenerationMock {
    generation: Arc<AtomicUsize>,
}

impl GenerationMock {
    fn new(gen: Arc<AtomicUsize>) -> Self {
        Self { generation: gen }
    }
}

impl SkillListingProvider for GenerationMock {
    fn rescan(&self) {}
    fn fingerprint(&self) -> String {
        format!("gen:{}", self.generation.load(Ordering::SeqCst))
    }
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        let gen = self.generation.load(Ordering::SeqCst);
        (0..=gen)
            .map(|i| format!("- **skill-{i}**: desc {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        self.generate_listing(_agent_id, _agent_skills)
    }
    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

/// Long chain: two skill-file changes, two assemblies, each
/// rebuilds with the latest skill set.
#[tokio::test]
async fn test_long_chain_reflects_latest_skills() {
    let gen = Arc::new(AtomicUsize::new(0));
    let provider = SkillsFragmentProvider::new(Arc::new(GenerationMock::new(gen.clone())));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "agent-1".to_string();

    // Cycle 1: initial state (1 skill)
    let key1 = provider.cache_key(&ctx).await.unwrap();
    assert!(key1.contains("gen:0"));
    let frag1 = provider.generate(&ctx).await.expect("fragment");
    assert!(frag1.content.contains("skill-0"));
    assert!(!frag1.content.contains("skill-1"));

    // Change: add skill-1
    gen.store(1, Ordering::SeqCst);
    let key2 = provider.cache_key(&ctx).await.unwrap();
    assert_ne!(key1, key2, "cache key must change after skill addition");
    let frag2 = provider.generate(&ctx).await.expect("fragment");
    assert!(frag2.content.contains("skill-1"), "must reflect new skill");

    // Change: add skill-2
    gen.store(2, Ordering::SeqCst);
    let key3 = provider.cache_key(&ctx).await.unwrap();
    assert_ne!(key2, key3, "cache key must change after second addition");
    let frag3 = provider.generate(&ctx).await.expect("fragment");
    assert!(
        frag3.content.contains("skill-2"),
        "must reflect latest skill"
    );
    assert!(
        frag3.content.contains("skill-1"),
        "must still include previous skill"
    );
}

// ------------------------------------------------------------------
// Dimension: Error path — fingerprint degrades when listing fails
// ------------------------------------------------------------------

/// Mock that simulates listing failure (empty output) with
/// a stable fingerprint that doesn't panic.
struct FailingListingMock {
    fp: String,
}

impl SkillListingProvider for FailingListingMock {
    fn rescan(&self) {}
    fn fingerprint(&self) -> String {
        self.fp.clone()
    }
    fn generate_listing(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        String::new() // simulates listing failure
    }
    fn generate_listing_excluding_conditional(
        &self,
        _agent_id: Option<&str>,
        _agent_skills: Option<&[String]>,
    ) -> String {
        String::new()
    }
    fn find_conditional_matches(
        &self,
        _paths: &[std::path::PathBuf],
    ) -> Vec<closeclaw_common::ConditionalSkillMatch> {
        vec![]
    }
}

/// When listing is empty (degraded), generate returns None without panic,
/// and cache_key still produces a valid key.
#[tokio::test]
async fn test_degraded_listing_empty_output_no_panic() {
    let mock = FailingListingMock {
        fp: "degraded".to_string(),
    };
    let provider = SkillsFragmentProvider::new(Arc::new(mock));
    let ctx = FragmentContext::test_default();

    // generate should return None (empty output), no panic
    let result = provider.generate(&ctx).await;
    assert!(result.is_none(), "empty listing must produce None");

    // cache_key should still work
    let key = provider.cache_key(&ctx).await.unwrap();
    assert!(key.ends_with("degraded"));
}

/// When listing fails across multiple fingerprint versions,
/// generate always returns None but cache keys remain distinct.
#[tokio::test]
async fn test_degraded_listing_fingerprint_evolution() {
    let mock_v1 = FailingListingMock {
        fp: "v1".to_string(),
    };
    let provider1 = SkillsFragmentProvider::new(Arc::new(mock_v1));
    let ctx = FragmentContext::test_default();
    let key1 = provider1.cache_key(&ctx).await.unwrap();
    assert!(provider1.generate(&ctx).await.is_none());

    let mock_v2 = FailingListingMock {
        fp: "v2".to_string(),
    };
    let provider2 = SkillsFragmentProvider::new(Arc::new(mock_v2));
    let key2 = provider2.cache_key(&ctx).await.unwrap();
    assert!(provider2.generate(&ctx).await.is_none());

    assert_ne!(
        key1, key2,
        "different degraded states must have distinct keys"
    );
}

// ------------------------------------------------------------------
// Dimension: Async regression (#2436 Bug A) — generate() and cache_key()
// called from async context must not panic. Prior to the fix,
// spawn_blocking + block_on in async caused "Cannot start a runtime
// from within a runtime".
// ------------------------------------------------------------------

/// Regression: generate() must not panic when called from a tokio async
/// context. Exercises the spawn_blocking path that replaced the
/// previous rt.block_on call.
#[tokio::test]
async fn test_generate_async_no_panic_regression_2436() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: "- **regression_skill**: test".to_string(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let ctx = FragmentContext::test_default();
    // Should not panic — the "Cannot start a runtime from within a
    // runtime" error is the specific regression being guarded.
    let result = provider.generate(&ctx).await;
    assert!(
        result.is_some(),
        "generate() must return a fragment for non-empty listing"
    );
}

/// Regression: cache_key() must not panic when called from a tokio async
/// context. Exercises the spawn_blocking path for fingerprint().
#[tokio::test]
async fn test_cache_key_async_no_panic_regression_2436() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "regression-agent".to_string();
    // Must not panic — fingerprint() previously ran via block_on.
    let key = provider.cache_key(&ctx).await;
    assert!(key.is_some(), "cache_key() must return a key");
}

/// Regression: full chain — generate() then cache_key() in the same
/// async task, simulating the builder pipeline. Both must complete
/// without panic.
#[tokio::test]
async fn test_full_chain_async_no_panic_regression_2436() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: "- **chain_skill**: chain test".to_string(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let mut ctx = FragmentContext::test_default();
    ctx.agent_id = "chain-agent".to_string();
    ctx.activated_skills = vec!["chain_skill".to_string()];

    // Both calls in the same async context — no nested runtimes.
    let key = provider.cache_key(&ctx).await;
    assert!(key.is_some(), "cache_key must succeed");
    let frag = provider.generate(&ctx).await;
    assert!(frag.is_some(), "generate must succeed");
    let frag = frag.unwrap();
    assert!(
        frag.content.contains("chain_skill"),
        "fragment must contain the skill"
    );
}

/// Regression: empty listing in async context — generate() returns None
/// without panic, cache_key() still works.
#[tokio::test]
async fn test_empty_async_no_panic_regression_2436() {
    let provider = SkillsFragmentProvider::new(Arc::new(MockListingProvider {
        output: String::new(),
        rescan_called: Arc::new(AtomicBool::new(false)),
    }));
    let ctx = FragmentContext::test_default();

    let key = provider.cache_key(&ctx).await;
    assert!(
        key.is_some(),
        "cache_key must succeed even with empty listing"
    );
    let result = provider.generate(&ctx).await;
    assert!(result.is_none(), "empty listing must return None");
}
