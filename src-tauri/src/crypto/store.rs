use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::Arc;
use tokio::sync::Mutex;
use once_cell::sync::Lazy;

use crate::commands::{Contact, Identity, Message, MessageType};

pub(crate) static DB: Lazy<Arc<Mutex<Option<Connection>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));

pub async fn init_db() -> Result<()> {
    let conn = Connection::open("vchat.db").context("Failed to open database")?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;",
    )?;

    conn.execute_batch(
        "        CREATE TABLE IF NOT EXISTS identity (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            public_key TEXT NOT NULL,
            onion_address TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            x25519_secret TEXT NOT NULL,
            ed25519_secret TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS contacts (
            id TEXT PRIMARY KEY,
            onion_address TEXT NOT NULL UNIQUE,
            public_key TEXT NOT NULL,
            display_name TEXT NOT NULL,
            added_at INTEGER NOT NULL,
            verified INTEGER DEFAULT 0,
            blocked INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            content TEXT,
            encrypted_content BLOB,
            timestamp INTEGER NOT NULL,
            encrypted INTEGER DEFAULT 1,
            message_type TEXT NOT NULL DEFAULT 'text',
            status TEXT NOT NULL DEFAULT 'sent',
            sequence_num INTEGER,
            reply_to TEXT,
            delivered INTEGER DEFAULT 0,
            read INTEGER DEFAULT 0,
            expires_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS sessions (
            peer_onion TEXT PRIMARY KEY,
            session_key TEXT NOT NULL,
            message_keys TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            expires_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            details TEXT,
            timestamp INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS groups (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            avatar BLOB,
            created_by TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS group_members (
            group_id TEXT NOT NULL,
            onion_address TEXT NOT NULL,
            public_key TEXT,
            display_name TEXT,
            role TEXT NOT NULL DEFAULT 'member',
            joined_at INTEGER NOT NULL,
            PRIMARY KEY (group_id, onion_address),
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS group_messages (
            id TEXT PRIMARY KEY,
            group_id TEXT NOT NULL,
            sender TEXT NOT NULL,
            content TEXT,
            encrypted_content BLOB,
            timestamp INTEGER NOT NULL,
            message_type TEXT NOT NULL DEFAULT 'text',
            sequence_num INTEGER,
            reply_to TEXT,
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS reactions (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL,
            sender TEXT NOT NULL,
            emoji TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            UNIQUE (message_id, sender, emoji)
        );

        CREATE TABLE IF NOT EXISTS call_log (
            id TEXT PRIMARY KEY,
            peer_onion TEXT NOT NULL,
            call_type TEXT NOT NULL,
            direction TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            duration_secs INTEGER,
            status TEXT NOT NULL DEFAULT 'ringing'
        );

        CREATE TABLE IF NOT EXISTS file_transfers (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            filename TEXT NOT NULL,
            mime_type TEXT,
            size INTEGER,
            encryption_key BLOB,
            chunk_dir TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            started_at INTEGER NOT NULL,
            completed_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS typing_indicators (
            peer_onion TEXT PRIMARY KEY,
            last_typing_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
        CREATE INDEX IF NOT EXISTS idx_group_messages_group ON group_messages(group_id);
        CREATE INDEX IF NOT EXISTS idx_group_messages_timestamp ON group_messages(timestamp);
        CREATE INDEX IF NOT EXISTS idx_reactions_message ON reactions(message_id);
        CREATE INDEX IF NOT EXISTS idx_call_log_peer ON call_log(peer_onion);
        CREATE INDEX IF NOT EXISTS idx_file_transfers_sender ON file_transfers(sender);
        CREATE INDEX IF NOT EXISTS idx_file_transfers_recipient ON file_transfers(recipient);
        CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);",
    )
    .context("Failed to create tables")?;

    let mut db = DB.lock().await;
    *db = Some(conn);
    Ok(())
}

pub async fn log_audit_event(event_type: &str, details: Option<&str>) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO audit_log (event_type, details, timestamp) VALUES (?1, ?2, ?3)",
        params![event_type, details, now],
    )
    .context("Failed to log audit event")?;
    Ok(())
}

pub async fn save_identity_with_keys(
    identity: &Identity,
    x25519_secret: &str,
    ed25519_secret: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO identity (id, public_key, onion_address, display_name, x25519_secret, ed25519_secret, created_at, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM identity WHERE id = 1), ?6), ?7)",
        params![
            identity.public_key,
            identity.onion_address,
            identity.display_name,
            x25519_secret,
            ed25519_secret,
            now,
            now,
        ],
    )
    .context("Failed to save identity")?;
    Ok(())
}

