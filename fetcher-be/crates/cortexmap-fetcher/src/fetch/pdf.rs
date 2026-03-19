use crate::FetchError;
use crate::retry::{is_fetch_retryable, with_request_retry};
use bytes::Bytes;
use cortexmap_infra::{HttpInfra, InfraContext};
use futures::stream::Stream;
use std::pin::Pin;

// NCBI PMC OA Service API to discover PDF URLs
const OA_API_URL: &str = "https://www.ncbi.nlm.nih.gov/pmc/utils/oa/oa.fcgi?id={PMCID}&format=pdf";

/// Strip "PMC" prefix from PMC ID if present
/// NCBI API expects "PMC" prefix for OA service
fn ensure_pmc_prefix(pmc_id: &str) -> String {
    if pmc_id.starts_with("PMC") {
        pmc_id.to_string()
    } else {
        format!("PMC{}", pmc_id)
    }
}

pub struct PdfStream {
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send + Sync>>,
    pub pmc_id: String,
}

pub async fn fetch_pdf<I: HttpInfra + Send + Sync + 'static>(
    pmc_id: String,
    ctx: InfraContext<I>,
) -> Result<PdfStream, FetchError> {
    // Ensure PMC prefix (OA service requires "PMC" prefix)
    let pmc_id_with_prefix = ensure_pmc_prefix(&pmc_id);
    
    // Step 1: Query the OA Service API to get the PDF URL
    let oa_url = OA_API_URL.replace("{PMCID}", &pmc_id_with_prefix);
    let oa_response = {
        let infra = ctx.infra.clone();
        let url = oa_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            |e| is_fetch_retryable(e),
            "OA Service",
        )
        .await?
    };
    
    if !oa_response.status().is_success() {
        return Err(FetchError::InvalidPdfSource(format!(
            "Failed to query OA service for {}: HTTP {}",
            pmc_id, oa_response.status()
        )));
    }
    
    // Parse the XML response to extract the PDF URL
    let xml_text = oa_response.text().await?;
    let pdf_url = extract_pdf_url_from_xml(&xml_text, &pmc_id)?;
    
    tracing::info!("Fetching PDF from: {}", pdf_url);
    
    // Step 2: Download the PDF from the FTP/HTTP URL
    let response = {
        let infra = ctx.infra.clone();
        let url = pdf_url.clone();
        with_request_retry(
            || {
                let infra = infra.clone();
                let url = url.clone();
                async move { Ok::<_, FetchError>(infra.get(&url).await?) }
            },
            |e| is_fetch_retryable(e),
            "PDF download",
        )
        .await?
    };
    
    // Check if the response was successful
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::InvalidPdfSource(format!(
            "Failed to fetch PDF for {}: HTTP {}",
            pmc_id, status
        )));
    }
    
    // Verify content type is PDF
    if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("application/pdf") && !content_type_str.contains("application/octet-stream") {
            tracing::warn!(
                "Unexpected content type for {}: {}",
                pmc_id,
                content_type_str
            );
        }
    }

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

