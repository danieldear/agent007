use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerAdvertisement {
    pub peer_id: String,
    pub endpoint: String,
    pub last_seen: DateTime<Utc>,
}

pub trait DiscoveryProvider {
    fn list_peers(&self) -> Vec<PeerAdvertisement>;
}

#[derive(Debug, Default)]
pub struct LocalDiscovery {
    peers: Vec<PeerAdvertisement>,
}

impl LocalDiscovery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_peer(&mut self, peer_id: impl Into<String>, endpoint: impl Into<String>) {
        self.peers.push(PeerAdvertisement {
            peer_id: peer_id.into(),
            endpoint: endpoint.into(),
            last_seen: Utc::now(),
        });
    }
}

impl DiscoveryProvider for LocalDiscovery {
    fn list_peers(&self) -> Vec<PeerAdvertisement> {
        self.peers.clone()
    }
}
