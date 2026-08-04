use thiserror::Error;

use crate::cluster_engine::ClusterError;
use crate::container_engine::EngineError;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("collection failed: {0}")]
    Collection(String),

    #[error("process {pid} not found")]
    ProcessNotFound { pid: u32 },

    #[error("permission denied: {0}")]
    Permission(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("channel closed")]
    ChannelClosed,

    #[error("container engine: {0}")]
    Engine(#[from] EngineError),

    #[error("cluster engine: {0}")]
    Cluster(#[from] ClusterError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let variants: Vec<CoreError> = vec![
            CoreError::Collection("test".into()),
            CoreError::ProcessNotFound { pid: 42 },
            CoreError::Permission("denied".into()),
            CoreError::Io(std::io::Error::other("io err")),
            CoreError::ChannelClosed,
            CoreError::Engine(EngineError::ConnectFailed("refused".into())),
            CoreError::Cluster(ClusterError::KubeconfigNotFound),
        ];
        for err in &variants {
            let msg = format!("{err}");
            assert!(!msg.is_empty(), "Display for {err:?} was empty");
        }
    }

    #[test]
    fn core_error_from_engine_error() {
        let eng = EngineError::ConnectFailed("refused".into());
        let core: CoreError = eng.into();
        assert!(matches!(
            core,
            CoreError::Engine(EngineError::ConnectFailed(_))
        ));
    }

    #[test]
    fn core_error_from_cluster_error() {
        let cluster = ClusterError::Unreachable("dns failed".into());
        let core: CoreError = cluster.into();
        assert!(matches!(
            core,
            CoreError::Cluster(ClusterError::Unreachable(_))
        ));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let core_err: CoreError = io_err.into();
        assert!(matches!(core_err, CoreError::Io(_)));
    }

    #[test]
    fn test_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CoreError>();
    }

    #[test]
    fn test_error_is_std_error() {
        let err = CoreError::ChannelClosed;
        let _boxed: Box<dyn std::error::Error> = Box::new(err);
    }
}
