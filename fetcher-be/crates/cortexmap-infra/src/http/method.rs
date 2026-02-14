use reqwest::Method;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum MethodCM {
    GET,
    POST,
    PUT,
    PATCH,
    DELETE,
    HEAD,
    OPTIONS,
    CONNECT,
    TRACE,
}

impl From<MethodCM> for Method {
    fn from(value: MethodCM) -> Self {
        match value {
            MethodCM::GET => Method::GET,
            MethodCM::POST => Method::POST,
            MethodCM::PUT => Method::PUT,
            MethodCM::PATCH => Method::PATCH,
            MethodCM::DELETE => Method::DELETE,
            MethodCM::HEAD => Method::HEAD,
            MethodCM::OPTIONS => Method::OPTIONS,
            MethodCM::CONNECT => Method::CONNECT,
            MethodCM::TRACE => Method::TRACE,
        }
    }
}
