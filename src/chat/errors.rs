use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("Failed to access chat cache: {0}")]
    CacheError(#[from] std::io::Error),
    #[error("Failed to (de)serialize chat: {0}")]
    SerdeError(#[from] serde_json::Error),
    #[error("Failed to build chat request: {0}")]
    RequestError(String),
    #[error("Failed to process stream: {0}")]
    StreamError(String),
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Tool error: {0}")]
    ToolError(String),
    #[error("Tokio join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}
