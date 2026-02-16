use crate::{AppError, Services};
use domain::{BrainRegionEntry, NewEmbedding, NewRegionSummary, RegionMapping, compute_hash};
use futures::future::join_all;
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasApp<S> {
    services: Arc<S>,
}

impl<E, S> BrainAtlasApp<S>
where
    E: std::error::Error + Send + Sync + 'static,
    S: Services<Error = E>,
{
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    pub async fn list(&self) -> Result<Vec<RegionMapping>, AppError<E>> {
        self.services.list().await.map_err(AppError::ServiceError)
    }

    pub async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, AppError<E>> {
        self.services
            .search(id)
            .await
            .map_err(AppError::ServiceError)
    }

    async fn get_region_by_uuid(&self, uuid: Uuid) -> Result<RegionMapping, AppError<E>> {
        // Get the list of all regions and find the one with matching UUID
        let regions = self.services.list().await.map_err(AppError::ServiceError)?;

        regions
            .into_iter()
            .find(|r| r.id == uuid)
            .ok_or_else(|| AppError::NotFound)
    }

    pub async fn process_region(
        &self,
        uuid: Uuid,
        batch_id: Uuid,
        s3_keys: Vec<String>,
    ) -> Result<Uuid, AppError<E>> {
        let region = self.get_region_by_uuid(uuid).await?;
        // 1. Download all S3 files and concatenate
        let mut full_text = String::new();
        for key in &s3_keys {
            let content = self
                .services
                .download(key)
                .await
                .map_err(AppError::ServiceError)?;
            full_text.push_str(&content);
            full_text.push_str("\n\n---\n\n");
        }

        // 2. Compute SHA-256 hash for deduplication
        let content_hash = compute_hash(&full_text);

        // 3. Check if we already processed this exact content
        if let Some(existing) = self
            .services
            .check_content_hash(region.region_id, &content_hash)
            .await
            .map_err(AppError::ServiceError)?
        {
            // Content unchanged, return existing summary ID
            return Ok(existing.summary_id);
        }

        // 4. Chunk the text (infallible operation)
        let chunks = self.services.chunk(&full_text, 1000, 200);

        // 5. Generate embeddings for all chunks in parallel
        let embedding_futures: Vec<_> = chunks
            .iter()
            .map(|chunk| self.services.generate_embedding(chunk))
            .collect();

        let embedding_results = join_all(embedding_futures).await;

        // 6. Collect embeddings and build NewEmbedding structs
        // Note: summary_id will be set by insert_summary_with_embeddings
        let new_embeddings: Vec<_> = embedding_results
            .into_iter()
            .enumerate()
            .map(|(idx, result)| {
                let embedding = result.map_err(AppError::ServiceError)?;
                Ok(NewEmbedding {
                    region_id: region.region_id,
                    summary_id: Uuid::nil(), // Placeholder - will be set in transaction
                    chunk_index: idx as i32,
                    chunk_text: chunks[idx].clone(),
                    embedding,
                })
            })
            .collect::<Result<Vec<_>, AppError<E>>>()?;

        // 7. Generate summary from all chunks
        let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
        let summary_text = self
            .services
            .summarize(chunk_refs)
            .await
            .map_err(AppError::ServiceError)?;

        // 8. Create NewRegionSummary with content hash and batch_id
        let new_summary = NewRegionSummary {
            region_id: region.region_id,
            name: region.name,
            acronym: region.acronym,
            summary: summary_text,
            content_hash,
            batch_id,
        };

        // 9. Insert summary + embeddings in transaction (atomic)
        let summary_id = self
            .services
            .insert_summary_with_embeddings(new_summary, new_embeddings)
            .await
            .map_err(AppError::ServiceError)?;

        Ok(summary_id)
    }

    /// Generate search queries for a brain region using LLM
    pub async fn generate_queries(
        &self,
        region_name: &str,
        count: u32,
    ) -> Result<Vec<String>, AppError<E>> {
        self.services
            .generate_queries(region_name, count)
            .await
            .map_err(AppError::ServiceError)
    }
}
