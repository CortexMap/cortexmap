use crate::{EnvInfra, Postgres, Query, QueryResult, ServiceError};
use app::ListBrainRegions;
use domain::RegionMapping;
use std::sync::Arc;

pub struct BrainAtlasListBrainRegions<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasListBrainRegions<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> ListBrainRegions for BrainAtlasListBrainRegions<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Postgres<Error = E> + EnvInfra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn list(&self) -> Result<Vec<RegionMapping>, Self::Error> {
        let db_uri = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let result = self
            .infra
            .execute_query(&db_uri, Query::ListRegions)
            .await
            .map_err(ServiceError::InfraError)?;

        let QueryResult::Regions(regions) = result else {
            return Err(ServiceError::InvalidResult);
        };

        Ok(regions)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `BrainAtlasListBrainRegions::list`. The infra fake only
    //! needs to satisfy the `Postgres + EnvInfra` bounds declared on the
    //! implementation; no other sub-traits are invoked on this code path.
    use super::*;
    use crate::{EnvInfra, Postgres, Query, QueryResult};
    use domain::RegionMapping;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(&'static str);

    struct MockInfra {
        env: HashMap<String, String>,
        regions: Vec<RegionMapping>,
        /// If Some, `execute_query` returns this error.
        fail_with: Option<&'static str>,
        /// Captured query-targets in order for assertion.
        calls: Mutex<Vec<String>>,
        /// When true, return a non-`Regions` variant to exercise the
        /// `InvalidResult` branch.
        return_wrong_variant: bool,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                env,
                regions: vec![],
                fail_with: None,
                calls: Mutex::new(vec![]),
                return_wrong_variant: false,
            }
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.env.get(key).cloned().ok_or(MockErr("missing env var"))
        }
    }

    #[async_trait::async_trait]
    impl Postgres for MockInfra {
        type Error = MockErr;
        async fn execute_query(&self, db: &str, q: Query) -> Result<QueryResult, Self::Error> {
            self.calls.lock().unwrap().push(db.to_string());
            if let Some(m) = self.fail_with {
                return Err(MockErr(m));
            }
            match q {
                Query::ListRegions => {
                    if self.return_wrong_variant {
                        Ok(QueryResult::Exists(true))
                    } else {
                        Ok(QueryResult::Regions(self.regions.clone()))
                    }
                }
                _ => unreachable!("only ListRegions is used by this service"),
            }
        }
    }

    #[tokio::test]
    async fn list_returns_regions_on_happy_path() {
        let mut infra = MockInfra::new();
        infra
            .regions
            .push(RegionMapping::new(1, "Region One".to_string()));
        infra
            .regions
            .push(RegionMapping::new(2, "Region Two".to_string()));
        let infra = Arc::new(infra);
        let svc = BrainAtlasListBrainRegions::new(infra.clone());

        let got = svc.list().await.expect("list succeeds");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].region_id, 1);
        assert_eq!(got[1].region_id, 2);

        // DATABASE_URL propagated to the infra call.
        let calls = infra.calls.lock().unwrap();
        assert_eq!(calls.as_slice(), &["postgres://mock".to_string()]);
    }

    #[tokio::test]
    async fn list_propagates_env_error_as_infra_error() {
        let mut infra = MockInfra::new();
        infra.env.remove("DATABASE_URL");
        let infra = Arc::new(infra);
        let svc = BrainAtlasListBrainRegions::new(infra);

        let err = svc.list().await.expect_err("should error when env missing");
        match err {
            ServiceError::InfraError(_) => {}
            other => panic!("expected InfraError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_propagates_infra_query_error() {
        let mut infra = MockInfra::new();
        infra.fail_with = Some("db down");
        let svc = BrainAtlasListBrainRegions::new(Arc::new(infra));

        let err = svc.list().await.expect_err("should error when query fails");
        match err {
            ServiceError::InfraError(e) => assert!(e.to_string().contains("db down")),
            other => panic!("expected InfraError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_returns_invalid_result_on_wrong_variant() {
        let mut infra = MockInfra::new();
        infra.return_wrong_variant = true;
        let svc = BrainAtlasListBrainRegions::new(Arc::new(infra));

        let err = svc.list().await.expect_err("wrong variant surfaces error");
        assert!(matches!(err, ServiceError::InvalidResult));
    }

    #[tokio::test]
    async fn list_returns_empty_vec_when_no_regions() {
        let infra = Arc::new(MockInfra::new());
        let svc = BrainAtlasListBrainRegions::new(infra);
        let got = svc.list().await.expect("list succeeds");
        assert!(got.is_empty());
    }
}
