use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const LEASE_TOKEN_BYTES: usize = 32;

pub fn generate_lease_token() -> String {
    let mut bytes = [0u8; LEASE_TOKEN_BYTES];
    getrandom::fill(&mut bytes).expect("system RNG is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn lease_hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

pub fn verify_lease(provided_token: &str, stored_hash: &str) -> bool {
    let computed = lease_hash(provided_token);
    computed.as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_32_random_bytes_base64url() {
        let first = generate_lease_token();
        let second = generate_lease_token();
        assert_ne!(first, second);
        let decoded = URL_SAFE_NO_PAD.decode(&first).unwrap();
        assert_eq!(decoded.len(), LEASE_TOKEN_BYTES);
    }

    #[test]
    fn lease_hash_is_deterministic_sha256() {
        let token = generate_lease_token();
        let hash = lease_hash(&token);
        assert_eq!(hash, lease_hash(&token));
        assert_ne!(hash, lease_hash("different-token"));
    }

    #[test]
    fn verify_lease_accepts_correct_and_rejects_wrong() {
        let token = generate_lease_token();
        let hash = lease_hash(&token);
        assert!(verify_lease(&token, &hash));
        assert!(!verify_lease("wrong-token", &hash));
        assert!(!verify_lease(&token, "wrong-hash"));
    }
}
