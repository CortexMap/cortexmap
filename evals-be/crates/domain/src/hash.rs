//! SHA-256 content hash helper. Re-exposed so callers don't need to depend
//! on `sha2` directly; matches brainatlas-be's `compute_hash` signature so
//! the hash space is interchangeable across services.

use sha2::{Digest, Sha256};

/// SHA-256 hex digest of the input string. Output is 64 lowercase hex chars.
///
/// This is the single source of truth for the cache key — any code that writes
/// or reads an `eval_scores.summary_hash` column MUST use this function so
/// identical bytes produce identical keys.
pub fn compute_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_hash_matches_known_value() {
        // Standard SHA-256 of the empty string.
        assert_eq!(
            compute_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn identical_inputs_produce_identical_hashes() {
        let a = compute_hash("the hippocampus supports declarative memory");
        let b = compute_hash("the hippocampus supports declarative memory");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn different_inputs_produce_different_hashes() {
        let a = compute_hash("hippocampus");
        let b = compute_hash("Hippocampus");
        assert_ne!(a, b);
    }

    /// `compute_hash` is a pure byte-level SHA-256 — it must NOT normalise
    /// whitespace. A trailing newline or an interior space change must
    /// produce a fresh cache key so the eval pipeline re-scores the summary.
    /// This test locks that contract: bump the hash (i.e. accidentally add
    /// trim/normalise) and the whole cache silently invalidates.
    #[test]
    fn whitespace_is_significant() {
        let base = "the hippocampus";
        let with_trailing_newline = "the hippocampus\n";
        let with_leading_space = " the hippocampus";
        let with_double_space = "the  hippocampus";

        assert_ne!(compute_hash(base), compute_hash(with_trailing_newline));
        assert_ne!(compute_hash(base), compute_hash(with_leading_space));
        assert_ne!(compute_hash(base), compute_hash(with_double_space));
    }

    /// Output is always exactly 64 lowercase hex chars, for every input
    /// length — including the 1-byte and 1 KiB edges.
    #[test]
    fn output_is_always_64_lowercase_hex() {
        let inputs = [
            "",
            "a",
            "hippocampus",
            &"x".repeat(1024),
            "unicode: αβγ 🧠 مرحبا",
        ];
        for s in inputs {
            let h = compute_hash(s);
            assert_eq!(h.len(), 64, "hash of {:?} has wrong length", s);
            assert!(
                h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "hash must be lowercase hex, got {}",
                h
            );
        }
    }

    /// Unicode bytes are hashed directly. Two visually-similar strings that
    /// differ only in byte representation (e.g. precomposed vs decomposed
    /// forms) produce different hashes — i.e. the function does NOT perform
    /// NFC/NFD normalisation. Callers that need semantic equality must
    /// normalise upstream before calling `compute_hash`.
    #[test]
    fn unicode_not_normalised() {
        // "é" precomposed (U+00E9) vs decomposed (U+0065 U+0301)
        let precomposed = "caf\u{00E9}";
        let decomposed = "cafe\u{0301}";
        assert_ne!(
            precomposed, decomposed,
            "sanity: the two byte representations must differ"
        );
        assert_ne!(
            compute_hash(precomposed),
            compute_hash(decomposed),
            "compute_hash is byte-level; callers must normalise upstream"
        );
    }

    /// Locks the exact SHA-256 of a short ASCII string against the known
    /// value from `sha256sum(1)`. If the upstream `sha2` crate ever
    /// produced a different output, a persisted cache row would silently
    /// stop matching — this test will catch that.
    #[test]
    fn known_vector_abc() {
        // `echo -n 'abc' | sha256sum`
        assert_eq!(
            compute_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
