use crate::FetchError;
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{HttpInfra, InfraContext};
use serde::Deserialize;

const PUBMOD_URL: &str = "https://www.ebi.ac.uk/europepmc/webservices/rest/search?format=json&pageSize={pageSize}&query={query}";

#[derive(Debug, Deserialize)]
pub struct PMCIDs {
    #[serde(rename = "resultList")]
    pub result: SearchResult,
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub result: Vec<SearchData>,
}
#[derive(Debug, Deserialize)]
pub struct SearchData {
    #[serde(default)]
    pub pmcid: Option<String>,
}

pub async fn fetch_metadata<I: HttpInfra>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<PMCIDs, FetchError> {
    let url = PUBMOD_URL
        .replace("{query}", blueprint.query.as_str())
        .replace("{pageSize}", blueprint.page_size.to_string().as_str());
    let resp = ctx.infra.get(&url).await?;
    let body = serde_json::from_slice(&resp.bytes().await?)?;
    Ok(body)
}
