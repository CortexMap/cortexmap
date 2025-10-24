use crate::{FetchError, PdfStream};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{ContentType, DatabaseInfra, InfraContext, NewPaper, S3Infra};
use futures::StreamExt;

pub async fn upload<I: DatabaseInfra + S3Infra + Send + Sync + 'static>(
    streams: Vec<PdfStream>,
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<(), FetchError> {
    for stream in streams {
        // TODO: skip if the paper alr exists in the DB.

        let key = determine_key(&stream.pmc_id, blueprint);
        // Map the stream to skip errors and unwrap Ok values
        let byte_stream = stream
            .stream
            .filter_map(|result| async move { result.ok() });
        let res = ctx
            .infra
            .put_s3(&key, ContentType::Pdf, Box::pin(byte_stream))
            .await;
        if let Ok(()) = res {
            // TODO: Upgrade err handling here.
            ctx.infra
                .insert_paper(NewPaper {
                    pmc_id: stream.pmc_id,
                    s3_key: key,
                    uid: uuid::Uuid::new_v4().to_string(),
                    query: blueprint.fetcher.query.clone(),
                })
                .await
                .map(|paper| {
                    tracing::info!("Uploaded paper: {:?}", paper);
                }).ok();
        }
    }

    Ok(())
}

fn determine_key(pmcid: &str, blueprint: &Blueprint) -> String {
    let prefix = sterilize_prefix(&blueprint.fetcher.upload_path_prefix);
    format!("{prefix}/{pmcid}")
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
