//! Long-term memory: SQLite storage, FTS5 recall, summaries and context assembly.
//!
//! Storage layout (see `migrations/0001_init.sql`):
//! `conversations` / `messages` / `summaries` / `facts` plus FTS5 indexes.
//! Recall strategy: recent turns verbatim + latest summary + top-K facts and
//! messages by keyword, all under a token budget.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::llm::ChatMessage;

/// Rough token estimate that works for mixed CJK/Latin text.
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as f32;
    // Latin ~4 chars/token, CJK ~1.5 chars/token; 2.5 is a safe middle ground.
    (chars / 2.5).ceil() as u32
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub persona_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredMessage {
    pub id: i64,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tokens: u32,
    pub created_at: i64,
}

impl StoredMessage {
    pub fn into_chat_message(self) -> Option<ChatMessage> {
        let role = match self.role.as_str() {
            "user" => crate::llm::Role::User,
            "assistant" => crate::llm::Role::Assistant,
            _ => return None,
        };
        Some(ChatMessage {
            role,
            content: self.content,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: i64,
    pub conversation_id: String,
    pub through_message_id: i64,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub id: i64,
    pub persona_id: String,
    pub key: String,
    pub value: String,
    pub confidence: f32,
    pub source_message_id: Option<i64>,
    pub updated_at: i64,
}

/// Input for [`MemoryStore::append_message`].
#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    pub conversation_id: &'a str,
    pub role: crate::llm::Role,
    pub content: &'a str,
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    pub tokens: Option<u32>,
}

/// Memory knobs, normally taken from the persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub window_turns: u32,
    pub long_term: bool,
    pub summarize_after_turns: u32,
    /// Token budget for the assembled memory block.
    pub budget_tokens: u32,
    /// How many recalled snippets to include.
    pub recall_limit: u32,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_turns: 12,
            long_term: true,
            summarize_after_turns: 20,
            budget_tokens: 2000,
            recall_limit: 5,
        }
    }
}

/// Everything memory contributes to one model call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryContext {
    pub recent: Vec<ChatMessage>,
    pub summary: Option<String>,
    pub facts: Vec<Fact>,
    pub recalled: Vec<StoredMessage>,
    pub estimated_tokens: u32,
}

impl MemoryContext {
    /// Render the memory block appended to the system prompt.
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        if let Some(summary) = &self.summary {
            parts.push(format!("【之前的对话摘要】\n{summary}"));
        }
        if !self.facts.is_empty() {
            let lines = self
                .facts
                .iter()
                .map(|f| format!("- {}：{}", f.key, f.value))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("【你记得关于用户的事】\n{lines}"));
        }
        if !self.recalled.is_empty() {
            let lines = self
                .recalled
                .iter()
                .map(|m| {
                    let who = if m.role == "user" { "用户" } else { "你" };
                    format!("- {who}：{}", m.content.replace('\n', " "))
                })
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("【可能相关的历史片段】\n{lines}"));
        }
        parts.join("\n\n")
    }
}

