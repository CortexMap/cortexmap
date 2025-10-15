use crate::FetchError;
use crate::fetch::metadata::fetch_metadata;
use crate::fetch::pdf::fetch_pdf;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{HttpInfra, InfraContext};

pub async fn fetch<I: HttpInfra>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<(), FetchError> {
    let meta = fetch_metadata(blueprint, ctx.clone()).await?;

    let mut pdf_streams = Vec::with_capacity(meta.result.result.len());
    for pmc_id in meta.result.result.into_iter().filter_map(|v| v.pmcid) {
        let stream = fetch_pdf(&pmc_id, ctx.clone()).await?;
        pdf_streams.push(stream);
    }

    Ok(())
}
