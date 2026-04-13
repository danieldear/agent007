use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::announce::{Announcement, CollaborationEnvelope, EnvelopeError};
use crate::identity::PeerIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollaborationConfig {
    pub enabled: bool,
    pub verify_payload_hash: bool,
}

impl Default for CollaborationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            verify_payload_hash: true,
        }
    }
}

impl CollaborationConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("AGENT007_COLLAB_SYNC_ENABLED", false),
            verify_payload_hash: env_bool("AGENT007_COLLAB_VERIFY_PAYLOAD_HASH", true),
        }
    }
}

#[derive(Debug, Error)]
pub enum P2pError {
    #[error("service is not running")]
    NotRunning,
    #[error("collaboration sync is disabled")]
    CollaborationDisabled,
    #[error("author peer is not registered: {peer_id}")]
    UnknownPeer { peer_id: String },
    #[error("author peer does not have signing material: {peer_id}")]
    MissingPeerSigningSecret { peer_id: String },
    #[error("invalid envelope signature for {envelope_id}")]
    InvalidEnvelopeSignature { envelope_id: String },
    #[error("payload hash mismatch for {envelope_id}")]
    PayloadHashMismatch { envelope_id: String },
    #[error("replayed collaboration envelope detected: {envelope_id}")]
    ReplayDetected { envelope_id: String },
    #[error("envelope validation failed: {0}")]
    EnvelopeValidation(#[from] EnvelopeError),
}

#[derive(Debug)]
pub struct P2pService {
    identity: PeerIdentity,
    running: bool,
    announcements: Vec<Announcement>,
    collaboration: CollaborationConfig,
    known_peers: HashMap<String, PeerIdentity>,
    collaboration_envelopes: Vec<CollaborationEnvelope>,
    seen_envelope_ids: HashSet<String>,
}

impl P2pService {
    pub fn new(identity: PeerIdentity) -> Self {
        Self::with_collaboration(identity, CollaborationConfig::default())
    }

