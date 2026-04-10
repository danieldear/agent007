use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TrustLevel {
    Unknown,
    Known,
    Trusted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerIdentity {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub trust: TrustLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signing_secret: Option<String>,
}

impl PeerIdentity {
    pub fn new(peer_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            display_name: None,
            trust: TrustLevel::Unknown,
            signing_secret: None,
        }
    }

    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    pub fn mark_trusted(mut self) -> Self {
        self.trust = TrustLevel::Trusted;
        self
    }

    pub fn with_signing_secret(mut self, secret: impl Into<String>) -> Self {
        self.signing_secret = Some(secret.into());
        self
    }

    pub fn has_signing_secret(&self) -> bool {
        self.signing_secret.is_some()
    }

    pub fn sign_message(&self, message: &str) -> Option<String> {
        let secret = self.signing_secret.as_deref()?;
        Some(compute_signature(&self.peer_id, secret, message))
    }

    pub fn verify_message_signature(&self, message: &str, signature: &str) -> bool {
        let Some(secret) = self.signing_secret.as_deref() else {
            return false;
        };
        compute_signature(&self.peer_id, secret, message) == signature
    }
}

fn compute_signature(peer_id: &str, secret: &str, message: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(peer_id.as_bytes());
    hasher.update([0]);
    hasher.update(secret.as_bytes());
    hasher.update([0]);
    hasher.update(message.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let identity = PeerIdentity::new("peer-1").with_signing_secret("local-dev-secret");
        let message = "artifact|123|payload";
        let signature = identity.sign_message(message).expect("signature");
        assert!(identity.verify_message_signature(message, &signature));
    }

    #[test]
    fn verify_fails_for_tampered_message() {
        let identity = PeerIdentity::new("peer-1").with_signing_secret("local-dev-secret");
        let signature = identity
            .sign_message("artifact|123|payload")
            .expect("signature");
        assert!(!identity.verify_message_signature("artifact|123|payload-tampered", &signature));
    }
}
