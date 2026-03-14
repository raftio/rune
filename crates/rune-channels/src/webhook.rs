use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

pub fn compute_signature(secret: &str, data: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC-SHA256 accepts any key size");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify webhook signature using constant-time comparison.
/// Expects header value in form "sha256=hexdigest" or just "hexdigest".
pub fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let expected = compute_signature(secret, body);
    let sig = signature.strip_prefix("sha256=").unwrap_or(signature).trim();
    constant_time_eq(expected.as_bytes(), sig.as_bytes())
}