    pub fn with_collaboration(identity: PeerIdentity, collaboration: CollaborationConfig) -> Self {
        Self {
            identity,
            running: false,
            announcements: Vec::new(),
            collaboration,
            known_peers: HashMap::new(),
            collaboration_envelopes: Vec::new(),
            seen_envelope_ids: HashSet::new(),
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn identity(&self) -> &PeerIdentity {
        &self.identity
    }

    pub fn collaboration_config(&self) -> &CollaborationConfig {
        &self.collaboration
    }

    pub fn set_collaboration_enabled(&mut self, enabled: bool) {
        self.collaboration.enabled = enabled;
    }

    pub fn register_peer(&mut self, peer: PeerIdentity) {
        self.known_peers.insert(peer.peer_id.clone(), peer);
    }

    pub fn known_peer(&self, peer_id: &str) -> Option<&PeerIdentity> {
        self.known_peers.get(peer_id)
    }

    pub fn publish(&mut self, announcement: Announcement) -> Result<(), P2pError> {
        if !self.running {
            return Err(P2pError::NotRunning);
        }
        self.announcements.push(announcement);
        Ok(())
    }

    pub fn announcements(&self) -> &[Announcement] {
        &self.announcements
    }

    pub fn ingest_envelope(
        &mut self,
        envelope: CollaborationEnvelope,
        payload: impl AsRef<[u8]>,
    ) -> Result<(), P2pError> {
        if !self.running {
            return Err(P2pError::NotRunning);
        }
        if !self.collaboration.enabled {
            return Err(P2pError::CollaborationDisabled);
        }

        let Some(author) = self.known_peers.get(&envelope.author_peer_id) else {
            return Err(P2pError::UnknownPeer {
                peer_id: envelope.author_peer_id.clone(),
            });
        };
        if !author.has_signing_secret() {
            return Err(P2pError::MissingPeerSigningSecret {
                peer_id: author.peer_id.clone(),
            });
        }

        let signature_valid = envelope.verify_with_identity(author)?;
        if !signature_valid {
            return Err(P2pError::InvalidEnvelopeSignature {
                envelope_id: envelope.envelope_id.clone(),
            });
        }
        if self.collaboration.verify_payload_hash && !envelope.verify_payload_hash(payload) {
            return Err(P2pError::PayloadHashMismatch {
                envelope_id: envelope.envelope_id.clone(),
            });
        }
        if self.seen_envelope_ids.contains(&envelope.envelope_id) {
            return Err(P2pError::ReplayDetected {
                envelope_id: envelope.envelope_id.clone(),
            });
        }

        self.seen_envelope_ids.insert(envelope.envelope_id.clone());
        self.collaboration_envelopes.push(envelope);
        Ok(())
    }

    pub fn collaboration_envelopes(&self) -> &[CollaborationEnvelope] {
        &self.collaboration_envelopes
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::announce::{AnnouncementKind, ArtifactKind};

    #[test]
    fn service_requires_start_before_publish() {
        let identity = PeerIdentity::new("peer-1");
        let mut service = P2pService::new(identity);
        let announcement = Announcement::new(
            AnnouncementKind::MemoryUpdated,
            "artifact-1",
            "memory changed",
            "peer-1",
        );

        let err = service.publish(announcement.clone()).unwrap_err();
        assert!(matches!(err, P2pError::NotRunning));

        service.start();
        service.publish(announcement).unwrap();
        assert_eq!(service.announcements().len(), 1);
    }

    #[test]
    fn ingest_requires_enabled_collaboration() {
        let local = PeerIdentity::new("peer-local").with_signing_secret("local-secret");
        let remote = PeerIdentity::new("peer-remote").with_signing_secret("shared-secret");
        let payload = "hello";
        let envelope = CollaborationEnvelope::new_signed(
            &remote,
            ArtifactKind::MemoryNote,
            "memory:1",
            "update",
            payload,
            vec!["memory".to_string()],
        )
        .expect("envelope");

        let mut service = P2pService::new(local);
        service.start();
        service.register_peer(remote);

        let err = service.ingest_envelope(envelope, payload).unwrap_err();
        assert!(matches!(err, P2pError::CollaborationDisabled));
    }

    #[test]
    fn ingest_rejects_hash_mismatch() {
        let local = PeerIdentity::new("peer-local").with_signing_secret("local-secret");
        let remote = PeerIdentity::new("peer-remote").with_signing_secret("shared-secret");
        let envelope = CollaborationEnvelope::new_signed(
            &remote,
            ArtifactKind::MemoryNote,
            "memory:1",
            "update",
            "payload-ok",
            vec!["memory".to_string()],
        )
        .expect("envelope");

        let mut service = P2pService::new(local);
        service.start();
        service.set_collaboration_enabled(true);
        service.register_peer(remote);

        let err = service
            .ingest_envelope(envelope, "payload-tampered")
            .unwrap_err();
        assert!(matches!(err, P2pError::PayloadHashMismatch { .. }));
    }

    #[test]
    fn ingest_rejects_unknown_peer_as_allowlist_violation() {
        let local = PeerIdentity::new("peer-local").with_signing_secret("local-secret");
        let remote = PeerIdentity::new("peer-remote").with_signing_secret("shared-secret");
        let payload = "payload-ok";
        let envelope = CollaborationEnvelope::new_signed(
            &remote,
            ArtifactKind::MemoryNote,
            "memory:1",
            "update",
            payload,
            vec!["memory".to_string()],
        )
        .expect("envelope");

        let mut service = P2pService::new(local);
        service.start();
        service.set_collaboration_enabled(true);

        let err = service.ingest_envelope(envelope, payload).unwrap_err();
        assert!(matches!(err, P2pError::UnknownPeer { .. }));
    }

    #[test]
    fn ingest_rejects_replayed_envelope() {
        let local = PeerIdentity::new("peer-local").with_signing_secret("local-secret");
        let remote = PeerIdentity::new("peer-remote").with_signing_secret("shared-secret");
        let payload = "payload-ok";
        let envelope = CollaborationEnvelope::new_signed(
            &remote,
            ArtifactKind::MemoryNote,
            "memory:1",
            "update",
            payload,
            vec!["memory".to_string()],
        )
        .expect("envelope");

        let mut service = P2pService::new(local);
        service.start();
        service.set_collaboration_enabled(true);
        service.register_peer(remote);

        service.ingest_envelope(envelope.clone(), payload).unwrap();
        let err = service.ingest_envelope(envelope, payload).unwrap_err();
        assert!(matches!(err, P2pError::ReplayDetected { .. }));
    }

    #[test]
    fn ingest_accepts_valid_envelope() {
        let local = PeerIdentity::new("peer-local").with_signing_secret("local-secret");
        let remote = PeerIdentity::new("peer-remote").with_signing_secret("shared-secret");
        let payload = "payload-ok";
        let envelope = CollaborationEnvelope::new_signed(
            &remote,
            ArtifactKind::MemoryNote,
            "memory:1",
            "update",
            payload,
            vec!["memory".to_string()],
        )
        .expect("envelope");

        let mut service = P2pService::new(local);
        service.start();
        service.set_collaboration_enabled(true);
        service.register_peer(remote);
        service.ingest_envelope(envelope, payload).unwrap();
        assert_eq!(service.collaboration_envelopes().len(), 1);
    }
}
