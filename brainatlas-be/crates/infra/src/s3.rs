use crate::error::InfraError;
use services::infra::S3Storage;

pub struct BrainAtlasS3 {
    // TODO: Add aws-sdk-s3 client when implementing
}

impl BrainAtlasS3 {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl S3Storage for BrainAtlasS3 {
    type Error = InfraError;

    async fn download(&self, bucket: &str, key: &str) -> Result<String, Self::Error> {
        // TODO: Implement S3 download
        // 1. Create S3 client from AWS SDK
        // 2. Get object: client.get_object().bucket(bucket).key(key).send().await
        // 3. Read body into bytes
        // 4. Convert to UTF-8 string
        
        tracing::warn!("S3Storage::download not yet implemented - returning empty");
        Err(InfraError::NotImplemented)
    }
}
