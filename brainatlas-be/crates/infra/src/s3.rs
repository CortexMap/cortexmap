use crate::error::InfraError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use services::infra::S3Storage;
use std::error::Error as StdError;
use tokio::sync::OnceCell;
use tracing::{error, info, warn};

pub struct BrainAtlasS3 {
    client: OnceCell<Client>,
    bucket: String,
}

/// Build a human-readable chain: "outer: middle: inner".
/// aws-sdk-rust's `Display` for `SdkError` prints only the top-level label
/// ("dispatch failure"); the useful detail lives in `Error::source()`.
fn error_chain<E: StdError + ?Sized>(e: &E) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(cause) = src {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        src = cause.source();
    }
    msg
}

impl BrainAtlasS3 {
    pub fn new(bucket: String) -> Self {
        // Empty bucket cannot produce valid S3 URLs; surface this early.
        assert!(
            !bucket.trim().is_empty(),
            "BrainAtlasS3::new called with empty bucket -- check S3_BUCKET env var"
        );
        Self {
            client: OnceCell::new(),
            bucket,
        }
    }

    async fn get_client(&self) -> &Client {
        self.client
            .get_or_init(|| async {
                let config = aws_config::defaults(BehaviorVersion::latest())
                    .region(
                        aws_config::meta::region::RegionProviderChain::default_provider()
                            .or_else("us-east-1"),
                    )
                    .load()
                    .await;
                Client::new(&config)
            })
            .await
    }
}

#[async_trait::async_trait]
impl S3Storage for BrainAtlasS3 {
    type Error = InfraError;

    /// Download an S3 object as UTF-8 text.
    ///
    /// Returns `Ok(None)` if the key does not exist (`NoSuchKey`) so callers
    /// can treat missing data as a normal, non-fatal condition (e.g. a paper
    /// whose summary component permanently failed to fetch). All other errors
    /// remain `Err`.
    async fn download(&self, key: &str) -> Result<Option<String>, Self::Error> {
        info!("Downloading from S3: s3://{}/{}", self.bucket, key);

        let client = self.get_client().await;

        // Get the object from S3
        let resp = match client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(r) => r,
            Err(SdkError::ServiceError(svc))
                if matches!(svc.err(), GetObjectError::NoSuchKey(_)) =>
            {
                warn!("S3 key missing (NoSuchKey): s3://{}/{}", self.bucket, key);
                return Ok(None);
            }
            Err(e) => {
                let chain = error_chain(&e);
                error!(
                    "S3 download failed for s3://{}/{}: {}",
                    self.bucket, key, chain
                );
                return Err(InfraError::S3(chain));
            }
        };

        // Read the body as bytes
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| {
                let chain = error_chain(&e);
                error!("Failed to read S3 object body: {}", chain);
                InfraError::S3(chain)
            })?
            .into_bytes();

        // Convert to UTF-8 string
        let text = String::from_utf8(bytes.to_vec()).map_err(|e| {
            error!("S3 object is not valid UTF-8: {}", e);
            InfraError::S3(format!("Invalid UTF-8: {}", e))
        })?;

        info!(
            "Successfully downloaded {} bytes from s3://{}/{}",
            text.len(),
            self.bucket,
            key
        );

        Ok(Some(text))
    }
}
