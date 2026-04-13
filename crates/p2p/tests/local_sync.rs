use agent007_p2p::{ArtifactKind, CollaborationEnvelope, P2pError, P2pService, PeerIdentity};
use agent007_sharing::{ShareArtifact, ShareArtifactKind, SharingPolicy};

#[test]
fn local_peer_sync_accepts_allowed_redacted_verified_envelope() {
    let author = PeerIdentity::new("peer-author").with_signing_secret("mesh-shared-secret");
    let receiver_identity =
        PeerIdentity::new("peer-receiver").with_signing_secret("receiver-secret");

    let mut receiver = P2pService::new(receiver_identity);
    receiver.start();
    receiver.set_collaboration_enabled(true);
    receiver.register_peer(author.clone());

    let policy = SharingPolicy::collaboration_default();
    let artifact = ShareArtifact::new(
        ShareArtifactKind::MemoryNote,
        "memory:1",
        "memory note updated",
        "owner=neo token=abc123 status=ok",
        vec!["memory".to_string()],
    );

    let decision = policy.filter_artifact(artifact);
    assert!(decision.allowed);
    assert!(decision.redaction_applied);
    let filtered = decision.artifact.expect("filtered artifact");

    let envelope = CollaborationEnvelope::new_signed(
        &author,
        ArtifactKind::MemoryNote,
        &filtered.artifact_id,
        &filtered.summary,
        filtered.payload.as_bytes(),
        filtered.labels.clone(),
    )
    .expect("envelope");

    receiver
        .ingest_envelope(envelope, filtered.payload.as_bytes())
        .expect("ingest should succeed");

    assert_eq!(receiver.collaboration_envelopes().len(), 1);
}

#[test]
fn local_peer_sync_rejects_tampered_payload_after_policy() {
    let author = PeerIdentity::new("peer-author").with_signing_secret("mesh-shared-secret");
    let receiver_identity =
        PeerIdentity::new("peer-receiver").with_signing_secret("receiver-secret");

    let mut receiver = P2pService::new(receiver_identity);
    receiver.start();
    receiver.set_collaboration_enabled(true);
    receiver.register_peer(author.clone());

    let policy = SharingPolicy::collaboration_default();
    let artifact = ShareArtifact::new(
        ShareArtifactKind::RunLearning,
        "run:1",
        "run learning",
        "status=ok api_key=key-123",
        vec!["learning".to_string()],
    );

    let decision = policy.filter_artifact(artifact);
    assert!(decision.allowed);
    let filtered = decision.artifact.expect("filtered artifact");

    let envelope = CollaborationEnvelope::new_signed(
        &author,
        ArtifactKind::RunLearning,
        &filtered.artifact_id,
        &filtered.summary,
        filtered.payload.as_bytes(),
        filtered.labels.clone(),
    )
    .expect("envelope");

    let err = receiver
        .ingest_envelope(envelope, b"status=ok api_key=tampered")
        .expect_err("tampered payload must be rejected");

    assert!(matches!(err, P2pError::PayloadHashMismatch { .. }));
}

#[test]
fn local_peer_sync_rejects_replayed_envelope() {
    let author = PeerIdentity::new("peer-author").with_signing_secret("mesh-shared-secret");
    let receiver_identity =
        PeerIdentity::new("peer-receiver").with_signing_secret("receiver-secret");

    let mut receiver = P2pService::new(receiver_identity);
    receiver.start();
    receiver.set_collaboration_enabled(true);
    receiver.register_peer(author.clone());

    let payload = "status=ok";
    let envelope = CollaborationEnvelope::new_signed(
        &author,
        ArtifactKind::RunLearning,
        "run:replay",
        "run learning",
        payload,
        vec!["learning".to_string()],
    )
    .expect("envelope");

    receiver
        .ingest_envelope(envelope.clone(), payload.as_bytes())
        .expect("first ingest should succeed");

    let err = receiver
        .ingest_envelope(envelope, payload.as_bytes())
        .expect_err("duplicate envelope must be rejected");

    assert!(matches!(err, P2pError::ReplayDetected { .. }));
}
