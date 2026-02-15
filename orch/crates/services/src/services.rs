use crate::completion_watcher::CompletionWatcher;
use crate::{Infra, ServiceError};
use app::CompletionOrchestrator;
use domain::{ConfigKey, PendingTask, PollResult, ProcessResult};
use std::error::Error;
use std::sync::Arc;

pub struct OrchServices<I> {
    completion_watcher: CompletionWatcher<I>,
}

impl<I: Infra> OrchServices<I> {
    pub fn new(infra: Arc<I>) -> Self {
        let completion_watcher = CompletionWatcher::new(infra);
        Self { completion_watcher }
    }
}

#[async_trait::async_trait]
impl<E, I> CompletionOrchestrator for OrchServices<I>
where
    E: Error + Send + Sync + 'static,
    I: Infra<Error = E>,
{
    type Error = ServiceError<E>;

    async fn poll(&self) -> Result<PollResult, Self::Error> {
        self.completion_watcher.poll().await
    }

    async fn process(&self, tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error> {
        self.completion_watcher.process(tasks).await
    }

    async fn get_config(&self, key: ConfigKey) -> Result<Option<String>, Self::Error> {
        self.completion_watcher.get_config(key).await
    }
}
