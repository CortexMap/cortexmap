use crate::error::InfraError;
use aws_config::BehaviorVersion;
use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use services::infra::S3Storage;
use std::error::Error as StdError;
use tokio::sync::OnceCell;
use tracing::{error, info, warn};

pub struct BrainAtlasS3 {
    client: OnceCell<Client>,
    endpoint: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
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
    /// Construct the S3 adapter.
    ///
    /// When both `access_key` and `secret_key` are supplied, a client is built
    /// with static credentials, path-style addressing, and (optionally) a
    /// custom `endpoint` -- the configuration required to talk to MinIO / NAS
    /// or any other S3-compatible store used in local/dev deployments. When
    /// they are absent, the client falls back to the AWS default credential
    /// chain (e.g. the EC2 instance profile) for native AWS S3.
    ///
    /// Empty-string values are treated as absent: passing `Some("")` to
    /// `endpoint_url` or `Credentials::from_keys` produces a broken SDK client
    /// (dispatch failures, empty-endpoint URLs), so the helper below strips
    /// them out and callers can't accidentally wire an empty env var through.
    pub fn new(
        endpoint: Option<&str>,
        access_key: Option<&str>,
        secret_key: Option<&str>,
        bucket: String,
    ) -> Self {
        // Empty bucket cannot produce valid S3 URLs; surface this early.
        assert!(
            !bucket.trim().is_empty(),
            "BrainAtlasS3::new called with empty bucket -- check S3_BUCKET env var"
        );
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
            bucket,
        }
    }

    async fn get_client(&self) -> &Client {
        self.client
            .get_or_init(|| async {
                match (&self.access_key, &self.secret_key) {
                    (Some(ak), Some(sk)) => {
                        // Static credentials (dev / MinIO / NAS). `force_path_style`
                        // is required for endpoint-style S3-compatible servers.
                        let creds = Credentials::new(ak, sk, None, None, "static");
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
                        // EC2 instance profile / default credential chain (native AWS S3).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_keeps_static_endpoint_and_credentials() {
        let s3 = BrainAtlasS3::new(
            Some("http://minio:9000"),
            Some("access"),
            Some("secret"),
            "bucket".to_string(),
        );
        assert_eq!(s3.endpoint.as_deref(), Some("http://minio:9000"));
        assert_eq!(s3.access_key.as_deref(), Some("access"));
        assert_eq!(s3.secret_key.as_deref(), Some("secret"));
        assert_eq!(s3.bucket, "bucket");
    }

    #[test]
    fn new_treats_empty_and_whitespace_as_absent() {
        // `${VAR:-}` from docker-compose yields empty strings; these must not
        // be forwarded to the SDK (they produce dispatch failures / broken
        // endpoint URLs), so they are normalised to `None`.
        let s3 = BrainAtlasS3::new(Some(""), Some("   "), Some(""), "bucket".to_string());
        assert!(s3.endpoint.is_none());
        assert!(s3.access_key.is_none());
        assert!(s3.secret_key.is_none());
    }

    #[test]
    fn new_trims_surrounding_whitespace() {
        let s3 = BrainAtlasS3::new(
            Some("  http://minio:9000  "),
            Some(" access "),
            Some(" secret "),
            "bucket".to_string(),
        );
        assert_eq!(s3.endpoint.as_deref(), Some("http://minio:9000"));
        assert_eq!(s3.access_key.as_deref(), Some("access"));
        assert_eq!(s3.secret_key.as_deref(), Some("secret"));
    }

    #[test]
    fn new_none_falls_back_to_default_chain() {
        // No static creds -> AWS default credential chain (EC2 instance profile).
        let s3 = BrainAtlasS3::new(None, None, None, "bucket".to_string());
        assert!(s3.endpoint.is_none());
        assert!(s3.access_key.is_none());
        assert!(s3.secret_key.is_none());
    }

    #[tokio::test]
    async fn get_client_builds_static_credentials_branch() {
        // Exercises the static-credentials branch: the client is constructed
        // from a custom endpoint + static keys without touching the network
        // or the AWS default credential chain (no I/O is performed until an
        // actual request is sent).
        let s3 = BrainAtlasS3::new(
            Some("http://localhost:9000"),
            Some("access"),
            Some("secret"),
            "bucket".to_string(),
        );
        // Construction succeeds and is cached in the OnceCell.
        let _client = s3.get_client().await;
        assert!(s3.client.get().is_some());
    }

    #[test]
    #[should_panic(expected = "empty bucket")]
    fn new_panics_on_empty_bucket() {
        let _ = BrainAtlasS3::new(None, None, None, "   ".to_string());
    }
}
