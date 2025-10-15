mod http;
mod infra;

use crate::infra::StdInfra;
use cortexmap_infra::InfraContext;
use std::sync::Arc;

pub fn get() -> InfraContext<StdInfra> {
    // TODO: ideally this function should only be called ones
    // but it's easy to make mistakes here,
    // so maybe we could initiate this statically
    // and always return the same instance.
    InfraContext {
        infra: Arc::new(StdInfra::new()),
    }
}
