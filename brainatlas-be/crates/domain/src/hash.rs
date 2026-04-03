/// SHA-256 hashing utility for content deduplication
use sha2::{Digest, Sha256};

/// Compute SHA-256 hash of text content
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash() {
        let content = "test content";
        let hash = compute_hash(content);
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars

        // Same content should produce same hash
        let hash2 = compute_hash(content);
        assert_eq!(hash, hash2);

        // Different content should produce different hash
        let hash3 = compute_hash("different content");
        assert_ne!(hash, hash3);
    }
}
