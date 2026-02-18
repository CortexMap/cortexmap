use crate::error::InfraError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    Client,
    config::{Credentials, Region},
};
use services::infra::S3Storage;
use std::sync::OnceLock;
use tracing::{error, info};

pub struct BrainAtlasS3 {
    client: OnceLock<Client>,
    endpoint: String,
    access_key: String,
    secret_key: String,
    bucket: String,
}

impl BrainAtlasS3 {
    pub fn new(endpoint: String, access_key: String, secret_key: String, bucket: String) -> Self {
        Self {
            client: OnceLock::new(),
            endpoint,
            access_key,
            secret_key,
            bucket,
        }
    }

    fn get_client(&self) -> &Client {
        self.client.get_or_init(|| {
            let credentials = Credentials::new(
                &self.access_key,
                &self.secret_key,
                None,     // session token
                None,     // expiration
                "static", // provider name
            );

            let config = aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .credentials_provider(credentials)
                .endpoint_url(&self.endpoint)
                .region(Region::new("us-east-1")) // Region doesn't matter for self-hosted S3
                .force_path_style(true) // Required for MinIO and other S3-compatible services
                .build();

            Client::from_conf(config)
        })
    }
}

#[async_trait::async_trait]
impl S3Storage for BrainAtlasS3 {
    type Error = InfraError;

    async fn download(&self, key: &str) -> Result<String, Self::Error> {
        info!("Downloading from S3: s3://{}/{}", self.bucket, key);

        let client = self.get_client();

        // Get the object from S3
        let resp = client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                error!(
                    "S3 download failed for s3://{}/{}: {}",
                    self.bucket, key, e
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
            self.bucket,
            key
        );

        Ok(text)
    }
}
