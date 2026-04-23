use crate::error::InfraError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use services::infra::S3Storage;
use tokio::sync::OnceCell;
use tracing::{error, info};

pub struct BrainAtlasS3 {
    client: OnceCell<Client>,
    bucket: String,
}

impl BrainAtlasS3 {
    pub fn new(bucket: String) -> Self {
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

    async fn download(&self, key: &str) -> Result<String, Self::Error> {
        info!("Downloading from S3: s3://{}/{}", self.bucket, key);

        let client = self.get_client().await;

        // Get the object from S3
        let resp = client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                error!("S3 download failed for s3://{}/{}: {}", self.bucket, key, e);
                InfraError::S3(e.to_string())
            })?;

        // Read the body as bytes
        let bytes = resp
            .body
            .collect()
            .await
            .map_err(|e| {
                error!("Failed to read S3 object body: {}", e);
                InfraError::S3(e.to_string())
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

        Ok(text)
    }
}
