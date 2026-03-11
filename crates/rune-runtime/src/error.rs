use thiserror::Error;
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replica_not_found_displays_uuid() {
        let id = Uuid::new_v4();
        let err = RuntimeError::ReplicaNotFound(id);
        assert!(err.to_string().contains(&id.to_string()));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn backend_error_displays_message() {
        let err = RuntimeError::Backend("connection refused".into());
        assert!(err.to_string().contains("Backend error"));
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn timeout_error_displays_uuid() {
        let id = Uuid::new_v4();
        let err = RuntimeError::Timeout(id);
        assert!(err.to_string().contains("Timeout"));
        assert!(err.to_string().contains(&id.to_string()));
    }

    #[test]
    fn spec_error_displays_message() {
        let err = RuntimeError::Spec("invalid spec.yaml".into());
        assert!(err.to_string().contains("Spec error"));
        assert!(err.to_string().contains("invalid spec.yaml"));
    }

    #[test]
    fn engine_error_displays_message() {
        let err = RuntimeError::Engine("model policy denied".into());
        assert!(err.to_string().contains("Engine error"));
        assert!(err.to_string().contains("model policy denied"));
    }

    #[test]
    fn tool_execution_error_displays_message() {
        let err = RuntimeError::ToolExecution("tool timed out".into());
        assert!(err.to_string().contains("Tool execution error"));
        assert!(err.to_string().contains("tool timed out"));
    }

    #[test]
    fn io_error_from_std_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: RuntimeError = io_err.into();
        assert!(err.to_string().contains("IO error"));
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Replica {0} not found")]
    ReplicaNotFound(Uuid),

    #[error("Backend error: {0}")]
    Backend(String),

    #[error("Timeout waiting for replica {0}")]
    Timeout(Uuid),

    #[error("Spec error: {0}")]
    Spec(String),

    #[error("Engine error: {0}")]
    Engine(String),

    #[error("Tool execution error: {0}")]
    ToolExecution(String),

    #[error("Storage error: {0}")]
    Storage(#[from] rune_storage::StorageError),

    #[error("Runtime store error: {0}")]
    RuntimeStore(#[from] rune_storage::RuntimeStoreError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
