//! Unit tests for the embedding module.
//!
//! Covers NgramEmbedder creation, cosine similarity, edge cases,
//! and determinism guarantees.

use crate::embedding::{cosine_similarity, EntityEmbedder, NgramEmbedder};

/// Shared corpus for building vocabulary.
fn corpus() -> Vec<&'static str> {
    vec![
        "rust",
        "rust language",
        "python",
        "python programming",
        "memory",
        "memory management",
        "entity",
        "entity type",
        "similarity",
        "cosine distance",
    ]
}

// ── Normal path ─────────────────────────────────────────────────────

/// Identical text produces cosine_similarity ≈ 1.0.
#[test]
fn test_identical_text_similarity_one() {
    let emb = NgramEmbedder::new(&corpus());
    let a = emb.embed("rust language");
    let b = emb.embed("rust language");
    let sim = cosine_similarity(&a, &b);
    assert!(
        (sim - 1.0).abs() < 1e-10,
        "identical text should have similarity ~1.0, got {sim}"
    );
}

/// Completely unrelated texts produce low cosine similarity.
#[test]
fn test_unrelated_text_low_similarity() {
    let emb = NgramEmbedder::new(&corpus());
    let a = emb.embed("rust language");
    let b = emb.embed("python programming");
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim < 0.6,
        "unrelated texts should have low similarity, got {sim}"
    );
}

/// Partially similar texts produce a middle-range cosine similarity.
#[test]
fn test_partial_similarity_middle_value() {
    let emb = NgramEmbedder::new(&corpus());
    let a = emb.embed("memory management");
    let b = emb.embed("memory entity");
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim > 0.3 && sim < 1.0,
        "partial similarity should be in (0.3, 1.0), got {sim}"
    );
}

// ── Boundary values ─────────────────────────────────────────────────

/// Empty string input produces zero or empty vector.
#[test]
fn test_empty_string_input() {
    let emb = NgramEmbedder::new(&corpus());
    let v = emb.embed("");
    assert!(
        v.is_empty() || v.iter().all(|x| *x == 0.0),
        "empty input should produce zero/empty vector"
    );
}

/// Single character input produces a valid L2-normalised vector.
#[test]
fn test_single_character_input() {
    let emb = NgramEmbedder::new(&corpus());
    let v = emb.embed("r");
    assert!(!v.is_empty(), "single char should produce non-empty vector");
    let mag: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    assert!(
        (mag - 1.0).abs() < 1e-10 || mag == 0.0,
        "vector should be L2-normalised, got mag {mag}"
    );
}

/// Very long text produces a valid vector (zero if no vocab matches).
#[test]
fn test_long_text_input() {
    let emb = NgramEmbedder::new(&corpus());
    // Repeated char text has no n-grams in vocabulary → zero vector
    let long = "a".repeat(10_000);
    let v = emb.embed(&long);
    assert!(!v.is_empty(), "long text should produce non-empty vector");
    let mag: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    // Either unit normalised or zero vector (no vocab matches)
    assert!(
        (mag - 1.0).abs() < 1e-10 || mag == 0.0,
        "long text vector should be L2-normalised or zero, got mag {mag}"
    );
    // Text with matching n-grams should produce non-zero vector
    let v2 = emb.embed("rust language management entity similarity");
    let mag2: f64 = v2.iter().map(|x| x * x).sum::<f64>().sqrt();
    assert!(
        (mag2 - 1.0).abs() < 1e-10,
        "text with vocab matches should be unit normalised"
    );
}

// ── Determinism ─────────────────────────────────────────────────────

/// Same input always produces the same embedding.
#[test]
fn test_deterministic_output() {
    let emb = NgramEmbedder::new(&corpus());
    let v1 = emb.embed("cosine similarity");
    let v2 = emb.embed("cosine similarity");
    assert_eq!(v1, v2, "deterministic embedding failed");
}

/// Cosine similarity is commutative: sim(a, b) == sim(b, a).
#[test]
fn test_similarity_commutative() {
    let emb = NgramEmbedder::new(&corpus());
    let a = emb.embed("rust");
    let b = emb.embed("entity type");
    let ab = cosine_similarity(&a, &b);
    let ba = cosine_similarity(&b, &a);
    assert!(
        (ab - ba).abs() < 1e-10,
        "similarity should be commutative: {ab} vs {ba}"
    );
}

// ── L2 normalisation ────────────────────────────────────────────────

/// Every non-zero embedding vector has unit magnitude.
#[test]
fn test_l2_normalised_vectors() {
    let emb = NgramEmbedder::new(&corpus());
    for text in &["rust", "memory management", "entity type", "a"] {
        let v = emb.embed(text);
        let mag: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!(
            (mag - 1.0).abs() < 1e-10,
            "vector for '{text}' should be unit, got mag {mag}"
        );
    }
}

// ── Zero vector handling ────────────────────────────────────────────

/// cosine_similarity on empty slices returns 0.0.
#[test]
fn test_cosine_similarity_empty_vectors() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
    assert_eq!(cosine_similarity(&[], &[1.0]), 0.0);
}

/// cosine_similarity on different-length vectors returns 0.0.
#[test]
fn test_cosine_similarity_different_lengths() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
}

// ── NgramEmbedder construction ──────────────────────────────────────

/// Embedder built from empty corpus still works (vocabulary is empty,
/// resulting in zero vectors).
#[test]
fn test_empty_corpus_embedder() {
    let emb = NgramEmbedder::new(&[]);
    let v = emb.embed("hello");
    assert!(
        v.is_empty() || v.iter().all(|x| *x == 0.0),
        "empty corpus should produce zero/empty vector"
    );
}
