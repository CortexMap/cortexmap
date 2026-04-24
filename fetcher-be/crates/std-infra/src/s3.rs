use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Region;
use bytes::Bytes;
use cortexmap_infra::{ContentType, InfraError, S3Infra};
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::OnceCell;

pub struct StdS3Infra {
    client: OnceCell<Client>,
    endpoint: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
    bucket: String,
}

impl StdS3Infra {
    pub fn new(
        endpoint: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
        bucket: &str,
    ) -> Self {
        // Treat `Some("")` as absent. Passing an empty string to
        // `endpoint_url` or `Credentials::from_keys` produces a broken SDK
        // client (dispatch failures, empty-endpoint URLs). The helper here
        // means callers can't accidentally wire an empty env var through.
        let non_empty = |v: Option<&str>| -> Option<String> {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Self {
            client: OnceCell::new(),
            endpoint: non_empty(endpoint),
            access_key: non_empty(access_key),
            secret_key: non_empty(secret_key),
            bucket: bucket.to_owned(),
        }
    }

    async fn get_client(&self) -> &Client {
        self.client
            .get_or_init(|| async {
                match (&self.access_key, &self.secret_key) {
                    (Some(ak), Some(sk)) => {
                        // Static credentials (dev / MinIO)
                        let creds = Credentials::from_keys(ak, sk, None);
                        let mut builder = aws_sdk_s3::config::Builder::new()
                            .behavior_version(BehaviorVersion::latest())
                            .region(Region::new("us-east-1"))
                            .credentials_provider(creds)
                            .force_path_style(true);
                        if let Some(ep) = &self.endpoint {
                            builder = builder.endpoint_url(ep);
                        }
                        Client::from_conf(builder.build())
                    }
                    _ => {
                        // EC2 instance profile / default credential chain
                        let config = aws_config::defaults(BehaviorVersion::latest())
                            .region(
                                aws_config::meta::region::RegionProviderChain::default_provider()
                                    .or_else("us-east-1"),
                            )
                            .load()
                            .await;
                        Client::new(&config)
                    }
                }
            })
            .await
    }
}

#[async_trait::async_trait]
impl S3Infra for StdS3Infra {
    async fn put_s3(
        &self,
        key: &str,
        content_type: ContentType,
        mut content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError> {
        // Collect the entire stream into a single Bytes buffer
        // This avoids streaming checksum issues with HTTP trailers
        let mut buffer = Vec::new();
        while let Some(chunk) = content.next().await {
            buffer.extend_from_slice(&chunk);
        }

        let byte_stream = aws_sdk_s3::primitives::ByteStream::from(buffer);

        let result = self
            .get_client()
            .await
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(byte_stream)
            .content_type(content_type.to_string())
            .send()
            .await;

        result?;

        Ok(())
    }

    async fn get_s3(&self, key: &str) -> Result<String, InfraError> {
        let result = self
            .get_client()
            .await
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;

        let body = result
            .body
            .collect()
            .await
            .map_err(|e| InfraError::S3Error(e.to_string()))?;

        let text = String::from_utf8(body.into_bytes().to_vec())
            .map_err(|e| InfraError::S3Error(e.to_string()))?;

        Ok(text)
    }
}
