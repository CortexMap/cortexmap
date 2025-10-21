use crate::FetchError;
use crate::fetch::metadata::fetch_metadata;
use crate::fetch::pdf::fetch_pdf;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{HttpInfra, InfraContext};

pub async fn fetch<I: HttpInfra + Send + Sync + 'static>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<(), FetchError> {
    let meta = fetch_metadata(blueprint, ctx.clone()).await?;

    let _pdf_streams = futures::future::join_all(
        meta.result
            .result
            .into_iter()
            .filter_map(|v| v.pmcid)
            .map(|pmc_id| tokio::spawn(fetch_pdf(pmc_id, ctx.clone()))),
    )
    .await
    .into_iter()
        // Ignoring all errors for now.
        // We need more powerful type to
        // catch list of errors.
        // TODO: maybe we could use `tailcall-valid`
        // for this.
    .flatten()
    .flatten()
    .collect::<Vec<_>>();

    Ok(())
}