/// SQLite-backed memory store. Cheap to clone via `Arc` at the call site.
pub struct MemoryStore {
    pub(crate) conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl MemoryStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .map_err(|e| Error::memory(format!("cannot open {}: {e}", path.display())))?;
        let store = Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::memory(format!("cannot open in-memory db: {e}")))?;
        let store = Self {
            conn: Mutex::new(conn),
            path: None,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Apply the schema. Idempotent.
    pub fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::memory(format!("schema migration failed: {e}")))?;
        Ok(())
    }

    pub fn create_conversation(&self, persona_id: &str, title: Option<&str>) -> Result<Conversation> {
        let conn = self.conn.lock();
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let title = title.unwrap_or("新对话");
        conn.execute(
            "INSERT INTO conversations (id, persona_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![id, persona_id, title, now],
        )
        .map_err(db_err)?;
        Ok(Conversation {
            id,
            persona_id: persona_id.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
            message_count: 0,
        })
    }

    pub fn conversations(&self, persona_id: Option<&str>) -> Result<Vec<Conversation>> {
        let conn = self.conn.lock();
        let mut out = Vec::new();
        let sql = "SELECT c.id, c.persona_id, c.title, c.created_at, c.updated_at,
                          (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id)
                   FROM conversations c
                   WHERE (?1 IS NULL OR c.persona_id = ?1)
                   ORDER BY c.updated_at DESC";
        let mut stmt = conn.prepare(sql).map_err(db_err)?;
        let rows = stmt
            .query_map(rusqlite::params![persona_id], |row| {
                Ok(Conversation {
                    id: row.get(0)?,
                    persona_id: row.get(1)?,
                    title: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    message_count: row.get(5)?,
                })
            })
            .map_err(db_err)?;
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM messages WHERE conversation_id = ?1", [id])
            .map_err(db_err)?;
        conn.execute("DELETE FROM summaries WHERE conversation_id = ?1", [id])
            .map_err(db_err)?;
        conn.execute("DELETE FROM conversations WHERE id = ?1", [id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn rename_conversation(&self, id: &str, title: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE conversations SET title = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, title, now_ms()],
        )
        .map_err(db_err)?;
        Ok(())
    }

    pub fn append_message(&self, msg: NewMessage<'_>) -> Result<StoredMessage> {
        let conn = self.conn.lock();
        let now = now_ms();
        let tokens = msg.tokens.unwrap_or_else(|| estimate_tokens(msg.content));
        let role = match msg.role {
            crate::llm::Role::User => "user",
            crate::llm::Role::Assistant => "assistant",
            crate::llm::Role::System => "system",
        };
        conn.execute(
            "INSERT INTO messages (conversation_id, role, content, provider, model, tokens, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                msg.conversation_id,
                role,
                msg.content,
                msg.provider,
                msg.model,
                tokens,
                now
            ],
        )
        .map_err(db_err)?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
            rusqlite::params![msg.conversation_id, now],
        )
        .map_err(db_err)?;
        Ok(StoredMessage {
            id,
            conversation_id: msg.conversation_id.to_string(),
            role: role.to_string(),
            content: msg.content.to_string(),
            provider: msg.provider.map(str::to_string),
            model: msg.model.map(str::to_string),
            tokens,
            created_at: now,
        })
    }

    /// Most recent messages, returned oldest-first.
    pub fn messages(&self, conversation_id: &str, limit: usize) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, role, content, provider, model, tokens, created_at
                 FROM messages WHERE conversation_id = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(rusqlite::params![conversation_id, limit as i64], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    tokens: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn latest_summary(&self, conversation_id: &str) -> Result<Option<Summary>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, conversation_id, through_message_id, content, created_at
                 FROM summaries WHERE conversation_id = ?1 ORDER BY through_message_id DESC LIMIT 1",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query(rusqlite::params![conversation_id]).map_err(db_err)?;
        match rows.next().map_err(db_err)? {
            Some(row) => Ok(Some(Summary {
                id: row.get(0).map_err(db_err)?,
                conversation_id: row.get(1).map_err(db_err)?,
                through_message_id: row.get(2).map_err(db_err)?,
                content: row.get(3).map_err(db_err)?,
                created_at: row.get(4).map_err(db_err)?,
            })),
            None => Ok(None),
        }
    }

    pub fn upsert_summary(
        &self,
        conversation_id: &str,
        through_message_id: i64,
        content: &str,
    ) -> Result<Summary> {
        let conn = self.conn.lock();
        let now = now_ms();
        conn.execute(
            "INSERT INTO summaries (conversation_id, through_message_id, content, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(conversation_id) DO UPDATE SET
               through_message_id = excluded.through_message_id,
               content = excluded.content,
               created_at = excluded.created_at",
            rusqlite::params![conversation_id, through_message_id, content, now],
        )
        .map_err(db_err)?;
        Ok(Summary {
            id: conn.last_insert_rowid(),
            conversation_id: conversation_id.to_string(),
            through_message_id,
            content: content.to_string(),
            created_at: now,
        })
    }

    /// User turns recorded after the latest summary.
    pub fn unsummarized_turns(&self, conversation_id: &str) -> Result<u32> {
        let conn = self.conn.lock();
        let through: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(through_message_id), 0) FROM summaries WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND id > ?2 AND role = 'user'",
                rusqlite::params![conversation_id, through],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        Ok(count as u32)
    }

    pub fn upsert_fact(
        &self,
        persona_id: &str,
        key: &str,
        value: &str,
        confidence: f32,
        source_message_id: Option<i64>,
    ) -> Result<Fact> {
        let conn = self.conn.lock();
        let now = now_ms();
        conn.execute(
            "INSERT INTO facts (persona_id, key, value, confidence, source_message_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(persona_id, key) DO UPDATE SET
               value = excluded.value,
               confidence = excluded.confidence,
               source_message_id = excluded.source_message_id,
               updated_at = excluded.updated_at",
            rusqlite::params![persona_id, key, value, confidence, source_message_id, now],
        )
        .map_err(db_err)?;
        let id: i64 = conn
            .query_row(
                "SELECT id FROM facts WHERE persona_id = ?1 AND key = ?2",
                rusqlite::params![persona_id, key],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        Ok(Fact {
            id,
            persona_id: persona_id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
            confidence,
            source_message_id,
            updated_at: now,
        })
    }

    pub fn list_facts(&self, persona_id: &str) -> Result<Vec<Fact>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, persona_id, key, value, confidence, source_message_id, updated_at
                 FROM facts WHERE persona_id = ?1 ORDER BY updated_at DESC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([persona_id], |row| {
                Ok(Fact {
                    id: row.get(0)?,
                    persona_id: row.get(1)?,
                    key: row.get(2)?,
                    value: row.get(3)?,
                    confidence: row.get(4)?,
                    source_message_id: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }

    pub fn delete_fact(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM facts WHERE id = ?1", [id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn clear_persona_memory(&self, persona_id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM facts WHERE persona_id = ?1", [persona_id])
            .map_err(db_err)?;
        conn.execute(
            "DELETE FROM messages WHERE conversation_id IN (SELECT id FROM conversations WHERE persona_id = ?1)",
            [persona_id],
        )
        .map_err(db_err)?;
        conn.execute(
            "DELETE FROM summaries WHERE conversation_id IN (SELECT id FROM conversations WHERE persona_id = ?1)",
            [persona_id],
        )
        .map_err(db_err)?;
        conn.execute("DELETE FROM conversations WHERE persona_id = ?1", [persona_id])
            .map_err(db_err)?;
        Ok(())
    }

    /// Full-text recall over this persona's messages.
    ///
    /// Uses the FTS5 trigram index (which handles CJK substrings) for queries
    /// of three or more characters and a `LIKE` fallback for shorter ones.
    pub fn search(&self, persona_id: &str, query: &str, limit: usize) -> Result<Vec<StoredMessage>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (sql, param): (String, String) = if trimmed.chars().count() >= 3 {
            (
                "SELECT m.id, m.conversation_id, m.role, m.content, m.provider, m.model, m.tokens, m.created_at
                 FROM messages_fts f
                 JOIN messages m ON m.id = f.rowid
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE messages_fts MATCH ?1 AND c.persona_id = ?2
                 ORDER BY rank LIMIT ?3"
                    .to_string(),
                fts_query(trimmed),
            )
        } else {
            (
                "SELECT m.id, m.conversation_id, m.role, m.content, m.provider, m.model, m.tokens, m.created_at
                 FROM messages m
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE m.content LIKE ?1 AND c.persona_id = ?2
                 ORDER BY m.id DESC LIMIT ?3"
                    .to_string(),
                format!("%{}%", trimmed.replace('%', "\\%")),
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(rusqlite::params![param, persona_id, limit as i64], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    tokens: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }

    /// Assemble the memory block for a new user message.
    pub fn build_context(
        &self,
        persona_id: &str,
        conversation_id: &str,
        cfg: &MemoryConfig,
        query: &str,
    ) -> Result<MemoryContext> {
        if !cfg.enabled {
            return Ok(MemoryContext::default());
        }
        let mut ctx = MemoryContext::default();
        let mut budget = cfg.budget_tokens;

        let recent_limit = (cfg.window_turns.max(1) * 2) as usize;
        let recent = self.messages(conversation_id, recent_limit)?;
        for msg in recent.into_iter().filter(|m| m.role != "system") {
            let cost = estimate_tokens(&msg.content);
            if cost > budget && !ctx.recent.is_empty() {
                break;
            }
            budget = budget.saturating_sub(cost);
            if let Some(chat) = msg.into_chat_message() {
                ctx.recent.push(chat);
            }
        }

        if cfg.long_term {
            if let Some(summary) = self.latest_summary(conversation_id)? {
                let cost = estimate_tokens(&summary.content);
                if cost <= budget {
                    budget -= cost;
                    ctx.summary = Some(summary.content);
                }
            }
            let facts = self.list_facts(persona_id)?;
            for fact in facts {
                let cost = estimate_tokens(&fact.value) + estimate_tokens(&fact.key);
                if cost > budget {
                    break;
                }
                budget -= cost;
                ctx.facts.push(fact);
            }
            if cfg.recall_limit > 0 {
                for hit in self.search(persona_id, query, cfg.recall_limit as usize)? {
                    let cost = estimate_tokens(&hit.content);
                    if cost > budget {
                        break;
                    }
                    if ctx.recent.iter().any(|r| r.content == hit.content) {
                        continue;
                    }
                    budget -= cost;
                    ctx.recalled.push(hit);
                }
            }
        }

        ctx.estimated_tokens = cfg.budget_tokens - budget;
        Ok(ctx)
    }
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::memory(e.to_string())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Turn free text into a safe FTS5 MATCH query (OR of quoted terms).
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS conversations (
    id          TEXT PRIMARY KEY,
    persona_id  TEXT NOT NULL,
    title       TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conversations_persona ON conversations(persona_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS messages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,
    content         TEXT NOT NULL,
    provider        TEXT,
    model           TEXT,
    tokens          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id, id);

CREATE TABLE IF NOT EXISTS summaries (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id    TEXT NOT NULL UNIQUE REFERENCES conversations(id) ON DELETE CASCADE,
    through_message_id INTEGER NOT NULL,
    content            TEXT NOT NULL,
    created_at         INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS facts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    persona_id        TEXT NOT NULL,
    key               TEXT NOT NULL,
    value             TEXT NOT NULL,
    confidence        REAL NOT NULL DEFAULT 1.0,
    source_message_id INTEGER,
    updated_at        INTEGER NOT NULL,
    UNIQUE(persona_id, key)
);
CREATE INDEX IF NOT EXISTS idx_facts_persona ON facts(persona_id, updated_at DESC);

-- trigram tokenizer: enables substring recall for CJK text (unicode61 would
-- index a whole Chinese sentence as a single token).
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content, content='messages', content_rowid='id', tokenize='trigram'
);
CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(
    key, value, content='facts', content_rowid='id', tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;
CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
    INSERT INTO facts_fts(rowid, key, value) VALUES (new.id, new.key, new.value);
END;
CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, key, value) VALUES ('delete', old.id, old.key, old.value);
END;
CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, key, value) VALUES ('delete', old.id, old.key, old.value);
    INSERT INTO facts_fts(rowid, key, value) VALUES (new.id, new.key, new.value);
END;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> MemoryStore {
        MemoryStore::open_in_memory().unwrap()
    }

    #[test]
    fn appends_and_reads_messages() {
        let s = store();
        let c = s.create_conversation("default", Some("测试")).unwrap();
        s.append_message(NewMessage {
            conversation_id: &c.id,
            role: crate::llm::Role::User,
            content: "我喜欢喝美式咖啡",
            provider: None,
            model: None,
            tokens: None,
        })
        .unwrap();
        s.append_message(NewMessage {
            conversation_id: &c.id,
            role: crate::llm::Role::Assistant,
            content: "记住了",
            provider: Some("deepseek"),
            model: None,
            tokens: None,
        })
        .unwrap();
        let msgs = s.messages(&c.id, 10).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(s.conversations(None).unwrap()[0].message_count, 2);
    }

    #[test]
    fn fts_recall_finds_earlier_fact() {
        let s = store();
        let c = s.create_conversation("default", None).unwrap();
        s.append_message(NewMessage {
            conversation_id: &c.id,
            role: crate::llm::Role::User,
            content: "我的猫叫鼠标",
            provider: None,
            model: None,
            tokens: None,
        })
        .unwrap();
        let hits = s.search("default", "猫", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("鼠标"));
        // Another persona must not see it.
        assert!(s.search("other", "猫", 5).unwrap().is_empty());
    }

    #[test]
    fn summary_tracking_counts_unsummarized_turns() {
        let s = store();
        let c = s.create_conversation("default", None).unwrap();
        for i in 0..3 {
            s.append_message(NewMessage {
                conversation_id: &c.id,
                role: crate::llm::Role::User,
                content: &format!("问题 {i}"),
                provider: None,
                model: None,
                tokens: None,
            })
            .unwrap();
        }
        assert_eq!(s.unsummarized_turns(&c.id).unwrap(), 3);
        let msgs = s.messages(&c.id, 10).unwrap();
        s.upsert_summary(&c.id, msgs.last().unwrap().id, "用户问了三个问题")
            .unwrap();
        assert_eq!(s.unsummarized_turns(&c.id).unwrap(), 0);
        assert!(s.latest_summary(&c.id).unwrap().is_some());
    }

    #[test]
    fn facts_upsert_and_clear() {
        let s = store();
        s.upsert_fact("default", "咖啡", "美式", 1.0, None).unwrap();
        s.upsert_fact("default", "咖啡", "拿铁", 0.9, None).unwrap();
        let facts = s.list_facts("default").unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, "拿铁");
        s.delete_fact(facts[0].id).unwrap();
        assert!(s.list_facts("default").unwrap().is_empty());
        s.upsert_fact("default", "猫", "鼠标", 1.0, None).unwrap();
        s.clear_persona_memory("default").unwrap();
        assert!(s.list_facts("default").unwrap().is_empty());
    }

    #[test]
    fn context_respects_window_and_budget() {
        let s = store();
        let c = s.create_conversation("default", None).unwrap();
        s.append_message(NewMessage {
            conversation_id: &c.id,
            role: crate::llm::Role::User,
            content: "我的猫叫鼠标",
            provider: None,
            model: None,
            tokens: None,
        })
        .unwrap();
        for i in 0..30 {
            s.append_message(NewMessage {
                conversation_id: &c.id,
                role: crate::llm::Role::User,
                content: &format!("消息 {i}"),
                provider: None,
                model: None,
                tokens: None,
            })
            .unwrap();
        }
        let cfg = MemoryConfig {
            window_turns: 3,
            ..MemoryConfig::default()
        };
        let ctx = s.build_context("default", &c.id, &cfg, "猫").unwrap();
        assert_eq!(ctx.recent.len(), 6);
        assert!(ctx.estimated_tokens <= cfg.budget_tokens);
        assert!(
            ctx.recalled.iter().any(|m| m.content.contains("鼠标")),
            "the early cat fact should be recalled: {:?}",
            ctx.recalled
        );
    }

    #[test]
    fn disabled_memory_returns_empty_context() {
        let s = store();
        let c = s.create_conversation("default", None).unwrap();
        let cfg = MemoryConfig {
            enabled: false,
            ..MemoryConfig::default()
        };
        let ctx = s.build_context("default", &c.id, &cfg, "").unwrap();
        assert!(ctx.recent.is_empty() && ctx.facts.is_empty() && ctx.summary.is_none());
    }

    #[test]
    fn estimates_tokens_for_mixed_text() {
        assert!(estimate_tokens("hello world") >= 4);
        assert!(estimate_tokens("你好，世界") >= 2);
        assert_eq!(estimate_tokens(""), 0);
    }
}
