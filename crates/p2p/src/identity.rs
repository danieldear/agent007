use serde::{Deserialize, Serialize};

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
}

impl PeerIdentity {
    pub fn new(peer_id: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            display_name: None,
            trust: TrustLevel::Unknown,
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
}
