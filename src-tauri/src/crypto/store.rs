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
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;

        CREATE TABLE IF NOT EXISTS identity (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            public_key TEXT NOT NULL,
            onion_address TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            x25519_secret TEXT NOT NULL,
            ed25519_secret TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            public_key TEXT NOT NULL,
            onion_address TEXT NOT NULL UNIQUE,
            added_at INTEGER NOT NULL,
            verified INTEGER DEFAULT 0,
            blocked INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            content TEXT NOT NULL,
            encrypted_content BLOB,
            timestamp INTEGER NOT NULL,
            encrypted INTEGER DEFAULT 1,
            message_type TEXT NOT NULL,
            delivered INTEGER DEFAULT 0,
            read INTEGER DEFAULT 0,
            sequence_num INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            peer_onion TEXT NOT NULL,
            noise_protocol_state BLOB,
            established_at INTEGER,
            last_active INTEGER,
            message_count INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            event TEXT NOT NULL,
            details TEXT,
            severity TEXT DEFAULT 'info'
        );

        CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
        CREATE INDEX IF NOT EXISTS idx_contacts_onion ON contacts(onion_address);
        CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
        ",
    )?;

    let mut db = DB.lock().await;
    *db = Some(conn);

    crate::error::audit_log("db_initialized", &format!("path={}", get_db_path().display()));

    Ok(())
}

pub async fn log_audit_event(event: &str, details: &str, severity: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "INSERT INTO audit_log (timestamp, event, details, severity) VALUES (?1, ?2, ?3, ?4)",
        params![
            chrono::Utc::now().timestamp(),
            event,
            details,
            severity,
        ],
    )?;

    Ok(())
}

pub async fn save_identity_with_keys(
    identity: &Identity,
    x25519_secret: &str,
    ed25519_secret: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "INSERT OR REPLACE INTO identity (id, public_key, onion_address, display_name, x25519_secret, ed25519_secret, updated_at) 
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            identity.public_key,
            identity.onion_address,
            identity.display_name,
            x25519_secret,
            ed25519_secret,
            chrono::Utc::now().timestamp(),
        ],
    )?;

    Ok(())
}

pub async fn save_identity(identity: &Identity) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "UPDATE identity SET public_key = ?1, display_name = ?2, updated_at = ?3 WHERE id = 1",
        params![
            identity.public_key,
            identity.display_name,
            chrono::Utc::now().timestamp(),
        ],
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

pub async fn load_x25519_secret() -> Result<Option<String>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let result = conn.query_row(
        "SELECT x25519_secret FROM identity WHERE id = 1",
        [],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(secret) => Ok(Some(secret)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn load_ed25519_secret() -> Result<Option<String>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let result = conn.query_row(
        "SELECT ed25519_secret FROM identity WHERE id = 1",
        [],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(secret) => Ok(Some(secret)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn save_contact(contact: &Contact) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "INSERT OR REPLACE INTO contacts (id, display_name, public_key, onion_address, added_at) 
         VALUES (?1, ?2, ?3, ?4, ?5)",
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
        "SELECT id, display_name, public_key, onion_address, added_at 
         FROM contacts WHERE blocked = 0 ORDER BY added_at DESC",
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

pub async fn delete_contact(onion_address: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "DELETE FROM contacts WHERE onion_address = ?1",
        params![onion_address],
    )?;

    Ok(())
}

pub async fn block_contact(onion_address: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute(
        "UPDATE contacts SET blocked = 1 WHERE onion_address = ?1",
        params![onion_address],
    )?;

    Ok(())
}

pub async fn save_message(message: &Message) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let message_type = serde_json::to_string(&message.message_type)?;

    conn.execute(
        "INSERT OR IGNORE INTO messages (id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num) 
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id,
            message.sender,
            message.recipient,
            message.content,
            message.timestamp,
            message.encrypted,
            message_type,
            message.sequence_num,
        ],
    )?;

    Ok(())
}

pub async fn save_message_with_encrypted(
    message: &Message,
    encrypted_content: &[u8],
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let message_type = serde_json::to_string(&message.message_type)?;

    conn.execute(
        "INSERT OR IGNORE INTO messages (id, sender, recipient, content, encrypted_content, timestamp, encrypted, message_type, sequence_num) 
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message.id,
            message.sender,
            message.recipient,
            message.content,
            encrypted_content,
            message.timestamp,
            message.encrypted,
            message_type,
            message.sequence_num,
        ],
    )?;

    Ok(())
}

pub async fn load_messages(contact_onion: &str) -> Result<Vec<Message>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let mut stmt = conn.prepare(
        "SELECT id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num 
         FROM messages 
         WHERE (sender = ?1 OR recipient = ?1) 
         ORDER BY timestamp ASC, sequence_num ASC",
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
            sequence_num: row.get(7)?,
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(messages)
}

pub async fn get_message_count(contact_onion: &str) -> Result<i64> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE sender = ?1 OR recipient = ?1",
        params![contact_onion],
        |row| row.get(0),
    )?;

    Ok(count)
}

pub async fn save_session(
    session_id: &str,
    peer_onion: &str,
    noise_state: &[u8],
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT OR REPLACE INTO sessions (id, peer_onion, noise_protocol_state, established_at, last_active) 
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, peer_onion, noise_state, now, now],
    )?;

    Ok(())
}

pub async fn load_session(peer_onion: &str) -> Result<Option<(String, Vec<u8>)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    let result = conn.query_row(
        "SELECT id, noise_protocol_state FROM sessions WHERE peer_onion = ?1 ORDER BY last_active DESC LIMIT 1",
        params![peer_onion],
        |row| {
            let id: String = row.get(0)?;
            let state: Vec<u8> = row.get(1)?;
            Ok((id, state))
        },
    );

    match result {
        Ok(data) => Ok(Some(data)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub async fn delete_all_data() -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().ok_or_else(|| anyhow::anyhow!("Database not initialized"))?;

    conn.execute_batch(
        "
        DELETE FROM messages;
        DELETE FROM contacts;
        DELETE FROM sessions;
        DELETE FROM identity;
        DELETE FROM audit_log;
        VACUUM;
        ",
    )?;

    crate::error::audit_log("data_wiped", "All local data deleted");
    Ok(())
}
