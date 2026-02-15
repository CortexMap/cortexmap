use std::sync::OnceLock;
use crate::error::InfraError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    Client,
    config::{Credentials, Region},
};
use services::infra::S3Storage;
use tracing::{error, info};

pub struct BrainAtlasS3 {
    client: OnceLock<Client>,
}

impl BrainAtlasS3 {
    pub fn new() -> Self {
        Self {
            client: OnceLock::new(),
        }
    }

    fn get_client(&self, endpoint: &str, access_key: &str, secret_key: &str) -> &Client {
        self.client.get_or_init(|| {
            // Create new client with custom endpoint and credentials
            let credentials = Credentials::new(
                access_key,
                secret_key,
                None,     // session token
                None,     // expiration
                "static", // provider name
            );

            let config = aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(credentials)
                .endpoint_url(endpoint)
                .region(Region::new("us-east-1")) // Region doesn't matter for self-hosted S3
                .force_path_style(true) // Required for MinIO and other S3-compatible services
                .build();

            let client = Client::from_conf(config);
            client
        })
    }
}

#[async_trait::async_trait]
impl S3Storage for BrainAtlasS3 {
    type Error = InfraError;

    async fn download(&self, key: &str) -> Result<String, Self::Error> {
        // Read credentials from environment
        let endpoint = std::env::var("S3_ENDPOINT").map_err(|_| {
            InfraError::S3("S3_ENDPOINT not set".to_string())
        })?;
        let access_key = std::env::var("S3_ACCESS_KEY").map_err(|_| {
            InfraError::S3("S3_ACCESS_KEY not set".to_string())
        })?;
        let secret_key = std::env::var("S3_SECRET_KEY").map_err(|_| {
            InfraError::S3("S3_SECRET_KEY not set".to_string())
        })?;
        let bucket = std::env::var("S3_BUCKET").map_err(|_| {
            InfraError::S3("S3_BUCKET not set".to_string())
        })?;

        info!("Downloading from S3: s3://{}/{}", bucket, key);

        let client = self.get_client(&endpoint, &access_key, &secret_key);

        // Get the object from S3
        let resp = client
            .get_object()
            .bucket(&bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                error!(
                    "S3 download failed for s3://{}/{}: {}",
                    bucket, key, e
                );
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
            bucket,
            key
        );

        Ok(text)
    }
}
