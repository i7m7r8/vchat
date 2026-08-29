use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransfer {
    pub id: String,
    pub conversation_id: String,
    pub file_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub sha3sum: String,
    pub mime_type: String,
    pub sender: String,
    pub recipient: String,
    pub status: TransferStatus,
    pub progress: f32,
    pub bytes_transferred: u64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub chunk_size: u32,
    pub total_chunks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransferStatus {
    Pending,
    WaitingForAccept,
    InProgress,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChunk {
    pub file_id: String,
    pub chunk_index: u32,
    pub data: Vec<u8>,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOffer {
    pub file_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub sha3sum: String,
    pub mime_type: String,
    pub sender: String,
    pub conversation_id: String,
}

pub struct FileTransferManager {
    transfers: Arc<RwLock<HashMap<String, FileTransfer>>>,
    storage_path: PathBuf,
    chunk_size: u32,
}

impl FileTransferManager {
    pub fn new(storage_path: PathBuf) -> Self {
        Self {
            transfers: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
            chunk_size: 64 * 1024, // 64KB chunks
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.storage_path).await?;
        tokio::fs::create_dir_all(self.storage_path.join("incoming")).await?;
        tokio::fs::create_dir_all(self.storage_path.join("outgoing")).await?;
        Ok(())
    }

    pub async fn offer_file(
        &self,
        conversation_id: String,
        file_path: PathBuf,
        recipient: String,
    ) -> Result<FileOffer> {
        let file_name = file_path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?
            .to_string();

        let metadata = fs::metadata(&file_path).await?;
        let file_size = metadata.len();

        // Calculate SHA3-256 hash
        let mut file = fs::File::open(&file_path).await?;
        let mut hasher = sha3::Sha3_256::new();
        let mut buffer = vec![0u8; 8192];
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 { break; }
            hasher.update(&buffer[..n]);
        }
        let sha3sum = hex::encode(hasher.finalize());

        let file_id = Uuid::new_v4().to_string();
        let mime_type = mime_guess::from_path(&file_path)
            .first_or_octet_stream()
            .to_string();

        let offer = FileOffer {
            file_id: file_id.clone(),
            file_name,
            file_size,
            sha3sum,
            mime_type,
            sender: "local".to_string(), // Will be set by caller
            conversation_id,
        };

        // Copy file to outgoing directory
        let dest = self.storage_path.join("outgoing").join(&file_id);
        fs::copy(&file_path, &dest).await?;

        info!("File offered: {} ({} bytes)", offer.file_name, file_size);
        Ok(offer)
    }

    pub async fn accept_file(&self, offer: FileOffer, save_path: PathBuf) -> Result<String> {
        let file_id = offer.file_id.clone();
        let transfer = FileTransfer {
            id: Uuid::new_v4().to_string(),
            conversation_id: offer.conversation_id.clone(),
            file_id: file_id.clone(),
            file_name: offer.file_name.clone(),
            file_size: offer.file_size,
            sha3sum: offer.sha3sum.clone(),
            mime_type: offer.mime_type.clone(),
            sender: offer.sender.clone(),
            recipient: "local".to_string(),
            status: TransferStatus::InProgress,
            progress: 0.0,
            bytes_transferred: 0,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            chunk_size: self.chunk_size,
            total_chunks: ((offer.file_size as f64) / (self.chunk_size as f64)).ceil() as u32,
        };

        self.transfers.write().await.insert(transfer.id.clone(), transfer.clone());

        // Create destination file
        tokio::fs::create_dir_all(save_path.parent().unwrap_or(Path::new("."))).await?;
        let mut dest_file = fs::File::create(&save_path).await?;

        // In a real implementation, we'd download chunks from the sender
        // For now, we just create a placeholder
        info!("File transfer accepted: {}", offer.file_name);

        Ok(transfer.id)
    }

    pub async fn send_chunk(&self, transfer_id: &str, chunk: FileChunk) -> Result<()> {
        let mut transfers = self.transfers.write().await;
        if let Some(transfer) = transfers.get_mut(transfer_id) {
            // Write chunk to file
            // In real implementation, append to destination file
            transfer.bytes_transferred += chunk.data.len() as u64;
            transfer.progress = transfer.bytes_transferred as f32 / transfer.file_size as f32;
            
            if transfer.bytes_transferred >= transfer.file_size {
                transfer.status = TransferStatus::Completed;
                transfer.completed_at = Some(chrono::Utc::now().timestamp());
            }
        }
        Ok(())
    }

    pub async fn get_transfer(&self, transfer_id: &str) -> Option<FileTransfer> {
        self.transfers.read().await.get(transfer_id).cloned()
    }

    pub async fn list_transfers(&self) -> Vec<FileTransfer> {
        self.transfers.read().await.values().cloned().collect()
    }

    pub async fn cancel_transfer(&self, transfer_id: &str) -> Result<()> {
        let mut transfers = self.transfers.write().await;
        if let Some(transfer) = transfers.get_mut(transfer_id) {
            transfer.status = TransferStatus::Cancelled;
        }
        Ok(())
    }

    pub async fn pause_transfer(&self, transfer_id: &str) -> Result<()> {
        let mut transfers = self.transfers.write().await;
        if let Some(transfer) = transfers.get_mut(transfer_id) {
            transfer.status = TransferStatus::Paused;
        }
        Ok(())
    }

    pub async fn resume_transfer(&self, transfer_id: &str) -> Result<()> {
        let mut transfers = self.transfers.write().await;
        if let Some(transfer) = transfers.get_mut(transfer_id) {
            if transfer.status == TransferStatus::Paused {
                transfer.status = TransferStatus::InProgress;
            }
        }
        Ok(())
    }
}