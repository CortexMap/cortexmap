use crate::FetchError;
use crate::retry::{is_fetch_retryable, with_request_retry};
use cortexmap_core::blueprint::Blueprint;
use cortexmap_infra::{HttpInfra, InfraContext};
use serde::{Deserialize, Serialize};

// NCBI eutils — no API key (3 req/sec/IP rate limit; we throttle accordingly)

const ESEARCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pmc&term={query}&retmode=json&retmax={pageSize}&api_key=e78b8b256471aa0ca51883512a8dfadb6c08";
const ESUMMARY_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi?db=pmc&id={ids}&retmode=json&api_key=e78b8b256471aa0ca51883512a8dfadb6c08";
const EFETCH_URL: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi?db=pmc&id={id}&retmode=xml&api_key=e78b8b256471aa0ca51883512a8dfadb6c08";

/// Strip "PMC" prefix from PMC ID if present
/// NCBI API expects numeric IDs without the "PMC" prefix
fn strip_pmc_prefix(pmc_id: &str) -> &str {
    pmc_id.strip_prefix("PMC").unwrap_or(pmc_id)
}

// ESearch response structures
#[derive(Debug, Deserialize)]
pub struct ESearchResponse {
    pub esearchresult: ESearchResult,
}

#[derive(Debug, Deserialize)]
pub struct ESearchResult {
    pub idlist: Vec<String>,
}

// ESummary response structures
#[derive(Debug, Deserialize)]
pub struct ESummaryResponse {
    pub result: serde_json::Value,
}

// Full article metadata for S3 storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleMetadata {
    pub uid: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub pubdate: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub articleids: Vec<ArticleId>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub issue: Option<String>,
    #[serde(default)]
    pub pages: Option<String>,
    #[serde(default)]
    pub fulljournalname: Option<String>,
    #[serde(default)]
    pub abstract_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub authtype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleId {
    pub idtype: String,
    pub value: String,
}

// Output structure with full metadata
#[derive(Debug)]
pub struct MetadataCollection {
    pub articles: Vec<ArticleWithMetadata>,
}

#[derive(Debug, Clone)]
pub struct ArticleWithMetadata {
    pub pmcid: String,
    pub metadata: ArticleMetadata,
}

pub async fn fetch_metadata<I: HttpInfra>(
    blueprint: &Blueprint,
    ctx: InfraContext<I>,
) -> Result<MetadataCollection, FetchError> {
    // Step 1: ESearch - Get list of PMC IDs
    let search_url = ESEARCH_URL
        .replace("{query}", blueprint.fetcher.query.as_str())
        .replace(
            "{pageSize}",
            blueprint.fetcher.page_size.to_string().as_str(),
        );

    tracing::info!("Making query at: {search_url}");
    let search_resp = {
        let infra = ctx.infra.clone();
        let url = search_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            is_fetch_retryable,
            "ESearch",
        )
        .await?
    };
    let search_result: ESearchResponse = serde_json::from_slice(&search_resp.bytes().await?)?;

    // If no results, return empty list
    if search_result.esearchresult.idlist.is_empty() {
        return Ok(MetadataCollection { articles: vec![] });
    }

    // Step 2: ESummary - Get full metadata for the IDs
    let ids = search_result.esearchresult.idlist.join(",");
    let summary_url = ESUMMARY_URL.replace("{ids}", &ids);

    tracing::info!("Trying to get paper summary at: {search_url}");
    let summary_resp = {
        let infra = ctx.infra.clone();
        let url = summary_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            is_fetch_retryable,
            "ESummary (batch)",
        )
        .await?
    };
    let summary_result: ESummaryResponse = serde_json::from_slice(&summary_resp.bytes().await?)?;

    // Add delay before starting abstract fetches to respect NCBI rate limit
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Extract full metadata for each article
    let mut articles = Vec::new();

    if let Some(result_obj) = summary_result.result.as_object() {
        for (index, id) in search_result.esearchresult.idlist.iter().enumerate() {
            // Add delay between requests to respect NCBI rate limit (max 3 req/sec without API key)
            // Use 500ms delay to be conservative (2 req/sec)
            if index > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if let Some(article) = result_obj.get(id)
                && let Ok(mut metadata) = serde_json::from_value::<ArticleMetadata>(article.clone())
            {
                // Fetch abstract for this article
                match fetch_abstract(id, ctx.clone()).await {
                    Ok(abstract_text) => {
                        metadata.abstract_text = Some(abstract_text);
                    }
                    Err(e) => {
                        tracing::debug!("Failed to fetch abstract for PMC {}: {}", id, e);
                        metadata.abstract_text = None;
                    }
                }

                // Find PMCID in articleids
                if let Some(article_id) =
                    metadata.articleids.iter().find(|aid| aid.idtype == "pmcid")
                {
                    articles.push(ArticleWithMetadata {
                        pmcid: article_id.value.clone(),
                        metadata,
                    });
                }
            }
        }
    }

    Ok(MetadataCollection { articles })
}

