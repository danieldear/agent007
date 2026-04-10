use thiserror::Error;

use crate::announce::Announcement;
use crate::identity::PeerIdentity;

#[derive(Debug, Error)]
pub enum P2pError {
    #[error("service is not running")]
    NotRunning,
}

#[derive(Debug)]
pub struct P2pService {
    identity: PeerIdentity,
    running: bool,
    announcements: Vec<Announcement>,
}

impl P2pService {
    pub fn new(identity: PeerIdentity) -> Self {
        Self {
            identity,
            running: false,
            announcements: Vec::new(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::announce::AnnouncementKind;

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
}