pub async fn save_identity(identity: &Identity) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE identity SET public_key = ?1, display_name = ?2, updated_at = ?3 WHERE id = 1",
        params![identity.public_key, identity.display_name, now],
    )
    .context("Failed to save identity")?;
    Ok(())
}

pub async fn load_identity() -> Result<Option<Identity>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result = conn
        .query_row(
            "SELECT public_key, onion_address, display_name FROM identity WHERE id = 1",
            [],
            |row| {
                Ok(Identity {
                    public_key: row.get(0)?,
                    onion_address: row.get(1)?,
                    display_name: row.get(2)?,
                })
            },
        )
        .optional()
        .context("Failed to load identity")?;
    Ok(result)
}

pub async fn load_x25519_secret() -> Result<Option<String>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result: Option<String> = conn
        .query_row(
            "SELECT x25519_secret FROM identity WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to load x25519 secret")?;
    Ok(result)
}

pub async fn load_ed25519_secret() -> Result<Option<String>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result: Option<String> = conn
        .query_row(
            "SELECT ed25519_secret FROM identity WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to load ed25519 secret")?;
    Ok(result)
}

pub async fn save_contact(contact: &Contact) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "INSERT OR REPLACE INTO contacts (id, onion_address, public_key, display_name, added_at, verified, blocked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            contact.id,
            contact.onion_address,
            contact.public_key,
            contact.display_name,
            contact.added_at,
            contact.verified as i32,
            contact.blocked as i32,
        ],
    )
    .context("Failed to save contact")?;
    Ok(())
}

pub async fn load_contacts() -> Result<Vec<Contact>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, display_name, public_key, onion_address, added_at, verified, blocked FROM contacts ORDER BY display_name",
    )?;
    let contacts = stmt
        .query_map([], |row| {
            Ok(Contact {
                id: row.get(0)?,
                display_name: row.get(1)?,
                public_key: row.get(2)?,
                onion_address: row.get(3)?,
                added_at: row.get(4)?,
                verified: row.get::<_, i32>(5)? != 0,
                blocked: row.get::<_, i32>(6)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load contacts")?;
    Ok(contacts)
}

pub async fn load_single_contact(onion_address: &str) -> Result<Option<Contact>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result = conn
        .query_row(
            "SELECT id, display_name, public_key, onion_address, added_at, verified, blocked FROM contacts WHERE onion_address = ?1",
            params![onion_address],
            |row| {
                Ok(Contact {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    public_key: row.get(2)?,
                    onion_address: row.get(3)?,
                    added_at: row.get(4)?,
                    verified: row.get::<_, i32>(5)? != 0,
                    blocked: row.get::<_, i32>(6)? != 0,
                })
            },
        )
        .optional()
        .context("Failed to load single contact")?;
    Ok(result)
}

pub async fn verify_contact(onion_address: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "UPDATE contacts SET verified = 1 WHERE onion_address = ?1",
        params![onion_address],
    )
    .context("Failed to verify contact")?;
    Ok(())
}

pub async fn mark_messages_read_for_peer(peer_onion: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let identity = load_identity().await?.map(|i| i.onion_address).unwrap_or_default();
    conn.execute(
        "UPDATE messages SET read = 1 WHERE sender = ?1 AND recipient = ?2 AND read = 0",
        params![peer_onion, identity],
    )
    .context("Failed to mark messages as read for peer")?;
    Ok(())
}

pub async fn set_message_ttl(message_id: &str, ttl_secs: u64) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let expires_at = Utc::now().timestamp() + ttl_secs as i64;
    conn.execute(
        "UPDATE messages SET expires_at = ?1 WHERE id = ?2",
        params![expires_at, message_id],
    )
    .context("Failed to set disappearing message TTL")?;
    Ok(())
}

