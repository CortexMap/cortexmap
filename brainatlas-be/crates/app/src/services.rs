use domain::RegionMapping;
use std::error::Error;

#[async_trait::async_trait]
pub trait ListBrainRegions: Send + Sync {
    type Error: Error + Send + Sync;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error>;
}

pub trait Services: ListBrainRegions<Error = <Self as Services>::Error> {
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: ListBrainRegions<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
