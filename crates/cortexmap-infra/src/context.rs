use std::sync::Arc;

// We can use InfraContext as a wrapper to collection of Infra
// Other approach could have been to hardcode a field for each infra
// like `pub http_infra: Arc<dyn HttpInfra>` or `pub http_infra: Arc<HttpInfra> (generics)`
// But then this will force the upstream code to implement ALL the available infra to write tests.
// For example if we have total of 5 infra from A to E and InfraContext is used in
// some small module which only needs the infra A and B, then we could easily write tests,
// by mocking the A and B infra instead of having to write mocks for A to E.

pub struct InfraContext<I> {
    pub infra: Arc<I>,
}

impl<I> Clone for InfraContext<I> {
    fn clone(&self) -> Self {
        Self {
            infra: self.infra.clone(),
        }
    }
}
