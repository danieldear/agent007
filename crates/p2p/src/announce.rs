use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::identity::PeerIdentity;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AnnouncementKind {
    MemoryUpdated,
    InsightPublished,
    SkillPublished,
    WorkflowPublished,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Announcement {
    pub kind: AnnouncementKind,
    pub artifact_id: String,
    pub summary: String,
    pub author_peer_id: String,
    pub timestamp: DateTime<Utc>,
    pub signature: Option<String>,
}

impl Announcement {
    pub fn new(
        kind: AnnouncementKind,
        artifact_id: impl Into<String>,
        summary: impl Into<String>,
        author_peer_id: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            artifact_id: artifact_id.into(),
            summary: summary.into(),
            author_peer_id: author_peer_id.into(),
            timestamp: Utc::now(),
            signature: None,
        }
    }

    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    MemoryNote,
    RunLearning,
    EvalSummary,
    Custom,
}

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("peer identity is missing signing secret")]
    MissingSigningSecret,
    #[error("envelope author does not match signing identity")]
    AuthorMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CollaborationEnvelope {
    pub envelope_id: String,
    pub artifact_kind: ArtifactKind,
    pub artifact_id: String,
    pub payload_hash: String,
    pub summary: String,
    pub author_peer_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub policy_labels: Vec<String>,
    pub signature: String,
}

impl CollaborationEnvelope {
    pub fn new_signed(
        identity: &PeerIdentity,
        artifact_kind: ArtifactKind,
        artifact_id: impl Into<String>,
        summary: impl Into<String>,
        payload: impl AsRef<[u8]>,
        policy_labels: Vec<String>,
    ) -> Result<Self, EnvelopeError> {
        Self::new_signed_at(
            identity,
            artifact_kind,
            artifact_id,
            summary,
            payload,
            policy_labels,
            Utc::now(),
        )
    }

    pub fn new_signed_at(
        identity: &PeerIdentity,
        artifact_kind: ArtifactKind,
        artifact_id: impl Into<String>,
        summary: impl Into<String>,
        payload: impl AsRef<[u8]>,
        policy_labels: Vec<String>,
        timestamp: DateTime<Utc>,
    ) -> Result<Self, EnvelopeError> {
        let artifact_id = artifact_id.into();
        let summary = summary.into();
        let payload_hash = hex::encode(Sha256::digest(payload.as_ref()));
        let envelope_id = make_envelope_id(
            &identity.peer_id,
            &artifact_id,
            &payload_hash,
            timestamp.timestamp_millis(),
        );
        let mut envelope = Self {
            envelope_id,
            artifact_kind,
            artifact_id,
            payload_hash,
            summary,
            author_peer_id: identity.peer_id.clone(),
            timestamp,
            policy_labels,
            signature: String::new(),
        };
        let signature = identity
            .sign_message(&envelope.canonical_payload())
            .ok_or(EnvelopeError::MissingSigningSecret)?;
        envelope.signature = signature;
        Ok(envelope)
    }

    pub fn canonical_payload(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.envelope_id,
            kind_code(&self.artifact_kind),
            self.artifact_id,
            self.payload_hash,
            self.author_peer_id,
            self.timestamp.timestamp_millis(),
            self.policy_labels.join(",")
        )
    }

    pub fn verify_with_identity(&self, identity: &PeerIdentity) -> Result<bool, EnvelopeError> {
        if identity.peer_id != self.author_peer_id {
            return Err(EnvelopeError::AuthorMismatch);
        }
        Ok(identity.verify_message_signature(&self.canonical_payload(), &self.signature))
    }

    pub fn verify_payload_hash(&self, payload: impl AsRef<[u8]>) -> bool {
        hex::encode(Sha256::digest(payload.as_ref())) == self.payload_hash
    }
}

fn kind_code(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::MemoryNote => "memory-note",
        ArtifactKind::RunLearning => "run-learning",
        ArtifactKind::EvalSummary => "eval-summary",
        ArtifactKind::Custom => "custom",
    }
}

fn make_envelope_id(
    author_peer_id: &str,
    artifact_id: &str,
    payload_hash: &str,
    timestamp_millis: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(author_peer_id.as_bytes());
    hasher.update([0]);
    hasher.update(artifact_id.as_bytes());
    hasher.update([0]);
    hasher.update(payload_hash.as_bytes());
    hasher.update([0]);
    hasher.update(timestamp_millis.to_string().as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest[..24].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_sign_and_verify_round_trip() {
        let peer = PeerIdentity::new("peer-1").with_signing_secret("shared-dev-secret");
        let payload = "top-secret=abc123";
        let envelope = CollaborationEnvelope::new_signed(
            &peer,
            ArtifactKind::MemoryNote,
            "memory:1",
            "updated memory note",
            payload,
            vec!["memory".to_string()],
        )
        .expect("envelope");
        assert!(envelope.verify_with_identity(&peer).unwrap());
        assert!(envelope.verify_payload_hash(payload));
    }

    #[test]
    fn deterministic_for_fixed_timestamp() {
        let peer = PeerIdentity::new("peer-1").with_signing_secret("shared-dev-secret");
        let timestamp = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);

        let first = CollaborationEnvelope::new_signed_at(
            &peer,
            ArtifactKind::RunLearning,
            "run:42",
            "learning update",
            "payload",
            vec!["learning".to_string()],
            timestamp,
        )
        .expect("first envelope");

        let second = CollaborationEnvelope::new_signed_at(
            &peer,
            ArtifactKind::RunLearning,
            "run:42",
            "learning update",
            "payload",
            vec!["learning".to_string()],
            timestamp,
        )
        .expect("second envelope");

        assert_eq!(first.envelope_id, second.envelope_id);
        assert_eq!(first.signature, second.signature);
    }
}