pub async fn delete_contact(onion_address: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute("DELETE FROM contacts WHERE onion_address = ?1", params![onion_address])
        .context("Failed to delete contact")?;
    Ok(())
}

pub async fn block_contact(onion_address: &str, blocked: bool) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "UPDATE contacts SET blocked = ?1 WHERE onion_address = ?2",
        params![blocked as i32, onion_address],
    )
    .context("Failed to block/unblock contact")?;
    Ok(())
}

pub async fn save_message(message: &Message) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let type_str = message.message_type.as_str();
    let encrypted_int: i32 = if message.encrypted { 1 } else { 0 };
    let delivered_int: i32 = if message.delivered { 1 } else { 0 };
    let read_int: i32 = if message.read { 1 } else { 0 };
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num, reply_to, delivered, read, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            message.id,
            message.sender,
            message.recipient,
            message.content,
            message.timestamp,
            encrypted_int,
            type_str,
            message.sequence_num,
            message.reply_to,
            delivered_int,
            read_int,
            message.expires_at,
        ],
    )
    .context("Failed to save message")?;
    Ok(())
}

pub async fn save_message_with_encrypted(
    id: &str,
    sender: &str,
    recipient: &str,
    content: Option<&str>,
    encrypted_content: Option<&[u8]>,
    timestamp: i64,
    message_type: &str,
    _status: &str,
    sequence_num: Option<i64>,
    reply_to: Option<&str>,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let encrypted_int: i32 = if encrypted_content.is_some() { 1 } else { 0 };
    conn.execute(
        "INSERT OR REPLACE INTO messages (id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num, reply_to)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, sender, recipient, content, timestamp, encrypted_int, message_type, sequence_num, reply_to],
    )
    .context("Failed to save message with encrypted content")?;
    Ok(())
}

pub async fn load_messages(peer: &str, limit: i64, offset: i64) -> Result<Vec<Message>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num, reply_to, delivered, read, expires_at
         FROM messages
         WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)
         ORDER BY timestamp ASC
         LIMIT ?3 OFFSET ?4",
    )?;
    let identity = load_identity().await?.map(|i| i.onion_address).unwrap_or_default();
    let messages = stmt
        .query_map(params![peer, identity, limit, offset], |row| {
            let type_str: String = row.get(6)?;
            let message_type = MessageType::from_str(&type_str);
            Ok(Message {
                id: row.get(0)?,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                encrypted: row.get::<_, i32>(5)? != 0,
                message_type,
                sequence_num: row.get(7)?,
                reply_to: row.get(8)?,
                delivered: row.get::<_, i32>(9)? != 0,
                read: row.get::<_, i32>(10)? != 0,
                expires_at: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load messages")?;
    Ok(messages)
}

pub async fn get_message_count(peer: &str) -> Result<i64> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let identity = load_identity().await?.map(|i| i.onion_address).unwrap_or_default();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)",
        params![peer, identity],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub async fn save_session(
    peer_onion: &str,
    session_key: &str,
    message_keys: Option<&str>,
    expires_at: Option<i64>,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO sessions (peer_onion, session_key, message_keys, created_at, updated_at, expires_at)
         VALUES (?1, ?2, ?3, COALESCE((SELECT created_at FROM sessions WHERE peer_onion = ?1), ?4), ?5, ?6)",
        params![peer_onion, session_key, message_keys, now, now, expires_at],
    )
    .context("Failed to save session")?;
    Ok(())
}

pub async fn load_session(peer_onion: &str) -> Result<Option<(String, Option<String>, Option<i64>)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result = conn
        .query_row(
            "SELECT session_key, message_keys, expires_at FROM sessions WHERE peer_onion = ?1",
            params![peer_onion],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .context("Failed to load session")?;
    Ok(result)
}

pub async fn delete_all_data() -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute_batch(
        "DELETE FROM typing_indicators;
         DELETE FROM settings;
         DELETE FROM file_transfers;
         DELETE FROM call_log;
         DELETE FROM reactions;
         DELETE FROM group_messages;
         DELETE FROM group_members;
         DELETE FROM groups;
         DELETE FROM audit_log;
         DELETE FROM sessions;
         DELETE FROM messages;
         DELETE FROM contacts;
         DELETE FROM identity;",
    )
    .context("Failed to delete all data")?;
    log_audit_event("data_deleted", Some("All data wiped")).await?;
    Ok(())
}

