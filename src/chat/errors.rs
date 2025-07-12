use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("Failed to access chat cache: {0}")]
    Cache(#[from] std::io::Error),
    #[error("Failed to (de)serialize chat: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Failed to build chat request: {0}")]
    Request(String),
    #[error("Failed to process stream: {0}")]
    Stream(String),
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Tool error: {0}")]
    Tool(String),
    #[error("Tokio join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}