/// Extract PDF URL from NCBI OA Service XML response
fn extract_pdf_url_from_xml(xml: &str, pmc_id: &str) -> Result<String, FetchError> {
    // Look for <link format="pdf" href="..."/>
    // Example: <link format="pdf" updated="2017-03-03 06:05:17"
    //           href="ftp://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/8e/71/WJR-9-27.PMC5334499.pdf"/>
    
    // Check if there's an error in the response
    if xml.contains("<error>") {
        let error_msg = xml
            .split("<error>")
            .nth(1)
            .and_then(|s| s.split("</error>").next())
            .unwrap_or("Unknown error from OA service");
        return Err(FetchError::NotFound(format!(
            "OA service error for {}: {}",
            pmc_id, error_msg
        )));
    }
    
    // Check if article is retracted
    if xml.contains("retracted=\"yes\"") {
        return Err(FetchError::InvalidPdfSource(format!(
            "Article {} has been retracted",
            pmc_id
        )));
    }
    
    // Find the PDF link - handle multi-line XML elements
    // Collapse whitespace to handle attributes split across lines
    let normalized = xml.split('\n').map(|line| line.trim()).collect::<Vec<_>>().join(" ");
    
    // Look for <link format="pdf" ... href="..." />
    if let Some(link_start) = normalized.find("<link") {
        let remaining = &normalized[link_start..];
        
        // Split by link tags and process each
        for segment in remaining.split("<link") {
            if segment.contains("format=\"pdf\"") && segment.contains("href=\"") {
                if let Some(href_start) = segment.find("href=\"") {
                    let url_start = href_start + 6; // length of 'href="'
                    if let Some(url_end) = segment[url_start..].find('"') {
                        let mut url = segment[url_start..url_start + url_end].to_string();
                        // Convert FTP URLs to HTTPS
                        if url.starts_with("ftp://ftp.ncbi.nlm.nih.gov/") {
                            url = url.replace("ftp://ftp.ncbi.nlm.nih.gov/", "https://ftp.ncbi.nlm.nih.gov/");
                        }
                        return Ok(url);
                    }
                }
            }
        }
    }
    
    Err(FetchError::NotFound(format!(
        "No PDF available for {} in OA subset",
        pmc_id
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pdf_url_success() {
        let xml = r#"<OA>
  <responseDate>2019-01-28 10:41:16</responseDate>
  <request id="PMC5334499">https://www.ncbi.nlm.nih.gov/utils/oa/oa.fcgi?id=PMC5334499</request>
  <records returned-count="1" total-count="1">
    <record id="PMC5334499" citation="World J Radiol. 2017 Feb 28; 9(2):27-33"
        license="CC BY-NC" retracted="no">
      <link format="tgz" updated="2017-03-17 13:10:45"
        href="ftp://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_package/8e/71/PMC5334499.tar.gz"/>
      <link format="pdf" updated="2017-03-03 06:05:17"
        href="ftp://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/8e/71/WJR-9-27.PMC5334499.pdf"/>
    </record>
  </records>
</OA>"#;
        
        let result = extract_pdf_url_from_xml(xml, "PMC5334499");
        assert!(result.is_ok());
        let url = result.unwrap();
        assert_eq!(url, "https://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/8e/71/WJR-9-27.PMC5334499.pdf");
    }

    #[test]
    fn test_extract_pdf_url_retracted() {
        let xml = r#"<OA>
  <records returned-count="1" total-count="1">
    <record id="PMC1234567" retracted="yes">
      <link format="pdf" href="ftp://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/test.pdf"/>
    </record>
  </records>
</OA>"#;
        
        let result = extract_pdf_url_from_xml(xml, "PMC1234567");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::InvalidPdfSource(_)));
    }

    #[test]
    fn test_extract_pdf_url_not_found() {
        let xml = r#"<OA>
  <records returned-count="0" total-count="0">
  </records>
</OA>"#;
        
        let result = extract_pdf_url_from_xml(xml, "PMC9999999");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::NotFound(_)));
    }

    #[test]
    fn test_extract_pdf_url_error_response() {
        let xml = r#"<OA>
  <error>Invalid PMC ID format</error>
</OA>"#;
        
        let result = extract_pdf_url_from_xml(xml, "INVALID");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FetchError::NotFound(_)));
    }

    #[test]
    fn test_extract_pdf_url_https_already() {
        let xml = r#"<OA>
  <records>
    <record retracted="no">
      <link format="pdf" href="https://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/test.pdf"/>
    </record>
  </records>
</OA>"#;
        
        let result = extract_pdf_url_from_xml(xml, "PMC1234567");
        assert!(result.is_ok());
        let url = result.unwrap();
        assert_eq!(url, "https://ftp.ncbi.nlm.nih.gov/pub/pmc/oa_pdf/test.pdf");
    }
}