// ── Groups ──────────────────────────────────────────────────────────────────

pub async fn save_group(
    id: &str,
    name: &str,
    description: Option<&str>,
    avatar: Option<&[u8]>,
    created_by: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO groups (id, name, description, avatar, created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM groups WHERE id = ?1), ?6), ?7)",
        params![id, name, description, avatar, created_by, now, now],
    )
    .context("Failed to save group")?;
    Ok(())
}

pub async fn load_groups() -> Result<Vec<(String, String, Option<String>, Option<Vec<u8>>, String, i64, i64)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, avatar, created_by, created_at, updated_at FROM groups ORDER BY updated_at DESC",
    )?;
    let groups = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load groups")?;
    Ok(groups)
}

pub async fn load_group_members(group_id: &str) -> Result<Vec<(String, String, Option<String>, Option<String>, String, i64)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT group_id, onion_address, public_key, display_name, role, joined_at FROM group_members WHERE group_id = ?1 ORDER BY joined_at",
    )?;
    let members = stmt
        .query_map(params![group_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load group members")?;
    Ok(members)
}

pub async fn add_group_member(
    group_id: &str,
    onion_address: &str,
    public_key: Option<&str>,
    display_name: Option<&str>,
    role: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO group_members (group_id, onion_address, public_key, display_name, role, joined_at)
         VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT joined_at FROM group_members WHERE group_id = ?1 AND onion_address = ?2), ?6))",
        params![group_id, onion_address, public_key, display_name, role, now],
    )
    .context("Failed to add group member")?;
    Ok(())
}

pub async fn remove_group_member(group_id: &str, onion_address: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "DELETE FROM group_members WHERE group_id = ?1 AND onion_address = ?2",
        params![group_id, onion_address],
    )
    .context("Failed to remove group member")?;
    Ok(())
}

// ── Group Messages ──────────────────────────────────────────────────────────

pub async fn save_group_message(
    id: &str,
    group_id: &str,
    sender: &str,
    content: Option<&str>,
    encrypted_content: Option<&[u8]>,
    timestamp: i64,
    message_type: &str,
    sequence_num: Option<i64>,
    reply_to: Option<&str>,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "INSERT OR REPLACE INTO group_messages (id, group_id, sender, content, encrypted_content, timestamp, message_type, sequence_num, reply_to)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, group_id, sender, content, encrypted_content, timestamp, message_type, sequence_num, reply_to],
    )
    .context("Failed to save group message")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE groups SET updated_at = ?1 WHERE id = ?2",
        params![now, group_id],
    )
    .context("Failed to update group timestamp")?;
    Ok(())
}

pub async fn load_group_messages(
    group_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, Option<String>, Option<Vec<u8>>, i64, String, Option<i64>, Option<String>)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, group_id, sender, content, encrypted_content, timestamp, message_type, sequence_num, reply_to
         FROM group_messages
         WHERE group_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let messages = stmt
        .query_map(params![group_id, limit, offset], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load group messages")?;
    Ok(messages)
}

// ── Reactions ───────────────────────────────────────────────────────────────

pub async fn save_reaction(
    id: &str,
    message_id: &str,
    sender: &str,
    emoji: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO reactions (id, message_id, sender, emoji, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, message_id, sender, emoji, now],
    )
    .context("Failed to save reaction")?;
    Ok(())
}

pub async fn load_reactions(message_id: &str) -> Result<Vec<(String, String, String, String, i64)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, message_id, sender, emoji, timestamp FROM reactions WHERE message_id = ?1 ORDER BY timestamp",
    )?;
    let reactions = stmt
        .query_map(params![message_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load reactions")?;
    Ok(reactions)
}

pub async fn remove_reaction(message_id: &str, sender: &str, emoji: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "DELETE FROM reactions WHERE message_id = ?1 AND sender = ?2 AND emoji = ?3",
        params![message_id, sender, emoji],
    )
    .context("Failed to remove reaction")?;
    Ok(())
}

