/// Newtype wrapper that makes `InteractError` `Sync`-safe.
#[derive(Debug)]
pub struct InteractErr(pub deadpool_diesel::InteractError);

impl std::fmt::Display for InteractErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for InteractErr {}

// SAFETY: InteractError is only ever moved, never shared across threads.
unsafe impl Sync for InteractErr {}

#[derive(Debug, thiserror::Error)]
pub enum InfraError {
    #[error("pool build error: {0}")]
    Build(#[from] deadpool_diesel::postgres::BuildError),
    #[error("pool error: {0}")]
    Pool(#[from] deadpool_diesel::PoolError),
    #[error("diesel error: {0}")]
    Diesel(#[from] diesel::result::Error),
    #[error("interact error: {0}")]
    Interact(#[from] InteractErr),
    #[error("env var not found: {0}")]
    EnvVarNotFound(String),
    #[error("not found")]
    NotFound,
    #[error("invalid data: {0}")]
    InvalidResponse(String),
}

impl From<deadpool_diesel::InteractError> for InfraError {
    fn from(e: deadpool_diesel::InteractError) -> Self {
        InfraError::Interact(InteractErr(e))
    }
}
