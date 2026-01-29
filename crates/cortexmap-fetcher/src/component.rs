use crate::fetch::metadata::{fetch_abstract, fetch_summary, ArticleMetadata};
use crate::fetch::pdf::PdfStream;
use crate::FetchError;
use bytes::Bytes;
use cortexmap_infra::{ComponentType, HttpInfra, InfraContext};
use futures::stream::Stream;
use std::pin::Pin;

/// Result of fetching a single component
pub enum ComponentResult {
    Summary(ArticleMetadata),
    Abstract(String),
    Pdf(PdfStream),
}

impl ComponentResult {
    /// Get the S3 key suffix for this component type
    pub fn key_suffix(&self) -> &'static str {
        match self {
            ComponentResult::Summary(_) => "summary.json",
            ComponentResult::Abstract(_) => "abstract.txt",
            ComponentResult::Pdf(_) => "paper.pdf",
        }
    }
    
    /// Convert component result to a byte stream for S3 upload
    pub fn into_byte_stream(
        self,
    ) -> Result<Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>, FetchError> {
        match self {
            ComponentResult::Summary(metadata) => {
                let json = serde_json::to_vec_pretty(&metadata)
                    .map_err(|e| FetchError::SerdeError(e))?;
                let stream = futures::stream::once(async move { Bytes::from(json) });
                Ok(Box::pin(stream))
            }
            ComponentResult::Abstract(text) => {
                let stream = futures::stream::once(async move { Bytes::from(text) });
                Ok(Box::pin(stream))
            }
            ComponentResult::Pdf(pdf_stream) => {
                use futures::StreamExt;
                let stream = pdf_stream
                    .stream
                    .filter_map(|result| async move { result.ok() });
                Ok(Box::pin(stream))
            }
        }
    }
}

/// Fetch a single component for a given PMC ID
pub async fn fetch_component<I: HttpInfra + Send + Sync + 'static>(
    pmc_id: String,
    component_type: ComponentType,
    ctx: InfraContext<I>,
) -> Result<ComponentResult, FetchError> {
    match component_type {
        ComponentType::Summary => {
            let metadata = fetch_summary(&pmc_id, ctx).await?;
            Ok(ComponentResult::Summary(metadata))
        }
        ComponentType::Abstract => {
            let abstract_text = fetch_abstract(&pmc_id, ctx).await?;
            Ok(ComponentResult::Abstract(abstract_text))
        }
        ComponentType::Pdf => {
            let pdf_stream = crate::fetch::pdf::fetch_pdf(pmc_id, ctx).await?;
            Ok(ComponentResult::Pdf(pdf_stream))
        }
    }
}

/// Determine S3 key for a component
pub fn determine_component_key(
    pmcid: &str,
    component_type: ComponentType,
    prefix: &str,
) -> String {
    let sterilized = sterilize_prefix(prefix);
    let suffix = match component_type {
        ComponentType::Summary => "summary.json",
        ComponentType::Abstract => "abstract.txt",
        ComponentType::Pdf => "paper.pdf",
    };
    format!("{sterilized}/{pmcid}/{suffix}")
}

// Always returns a valid path WITHOUT trailing slash (`/`)
fn sterilize_prefix<T: ToString>(prefix: T) -> String {
    let prefix = prefix.to_string();
    prefix
        .split('/')
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_component_key() {
        let pmc_id = "PMC12345";
        let prefix = "papers";

        assert_eq!(
            determine_component_key(pmc_id, ComponentType::Summary, prefix),
            "papers/PMC12345/summary.json"
        );
        assert_eq!(
            determine_component_key(pmc_id, ComponentType::Abstract, prefix),
            "papers/PMC12345/abstract.txt"
        );
        assert_eq!(
            determine_component_key(pmc_id, ComponentType::Pdf, prefix),
            "papers/PMC12345/paper.pdf"
        );
    }

    #[test]
    fn test_sterilize_prefix() {
        assert_eq!(sterilize_prefix("papers"), "papers");
        assert_eq!(sterilize_prefix("papers/"), "papers");
        assert_eq!(sterilize_prefix("/papers"), "papers");
        assert_eq!(sterilize_prefix("/papers/"), "papers");
        assert_eq!(sterilize_prefix("papers/subset"), "papers/subset");
        assert_eq!(sterilize_prefix("//papers//subset//"), "papers/subset");
    }
}
