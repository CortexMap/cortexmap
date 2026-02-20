/// Text chunking implementation for BrainAtlasServices

pub struct TextChunker;

impl TextChunker {
    pub fn new() -> Self {
        Self
    }
    
    /// Chunk text into overlapping segments
    ///
    /// # Arguments
    /// * `text` - The input text to chunk
    /// * `chunk_size` - Maximum size of each chunk in characters
    /// * `overlap` - Number of characters to overlap between chunks
    ///
    /// # Returns
    /// Vector of text chunks
    pub fn chunk(&self, text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
        if text.is_empty() {
            return Vec::new();
        }

        if chunk_size == 0 {
            return vec![text.to_string()];
        }

        if overlap >= chunk_size {
            // Invalid overlap, treat as no overlap
            return self.chunk(text, chunk_size, 0);
        }

        let mut chunks = Vec::new();
        let mut start = 0;
        let text_len = text.len();

        while start < text_len {
            let mut end = (start + chunk_size).min(text_len);
            
            // Ensure we don't split in the middle of a UTF-8 character
            while end < text_len && !text.is_char_boundary(end) {
                end -= 1;
            }
            
            let chunk = &text[start..end];
            chunks.push(chunk.to_string());

            if end >= text_len {
                break;
            }

            // Move start forward by (chunk_size - overlap), ensuring char boundary
            let mut next_start = start + chunk_size - overlap;
            while next_start < text_len && !text.is_char_boundary(next_start) {
                next_start += 1;
            }
            start = next_start;
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_text() {
        let chunker = TextChunker::new();
        let chunks = chunker.chunk("", 100, 20);
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_text_smaller_than_chunk_size() {
        let chunker = TextChunker::new();
        let text = "Hello world";
        let chunks = chunker.chunk(text, 100, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn test_exact_chunk_size() {
        let chunker = TextChunker::new();
        let text = "12345678901234567890"; // 20 chars
        let chunks = chunker.chunk(text, 20, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_multiple_chunks_no_overlap() {
        let chunker = TextChunker::new();
        let text = "12345678901234567890ABCDEFGHIJ"; // 30 chars
        let chunks = chunker.chunk(text, 10, 0);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "1234567890");
        assert_eq!(chunks[1], "1234567890");
        assert_eq!(chunks[2], "ABCDEFGHIJ");
    }

    #[test]
    fn test_multiple_chunks_with_overlap() {
        let chunker = TextChunker::new();
        let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"; // 26 chars
        let chunks = chunker.chunk(text, 10, 3);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], "ABCDEFGHIJ");
        assert_eq!(chunks[1], "HIJKLMNOPQ"); // overlaps HIJ
        assert_eq!(chunks[2], "OPQRSTUVWX"); // overlaps OPQ
        assert_eq!(chunks[3], "VWXYZ");      // overlaps VWX (partial)
    }

    #[test]
    fn test_overlap_larger_than_chunk_size() {
        let chunker = TextChunker::new();
        // Should fallback to no overlap
        let text = "ABCDEFGHIJKLMNOP";
        let chunks = chunker.chunk(text, 5, 10);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], "ABCDE");
        assert_eq!(chunks[1], "FGHIJ");
    }

    #[test]
    fn test_zero_chunk_size() {
        let chunker = TextChunker::new();
        let text = "Hello world";
        let chunks = chunker.chunk(text, 0, 5);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello world");
    }

    #[test]
    fn test_real_world_example() {
        let chunker = TextChunker::new();
        let text = "The hippocampus is a crucial structure in the brain. \
                    It plays a vital role in memory formation. \
                    Research shows it's particularly important for spatial navigation.";
        
        let chunks = chunker.chunk(text, 50, 10);
        
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
