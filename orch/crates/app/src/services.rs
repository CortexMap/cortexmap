use domain::{ConfigKey, PendingTask, PollResult, ProcessResult};
use std::error::Error;

#[async_trait::async_trait]
pub trait CompletionOrchestrator: Send + Sync {
    type Error: Error + Send + Sync;

    /// Poll for completed fetch tasks that need LLM processing
    /// Returns tasks that haven't been processed yet
    async fn poll(&self) -> Result<PollResult, Self::Error>;

    /// Process a list of pending tasks
    /// Calls brainatlas API to chunk/embed/summarize each task
    async fn process(&self, tasks: Vec<PendingTask>) -> Result<ProcessResult, Self::Error>;
    
    /// Get a configuration value by key
    async fn get_config(&self, key: ConfigKey) -> Result<Option<String>, Self::Error>;
}

pub trait Services: CompletionOrchestrator<Error = <Self as Services>::Error> {
    type Error: Error + Send + Sync;
}

impl<E, T> Services for T
where
    T: CompletionOrchestrator<Error = E>,
    E: Error + Send + Sync,
{
    type Error = E;
}
