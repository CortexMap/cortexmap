use bytes::Bytes;
use derive_builder::Builder;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

#[derive(Builder)]
#[builder(setter(into))]
pub struct ResponseCM {
    pub status: StatusCode,
    #[builder(default)]
    pub headers: HeaderMap,
    pub body: Bytes,
}
