use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("request error: {0}")]
    Request(String),
    #[error("response error: {0}")]
    Response(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("unknown error: {0}")]
    Unknown(String),
}
