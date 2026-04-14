use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("collection failed: {0}")]
    Collection(String),

    #[error("process {pid} not found")]
    ProcessNotFound { pid: u32 },

    #[error("permission denied: {0}")]
    Permission(String),
}