// ── Call Log ────────────────────────────────────────────────────────────────

pub async fn save_call_log(
    id: &str,
    peer_onion: &str,
    call_type: &str,
    direction: &str,
    started_at: i64,
    ended_at: Option<i64>,
    duration_secs: Option<i64>,
    status: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "INSERT OR REPLACE INTO call_log (id, peer_onion, call_type, direction, started_at, ended_at, duration_secs, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, peer_onion, call_type, direction, started_at, ended_at, duration_secs, status],
    )
    .context("Failed to save call log")?;
    Ok(())
}

pub async fn load_call_log(limit: i64, offset: i64) -> Result<Vec<(String, String, String, String, i64, Option<i64>, Option<i64>, String)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let mut stmt = conn.prepare(
        "SELECT id, peer_onion, call_type, direction, started_at, ended_at, duration_secs, status
         FROM call_log ORDER BY started_at DESC LIMIT ?1 OFFSET ?2",
    )?;
    let entries = stmt
        .query_map(params![limit, offset], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load call log")?;
    Ok(entries)
}

// ── File Transfers ──────────────────────────────────────────────────────────

pub async fn save_file_transfer(
    id: &str,
    sender: &str,
    recipient: &str,
    filename: &str,
    mime_type: Option<&str>,
    size: Option<i64>,
    encryption_key: Option<&[u8]>,
    chunk_dir: Option<&str>,
    status: &str,
) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO file_transfers (id, sender, recipient, filename, mime_type, size, encryption_key, chunk_dir, status, started_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
        params![id, sender, recipient, filename, mime_type, size, encryption_key, chunk_dir, status, now],
    )
    .context("Failed to save file transfer")?;
    Ok(())
}

pub async fn update_file_transfer_status(id: &str, status: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let completed_at = if status == "completed" || status == "failed" {
        Some(Utc::now().timestamp())
    } else {
        None
    };
    conn.execute(
        "UPDATE file_transfers SET status = ?1, completed_at = COALESCE(?2, completed_at) WHERE id = ?3",
        params![status, completed_at, id],
    )
    .context("Failed to update file transfer status")?;
    Ok(())
}

pub async fn load_file_transfers(
    peer: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<(String, String, String, String, Option<String>, Option<i64>, Option<Vec<u8>>, Option<String>, String, i64, Option<i64>)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let identity = load_identity().await?.map(|i| i.onion_address).unwrap_or_default();
    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match peer {
        Some(p) => (
            "SELECT id, sender, recipient, filename, mime_type, size, encryption_key, chunk_dir, status, started_at, completed_at
             FROM file_transfers
             WHERE (sender = ?1 OR recipient = ?1) AND (sender = ?2 OR recipient = ?2)
             ORDER BY started_at DESC LIMIT ?3 OFFSET ?4"
                .to_string(),
            vec![
                Box::new(p.to_string()),
                Box::new(identity),
                Box::new(limit),
                Box::new(offset),
            ],
        ),
        None => (
            "SELECT id, sender, recipient, filename, mime_type, size, encryption_key, chunk_dir, status, started_at, completed_at
             FROM file_transfers
             WHERE sender = ?1 OR recipient = ?1
             ORDER BY started_at DESC LIMIT ?2 OFFSET ?3"
                .to_string(),
            vec![
                Box::new(identity),
                Box::new(limit),
                Box::new(offset),
            ],
        ),
    };
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let transfers = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to load file transfers")?;
    Ok(transfers)
}

// ── Settings ────────────────────────────────────────────────────────────────

pub async fn get_setting(key: &str) -> Result<Option<String>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
        .optional()
        .context("Failed to get setting")?;
    Ok(result)
}

pub async fn set_setting(key: &str, value: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
        params![key, value, now],
    )
    .context("Failed to set setting")?;
    Ok(())
}

// ── Typing Indicators ───────────────────────────────────────────────────────

