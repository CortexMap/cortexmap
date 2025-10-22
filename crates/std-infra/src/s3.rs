use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::Region;
use bytes::Bytes;
use cortexmap_infra::{ContentType, InfraError, S3Infra};
use futures::{Stream, StreamExt};
use std::pin::Pin;

pub struct StdS3Infra {
    client: Client,
    bucket: String,
}

impl StdS3Infra {
    pub fn new(endpoint: &str, access_key: &str, secret_key: &str, bucket: &str) -> Self {
        let creds = Credentials::from_keys(access_key, secret_key, None);
        let cfg = aws_sdk_s3::config::Builder::new()
            // region doesn't matter rn
            .region(Region::new("us-east-1"))
            .endpoint_url(endpoint)
            .credentials_provider(creds)
            .force_path_style(true) // IMPORTANT for MinIO/most S3 compatibles
            .build();
        let client = Client::from_conf(cfg);
        Self {
            client,
            bucket: bucket.to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl S3Infra for StdS3Infra {
    async fn put_s3(
        &self,
        key: &str,
        content_type: ContentType,
        content: Pin<Box<dyn Stream<Item = Bytes> + Send + Sync>>,
    ) -> Result<(), InfraError> {
        // Convert the stream into http_body_util::StreamBody
        let stream_body = http_body_util::StreamBody::new(
            content.map(|bytes| Ok::<_, std::convert::Infallible>(http_body::Frame::data(bytes))),
        );

        // Convert to AWS SDK types
        let byte_stream = aws_sdk_s3::primitives::ByteStream::from_body_1_x(
            aws_smithy_types::body::SdkBody::from_body_1_x(
                http_body_util::combinators::BoxBody::new(stream_body),
            ),
        );

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(byte_stream)
            .content_type(content_type.to_string())
            .send()
            .await?;

        Ok(())
    }
}
