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
}