pub async fn update_typing_indicator(peer_onion: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO typing_indicators (peer_onion, last_typing_at) VALUES (?1, ?2)",
        params![peer_onion, now],
    )
    .context("Failed to update typing indicator")?;
    Ok(())
}

pub async fn get_typing_indicator(peer_onion: &str) -> Result<Option<i64>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let result: Option<i64> = conn
        .query_row(
            "SELECT last_typing_at FROM typing_indicators WHERE peer_onion = ?1",
            params![peer_onion],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to get typing indicator")?;
    Ok(result)
}

// ── Message Status ──────────────────────────────────────────────────────────

pub async fn mark_message_delivered(message_id: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "UPDATE messages SET delivered = 1 WHERE id = ?1",
        params![message_id],
    )
    .context("Failed to mark message as delivered")?;
    Ok(())
}

pub async fn mark_message_read(message_id: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute(
        "UPDATE messages SET read = 1 WHERE id = ?1",
        params![message_id],
    )
    .context("Failed to mark message as read")?;
    Ok(())
}

// ── Search ──────────────────────────────────────────────────────────────────

pub async fn search_messages(query: &str, limit: i64) -> Result<Vec<Message>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, sender, recipient, content, timestamp, encrypted, message_type, sequence_num, reply_to, delivered, read, expires_at
         FROM messages
         WHERE content LIKE ?1
         ORDER BY timestamp DESC
         LIMIT ?2",
    )?;
    let messages = stmt
        .query_map(params![pattern, limit], |row| {
            let type_str: String = row.get(6)?;
            let message_type = MessageType::from_str(&type_str);
            Ok(Message {
                id: row.get(0)?,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
                encrypted: row.get::<_, i32>(5)? != 0,
                message_type,
                sequence_num: row.get(7)?,
                reply_to: row.get(8)?,
                delivered: row.get::<_, i32>(9)? != 0,
                read: row.get::<_, i32>(10)? != 0,
                expires_at: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to search messages")?;
    Ok(messages)
}

pub async fn search_group_messages(
    group_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<(String, String, String, Option<String>, Option<Vec<u8>>, i64, String, Option<i64>, Option<String>)>> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, group_id, sender, content, encrypted_content, timestamp, message_type, sequence_num, reply_to
         FROM group_messages
         WHERE group_id = ?1 AND content LIKE ?2
         ORDER BY timestamp DESC
         LIMIT ?3",
    )?;
    let messages = stmt
        .query_map(params![group_id, pattern, limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to search group messages")?;
    Ok(messages)
}

// ── Delete Messages ─────────────────────────────────────────────────────────

pub async fn delete_message(message_id: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute("DELETE FROM messages WHERE id = ?1", params![message_id])
        .context("Failed to delete message")?;
    conn.execute("DELETE FROM reactions WHERE message_id = ?1", params![message_id])
        .context("Failed to delete message reactions")?;
    Ok(())
}

pub async fn delete_group_message(message_id: &str) -> Result<()> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    conn.execute("DELETE FROM group_messages WHERE id = ?1", params![message_id])
        .context("Failed to delete group message")?;
    conn.execute("DELETE FROM reactions WHERE message_id = ?1", params![message_id])
        .context("Failed to delete group message reactions")?;
    Ok(())
}

// ── Cleanup ─────────────────────────────────────────────────────────────────

pub async fn cleanup_expired_messages(max_age_secs: i64) -> Result<usize> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let cutoff = Utc::now().timestamp() - max_age_secs;
    let deleted = conn
        .execute("DELETE FROM messages WHERE timestamp < ?1", params![cutoff])
        .context("Failed to cleanup expired messages")?;
    Ok(deleted)
}

pub async fn cleanup_typing_indicators(max_age_secs: i64) -> Result<usize> {
    let db = DB.lock().await;
    let conn = db.as_ref().context("Database not initialized")?;
    let cutoff = Utc::now().timestamp() - max_age_secs;
    let deleted = conn
        .execute("DELETE FROM typing_indicators WHERE last_typing_at < ?1", params![cutoff])
        .context("Failed to cleanup typing indicators")?;
    Ok(deleted)
}
