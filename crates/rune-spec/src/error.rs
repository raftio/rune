use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("IO error reading {0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),

    #[error("Parse error in {0}: {1}")]
    Parse(String, String),

    #[error("Validation error: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io;

    #[test]
    fn io_error_display_contains_path_and_message() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "no such file");
        let err = SpecError::Io(PathBuf::from("/some/path.yaml"), io_err);
        let msg = err.to_string();
        assert!(msg.contains("IO error"));
        assert!(msg.contains("/some/path.yaml"));
        assert!(msg.contains("no such file"));
    }

    #[test]
    fn parse_error_display_contains_source_and_message() {
        let err = SpecError::Parse("runefile.yaml".into(), "unexpected token at line 3".into());
        let msg = err.to_string();
        assert!(msg.contains("Parse error"));
        assert!(msg.contains("runefile.yaml"));
        assert!(msg.contains("unexpected token at line 3"));
    }

    #[test]
    fn validation_error_display_contains_message() {
        let err = SpecError::Validation("project name cannot be empty".into());
        let msg = err.to_string();
        assert!(msg.contains("Validation error"));
        assert!(msg.contains("project name cannot be empty"));
    }

    #[test]
    fn io_error_exposes_source_chain() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let err = SpecError::Io(PathBuf::from("/secret"), io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn parse_error_has_no_source() {
        let err = SpecError::Parse("f.yaml".into(), "bad".into());
        assert!(err.source().is_none());
    }

    #[test]
    fn validation_error_has_no_source() {
        let err = SpecError::Validation("bad input".into());
        assert!(err.source().is_none());
    }
}
