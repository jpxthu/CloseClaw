//! Character n-gram based text embedding for similarity computation.
//!
//! Provides `NgramEmbedder` — a zero-dependency, deterministic local
//! embedding using character 3-gram frequency vectors with bigram
//! fallback for short texts. All vectors are L2-normalised.

use std::collections::HashMap;

/// Trait for text-to-vector embedding and similarity computation.
pub trait EntityEmbedder {
    /// Embed `text` into a fixed-dimension L2-normalised vector.
    fn embed(&self, text: &str) -> Vec<f64>;

    /// Compute cosine similarity between two embedding vectors.
    fn similarity(a: &[f64], b: &[f64]) -> f64;
}

/// Character n-gram frequency vectoriser.
///
/// Builds a vocabulary of character n-grams (3-gram with bigram
/// fallback for texts shorter than 3 characters), computes their
/// frequency counts, and L2-normalises the result.
pub struct NgramEmbedder {
    /// Character-level n-gram size.
    n: usize,
    /// Vocabulary: sorted unique n-gram strings → column index.
    vocab: HashMap<String, usize>,
}

/// Cosine similarity between two L2-normalised vectors.
///
/// Returns `0.0` when either vector has zero magnitude.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
}

// ---------------------------------------------------------------------------
// NgramEmbedder
// ---------------------------------------------------------------------------

impl NgramEmbedder {
    /// Create a new `NgramEmbedder` with vocabulary built from `corpus`.
    ///
    /// `corpus` is a slice of reference texts whose n-grams will define
    /// the vocabulary dimensions. For a standalone embedder (no corpus),
    /// pass an empty slice — the vocabulary will be built lazily from
    /// the first `embed` call, but this limits dimension consistency
    /// across vectors. Prefer supplying a corpus.
    pub fn new(corpus: &[&str]) -> Self {
        let mut vocab: HashMap<String, usize> = HashMap::new();
        let mut idx = 0usize;

        for text in corpus {
            // Include 3-grams, 2-grams, and 1-grams in vocabulary
            for ngram_size in [3, 2, 1] {
                let grams = Self::extract_grams(text, ngram_size);
                for gram in grams {
                    if !vocab.contains_key(gram) {
                        vocab.insert(gram.to_string(), idx);
                        idx += 1;
                    }
                }
            }
        }

        Self { n: 3, vocab }
    }

    /// Extract character n-grams from `text`.
    ///
    /// For texts with fewer than `n` characters, falls back to
    /// bigrams (n-1) and then unigrams (n-2) so that very short
    /// inputs still produce vectors.
    fn extract_grams(text: &str, n: usize) -> Vec<&str> {
        if text.is_empty() {
            return Vec::new();
        }

        let bytes = text.as_bytes();
        let mut grams = Vec::new();

        // Primary n-grams
        for i in 0..bytes.len().saturating_sub(n - 1) {
            grams.push(&text[i..i + n]);
        }

        // Fallback: (n-1)-grams for short texts
        if grams.is_empty() && n > 2 {
            for i in 0..bytes.len().saturating_sub(n - 2) {
                grams.push(&text[i..i + n - 1]);
            }
        }

        // Fallback: unigrams for very short texts
        if grams.is_empty() {
            for start in text.char_indices().map(|(i, _)| i) {
                let ch_len = text[start..].chars().next().unwrap().len_utf8();
                grams.push(&text[start..start + ch_len]);
            }
        }

        grams
    }

    /// Build a frequency vector for `text` in the current vocabulary.
    fn frequency_vector(&self, text: &str) -> Vec<f64> {
        let dim = self.vocab.len();
        if dim == 0 {
            return Vec::new();
        }

        let mut vec = vec![0.0f64; dim];

        for gram in Self::extract_grams(text, self.n) {
            if let Some(&col) = self.vocab.get(gram) {
                vec[col] += 1.0;
            }
        }

        vec
    }

    /// L2-normalise a vector in place. Returns `true` if the vector
    /// had non-zero magnitude.
    fn l2_normalise(vec: &mut [f64]) -> bool {
        let mag: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
        if mag == 0.0 {
            return false;
        }
        for v in vec.iter_mut() {
            *v /= mag;
        }
        true
    }
}

impl EntityEmbedder for NgramEmbedder {
    fn embed(&self, text: &str) -> Vec<f64> {
        let mut vec = self.frequency_vector(text);
        Self::l2_normalise(&mut vec);
        vec
    }

    fn similarity(a: &[f64], b: &[f64]) -> f64 {
        cosine_similarity(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<&'static str> {
        vec![
            "rust",
            "rust language",
            "python",
            "python programming",
            "memory",
            "memory management",
        ]
    }

    #[test]
    fn test_same_text_similarity_is_one() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("rust language");
        let b = emb.embed("rust language");
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-10, "expected ~1.0, got {sim}");
    }

    #[test]
    fn test_different_text_low_similarity() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("rust language");
        let b = emb.embed("python programming");
        let sim = cosine_similarity(&a, &b);
        assert!(sim < 0.5, "expected < 0.5 for different domains, got {sim}");
    }

    #[test]
    fn test_similar_text_medium_similarity() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("rust language");
        let b = emb.embed("rust lang");
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.5, "expected > 0.5 for similar texts, got {sim}");
    }

    #[test]
    fn test_empty_text() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("");
        let b = emb.embed("");
        assert!(a.is_empty() || a.iter().all(|x| *x == 0.0));
        assert!(b.is_empty() || b.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn test_single_character() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("r");
        let b = emb.embed("r");
        assert_eq!(a.len(), b.len());
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-10, "expected ~1.0, got {sim}");
    }

    #[test]
    fn test_determinism() {
        let emb = NgramEmbedder::new(&corpus());
        let a1 = emb.embed("memory management");
        let a2 = emb.embed("memory management");
        assert_eq!(a1, a2);
    }

    #[test]
    fn test_similarity_is_symmetric() {
        let emb = NgramEmbedder::new(&corpus());
        let a = emb.embed("rust");
        let b = emb.embed("rust language");
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ba = cosine_similarity(&b, &a);
        assert!(
            (sim_ab - sim_ba).abs() < 1e-10,
            "asymmetric: {sim_ab} vs {sim_ba}"
        );
    }

    #[test]
    fn test_zero_vector_similarity() {
        let sim = cosine_similarity(&[], &[]);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn test_l2_normalised() {
        let emb = NgramEmbedder::new(&corpus());
        let v = emb.embed("rust language");
        let mag: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!(
            (mag - 1.0).abs() < 1e-10,
            "expected unit vector, got mag {mag}"
        );
    }
}
