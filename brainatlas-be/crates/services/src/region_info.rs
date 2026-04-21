use crate::{EnvInfra, Postgres, Query, QueryResult, ServiceError};
use app::BrainRegionInfo;
use domain::BrainRegionEntry;
use std::sync::Arc;
use uuid::Uuid;

pub struct BrainAtlasRegionInfo<I> {
    infra: Arc<I>,
}

impl<I> BrainAtlasRegionInfo<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> BrainRegionInfo for BrainAtlasRegionInfo<I>
where
    E: std::error::Error + Send + Sync + 'static,
    I: Postgres<Error = E> + EnvInfra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn search(&self, id: Uuid) -> Result<Vec<BrainRegionEntry>, Self::Error> {
        let db_uri = self
            .infra
            .get("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        let QueryResult::Region(entries) = self
            .infra
            .execute_query(&db_uri, Query::GetRegionById(id))
            .await
            .map_err(ServiceError::InfraError)?
        else {
            return Err(ServiceError::InvalidResult);
        };

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `BrainAtlasRegionInfo::search`. Only `Postgres + EnvInfra`
    //! are required by the bounds on the trait impl.
    use super::*;
    use crate::{EnvInfra, Postgres, Query, QueryResult};
    use chrono::Utc;
    use domain::BrainRegionEntry;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, thiserror::Error)]
    #[error("mock error: {0}")]
    struct MockErr(&'static str);

    struct MockInfra {
        env: HashMap<String, String>,
        entries: Vec<BrainRegionEntry>,
        /// Captured id passed to `GetRegionById`.
        seen_id: Mutex<Option<Uuid>>,
        fail_with: Option<&'static str>,
        return_wrong_variant: bool,
    }

    impl MockInfra {
        fn new() -> Self {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_string(), "postgres://mock".to_string());
            Self {
                env,
                entries: vec![],
                seen_id: Mutex::new(None),
                fail_with: None,
                return_wrong_variant: false,
            }
        }
    }

    impl EnvInfra for MockInfra {
        type Error = MockErr;
        fn get(&self, key: &str) -> Result<String, Self::Error> {
            self.env.get(key).cloned().ok_or(MockErr("env missing"))
        }
    }

    #[async_trait::async_trait]
    impl Postgres for MockInfra {
        type Error = MockErr;
        async fn execute_query(
            &self,
            _db: &str,
            q: Query,
        ) -> Result<QueryResult, Self::Error> {
            if let Some(m) = self.fail_with {
                return Err(MockErr(m));
            }
            match q {
                Query::GetRegionById(id) => {
                    *self.seen_id.lock().unwrap() = Some(id);
                    if self.return_wrong_variant {
                        Ok(QueryResult::Exists(false))
                    } else {
                        Ok(QueryResult::Region(self.entries.clone()))
                    }
                }
                _ => unreachable!("region_info only issues GetRegionById"),
            }
        }
    }

    fn sample_entry() -> BrainRegionEntry {
        BrainRegionEntry {
            region_id: 1,
            name: "hippocampus".to_string(),
            acronym: "HIP".to_string(),
            summary: "important for memory".to_string(),
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn search_returns_entries_for_id() {
        let mut infra = MockInfra::new();
        infra.entries.push(sample_entry());
        let infra = Arc::new(infra);
        let svc = BrainAtlasRegionInfo::new(infra.clone());
        let id = Uuid::new_v4();
        let got = svc.search(id).await.expect("search succeeds");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].region_id, 1);
        assert_eq!(*infra.seen_id.lock().unwrap(), Some(id), "id propagated");
    }

    #[tokio::test]
    async fn search_returns_empty_vec_when_no_rows() {
        let svc = BrainAtlasRegionInfo::new(Arc::new(MockInfra::new()));
        let got = svc.search(Uuid::new_v4()).await.expect("search succeeds");
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn search_propagates_env_error() {
        let mut infra = MockInfra::new();
        infra.env.remove("DATABASE_URL");
        let svc = BrainAtlasRegionInfo::new(Arc::new(infra));
        let err = svc.search(Uuid::new_v4()).await.expect_err("expected err");
        assert!(matches!(err, ServiceError::InfraError(_)));
    }

    #[tokio::test]
    async fn search_propagates_infra_query_error() {
        let mut infra = MockInfra::new();
        infra.fail_with = Some("bang");
        let svc = BrainAtlasRegionInfo::new(Arc::new(infra));
        let err = svc.search(Uuid::new_v4()).await.expect_err("expected err");
        match err {
            ServiceError::InfraError(e) => assert!(e.to_string().contains("bang")),
            other => panic!("expected InfraError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_returns_invalid_result_on_wrong_variant() {
        let mut infra = MockInfra::new();
        infra.return_wrong_variant = true;
        let svc = BrainAtlasRegionInfo::new(Arc::new(infra));
        let err = svc
            .search(Uuid::new_v4())
            .await
            .expect_err("wrong variant surfaces error");
        assert!(matches!(err, ServiceError::InvalidResult));
    }
}
