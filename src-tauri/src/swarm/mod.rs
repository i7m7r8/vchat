use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::crypto::keys::Ed25519KeyPair;

pub const SWARM_DIR: &str = "swarms";
pub const ADMIN_DIR: &str = "admins";
pub const MEMBER_DIR: &str = "members";
pub const DEVICE_DIR: &str = "devices";
pub const INVITED_DIR: &str = "invited";
pub const BANNED_DIR: &str = "banned";
pub const CRL_DIR: &str = "crls";
pub const MESSAGE_DIR: &str = "messages";
pub const FILE_DIR: &str = "files";
pub const PROFILE_FILE: &str = "profile.vcf";
pub const CONVERSATION_FILE: &str = "conversation.json";
pub const COMMITS_DIR: &str = "commits";

/// A Jami-style directory tree. Each key is a project-relative path
/// (e.g. `members/alice`, `messages/<uuid>.json`, `profile.vcf`) and its
/// value is the file content. Commits carry an immutable full-tree snapshot.
pub type SwarmTree = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum CommitType {
    Initial = 0,
    Text = 1,
    File = 2,
    MemberAdd = 3,
    MemberRemove = 4,
    Ban = 5,
    Unban = 6,
    DeviceAdd = 7,
    ProfileUpdate = 8,
    Vote = 9,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConversationMode {
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
pub struct Conversation {
    pub id: String,
    pub mode: ConversationMode,
    pub admins: Vec<String>,
    pub members: Vec<String>,
    pub devices: Vec<DeviceCertificate>,
    pub invited: Vec<String>,
    pub banned: Vec<String>,
    pub crls: Vec<Vec<u8>>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationProfile {
    pub title: Option<String>,
    pub description: Option<String>,
    pub avatar: Option<String>,
}

/// An append-only unit of conversation history. The `id` is the SHA-256 of the
/// canonical body and the `signature` is an Ed25519 signature (by `device_cert`)
/// over that same canonical body, so any modification breaks both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub author: String,
    pub device_cert: Vec<u8>,
    pub timestamp: i64,
    pub payload_type: CommitType,
    pub message: String,
    pub tree: SwarmTree,
    pub signature: Vec<u8>,
}

fn build_signing_input(
    parent_ids: &[String],
    author: &str,
    device_cert: &[u8],
    timestamp: i64,
    payload_type: CommitType,
    tree: &SwarmTree,
    message: &str,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"vchat-swarm-v1");
    for p in parent_ids {
        buf.extend_from_slice(p.as_bytes());
        buf.push(0xff);
    }
    buf.extend_from_slice(b"|author|");
    buf.extend_from_slice(author.as_bytes());
    buf.extend_from_slice(b"|dev|");
    buf.extend_from_slice(device_cert);
    buf.extend_from_slice(&timestamp.to_le_bytes());
    buf.push(payload_type as u8);
    buf.extend_from_slice(b"|tree|");
    for (k, v) in tree {
        buf.extend_from_slice(k.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
        buf.extend_from_slice(v);
    }
    buf.extend_from_slice(b"|msg|");
    buf.extend_from_slice(message.as_bytes());
    buf
}

impl Commit {
    pub fn new(
        parent_ids: Vec<String>,
        author: String,
        device_cert: Vec<u8>,
        timestamp: i64,
        payload_type: CommitType,
        message: String,
        tree: SwarmTree,
        signing_key: &Ed25519KeyPair,
    ) -> Commit {
        let input = build_signing_input(
            &parent_ids,
            &author,
            &device_cert,
            timestamp,
            payload_type,
            &tree,
            &message,
        );
        let id = hex::encode(Sha256::digest(&input));
        let signature = signing_key.sign(&input).to_vec();
        Commit {
            id,
            parent_ids,
            author,
            device_cert,
            timestamp,
            payload_type,
            message,
            tree,
            signature,
        }
    }

    /// Recomputes the canonical body and re-derives `id` + verifies the
    /// Ed25519 signature against `device_cert`. Fails on any tampering.
    pub fn verify(&self) -> bool {
        let input = build_signing_input(
            &self.parent_ids,
            &self.author,
            &self.device_cert,
            self.timestamp,
            self.payload_type,
            &self.tree,
            &self.message,
        );
        if hex::encode(Sha256::digest(&input)) != self.id {
            return false;
        }
        if self.signature.len() != 64 || self.device_cert.len() != 32 {
            return false;
        }
        let sig: [u8; 64] = match self.signature.as_slice().try_into() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let pk: [u8; 32] = match self.device_cert.as_slice().try_into() {
            Ok(k) => k,
            Err(_) => return false,
        };
        let vk = match VerifyingKey::from_bytes(&pk) {
            Ok(v) => v,
            Err(_) => return false,
        };
        vk.verify_strict(&input, &Signature::from_bytes(&sig)).is_ok()
    }
}

/// The per-conversation append-only swarm store. Persists to
/// `root/conversation.json` and `root/commits/<id>.json` and is resumable.
pub struct SwarmStore {
    conversation: Conversation,
    commits: BTreeMap<String, Commit>,
    root: PathBuf,
    identity: String,
    device_id: String,
    signing_key: Ed25519KeyPair,
}

fn tree_dirs() -> &'static [&'static str] {
    &[
        ADMIN_DIR,
        MEMBER_DIR,
        DEVICE_DIR,
        INVITED_DIR,
        BANNED_DIR,
        CRL_DIR,
        MESSAGE_DIR,
        FILE_DIR,
    ]
}

fn seed_tree(identity: &str, device_id: &str) -> SwarmTree {
    let mut tree = SwarmTree::new();
    tree.insert(format!("{ADMIN_DIR}/{identity}"), Vec::new());
    tree.insert(format!("{MEMBER_DIR}/{identity}"), Vec::new());
    tree.insert(format!("{DEVICE_DIR}/{device_id}"), identity.as_bytes().to_vec());
    tree.insert(
        PROFILE_FILE.to_string(),
        format!("BEGIN:VCARD\nVERSION:3.0\nFN:{identity}\nEND:VCARD").into_bytes(),
    );
    tree
}

impl SwarmStore {
    /// Initialises a brand new conversation store on disk (genesis commit).
    pub async fn create(
        root: PathBuf,
        id: String,
        mode: ConversationMode,
        identity: String,
        device_id: String,
        signing_key: Ed25519KeyPair,
        profile: ConversationProfile,
    ) -> Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        for dir in tree_dirs() {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }
        tokio::fs::create_dir_all(root.join(COMMITS_DIR)).await?;

        let now = chrono::Utc::now().timestamp();
        let conversation = Conversation {
            id: id.clone(),
            mode,
            admins: vec![identity.clone()],
            members: vec![identity.clone()],
            devices: vec![DeviceCertificate {
                device_id: device_id.clone(),
                public_key: signing_key.verifying_key.to_bytes().to_vec(),
                signature: Vec::new(),
            }],
            invited: vec![],
            banned: vec![],
            crls: vec![],
            created_at: now,
            updated_at: now,
        };

        let mut tree = seed_tree(&identity, &device_id);
        if !profile.title.is_none() || !profile.description.is_none() || !profile.avatar.is_none() {
            let vcf = serde_json::to_string(&profile)?;
            tree.insert(PROFILE_FILE.to_string(), vcf.into_bytes());
        }

        let genesis = Commit::new(
            vec![],
            identity.clone(),
            signing_key.verifying_key.to_bytes().to_vec(),
            now,
            CommitType::Initial,
            format!("conversation {} created", id),
            tree,
            &signing_key,
        );

        let mut store = Self {
            conversation,
            commits: HashMap::new(),
            root,
            identity,
            device_id,
            signing_key,
        };
        store.commits.insert(genesis.id.clone(), genesis);
        store.persist().await?;
        info!("Created swarm conversation {id}");
        Ok(store)
    }

    /// Opens (and resumes) a persisted store at `root`. The local signing key
    /// is only used to sign new commits; existing commits are kept as-is.
    pub async fn open(
        root: PathBuf,
        identity: String,
        device_id: String,
        signing_key: Ed25519KeyPair,
    ) -> Result<Self> {
        for dir in tree_dirs() {
            tokio::fs::create_dir_all(root.join(dir)).await?;
        }
        tokio::fs::create_dir_all(root.join(COMMITS_DIR)).await?;
        let mut store = Self {
            conversation: Conversation {
                id: String::new(),
                mode: ConversationMode::InvitesOnly,
                admins: vec![],
                members: vec![],
                devices: vec![],
                invited: vec![],
                banned: vec![],
                crls: vec![],
                created_at: 0,
                updated_at: 0,
            },
            commits: HashMap::new(),
            root,
            identity,
            device_id,
            signing_key,
        };
        store.load().await?;
        Ok(store)
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn commits(&self) -> &BTreeMap<String, Commit> {
        &self.commits
    }

    pub fn commits_mut(&mut self) -> &mut BTreeMap<String, Commit> {
        &mut self.commits
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Commit ids that are not referenced as a parent by any other commit.
    pub fn head_ids(&self) -> Vec<String> {
        let mut referenced: std::collections::HashSet<&str> = Default::default();
        for c in self.commits.values() {
            for p in &c.parent_ids {
                referenced.insert(p.as_str());
            }
        }
        self.commits
            .keys()
            .filter(|id| !referenced.contains(id.as_str()))
            .cloned()
            .collect()
    }

    pub fn head_id(&self) -> Option<String> {
        let mut heads = self.head_ids();
        if heads.len() <= 1 {
            return heads.pop();
        }
        heads.into_iter().max()
    }

    /// Orders commits topologically by timestamp so the latest snapshot wins.
    fn reachable_from<'a>(&'a self, heads: &[String]) -> Vec<&'a Commit> {
        let mut stack: Vec<&str> = heads.iter().map(|s| s.as_str()).collect();
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut all: Vec<&Commit> = Vec::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.to_string()) {
                continue;
            }
            if let Some(c) = self.commits.get(id) {
                all.push(c);
                for p in &c.parent_ids {
                    stack.push(p.as_str());
                }
            }
        }
        all.sort_by_key(|c| c.timestamp);
        all
    }

    /// Last-writer-wins resolution of a tree over all reachable commits. This
    /// is what makes two independently-derived branches collapse to one state.
    pub fn resolved_tree(&self, heads: &[String]) -> SwarmTree {
        let reachable = self.reachable_from(heads);
        let mut by_path: BTreeMap<String, (i64, String, Vec<u8>)> = BTreeMap::new();
        for c in &reachable {
            for (k, v) in &c.tree {
                let entry = by_path
                    .entry(k.clone())
                    .or_insert_with(|| (c.timestamp, c.id.clone(), v.clone()));
                if c.timestamp > entry.0 || (c.timestamp == entry.0 && c.id > entry.1) {
                    *entry = (c.timestamp, c.id.clone(), v.clone());
                }
            }
        }
        by_path
            .into_iter()
            .map(|(k, (_, _, v))| (k, v))
            .collect()
    }

    fn head_tree(&self) -> SwarmTree {
        self.resolved_tree(&self.head_ids())
    }

    /// Appends a commit that applies `delta` on top of the current head tree.
    fn commit_delta(
        &mut self,
        payload_type: CommitType,
        message: String,
        delta: SwarmTree,
    ) -> Result<Commit> {
        let mut tree = self.head_tree();
        for (k, v) in delta {
            tree.insert(k, v);
        }
        let parents = self.head_ids();
        let commit = Commit::new(
            parents,
            self.identity.clone(),
            self.signing_key.verifying_key.to_bytes().to_vec(),
            chrono::Utc::now().timestamp(),
            payload_type,
            message,
            tree,
            &self.signing_key,
        );
        self.commits.insert(commit.id.clone(), commit.clone());
        self.conversation.updated_at = chrono::Utc::now().timestamp();
        Ok(commit)
    }

    pub fn append_text_message(&mut self, body: &str) -> Result<Commit> {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let mut delta = SwarmTree::new();
        delta.insert(
            format!("{MESSAGE_DIR}/{}.json", msg_id),
            serde_json::to_vec(&serde_json::json!({
                "id": msg_id,
                "author": self.identity,
                "device": self.device_id,
                "ts": chrono::Utc::now().timestamp(),
                "body": body,
            }))?,
        );
        self.commit_delta(CommitType::Text, body.to_string(), delta)
    }

    pub fn add_member(&mut self, member: &str) -> Result<Commit> {
        if !self.conversation.members.iter().any(|m| m == member) {
            self.conversation.members.push(member.to_string());
        }
        if !self.conversation.admins.iter().any(|m| m == member) {
            // Members are invited by admins; in pure-Rust swarm anyone can be
            // granted membership by an admin-created commit. We record the
            // membership file entry below.
        }
        let mut delta = SwarmTree::new();
        delta.insert(format!("{MEMBER_DIR}/{member}"), Vec::new());
        self.commit_delta(
            CommitType::MemberAdd,
            format!("added member {member}"),
            delta,
        )
    }

    pub fn remove_member(&mut self, member: &str) -> Result<Commit> {
        self.conversation.members.retain(|m| m != member);
        let mut delta = SwarmTree::new();
        delta.insert(format!("{MEMBER_DIR}/{member}"), b"removed".to_vec());
        self.commit_delta(
            CommitType::MemberRemove,
            format!("removed member {member}"),
            delta,
        )
    }

    pub fn ban(&mut self, member: &str) -> Result<Commit> {
        if !self.conversation.banned.iter().any(|m| m == member) {
            self.conversation.banned.push(member.to_string());
        }
        self.conversation.members.retain(|m| m != member);
        let mut delta = SwarmTree::new();
        delta.insert(format!("{BANNED_DIR}/{member}"), Vec::new());
        self.commit_delta(CommitType::Ban, format!("banned {member}"), delta)
    }

    pub fn unban(&mut self, member: &str) -> Result<Commit> {
        self.conversation.banned.retain(|m| m != member);
        let mut delta = SwarmTree::new();
        delta.insert(format!("{BANNED_DIR}/{member}"), b"unbanned".to_vec());
        self.commit_delta(CommitType::Unban, format!("unbanned {member}"), delta)
    }

    pub fn add_device(&mut self, cert: &DeviceCertificate) -> Result<Commit> {
        if !self
            .conversation
            .devices
            .iter()
            .any(|d| d.device_id == cert.device_id)
        {
            self.conversation.devices.push(cert.clone());
        }
        let mut delta = SwarmTree::new();
        delta.insert(
            format!("{DEVICE_DIR}/{}", cert.device_id),
            cert.public_key.clone(),
        );
        self.commit_delta(
            CommitType::DeviceAdd,
            format!("added device {}", cert.device_id),
            delta,
        )
    }

    /// Verifies the whole log: every commit's id + signature + structural
    /// integrity (parents exist, DAG is acyclic).
    pub fn verify_log(&self) -> VerifyReport {
        let mut errors = Vec::new();
        let mut index: std::collections::HashSet<&str> =
            self.commits.keys().map(|s| s.as_str()).collect();

        for c in self.commits.values() {
            if !c.verify() {
                errors.push(format!("commit {} failed signature/id verification", c.id));
            }
            for p in &c.parent_ids {
                if !index.contains(p.as_str()) {
                    errors.push(format!(
                        "commit {} references missing parent {}",
                        c.id, p
                    ));
                }
            }
        }

        if self
            .commits
            .values()
            .any(|c| c.parent_ids.iter().any(|p| !index.contains(p.as_str())))
        {
            // Already reported above; keep acyclicity separate.
        }

        if self.has_cycle() {
            errors.push("commit DAG contains a cycle".to_string());
        }

        VerifyReport {
            checked: self.commits.len(),
            errors,
        }
    }

    fn has_cycle(&self) -> bool {
        #[derive(PartialEq)]
        enum Mark {
            Unvisited,
            InStack,
            Done,
        }
        let mut marks: HashMap<&str, Mark> =
            self.commits.keys().map(|k| (k.as_str(), Mark::Unvisited)).collect();
        fn visit<'a>(
            id: &'a str,
            commits: &'a BTreeMap<String, Commit>,
            marks: &mut HashMap<&'a str, Mark>,
        ) -> bool {
            match marks.get(id) {
                Some(Mark::InStack) => return true,
                Some(Mark::Done) => return false,
                _ => {}
            }
            marks.insert(id, Mark::InStack);
            if let Some(c) = commits.get(id) {
                for p in &c.parent_ids {
                    if visit(p.as_str(), commits, marks) {
                        return true;
                    }
                }
            }
            marks.insert(id, Mark::Done);
            false
        }
        let ids: Vec<String> = self.commits.keys().cloned().collect();
        for id in &ids {
            if visit(id.as_str(), &self.commits, &mut marks) {
                return true;
            }
        }
        false
    }

    /// Merges another store's commit set into this one, preserving both
    /// branches and collapsing them with a last-writer-wins merge commit.
    /// Returns the number of commits newly adopted and whether a merge commit
    /// was created.
    pub fn merge(&mut self, other: &SwarmStore) -> Result<MergeReport> {
        let before = self.commits.len();
        for (id, c) in other.commits.iter() {
            self.commits.entry(id.clone()).or_insert_with(|| c.clone());
        }
        let merged_heads = self.head_ids();

        let mut created_merge = None;
        if merged_heads.len() > 1 {
            let mut sorted = merged_heads.clone();
            sorted.sort();
            let tree = self.resolved_tree(&sorted);
            let merge_commit = Commit::new(
                sorted,
                self.identity.clone(),
                self.signing_key.verifying_key.to_bytes().to_vec(),
                chrono::Utc::now().timestamp(),
                CommitType::Text,
                "merged divergent branches".to_string(),
                tree,
                &self.signing_key,
            );
            self.commits
                .insert(merge_commit.id.clone(), merge_commit.clone());
            created_merge = Some(merge_commit);
            self.conversation.updated_at = chrono::Utc::now().timestamp();
        }

        let added = self.commits.len() - before;
        Ok(MergeReport { added_commits: added, merge_commit: created_merge })
    }

    /// Commits present in `other` but missing locally.
    pub fn missing_commits<'a>(&'a self, other: &'a SwarmStore) -> Vec<&'a Commit> {
        other
            .commits
            .values()
            .filter(|c| !self.commits.contains_key(&c.id))
            .collect()
    }

    pub fn diff_trees(from: &SwarmTree, to: &SwarmTree) -> SwarmDiff {
        let mut diff = SwarmDiff::default();
        for (k, v) in to {
            match from.get(k) {
                Some(old) if old == v => {}
                Some(old) => diff.modified.push((k.clone(), old.clone(), v.clone())),
                None => diff.added.push((k.clone(), v.clone())),
            }
        }
        for (k, _) in from {
            if !to.contains_key(k) {
                diff.removed.push(k.clone());
            }
        }
        diff.added.sort();
        diff.removed.sort();
        diff.modified.sort_by(|a, b| a.0.cmp(&b.0));
        diff
    }

    // ── Persistence ────────────────────────────────────────────────────────

    pub async fn persist(&self) -> Result<()> {
        tokio::fs::create_dir_all(self.root.join(COMMITS_DIR)).await?;
        let conv_bytes = serde_json::to_vec_pretty(&self.conversation)?;
        tokio::fs::write(self.root.join(CONVERSATION_FILE), conv_bytes).await?;
        for c in self.commits.values() {
            let bytes = serde_json::to_vec_pretty(c)?;
            tokio::fs::write(self.root.join(COMMITS_DIR).join(format!("{}.json", c.id)), bytes)
                .await?;
        }
        Ok(())
    }

    pub async fn load(&mut self) -> Result<()> {
        let conv_bytes = tokio::fs::read(self.root.join(CONVERSATION_FILE)).await?;
        let conversation: Conversation = serde_json::from_slice(&conv_bytes)?;
        self.conversation = conversation;

        let mut commits = HashMap::new();
        let mut entries = tokio::fs::read_dir(self.root.join(COMMITS_DIR)).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let bytes = tokio::fs::read(&path).await?;
                if let Ok(c) = serde_json::from_slice::<Commit>(&bytes) {
                    commits.insert(c.id.clone(), c);
                } else {
                    warn!("Skipping malformed commit file {}", path.display());
                }
            }
        }
        self.commits = commits;
        if self.conversation.id.is_empty() {
            bail!("conversation.json missing or empty at {}", self.root.display());
        }
        debug!(
            "Loaded swarm conversation {} ({} commits)",
            self.conversation.id,
            self.commits.len()
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct SwarmDiff {
    pub added: Vec<(String, Vec<u8>)>,
    pub removed: Vec<String>,
    pub modified: Vec<(String, Vec<u8>, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub checked: usize,
    pub errors: Vec<String>,
}

impl VerifyReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct MergeReport {
    pub added_commits: usize,
    pub merge_commit: Option<Commit>,
}

/// A wire-level bundle of commits exchanged between peers during sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmSyncMessage {
    pub conversation_id: String,
    pub device_cert: Vec<u8>,
    pub commits: Vec<Commit>,
}

/// Transport abstraction for peer-to-peer swarm sync. `pull` reads an
/// out-of-band received commit set (e.g., via the Tor wire channel), `push`
/// transmits the local commit set to a peer's onion address.
#[async_trait]
pub trait SwarmTransport: Send + Sync {
    async fn push(&self, peer_onion: &str, msg: &SwarmSyncMessage) -> Result<()>;
    async fn pull(&self, peer_onion: &str, conversation_id: &str) -> Result<Vec<Commit>>;
}

/// Default transport: the existing pure-Rust Tor wire channel.
/// Push serialises a sync message and hands it to `try_send_wire`. Presence of
/// the message on the wire is fire-and-forget; reception is handled by the
/// inbound listener (out of scope for the swarm module).
#[derive(Debug, Clone, Default)]
pub struct TorSwarmTransport;

#[async_trait]
impl SwarmTransport for TorSwarmTransport {
    async fn push(&self, peer_onion: &str, msg: &SwarmSyncMessage) -> Result<()> {
        let bytes = serde_json::to_vec(msg)?;
        crate::messaging::try_send_wire(peer_onion, &bytes).await;
        Ok(())
    }

    async fn pull(&self, _peer_onion: &str, _conversation_id: &str) -> Result<Vec<Commit>> {
        // Fire-and-forget wire channel cannot perform an in-band request;
        // inbound pull is delivered via the message listener.
        Ok(Vec::new())
    }
}

/// Multi-conversation manager. Kept from the prior (git2) API so existing
/// integraters are not broken, now backed by the pure-Rust swarm store.
pub struct SwarmManager {
    base: PathBuf,
    identity: String,
    device_id: String,
    signing_key: Ed25519KeyPair,
    stores: Arc<RwLock<HashMap<String, SwarmStore>>>,
}

impl SwarmManager {
    pub async fn new(base_path: PathBuf, identity: String) -> Result<Self> {
        let signing_key = Ed25519KeyPair::generate()?;
        Self::with_device_key(base_path, identity, "device-main", signing_key).await
    }

    pub async fn with_device_key(
        base_path: PathBuf,
        identity: String,
        device_id: &str,
        signing_key: Ed25519KeyPair,
    ) -> Result<Self> {
        let base = base_path.join(SWARM_DIR);
        tokio::fs::create_dir_all(&base).await?;
        Ok(Self {
            base,
            identity,
            device_id: device_id.to_string(),
            signing_key,
            stores: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn store_root(&self, conversation_id: &str) -> PathBuf {
        self.base.join(conversation_id)
    }

    pub async fn create_conversation(
        &self,
        mode: ConversationMode,
        profile: ConversationProfile,
    ) -> Result<Conversation> {
        let id = uuid::Uuid::new_v4().to_string();
        let key = Ed25519KeyPair::from_bytes(&self.signing_key.signing_key().to_bytes())?;
        let store = SwarmStore::create(
            self.store_root(&id),
            id.clone(),
            mode,
            self.identity.clone(),
            self.device_id.clone(),
            key,
            profile,
        )
        .await?;
        let conversation = store.conversation().clone();
        self.stores.write().await.insert(id, store);
        info!("Created conversation {} mode {:?}", conversation.id, mode);
        Ok(conversation)
    }

    pub async fn get_conversation(&self, conversation_id: &str) -> Option<Conversation> {
        let read = self.stores.read().await;
        read.get(conversation_id).map(|s| s.conversation().clone())
    }

    pub async fn list_conversations(&self) -> Vec<Conversation> {
        let read = self.stores.read().await;
        read.values()
            .map(|s| s.conversation().clone())
            .collect()
    }

    pub async fn add_message(&self, conversation_id: &str, body: &str) -> Result<Commit> {
        let mut write = self.stores.write().await;
        let store = write
            .get_mut(conversation_id)
            .ok_or_else(|| anyhow!("Conversation {conversation_id} not loaded"))?;
        let commit = store.append_text_message(body)?;
        store.persist().await?;
        Ok(commit)
    }

    pub async fn sync_conversation(
        &self,
        conversation_id: &str,
        peer_onion: &str,
    ) -> Result<MergeReport> {
        let transport = TorSwarmTransport;
        let read = self.stores.read().await;
        let store = read
            .get(conversation_id)
            .ok_or_else(|| anyhow!("Conversation {conversation_id} not loaded"))?;

        let msg = SwarmSyncMessage {
            conversation_id: conversation_id.to_string(),
            device_cert: store.conversation().devices[0].public_key.clone(),
            commits: store.commits().values().cloned().collect(),
        };
        transport.push(peer_onion, &msg).await?;
        debug!("Synced conversation {conversation_id} with peer {peer_onion}");
        Ok(MergeReport { added_commits: 0, merge_commit: None })
    }

    pub async fn invite_to_conversation(&self, _conversation_id: &str, _peer: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vchat-swarm-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn copy_store(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        std::fs::copy(
            src.join(CONVERSATION_FILE),
            dst.join(CONVERSATION_FILE),
        )
        .unwrap();
        std::fs::create_dir_all(dst.join(COMMITS_DIR)).unwrap();
        for dir in tree_dirs() {
            std::fs::create_dir_all(dst.join(dir)).unwrap();
        }
        for entry in std::fs::read_dir(src.join(COMMITS_DIR)).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dst.join(COMMITS_DIR).join(entry.file_name())).unwrap();
        }
    }

    async fn gen_store(root: PathBuf, identity: &str, id: &str) -> SwarmStore {
        let key = Ed25519KeyPair::generate().unwrap();
        SwarmStore::create(
            root,
            id.to_string(),
            ConversationMode::InvitesOnly,
            identity.to_string(),
            format!("dev-{identity}"),
            key,
            ConversationProfile::default(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn independent_stores_merge_to_identical_state() {
        let root_a = tmp("a");
        let root_b = tmp("b");
        let conv_id = uuid::Uuid::new_v4().to_string();

        let mut a = gen_store(root_a.clone(), "alice", &conv_id).await;
        a.append_text_message("hello from alice").unwrap();
        a.persist().await.unwrap();

        copy_store(&root_a, &root_b);
        let key_b = Ed25519KeyPair::generate().unwrap();
        let mut b = SwarmStore::open(root_b, "bob".to_string(), "dev-bob".to_string(), key_b)
            .await
            .unwrap();
        b.append_text_message("hi from bob").unwrap();

        let a_before: Vec<String> = a.commits.keys().cloned().collect();
        let report = a.merge(&b).unwrap();
        assert!(report.merge_commit.is_some(), "diverged branches need a merge commit");
        assert!(a_before.len() < a.commits.len());

        let _ = b.merge(&a).unwrap();

        let mut a_ids: Vec<String> = a.commits.keys().cloned().collect();
        let mut b_ids: Vec<String> = b.commits.keys().cloned().collect();
        a_ids.sort();
        b_ids.sort();
        assert_eq!(a_ids, b_ids, "commit DAGs must converge");

        let a_head = a.head_id().unwrap();
        let b_head = b.head_id().unwrap();
        assert_eq!(a_head, b_head);
        assert_eq!(
            a.resolved_tree(&[a_head]),
            b.resolved_tree(&[b_head]),
            "resolved trees must be identical after mutual merge"
        );
    }

    #[tokio::test]
    async fn merge_is_idempotent() {
        let root_a = tmp("ia");
        let root_b = tmp("ib");
        let conv_id = uuid::Uuid::new_v4().to_string();
        let mut a = gen_store(root_a.clone(), "alice", &conv_id).await;
        a.append_text_message("m1").unwrap();
        a.persist().await.unwrap();
        copy_store(&root_a, &root_b);
        let key_b = Ed25519KeyPair::generate().unwrap();
        let mut b = SwarmStore::open(root_b, "bob".to_string(), "dev-bob".to_string(), key_b)
            .await
            .unwrap();
        b.append_text_message("m2").unwrap();

        let _ = a.merge(&b).unwrap();
        let size_after_first = a.commits.len();
        let _ = a.merge(&b).unwrap();
        assert_eq!(a.commits.len(), size_after_first, "re-merging must not grow");
        assert!(a.verify_log().is_valid());
    }

    #[tokio::test]
    async fn tampered_commit_fails_verification() {
        let mut s = gen_store(tmp("t"), "alice", &uuid::Uuid::new_v4().to_string()).await;
        s.append_text_message("genuine").unwrap();
        assert!(s.verify_log().is_valid());

        let head = s.head_id().unwrap();
        {
            let c = s.commits_mut().get_mut(&head).unwrap();
            c.tree.insert("messages/injected.json".to_string(), b"evil".to_vec());
        }
        let report = s.verify_log();
        assert!(!report.is_valid(), "tampered tree must be detected");
        assert!(report.errors.iter().any(|e| e.contains(&head)));

        // Restore cleanliness check.
        let restored = s.commits_mut().get_mut(&head).unwrap();
        restored.tree.remove("messages/injected.json");
        assert!(s.verify_log().is_valid());
    }

    #[tokio::test]
    async fn member_add_and_remove_update_conversation() {
        let mut s = gen_store(tmp("m"), "alice", &uuid::Uuid::new_v4().to_string()).await;
        assert_eq!(s.conversation().members.len(), 1);
        assert!(s.conversation().members.contains(&"alice".to_string()));

        let _ = s.add_member("bob").unwrap();
        let _ = s.add_member("carol").unwrap();
        assert!(s.conversation().members.contains(&"bob".to_string()));
        assert!(s.conversation().members.contains(&"carol".to_string()));

        let _ = s.remove_member("bob").unwrap();
        assert!(!s.conversation().members.contains(&"bob".to_string()));
        assert!(s.conversation().members.contains(&"carol".to_string()));
        assert!(s.verify_log().is_valid());
    }

    #[tokio::test]
    async fn device_registration_and_add() {
        let mut s = gen_store(tmp("d"), "alice", &uuid::Uuid::new_v4().to_string()).await;
        assert_eq!(s.conversation().devices.len(), 1);
        let cert = DeviceCertificate {
            device_id: "dev-phone".to_string(),
            public_key: vec![1u8; 32],
            signature: vec![],
        };
        let _ = s.add_device(&cert).unwrap();
        assert_eq!(s.conversation().devices.len(), 2);
        assert!(s.verify_log().is_valid());
    }

    #[tokio::test]
    async fn diff_reports_add_remove_modify() {
        let mut from = SwarmTree::new();
        from.insert("a".to_string(), b"1".to_vec());
        from.insert("b".to_string(), b"2".to_vec());
        let mut to = SwarmTree::new();
        to.insert("a".to_string(), b"changed".to_vec());
        to.insert("c".to_string(), b"3".to_vec());

        let d = SwarmStore::diff_trees(&from, &to);
        assert_eq!(d.added, vec![("c".to_string(), b"3".to_vec())]);
        assert_eq!(d.removed, vec!["b".to_string()]);
        assert_eq!(d.modified, vec![("a".to_string(), b"1".to_vec(), b"changed".to_vec())]);
    }

    #[tokio::test]
    async fn persist_and_reopen_roundtrip() {
        let root = tmp("p");
        let mut s = gen_store(root.clone(), "alice", &uuid::Uuid::new_v4().to_string()).await;
        s.append_text_message("stored message").unwrap();
        let n = s.commits.len();
        s.persist().await.unwrap();

        let key = Ed25519KeyPair::generate().unwrap();
        let reopened = SwarmStore::open(root, "alice".to_string(), "dev-alice".to_string(), key)
            .await
            .unwrap();
        assert_eq!(reopened.commits.len(), n);
        assert!(reopened.verify_log().is_valid());
        assert!(reopened
            .commits()
            .values()
            .any(|c| c.tree.keys().any(|k| k.starts_with(MESSAGE_DIR))));
    }
}
