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
            ComponentResult::Summary(_) => "summary.md",
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
                let markdown = metadata_to_markdown(&metadata);
                let stream = futures::stream::once(async move { Bytes::from(markdown) });
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

/// Convert ArticleMetadata to Markdown format
fn metadata_to_markdown(metadata: &ArticleMetadata) -> String {
    let mut md = String::new();
    
    // Title
    if let Some(ref title) = metadata.title {
        md.push_str(&format!("# {}\n\n", title));
    }
    
    // Metadata section
    md.push_str("## Metadata\n\n");
    md.push_str(&format!("- **PMC ID**: PMC{}\n", metadata.uid));
    
    // DOI (from articleids)
    if let Some(doi_id) = metadata.articleids.iter().find(|id| id.idtype == "doi") {
        md.push_str(&format!("- **DOI**: {}\n", doi_id.value));
    }
    
    // Journal
    if let Some(ref journal) = metadata.fulljournalname {
        md.push_str(&format!("- **Journal**: {}\n", journal));
    } else if let Some(ref source) = metadata.source {
        md.push_str(&format!("- **Journal**: {}\n", source));
    }
    
    // Publication Date
    if let Some(ref pubdate) = metadata.pubdate {
        md.push_str(&format!("- **Publication Date**: {}\n", pubdate));
    }
    
    // Volume/Issue/Pages
    if let Some(ref volume) = metadata.volume {
        md.push_str(&format!("- **Volume**: {}\n", volume));
    }
    if let Some(ref issue) = metadata.issue {
        md.push_str(&format!("- **Issue**: {}\n", issue));
    }
    if let Some(ref pages) = metadata.pages {
        md.push_str(&format!("- **Pages**: {}\n", pages));
    }
    
    // Authors
    if !metadata.authors.is_empty() {
        md.push_str("- **Authors**: ");
        let author_names: Vec<String> = metadata.authors.iter()
            .map(|a| a.name.clone())
            .collect();
        md.push_str(&author_names.join(", "));
        md.push_str("\n");
    }
    
    md.push_str("\n");
    
    // Abstract (if available)
    if let Some(ref abstract_text) = metadata.abstract_text {
        md.push_str("## Abstract\n\n");
        md.push_str(abstract_text);
        md.push_str("\n\n");
    }
    
    // Article IDs (PMC, PMID, DOI, etc.)
    if !metadata.articleids.is_empty() {
        md.push_str("## Article IDs\n\n");
        for article_id in &metadata.articleids {
            md.push_str(&format!("- **{}**: {}\n", article_id.idtype.to_uppercase(), article_id.value));
        }
        md.push_str("\n");
    }
    
    md
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
        ComponentType::Summary => "summary.md",
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
            "papers/PMC12345/summary.md"
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
