use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal: {0}")]
    Terminal(#[from] std::io::Error),

    #[error("render: {0}")]
    Render(String),
}
