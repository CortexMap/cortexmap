use crate::FetchError;
use bytes::Bytes;
use cortexmap_infra::{HttpInfra, InfraContext};
use futures::stream::Stream;
use std::pin::Pin;

const URL: &str = "https://europepmc.org/backend/ptpmcrender.fcgi?amp;blobtype=pdf&accid={PMCID}";

pub struct PdfStream {
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    pub pmc_id: String,
}

pub async fn fetch_pdf<I: HttpInfra + Send + Sync + 'static>(
    pmc_id: String,
    ctx: InfraContext<I>,
) -> Result<PdfStream, FetchError> {
    let url = URL.replace("{PMCID}", &pmc_id);
    let response = ctx.infra.get(&url).await?;

    let stream = futures::stream::unfold(response, |mut resp| async move {
        match resp.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), resp)),
            Ok(None) => None,
            Err(e) => Some((Err(e), resp)),
        }
    });

    Ok(PdfStream {
        stream: Box::pin(stream),
        pmc_id,
    })
}