/// Fetch summary (metadata without abstract) for a single PMC ID
pub async fn fetch_summary<I: HttpInfra>(
    pmc_uid: &str,
    ctx: InfraContext<I>,
) -> Result<ArticleMetadata, FetchError> {
    // Strip "PMC" prefix for API call (NCBI expects numeric IDs)
    let numeric_id = strip_pmc_prefix(pmc_uid);

    // ESummary for a single ID
    let summary_url = ESUMMARY_URL.replace("{ids}", numeric_id);

    tracing::info!("Fetching summary for PMC {}", pmc_uid);
    let summary_resp = {
        let infra = ctx.infra.clone();
        let url = summary_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            is_fetch_retryable,
            "ESummary (single)",
        )
        .await?
    };
    let summary_result: ESummaryResponse = serde_json::from_slice(&summary_resp.bytes().await?)?;

    // Extract metadata for this specific ID (use numeric_id for lookup)
    if let Some(result_obj) = summary_result.result.as_object()
        && let Some(article) = result_obj.get(numeric_id)
    {
        let metadata = serde_json::from_value::<ArticleMetadata>(article.clone())
            .map_err(FetchError::SerdeError)?;
        return Ok(metadata);
    }

    Err(FetchError::NotFound(format!(
        "Summary not found for PMC {}",
        pmc_uid
    )))
}

// Helper function to fetch abstract from PMC
pub async fn fetch_abstract<I: HttpInfra>(
    pmc_uid: &str,
    ctx: InfraContext<I>,
) -> Result<String, FetchError> {
    // Strip "PMC" prefix for API call (NCBI expects numeric IDs)
    let numeric_id = strip_pmc_prefix(pmc_uid);
    let fetch_url = EFETCH_URL.replace("{id}", numeric_id);

    let resp = {
        let infra = ctx.infra.clone();
        let url = fetch_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            is_fetch_retryable,
            "EFetch (abstract)",
        )
        .await?
    };
    let xml_content = resp.text().await?;

    // Extract abstract from XML using simple text processing
    // The abstract is contained within <abstract>...</abstract> tags
    extract_abstract_from_xml(&xml_content)
}

// Extract abstract text from XML, removing all HTML/XML tags
fn extract_abstract_from_xml(xml: &str) -> Result<String, FetchError> {
    // Find the abstract section
    if let Some(start) = xml.find("<abstract")
        && let Some(abs_start) = xml[start..].find('>')
    {
        let content_start = start + abs_start + 1;
        if let Some(end) = xml[content_start..].find("</abstract>") {
            let abstract_xml = &xml[content_start..content_start + end];

            // Split by common section tags and join with paragraph breaks
            let mut result = abstract_xml.to_string();

            // Add paragraph breaks before common abstract section tags
            result = result.replace("<sec>", "

");
            result = result.replace("</sec>", "");
            result = result.replace("<title>", "**");
            result = result.replace("</title>", ":**

");
            result = result.replace("<p>", "");
            result = result.replace("</p>", "

");

            // Remove all remaining XML/HTML tags
            let cleaned = result
                .split('<')
                .map(|s| {
                    if let Some(pos) = s.find('>') {
                        &s[pos + 1..]
                    } else {
                        s
                    }
                })
                .collect::<Vec<_>>()
                .join("")
                .replace("&#8239;", " ")
                .replace("&#x202f;", " ")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&")
                // Replace multiple newlines with double newlines for cleaner formatting
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("

");

            if !cleaned.trim().is_empty() {
                return Ok(cleaned.trim().to_string());
            }
        }
    }

    Err(FetchError::NotFound(
        "Abstract not found in XML".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_pmc_prefix_removes_only_leading_pmc_prefix() {
        assert_eq!(strip_pmc_prefix("PMC12345"), "12345");
        assert_eq!(strip_pmc_prefix("12345"), "12345");
        assert_eq!(strip_pmc_prefix("PMCID12345"), "ID12345");
    }

    #[test]
    fn test_extract_abstract_from_xml_formats_sections_and_decodes_entities() {
        let xml = "<article><front><abstract><sec><title>Background</title><p>Line &lt;one&gt; &amp; more&#8239;text</p></sec><p>Second paragraph</p></abstract></front></article>";

        let abstract_text = extract_abstract_from_xml(xml).unwrap();

        assert_eq!(
            abstract_text,
            "**Background:**

Line <one> & more text

Second paragraph"
        );
    }

    #[test]
    fn test_extract_abstract_from_xml_errors_when_missing() {
        let xml = "<article><body><p>No abstract here</p></body></article>";

        let error = extract_abstract_from_xml(xml).unwrap_err();
        assert!(
            matches!(error, FetchError::NotFound(message) if message == "Abstract not found in XML")
        );
    }

    #[test]
    fn test_extract_abstract_from_xml_errors_when_only_tags_remain() {
        let xml = "<article><abstract><sec><italic></italic></sec></abstract></article>";

        let error = extract_abstract_from_xml(xml).unwrap_err();
        assert!(
            matches!(error, FetchError::NotFound(message) if message == "Abstract not found in XML")
        );
    }
}