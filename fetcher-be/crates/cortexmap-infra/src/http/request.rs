use crate::http::{HttpError, MethodCM};
use bytes::Bytes;
use reqwest::header::HeaderMap;
use reqwest::{Request, RequestBuilder};
use url::Url;

pub struct RequestCM {
    pub url: Url,
    pub method: MethodCM,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
}

impl RequestCM {
    pub fn into_reqwest(self) -> Result<Request, HttpError> {
        let request = Request::new(self.method.into(), self.url);
        let mut builder =
            RequestBuilder::from_parts(Default::default(), request).headers(self.headers);
        if let Some(body) = self.body {
            builder = builder.body(body);
        }
        Ok(builder.build()?)
    }
}
