use crate::fetch::metadata::MetadataCollection;
use crate::retry::{is_infra_retryable, with_request_retry};
use crate::{FetchError, PdfStream};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{ContentType, DatabaseInfra, InfraContext, NewPaper, S3Infra};
use futures::StreamExt;
use std::collections::HashMap;

pub async fn upload<I: DatabaseInfra + S3Infra + Send + Sync + 'static>(
    streams: Vec<PdfStream>,
    metadata_collection: MetadataCollection,
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<(), FetchError> {
    // Create a map of pmc_id -> metadata for quick lookup
    let metadata_map: HashMap<String, _> = metadata_collection
        .articles
        .into_iter()
        .map(|article| (article.pmcid.clone(), article.metadata))
        .collect();

    for stream in streams {
        // Note: Could check if paper exists before uploading to avoid duplicates.
        // Current behavior: re-upload and update database entry.

        let pdf_key = determine_pdf_key(&stream.pmc_id, blueprint);
        let metadata_key = determine_metadata_key(&stream.pmc_id, blueprint);
        
        // Upload metadata to S3 as JSON with retry
        if let Some(metadata) = metadata_map.get(&stream.pmc_id) {
            let metadata_json = serde_json::to_vec_pretty(metadata)
                .map_err(|e| FetchError::SerdeError(e))?;
            let metadata_bytes = bytes::Bytes::from(metadata_json);

            let upload_result = {
                let infra = ctx.infra.clone();
                let key = metadata_key.clone();
                let data = metadata_bytes;
                with_request_retry(
                    || {
                        let infra = infra.clone();
                        let key = key.clone();
                        let data = data.clone();
                        async move {
                            let stream = futures::stream::once(async move { data });
                            infra.put_s3(&key, ContentType::Json, Box::pin(stream)).await
                        }
                    },
                    |e| is_infra_retryable(e),
                    "S3 metadata upload (legacy)",
                )
                .await
            };

            match upload_result {
                Ok(_) => tracing::info!("Uploaded metadata to S3: {}", metadata_key),
                Err(e) => tracing::warn!("Failed to upload metadata for {}: {:?}", stream.pmc_id, e),
            }
        }
        
        // Upload PDF to S3 (streaming -- cannot easily retry mid-stream)
        let byte_stream = stream
            .stream
            .filter_map(|result| async move { result.ok() });
            
        let res = ctx
            .infra
            .put_s3(&pdf_key, ContentType::Pdf, Box::pin(byte_stream))
            .await;
            
        if let Ok(()) = res {
            // Insert minimal record into PostgreSQL as an index
            ctx.infra
                .insert_paper(NewPaper {
                    pmc_id: stream.pmc_id.clone(),
                    s3_url: pdf_key.clone(),
                    uid: uuid::Uuid::new_v4().to_string(),
                    query: blueprint.fetcher.query.clone(),
                })
                .await
                .map(|paper| {
                    tracing::info!("Uploaded PDF and indexed paper: {:?}", paper);
                })
                .ok();
        }
    }

    Ok(())
}

fn determine_pdf_key(pmcid: &str, blueprint: &Blueprint) -> String {
    let prefix = sterilize_prefix(&blueprint.fetcher.upload_path_prefix);
    format!("{prefix}/{pmcid}/paper.pdf")
}

fn determine_metadata_key(pmcid: &str, blueprint: &Blueprint) -> String {
    let prefix = sterilize_prefix(&blueprint.fetcher.upload_path_prefix);
    format!("{prefix}/{pmcid}/metadata.json")
}

// Always returns a valid path
// WITHOUT tailing slash (`/`)
fn sterilize_prefix<T: ToString>(prefix: T) -> String {
    let prefix = prefix.to_string();
    prefix
        .split('/')
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
