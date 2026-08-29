use anyhow::{Context, Result};
use git2::{Repository, Signature, Oid, DiffOptions, DiffFormat};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub mode: SwarmMode,
    pub admins: Vec<String>,
    pub members: Vec<String>,
    pub devices: Vec<DeviceCertificate>,
    pub invited: Vec<String>,
    pub banned: Vec<String>,
    pub crls: Vec<Vec<u8>>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SwarmMode {
    OneToOne = 0,
    AdminInvitesOnly = 1,
    InvitesOnly = 2,
    Public = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCertificate {
    pub device_id: String,
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub author: String,
    pub author_device: String,
    pub message_type: MessageType,
    pub payload: MessagePayload,
    pub timestamp: i64,
    pub parent_commit: Option<String>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    Text,
    File,
    Image,
    Video,
    Audio,
    VoiceNote,
    System,
    Vote,
    Member,
    ProfileUpdate,
    FileTransfer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessagePayload {
    Text { body: String },
    File { name: String, size: u64, sha3sum: String, tid: String },
    Image { width: u32, height: u32, sha3sum: String, tid: String },
    Video { width: u32, height: u32, duration: u32, sha3sum: String, tid: String },
    Audio { duration: u32, sha3sum: String, tid: String },
    VoiceNote { duration: u32, sha3sum: String, tid: String },
    System { action: String },
    Vote { vote_type: String, target: String },
    Member { action: String, member: String },
    ProfileUpdate { title: Option<String>, description: Option<String>, avatar: Option<String> },
    FileTransfer { file_id: String, offset: u64, data: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationProfile {
    pub title: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<String>,
}

pub struct SwarmManager {
    repo_path: PathBuf,
    repo: Arc<RwLock<Option<Repository>>>,
    identity: String,
    conversations: Arc<RwLock<HashMap<String, Conversation>>>,
}

impl SwarmManager {
    pub async fn new(base_path: PathBuf, identity: String) -> Result<Self> {
        let repo_path = base_path.join("swarms");
        tokio::fs::create_dir_all(&repo_path).await?;

        Ok(Self {
            repo_path,
            repo: Arc::new(RwLock::new(None)),
            identity,
            conversations: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        // Check if repo exists, if not create it
        let repo_path = self.repo_path.clone();
        let repo = tokio::task::spawn_blocking(move || {
            if repo_path.join(".git").exists() {
                Repository::open(&repo_path)
            } else {
                Repository::init(&repo_path)
            }
        }).await??;

        *self.repo.write().await = Some(repo);
        info!("Swarm repository initialized at {:?}", self.repo_path);
        Ok(())
    }

    pub async fn create_conversation(
        &self,
        mode: SwarmMode,
        peer_identity: Option<String>,
    ) -> Result<Conversation> {
        let conversation_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let mut conversation = Conversation {
            id: conversation_id.clone(),
            mode,
            admins: vec![self.identity.clone()],
            members: vec![self.identity.clone()],
            devices: vec![],
            invited: vec![],
            banned: vec![],
            crls: vec![],
            created_at: chrono::Utc::now().timestamp(),
            updated_at: now,
        };

        if let Some(peer) = peer_identity {
            conversation.invited.push(peer);
        }

        // Create Git repo for this conversation
        self.create_conversation_repo(&conversation).await?;

        self.conversations.write().await.insert(conversation_id.clone(), conversation.clone());

        info!("Created conversation {} with mode {:?}", conversation_id, mode);
        Ok(conversation)
    }

    fn create_conversation_repo(&self, conversation: &Conversation) -> Result<()> {
        let repo_path = self.repo_path.join(&conversation.id);
        std::fs::create_dir_all(&repo_path)?;

        let repo = Repository::init(&repo_path)?;

        // Create initial commit with conversation metadata
        let sig = Signature::now(&self.identity, "vchat@vchat.local")?;

        // Create directory structure
        let dirs = vec!["admins", "members", "devices", "invited", "banned", "crls", "votes/ban/members", "votes/unban/members"];
        for dir in dirs {
            std::fs::create_dir_all(repo_path.join(dir))?;
        }

        // Write initial files
        let admin_file = repo_path.join("admins").join(&self.identity);
        std::fs::write(&admin_file, "")?;

        let member_file = repo_path.join("members").join(&self.identity);
        std::fs::write(&member_file, "")?;

        // Write conversation mode
        let initial_commit_msg = serde_json::json!({
            "type": "initial",
            "mode": conversation.mode as u8,
        }).to_string();

        // Create initial commit
        let mut index = repo.index()?;
        index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &initial_commit_msg,
            &tree,
            &[],
        )?;

        info!("Created conversation repo for {}", conversation.id);
        Ok(())
    }

    pub async fn add_message(&self, message: Message) -> Result<()> {
        let conversation = self.conversations.read().await
            .get(&message.conversation_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

        let repo_path = self.repo_path.join(&message.conversation_id);
        let repo = Repository::open(&repo_path)?;

        // Write message as a file
        let msg_path = repo_path.join("messages").join(format!("{}.json", message.id));
        std::fs::create_dir_all(msg_path.parent().unwrap())?;
        let msg_data = serde_json::to_vec_pretty(&message)?;
        std::fs::write(&msg_path, &msg_data)?;

        // Commit the message
        let sig = Signature::now(&self.identity, "vchat@vchat.local")?;
        let mut index = repo.index()?;
        index.add_path(Path::new("messages").join(format!("{}.json", message.id)))?;
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        let parent_commit = if let Some(parent) = message.parent_commit {
            Some(repo.find_commit(Oid::from_str(&parent)?)?)
        } else {
            None
        };

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        repo.commit(
            Some("HEAD"),
            &Signature::now(&self.identity, "vchat@vchat.local")?,
            &Signature::now(&self.identity, "vchat@vchat.local")?,
            &serde_json::to_string(&message)?,
            &repo.find_tree(repo.index()?.write_tree()?)?,
            &parents,
        )?;

        info!("Added message {} to conversation {}", message.id, message.conversation_id);
        Ok(())
    }

    pub async fn sync_conversation(&self, conversation_id: &str, peer_identity: &str) -> Result<()> {
        // In a real implementation, this would connect to the peer and pull/push
        // For now, we just log
        debug!("Syncing conversation {} with peer {}", conversation_id, peer_identity);
        Ok(())
    }

    pub async fn invite_to_conversation(
        &self,
        conversation_id: &str,
        peer_identity: &str,
    ) -> Result<()> {
        let mut conversations = self.conversations.write().await;
        let conversation = conversations.get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

        if !conversation.invited.contains(&peer_identity.to_string()) {
            conversation.invited.push(peer_identity.to_string());
            conversation.updated_at = chrono::Utc::now().timestamp();
        }

        Ok(())
    }

    pub async fn accept_invite(&self, conversation_id: &str) -> Result<()> {
        let mut conversations = self.conversations.write().await;
        let conversation = conversations.get_mut(conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

        // Move from invited to members
        if let Some(pos) = conversation.invited.iter().position(|x| x == &self.identity) {
            conversation.invited.remove(pos);
            conversation.members.push(self.identity.clone());
            conversation.updated_at = chrono::Utc::now().timestamp();
        }

        Ok(())
    }

    pub async fn get_conversation(&self, conversation_id: &str) -> Option<Conversation> {
        self.conversations.read().await.get(conversation_id).cloned()
    }

    pub async fn list_conversations(&self) -> Vec<Conversation> {
        self.conversations.read().await.values().cloned().collect()
    }
}

// Git OID helper
use std::str::FromStr;
impl FromStr for git2::Oid {
    type Err = git2::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        git2::Oid::from_str(s)
    }
}