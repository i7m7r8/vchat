use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use tokio::sync::Mutex;
use once_cell::sync::Lazy;

use crate::commands::{Contact, Identity, Message, MessageType};

static DB: Lazy<Mutex<Option<Connection>>> = Lazy::new(|| Mutex::new(None));

fn get_db_path() -> PathBuf {
    let mut path = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("vchat");
    std::fs::create_dir_all(&path).ok();
    path.push("vchat.db");
    path
}

pub async fn init_db() -> Result<()> {
    let path = get_db_path();
    let conn = Connection::open(&path)?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS identity (
            id INTEGER PRIMARY KEY,
            public_key TEXT NOT NULL,
            onion_address TEXT NOT NULL,
            display_name TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            public_key TEXT NOT NULL,
            onion_address TEXT NOT NULL UNIQUE,
            added_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            encrypted BOOLEAN DEFAULT TRUE,
            message_type TEXT NOT NULL
        );
        ",
    )?;

    let mut db = DB.lock().await;
    *db = Some(conn);

    Ok(())
}

pub async fn save_identity(identity: &Identity) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "INSERT OR REPLACE INTO identity (id, public_key, onion_address, display_name) VALUES (1, ?1, ?2, ?3)",
        params![identity.public_key, identity.onion_address, identity.display_name],
    )?;

    Ok(())
}

pub async fn load_identity() -> Result<Option<Identity>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let result = conn.query_row(
        "SELECT public_key, onion_address, display_name FROM identity WHERE id = 1",
        [],
        |row| {
            Ok(Identity {
                public_key: row.get(0)?,
                onion_address: row.get(1)?,
                display_name: row.get(2)?,
            })
        },
    );

    match result {
        Ok(identity) => Ok(Some(identity)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn save_contact(contact: &Contact) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "INSERT OR REPLACE INTO contacts (id, display_name, public_key, onion_address, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            contact.id,
            contact.display_name,
            contact.public_key,
            contact.onion_address,
            contact.added_at,
        ],
    )?;

    Ok(())
}

pub async fn load_contacts() -> Result<Vec<Contact>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let mut stmt = conn.prepare(
        "SELECT id, display_name, public_key, onion_address, added_at FROM contacts ORDER BY added_at DESC",
    )?;

    let contacts = stmt.query_map([], |row| {
        Ok(Contact {
            id: row.get(0)?,
            display_name: row.get(1)?,
            public_key: row.get(2)?,
            onion_address: row.get(3)?,
            added_at: row.get(4)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(contacts)
}

pub async fn save_message(message: &Message) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let message_type = serde_json::to_string(&message.message_type)?;

    conn.execute(
        "INSERT INTO messages (id, sender, recipient, content, timestamp, encrypted, message_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            message.id,
            message.sender,
            message.recipient,
            message.content,
            message.timestamp,
            message.encrypted,
            message_type,
        ],
    )?;

    Ok(())
}

pub async fn load_messages(contact_onion: &str) -> Result<Vec<Message>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let mut stmt = conn.prepare(
        "SELECT id, sender, recipient, content, timestamp, encrypted, message_type FROM messages WHERE sender = ?1 OR recipient = ?1 ORDER BY timestamp ASC",
    )?;

    let messages = stmt.query_map(params![contact_onion], |row| {
        let message_type_str: String = row.get(6)?;
        let message_type: MessageType = serde_json::from_str(&message_type_str)
            .unwrap_or(MessageType::Text);

        Ok(Message {
            id: row.get(0)?,
            sender: row.get(1)?,
            recipient: row.get(2)?,
            content: row.get(3)?,
            timestamp: row.get(4)?,
            encrypted: row.get(5)?,
            message_type,
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(messages)
}
