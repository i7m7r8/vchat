use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

use crate::webrtc::SharedWebRTCState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConferenceMember {
    pub peer_onion: String,
    pub peer_id: String,
    pub audio_enabled: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshLink {
    pub peer_a: String,
    pub peer_b: String,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conference {
    pub conference_id: String,
    pub host_onion: String,
    pub members: Vec<ConferenceMember>,
    pub mesh_links: Vec<MeshLink>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct ConferenceManager {
    conferences: Arc<RwLock<HashMap<String, Conference>>>,
    webrtc_state: SharedWebRTCState,
}

impl ConferenceManager {
    pub fn new(webrtc_state: SharedWebRTCState) -> Self {
        Self {
            conferences: Arc::new(RwLock::new(HashMap::new())),
            webrtc_state,
        }
    }

    pub async fn create_conference(&self, host_onion: String) -> Result<Conference> {
        let conference_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let conference = Conference {
            conference_id: conference_id.clone(),
            host_onion: host_onion.clone(),
            members: vec![ConferenceMember {
                peer_onion: host_onion,
                peer_id: Uuid::new_v4().to_string(),
                audio_enabled: true,
                video_enabled: false,
                screen_sharing: false,
                connected: true,
            }],
            mesh_links: vec![],
            created_at: now,
            updated_at: now,
        };

        self.conferences
            .write()
            .await
            .insert(conference_id.clone(), conference.clone());

        info!("Created conference {}", conference_id);
        Ok(conference)
    }

    pub async fn join_conference(
        &self,
        conference_id: &str,
        peer_onion: String,
        peer_id: String,
    ) -> Result<Conference> {
        let mut conferences = self.conferences.write().await;
        let conference = conferences
            .get_mut(conference_id)
            .ok_or_else(|| anyhow::anyhow!("Conference not found"))?;

        // Check if already a member
        if conference.members.iter().any(|m| m.peer_onion == peer_onion) {
            bail!("Peer already in conference");
        }

        conference.members.push(ConferenceMember {
            peer_onion: peer_onion.clone(),
            peer_id,
            audio_enabled: true,
            video_enabled: false,
            screen_sharing: false,
            connected: false, // Will connect mesh links
        });
        conference.updated_at = chrono::Utc::now().timestamp();

        // Create mesh links to all existing members
        for member in &conference.members {
            if member.peer_onion != peer_onion {
                let call_id = Uuid::new_v4().to_string();
                conference.mesh_links.push(MeshLink {
                    peer_a: peer_onion.clone(),
                    peer_b: member.peer_onion.clone(),
                    call_id: call_id.clone(),
                });
            }
        }

        info!("Peer {} joined conference {}", peer_onion, conference_id);
        Ok(conference.clone())
    }

    pub async fn leave_conference(&self, conference_id: &str, peer_onion: &str) -> Result<()> {
        let mut conferences = self.conferences.write().await;
        let conference = conferences
            .get_mut(conference_id)
            .ok_or_else(|| anyhow::anyhow!("Conference not found"))?;

        // Remove member
        conference.members.retain(|m| m.peer_onion != peer_onion);
        // Remove mesh links involving this peer
        conference.mesh_links.retain(|l| l.peer_a != peer_onion && l.peer_b != peer_onion);
        conference.updated_at = chrono::Utc::now().timestamp();

        // If host left and no members, destroy conference
        if conference.members.is_empty() {
            conferences.remove(conference_id);
        }

        info!("Peer {} left conference {}", peer_onion, conference_id);
        Ok(())
    }

    pub async fn get_conference(&self, conference_id: &str) -> Option<Conference> {
        self.conferences.read().await.get(conference_id).cloned()
    }

    pub async fn list_conferences(&self) -> Vec<Conference> {
        self.conferences.read().await.values().cloned().collect()
    }

    pub async fn set_member_audio(&self, conference_id: &str, peer_onion: &str, enabled: bool) -> Result<()> {
        let mut conferences = self.conferences.write().await;
        if let Some(conf) = conferences.get_mut(conference_id) {
            if let Some(m) = conf.members.iter_mut().find(|m| m.peer_onion == peer_onion) {
                m.audio_enabled = enabled;
                conf.updated_at = chrono::Utc::now().timestamp();
            }
        }
        Ok(())
    }

    pub async fn set_member_video(&self, conference_id: &str, peer_onion: &str, enabled: bool) -> Result<()> {
        let mut conferences = self.conferences.write().await;
        if let Some(conf) = conferences.get_mut(conference_id) {
            if let Some(m) = conf.members.iter_mut().find(|m| m.peer_onion == peer_onion) {
                m.video_enabled = enabled;
                conf.updated_at = chrono::Utc::now().timestamp();
            }
        }
        Ok(())
    }

    pub async fn set_member_screen_share(&self, conference_id: &str, peer_onion: &str, enabled: bool) -> Result<()> {
        let mut conferences = self.conferences.write().await;
        if let Some(conf) = conferences.get_mut(conference_id) {
            if let Some(m) = conf.members.iter_mut().find(|m| m.peer_onion == peer_onion) {
                m.screen_sharing = enabled;
                conf.updated_at = chrono::Utc::now().timestamp();
            }
        }
        Ok(())
    }

    /// Get all mesh link call IDs for a peer in a conference
    pub async fn get_peer_call_ids(&self, conference_id: &str, peer_onion: &str) -> Vec<String> {
        let conferences = self.conferences.read().await;
        conferences
            .get(conference_id)
            .map(|c| {
                c.mesh_links
                    .iter()
                    .filter(|l| l.peer_a == peer_onion || l.peer_b == peer_onion)
                    .map(|l| l.call_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Forward a media frame from one peer to all other peers in the conference
    pub async fn forward_media(&self, conference_id: &str, from_peer: &str, frame_id: u32, data: &[u8]) -> Result<()> {
        let call_ids = self.get_peer_call_ids(conference_id, from_peer).await;
        
        for call_id in call_ids {
            // Forward through the existing media session
            let _ = crate::webrtc::send_media_frame(
                self.webrtc_state.clone(),
                &call_id,
                frame_id,
                data,
            ).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webrtc::create_state;

    #[tokio::test]
    async fn test_create_conference() {
        let webrtc = create_state();
        let mgr = ConferenceManager::new(webrtc);
        let conf = mgr.create_conference("alice.onion".to_string()).await.unwrap();
        assert_eq!(conf.members.len(), 1);
        assert_eq!(conf.host_onion, "alice.onion");
    }

    #[tokio::test]
    async fn test_join_leave_conference() {
        let webrtc = create_state();
        let mgr = ConferenceManager::new(webrtc);
        let conf = mgr.create_conference("alice.onion".to_string()).await.unwrap();
        
        let joined = mgr.join_conference(&conf.conference_id, "bob.onion".to_string(), "bob-id".to_string()).await.unwrap();
        assert_eq!(joined.members.len(), 2);
        assert_eq!(joined.mesh_links.len(), 1);
        
        mgr.leave_conference(&conf.conference_id, "bob.onion").await.unwrap();
        let after = mgr.get_conference(&conf.conference_id).await.unwrap();
        assert_eq!(after.members.len(), 1);
    }

    #[tokio::test]
    async fn test_mesh_links_created() {
        let webrtc = create_state();
        let mgr = ConferenceManager::new(webrtc);
        let conf = mgr.create_conference("alice.onion".to_string()).await.unwrap();
        
        let _ = mgr.join_conference(&conf.conference_id, "bob.onion".to_string(), "bob-id".to_string()).await.unwrap();
        let _ = mgr.join_conference(&conf.conference_id, "carol.onion".to_string(), "carol-id".to_string()).await.unwrap();
        
        let conf = mgr.get_conference(&conf.conference_id).await.unwrap();
        // In a 3-person mesh: 3 links (A-B, A-C, B-C)
        assert_eq!(conf.mesh_links.len(), 3);
    }
}