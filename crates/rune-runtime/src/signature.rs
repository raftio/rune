//! Signature and provenance verification for agent images.
//! MVP: stub; full implementation would use sigstore/cosign.

use crate::error::RuntimeError;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_signature_ref_always_passes() {
        let result = verify_image_signature("docker.io/my/agent:latest", "sha256:abc", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn empty_string_signature_ref_passes() {
        let result = verify_image_signature("docker.io/my/agent:latest", "sha256:abc", Some("")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn present_signature_ref_passes_as_stub() {
        // MVP stub: always Ok when signature_ref is present
        let result = verify_image_signature(
            "docker.io/my/agent:latest",
            "sha256:abc123",
            Some("cosign://sig-ref"),
        )
        .await;
        assert!(result.is_ok());
    }
}

/// Verify image signature/provenance before deployment.
/// Returns Ok(()) if verification passes or is skipped (no signature_ref).
pub async fn verify_image_signature(
    _image_ref: &str,
    _image_digest: &str,
    _signature_ref: Option<&str>,
) -> Result<(), RuntimeError> {
    if _signature_ref.is_none() || _signature_ref == Some("") {
        return Ok(());
    }
    // TODO: Integrate sigstore/cosign verification
    // For MVP we allow deployment without verification when signature_ref is set
    // but we don't actively verify yet.
    tracing::info!(
        image_ref = _image_ref,
        signature_ref = _signature_ref.unwrap(),
        "Signature verification not yet implemented; allowing deployment"
    );
    Ok(())
}
