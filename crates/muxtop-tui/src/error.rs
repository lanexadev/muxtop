use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal: {0}")]
    Terminal(#[from] std::io::Error),

    #[error("render: {0}")]
    Render(String),

    #[error("channel: {0}")]
    Channel(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tui_error_send_sync() {
        fn assert_bounds<T: Send + Sync + std::error::Error>() {}
        assert_bounds::<TuiError>();
    }

    #[test]
    fn test_tui_error_display() {
        let variants: Vec<TuiError> = vec![
            TuiError::Terminal(std::io::Error::other("io")),
            TuiError::Render("render".into()),
            TuiError::Channel("closed".into()),
        ];
        for err in &variants {
            assert!(!format!("{err}").is_empty());
        }
    }

    #[test]
    fn test_tui_error_from_io() {
        let io_err = std::io::Error::other("test");
        let tui_err: TuiError = io_err.into();
        assert!(matches!(tui_err, TuiError::Terminal(_)));
    }
}
