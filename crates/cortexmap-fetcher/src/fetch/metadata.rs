use crate::FetchError;
use cortexmap_core::blueprint::Blueprint;
use serde::Deserialize;

const PUBMOD_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term={query}&retmode=json";

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    esearchresult: SearchData,
}

#[derive(Debug, Deserialize)]
pub struct SearchData {
    idlist: Vec<String>,
}

pub fn fetch_metadata(blueprint: &Blueprint) -> Result<SearchResult, FetchError> {
    let _url = PUBMOD_URL.replace("{query}", blueprint.query.as_str());
    todo!()
}
