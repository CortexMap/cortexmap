use cortexmap_infra::papers;
use cortexmap_infra::{DatabaseInfra, InfraError, NewPaper, Paper};
use diesel::PgConnection;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};

pub type DbPool = Pool<ConnectionManager<PgConnection>>;

pub struct StdDatabaseInfra {
    pool: DbPool,
}

impl StdDatabaseInfra {
    pub fn new(database_url: &str) -> Result<Self, InfraError> {
        let manager = ConnectionManager::<PgConnection>::new(database_url);
        let pool = Pool::builder()
            .max_size(10)
            .build(manager)?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl DatabaseInfra for StdDatabaseInfra {
    async fn insert_paper(&self, new_paper: NewPaper) -> Result<Paper, InfraError> {
        let pool = self.pool.clone();

        Ok(tokio::task::spawn_blocking(move || {
            let mut conn = pool.get()?;

            Ok::<_, InfraError>(
                diesel::insert_into(papers::table)
                    .values(&new_paper)
                    .get_result(&mut conn)?,
            )
        })
        .await??)
    }
}
