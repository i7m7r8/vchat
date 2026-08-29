use anyhow::{Context, Result};
use ed25519_dalek::{
    pkcs8::{EncodePrivateKey, EncodePublicKey},
    SigningKey, VerifyingKey,
};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCertificate {
    pub device_id: String,
    pub device_name: String,
    pub public_key: Vec<u8>,
    pub account_pubkey: Vec<u8>,
    pub signature: Vec<u8>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCertificate {
    pub account_id: String,
    pub ca_pubkey: Vec<u8>,
    pub account_pubkey: Vec<u8>,
    pub signature: Vec<u8>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Identity {
    pub signing_key: SigningKey,
    pub verifying_key: VerifyingKey,
    pub device_id: String,
    pub device_name: String,
    pub device_cert: DeviceCertificate,
    pub account_cert: Option<AccountCertificate>,
}

impl Identity {
    pub fn generate(device_name: String) -> Result<Self> {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        let device_id = Uuid::new_v4().to_string();

        let now = chrono::Utc::now().timestamp();
        let device_cert = DeviceCertificate {
            device_id: device_id.clone(),
            device_name,
            public_key: verifying_key.to_bytes().to_vec(),
            account_pubkey: Vec::new(), // Self-signed for now
            signature: Vec::new(),
            created_at: now,
            expires_at: None,
        };

        Ok(Self {
            signing_key,
            verifying_key,
            device_id,
            device_name: device_cert.device_name.clone(),
            device_cert,
            account_cert: None,
        })
    }

    pub fn identity_string(&self) -> String {
        base64::encode_config(&self.verifying_key.to_bytes(), base64::URL_SAFE_NO_PAD)
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        use ed25519_dalek::Signer;
        self.signing_key.sign(msg).to_bytes().to_vec()
    }

    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> Result<()> {
        use ed25519_dalek::Verifier;
        let sig = ed25519_dalek::Signature::from_bytes(sig.try_into()?)?;
        self.verifying_key.verify_strict(msg, &sig)?;
        Ok(())
    }

    pub async fn save(&self, path: &PathBuf) -> Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    pub async fn load(path: &PathBuf) -> Result<Self> {
        let data = tokio::fs::read(path).await?;
        let identity: Self = serde_json::from_slice(&data)?;
        Ok(identity)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub identity_string: String,
    pub display_name: String,
    pub device_cert: DeviceCertificate,
    pub added_at: i64,
    pub verified: bool,
    pub blocked: bool,
}

pub struct IdentityManager {
    identity: Arc<RwLock<Option<Identity>>>,
    contacts: Arc<RwLock<HashMap<String, Contact>>>,
    storage_path: PathBuf,
}

impl IdentityManager {
    pub fn new(storage_path: PathBuf) -> Self {
        Self {
            identity: Arc::new(RwLock::new(None)),
            contacts: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.storage_path).await?;

        let identity_path = self.storage_path.join("identity.json");
        if identity_path.exists() {
            let identity = Identity::load(&identity_path).await?;
            *self.identity.write().await = Some(identity);
        } else {
            let identity = Identity::generate("vchat device".to_string())?;
            identity.save(&identity_path).await?;
            *self.identity.write().await = Some(identity);
        }

        let contacts_path = self.storage_path.join("contacts.json");
        if contacts_path.exists() {
            let data = tokio::fs::read(&contacts_path).await?;
            let contacts: HashMap<String, Contact> = serde_json::from_slice(&data)?;
            *self.contacts.write().await = contacts;
        }

        Ok(())
    }

    pub async fn get_identity(&self) -> Option<Identity> {
        self.identity.read().await.clone()
    }

    pub async fn get_identity_string(&self) -> Option<String> {
        self.identity.read().await.as_ref().map(|i| i.identity_string())
    }

    pub async fn add_contact(&self, contact: Contact) -> Result<()> {
        self.contacts
            .write()
            .await
            .insert(contact.identity_string.clone(), contact);
        self.save_contacts().await
    }

    pub async fn remove_contact(&self, identity_string: &str) -> Result<()> {
        self.contacts.write().await.remove(identity_string);
        self.save_contacts().await
    }

    pub async fn get_contacts(&self) -> Vec<Contact> {
        self.contacts.read().await.values().cloned().collect()
    }

    pub async fn get_contact(&self, identity_string: &str) -> Option<Contact> {
        self.contacts.read().await.get(identity_string).cloned()
    }

    async fn save_contacts(&self) -> Result<()> {
        let contacts = self.contacts.read().await.clone();
        let data = serde_json::to_vec_pretty(&contacts)?;
        tokio::fs::write(self.storage_path.join("contacts.json"), data).await?;
        Ok(())
    }
}

impl Default for IdentityManager {
    fn default() -> Self {
        Self::new(dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("vchat"))
    }
}