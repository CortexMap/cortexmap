use crate::fetch::metadata::fetch_metadata;
use crate::fetch::pdf::fetch_pdf;
use crate::{upload, FetchError};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{DatabaseInfra, HttpInfra, InfraContext, S3Infra};

pub async fn fetch<I: HttpInfra + DatabaseInfra + S3Infra + Send + Sync + 'static>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<(), FetchError> {
    let metadata_collection = fetch_metadata(blueprint, ctx.clone()).await?;

    let pdf_streams = futures::future::join_all(
        metadata_collection
            .articles
            .iter()
            .map(|article| {
                let pmc_id = article.pmcid.clone();
                tokio::spawn(fetch_pdf(pmc_id, ctx.clone()))
            }),
    )
    .await
    .into_iter()
    // Note: Currently ignoring errors to avoid failing on first error.
    // Could use a more sophisticated error collection strategy (e.g., tailcall-valid)
    // to capture partial failures while continuing processing.
    .flatten()
    .flatten()
    .collect::<Vec<_>>();

    upload::upload(pdf_streams, metadata_collection, blueprint, ctx).await
}
