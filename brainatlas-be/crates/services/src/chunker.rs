/// Text chunking utility for breaking down large documents
/// into smaller pieces suitable for embedding generation.

/// Chunk text into overlapping segments
///
/// # Arguments
/// * `text` - The input text to chunk
/// * `chunk_size` - Maximum size of each chunk in characters
/// * `overlap` - Number of characters to overlap between chunks
///
/// # Returns
/// Vector of text chunks
///
/// # Example
/// ```
/// let text = "The quick brown fox jumps over the lazy dog";
/// let chunks = chunk_text(text, 20, 5);
/// // chunks[0]: "The quick brown fox "
/// // chunks[1]: " fox jumps over the "
/// // chunks[2]: " the lazy dog"
/// ```
pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    if chunk_size == 0 {
        return vec![text.to_string()];
    }

    if overlap >= chunk_size {
        // Invalid overlap, treat as no overlap
        return chunk_text(text, chunk_size, 0);
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let text_len = text.len();

    while start < text_len {
        let end = (start + chunk_size).min(text_len);
        let chunk = &text[start..end];
        chunks.push(chunk.to_string());

        if end >= text_len {
            break;
        }

        // Move start forward by (chunk_size - overlap)
        start += chunk_size - overlap;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_text() {
        let chunks = chunk_text("", 100, 20);
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_text_smaller_than_chunk_size() {
        let text = "Hello world";
        let chunks = chunk_text(text, 100, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn test_exact_chunk_size() {
        let text = "12345678901234567890"; // 20 chars
        let chunks = chunk_text(text, 20, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_multiple_chunks_no_overlap() {
        let text = "12345678901234567890ABCDEFGHIJ"; // 30 chars
        let chunks = chunk_text(text, 10, 0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "1234567890");
        assert_eq!(chunks[1], "1234567890");
        assert_eq!(chunks[2], "ABCDEFGHIJ");
    }

    #[test]
    fn test_multiple_chunks_with_overlap() {
        let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"; // 26 chars
        let chunks = chunk_text(text, 10, 3);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], "ABCDEFGHIJ");
        assert_eq!(chunks[1], "HIJKLMNOPQ"); // overlaps HIJ
        assert_eq!(chunks[2], "OPQRSTUVWX"); // overlaps OPQ
        assert_eq!(chunks[3], "VWXYZ");      // overlaps VWX (partial)
    }

    #[test]
    fn test_overlap_larger_than_chunk_size() {
        // Should fallback to no overlap
        let text = "ABCDEFGHIJKLMNOP";
        let chunks = chunk_text(text, 5, 10);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], "ABCDE");
        assert_eq!(chunks[1], "FGHIJ");
    }

    #[test]
    fn test_zero_chunk_size() {
        let text = "Hello world";
        let chunks = chunk_text(text, 0, 5);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn test_real_world_example() {
        let text = "The hippocampus is a crucial structure in the brain. \
                    It plays a vital role in memory formation. \
                    Research shows it's particularly important for spatial navigation.";
        
        let chunks = chunk_text(text, 50, 10);
        
        // Should create overlapping chunks
        assert!(chunks.len() > 1);
        
        // Each chunk should be <= 50 chars (except possibly the last)
        for (i, chunk) in chunks.iter().enumerate() {
            if i < chunks.len() - 1 {
                assert!(chunk.len() <= 50, "Chunk {} is {} chars", i, chunk.len());
            }
        }
        
        // Adjacent chunks should overlap
        if chunks.len() > 1 {
            let overlap_text = &chunks[0][chunks[0].len() - 10..];
            assert!(chunks[1].starts_with(overlap_text));
        }
    }
}
