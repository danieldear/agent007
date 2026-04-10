use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
