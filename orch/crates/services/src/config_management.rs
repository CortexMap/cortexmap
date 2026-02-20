use crate::{CacheClient, EnvInfra, ServiceError};
use crate::cache_keys::{self, cached_or_fetch, invalidate};
use app::ConfigManagement;
use domain::{ConfigEntry, ConfigEntryUpdate, ConfigKey};
use std::error::Error;
use std::sync::Arc;

pub struct OrchConfigManagement<I> {
    infra: Arc<I>,
}

impl<I> OrchConfigManagement<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<E, I> ConfigManagement for OrchConfigManagement<I>
where
    E: Error + Send + Sync + 'static,
    I: EnvInfra<Error = E> + crate::OrchDatabase<Error = E> + CacheClient<Error = E> + Send + Sync,
{
    type Error = ServiceError<E>;

    async fn get_all_config(&self) -> Result<Vec<ConfigEntry>, Self::Error> {
        let infra = &self.infra;
        cached_or_fetch(
            infra.as_ref(),
            &cache_keys::config_all(),
            cache_keys::TTL_MEDIUM,
            || async {
                let database_url = infra
                    .get_env_var("DATABASE_URL")
                    .map_err(ServiceError::InfraError)?;

                let configs = infra
                    .get_all_config(&database_url)
                    .await
                    .map_err(ServiceError::InfraError)?;

                // Convert OrchConfig to ConfigEntry
                Ok(configs
                    .into_iter()
                    .map(|c| ConfigEntry {
                        key: c.key,
                        value: c.value,
                        description: c.description,
                        updated_at: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                            c.updated_at,
                            chrono::Utc,
                        ),
                    })
                    .collect())
            },
        )
        .await
    }

    async fn update_config(
        &self,
        entries: Vec<ConfigEntryUpdate>,
    ) -> Result<Vec<ConfigEntry>, Self::Error> {
        let database_url = self
            .infra
            .get_env_var("DATABASE_URL")
            .map_err(ServiceError::InfraError)?;

        // Update each config entry
        for entry in &entries {
            // Try to parse as ConfigKey to validate
            let config_key = entry.key.parse::<ConfigKey>().map_err(|_| {
                ServiceError::InvalidConfig {
                    reason: format!("Invalid config key: {}", entry.key),
                }
            })?;

            self.infra
                .update_config(&database_url, config_key, &entry.value)
                .await
                .map_err(ServiceError::InfraError)?;
        }

        // Invalidate config cache after update
        invalidate(self.infra.as_ref(), &cache_keys::config_all()).await;

        // Return all configs after update (will re-fetch from DB and re-populate cache)
        self.get_all_config().await
    }
}
