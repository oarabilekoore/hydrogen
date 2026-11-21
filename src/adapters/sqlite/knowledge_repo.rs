use crate::error::{HydrogenError, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageRecord {
    pub id: i64,
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub sources_json: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationRecord {
    pub conversation: ConversationSummary,
    pub messages: Vec<MessageRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    pub kind: String,
    pub id: i64,
    pub title: String,
    pub excerpt: String,
    pub updated_at: i64,
    pub url: Option<String>,
    pub conversation_id: Option<i64>,
    pub score: f64,
}

pub struct KnowledgeRepository {
    db_path: PathBuf,
}

impl KnowledgeRepository {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let repo = Self { db_path: path.to_path_buf() };
        let conn = repo.connect()?;
        repo.migrate(&conn)?;
        Ok(repo)
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(Duration::from_secs(15))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Ok(conn)
    }

    fn migrate(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL DEFAULT 'New conversation',
                model TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                archived INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_conversations_updated
                ON conversations(archived, updated_at DESC);
             CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK(role IN ('system','user','assistant','tool')),
                content TEXT NOT NULL,
                model TEXT,
                sources_json TEXT,
                content_hash TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE INDEX IF NOT EXISTS idx_messages_conversation
                ON messages(conversation_id, id);
             CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                conversation_id UNINDEXED, message_id UNINDEXED, role UNINDEXED,
                content, tokenize='porter unicode61'
             );
             CREATE TABLE IF NOT EXISTS message_attachments (
                id INTEGER PRIMARY KEY,
                message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
                local_path TEXT NOT NULL,
                content_hash TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS web_pages (
                id INTEGER PRIMARY KEY,
                canonical_url TEXT NOT NULL UNIQUE,
                url TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL DEFAULT '',
                published_at TEXT NOT NULL DEFAULT '',
                site_name TEXT NOT NULL DEFAULT '',
                readable_text TEXT NOT NULL DEFAULT '',
                saved_html TEXT NOT NULL DEFAULT '',
                content_hash TEXT NOT NULL DEFAULT '',
                first_seen INTEGER NOT NULL DEFAULT (unixepoch()),
                last_seen INTEGER NOT NULL DEFAULT (unixepoch()),
                visits INTEGER NOT NULL DEFAULT 1,
                manually_saved INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_web_pages_last_seen
                ON web_pages(last_seen DESC);
             CREATE VIRTUAL TABLE IF NOT EXISTS web_pages_fts USING fts5(
                page_id UNINDEXED, title, author, site_name, url UNINDEXED,
                body, tokenize='porter unicode61'
             );
             CREATE TABLE IF NOT EXISTS web_page_versions (
                id INTEGER PRIMARY KEY,
                page_id INTEGER NOT NULL REFERENCES web_pages(id) ON DELETE CASCADE,
                content_hash TEXT NOT NULL,
                readable_text TEXT NOT NULL,
                captured_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(page_id, content_hash)
             );
             CREATE TABLE IF NOT EXISTS web_visits (
                id INTEGER PRIMARY KEY,
                page_id INTEGER NOT NULL REFERENCES web_pages(id) ON DELETE CASCADE,
                visited_at INTEGER NOT NULL DEFAULT (unixepoch()),
                source TEXT NOT NULL DEFAULT 'extension'
             );
             CREATE INDEX IF NOT EXISTS idx_web_visits_page
                ON web_visits(page_id, visited_at DESC);
             CREATE TABLE IF NOT EXISTS capture_sources (
                id INTEGER PRIMARY KEY,
                source_key TEXT NOT NULL UNIQUE,
                source_type TEXT NOT NULL,
                last_seen INTEGER NOT NULL DEFAULT (unixepoch()),
                metadata_json TEXT NOT NULL DEFAULT '{}'
             );
             CREATE TABLE IF NOT EXISTS knowledge_collections (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE TABLE IF NOT EXISTS collection_items (
                collection_id INTEGER NOT NULL REFERENCES knowledge_collections(id) ON DELETE CASCADE,
                item_type TEXT NOT NULL,
                item_id INTEGER NOT NULL,
                added_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY(collection_id, item_type, item_id)
             );
             PRAGMA user_version = 7;
             COMMIT;"
        )?;
        Ok(())
    }

    pub fn create_conversation(&self, title: &str, model: &str) -> Result<i64> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO conversations(title, model) VALUES(?1, ?2)",
            params![if title.trim().is_empty() { "New conversation" } else { title.trim() }, model],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn append_message(
        &self,
        conversation_id: i64,
        role: &str,
        content: &str,
        model: Option<&str>,
        sources_json: Option<&str>,
    ) -> Result<i64> {
        if !matches!(role, "system" | "user" | "assistant" | "tool") {
            return Err(HydrogenError::InvalidInput(format!("unsupported message role: {role}")));
        }
        let conn = self.connect()?;
        let tx = conn.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [conversation_id], |row| row.get(0)
        )?;
        if !exists {
            return Err(HydrogenError::InvalidInput(format!("conversation {conversation_id} does not exist")));
        }
        let digest = blake3::hash(content.as_bytes()).to_hex().to_string();
        tx.execute(
            "INSERT INTO messages(conversation_id, role, content, model, sources_json, content_hash)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![conversation_id, role, content, model, sources_json, digest],
        )?;
        let message_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO messages_fts(conversation_id, message_id, role, content)
             VALUES(?1, ?2, ?3, ?4)",
            params![conversation_id, message_id, role, content],
        )?;
        if role == "user" {
            let title: Option<String> = tx.query_row(
                "SELECT title FROM conversations WHERE id=?1", [conversation_id], |row| row.get(0)
            )?;
            if title.as_deref() == Some("New conversation") {
                let compact: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
                let title: String = compact.chars().take(72).collect();
                tx.execute("UPDATE conversations SET title=?1 WHERE id=?2", params![title, conversation_id])?;
            }
        }
        tx.execute(
            "UPDATE conversations SET updated_at=unixepoch() WHERE id=?1",
            [conversation_id],
        )?;
        tx.commit()?;
        Ok(message_id)
    }

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<ConversationSummary>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.title, c.created_at, c.updated_at, COUNT(m.id)
             FROM conversations c LEFT JOIN messages m ON m.conversation_id=c.id
             WHERE c.archived=0 GROUP BY c.id ORDER BY c.updated_at DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map([limit.clamp(1, 200) as i64], |row| Ok(ConversationSummary {
            id: row.get(0)?, title: row.get(1)?, created_at: row.get(2)?,
            updated_at: row.get(3)?, message_count: row.get(4)?,
        }))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_conversation(&self, id: i64) -> Result<Option<ConversationRecord>> {
        let conn = self.connect()?;
        let summary = conn.query_row(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.conversation_id=c.id)
             FROM conversations c WHERE c.id=?1",
            [id],
            |row| Ok(ConversationSummary {
                id: row.get(0)?, title: row.get(1)?, created_at: row.get(2)?,
                updated_at: row.get(3)?, message_count: row.get(4)?,
            })
        ).optional()?;
        let Some(conversation) = summary else { return Ok(None); };
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content, model, sources_json, created_at
             FROM messages WHERE conversation_id=?1 ORDER BY id"
        )?;
        let messages = stmt.query_map([id], |row| Ok(MessageRecord {
            id: row.get(0)?, conversation_id: row.get(1)?, role: row.get(2)?,
            content: row.get(3)?, model: row.get(4)?, sources_json: row.get(5)?,
            created_at: row.get(6)?,
        }))?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(ConversationRecord { conversation, messages }))
    }

    pub fn search_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeHit>> {
        const STOP: &[&str] = &["what","which","where","when","who","why","how","is","are","was","were","the","a","an","in","on","of","for","to","and","or","with","from","explain","define","describe","about"];
        let terms: Vec<String> = query.split_whitespace()
            .map(|term| term.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase())
            .filter(|term| term.chars().count() >= 2 && !STOP.contains(&term.as_str()))
            .take(12).collect();
        if terms.is_empty() { return Ok(Vec::new()); }
        let fts_query = terms.iter().map(|term| format!("\"{}\"*", term.replace('"', ""))).collect::<Vec<_>>().join(" OR ");
        let conn = self.connect()?;
        let mut hits = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT m.conversation_id, c.title, snippet(messages_fts, 3, '', '', '…', 24),
                        c.updated_at, m.conversation_id, bm25(messages_fts)
                 FROM messages_fts m JOIN conversations c ON c.id=m.conversation_id
                 WHERE messages_fts MATCH ?1 ORDER BY bm25(messages_fts) LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![fts_query, limit.clamp(1, 100) as i64], |row| Ok(KnowledgeHit {
                kind: "conversation".into(), id: row.get(0)?, title: row.get(1)?,
                excerpt: row.get(2)?, updated_at: row.get(3)?, url: None,
                conversation_id: Some(row.get(4)?),
                score: -row.get::<_, f64>(5)?,
            }))?;
            hits.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        {
            let mut stmt = conn.prepare(
                "SELECT p.id, p.title, snippet(web_pages_fts, 5, '', '', '…', 24),
                        p.last_seen, p.url, bm25(web_pages_fts)
                 FROM web_pages_fts f JOIN web_pages p ON p.id=CAST(f.page_id AS INTEGER)
                 WHERE web_pages_fts MATCH ?1 ORDER BY bm25(web_pages_fts) LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![fts_query, limit.clamp(1, 100) as i64], |row| Ok(KnowledgeHit {
                kind: "web_page".into(), id: row.get(0)?, title: row.get(1)?,
                excerpt: row.get(2)?, updated_at: row.get(3)?, url: Some(row.get(4)?),
                conversation_id: None,
                score: -row.get::<_, f64>(5)?,
            }))?;
            hits.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        hits.sort_by(|a,b| b.score.total_cmp(&a.score).then_with(|| b.updated_at.cmp(&a.updated_at)));
        let mut seen = HashSet::new();
        hits.retain(|hit| seen.insert((hit.kind.clone(), hit.id)));
        hits.truncate(limit.clamp(1, 100));
        Ok(hits)
    }

    pub fn save_web_page(
        &self, url: &str, canonical_url: &str, title: &str, author: &str,
        published_at: &str, site_name: &str, readable_text: &str, saved_html: &str,
        content_hash: &str, manually_saved: bool, track_visit: bool, version_pages: bool,
    ) -> Result<i64> {
        let conn = self.connect()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO web_pages(canonical_url,url,title,author,published_at,site_name,readable_text,saved_html,content_hash,manually_saved)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(canonical_url) DO UPDATE SET
                url=excluded.url, title=excluded.title, author=excluded.author,
                published_at=excluded.published_at, site_name=excluded.site_name,
                readable_text=excluded.readable_text, saved_html=excluded.saved_html,
                content_hash=excluded.content_hash,
                last_seen=unixepoch(), visits=web_pages.visits+1,
                manually_saved=MAX(web_pages.manually_saved, excluded.manually_saved)",
            params![canonical_url,url,title,author,published_at,site_name,readable_text,saved_html,content_hash,manually_saved as i64],
        )?;
        let page_id: i64 = tx.query_row(
            "SELECT id FROM web_pages WHERE canonical_url=?1", [canonical_url], |row| row.get(0)
        )?;
        tx.execute("DELETE FROM web_pages_fts WHERE page_id=?1", [page_id.to_string()])?;
        tx.execute(
            "INSERT INTO web_pages_fts(page_id,title,author,site_name,url,body)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![page_id,title,author,site_name,url,readable_text],
        )?;
        if version_pages {
            tx.execute(
                "INSERT OR IGNORE INTO web_page_versions(page_id,content_hash,readable_text)
                 VALUES(?1,?2,?3)", params![page_id,content_hash,readable_text]
            )?;
        }
        if track_visit {
            tx.execute("INSERT INTO web_visits(page_id) VALUES(?1)", [page_id])?;
        }
        tx.commit()?;
        Ok(page_id)
    }

    pub fn get_web_page(&self, id: i64) -> Result<Option<serde_json::Value>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT id,canonical_url,url,title,author,published_at,site_name,readable_text,
                    saved_html,content_hash,first_seen,last_seen,visits,manually_saved
             FROM web_pages WHERE id=?1",
            [id],
            |row| Ok(serde_json::json!({
                "id":row.get::<_,i64>(0)?,
                "canonical_url":row.get::<_,String>(1)?,
                "url":row.get::<_,String>(2)?,
                "title":row.get::<_,String>(3)?,
                "author":row.get::<_,String>(4)?,
                "published_at":row.get::<_,String>(5)?,
                "site_name":row.get::<_,String>(6)?,
                "readable_text":row.get::<_,String>(7)?,
                "saved_html":row.get::<_,String>(8)?,
                "content_hash":row.get::<_,String>(9)?,
                "first_seen":row.get::<_,i64>(10)?,
                "last_seen":row.get::<_,i64>(11)?,
                "visits":row.get::<_,i64>(12)?,
                "manually_saved":row.get::<_,i64>(13)? != 0
            }))
        ).optional().map_err(Into::into)
    }

    pub fn import_captured_conversation(
        &self,
        source_key: &str,
        title: &str,
        platform: &str,
        messages: &[(String, String)],
    ) -> Result<Option<i64>> {
        let conn = self.connect()?;
        let tx = conn.unchecked_transaction()?;
        let metadata: Option<String> = tx.query_row(
            "SELECT metadata_json FROM capture_sources WHERE source_key=?1",
            [source_key], |row| row.get(0)
        ).optional()?;

        let (conversation_id, is_new) = if let Some(metadata) = metadata {
            let value: serde_json::Value = serde_json::from_str(&metadata).unwrap_or_default();
            let id = value.get("conversation_id").and_then(serde_json::Value::as_i64)
                .ok_or_else(|| HydrogenError::Config("captured conversation metadata is invalid".into()))?;
            tx.execute("UPDATE capture_sources SET last_seen=unixepoch() WHERE source_key=?1", [source_key])?;
            (id, false)
        } else {
            tx.execute(
                "INSERT INTO conversations(title,model) VALUES(?1,?2)",
                params![if title.trim().is_empty() { "Imported conversation" } else { title.trim() }, platform],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO capture_sources(source_key,source_type,metadata_json)
                 VALUES(?1,'conversation',?2)",
                params![source_key, serde_json::json!({"platform":platform,"conversation_id":id}).to_string()],
            )?;
            (id, true)
        };

        let mut inserted = 0usize;
        for (role, content) in messages {
            if !matches!(role.as_str(), "user" | "assistant") || content.trim().is_empty() {
                continue;
            }
            let digest = blake3::hash(content.as_bytes()).to_hex().to_string();
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages
                 WHERE conversation_id=?1 AND role=?2 AND content_hash=?3)",
                params![conversation_id,role,digest], |row| row.get(0)
            )?;
            if exists { continue; }
            tx.execute(
                "INSERT INTO messages(conversation_id,role,content,model,content_hash)
                 VALUES(?1,?2,?3,?4,?5)",
                params![conversation_id,role,content,platform,digest],
            )?;
            let message_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO messages_fts(conversation_id,message_id,role,content)
                 VALUES(?1,?2,?3,?4)",
                params![conversation_id,message_id,role,content],
            )?;
            inserted += 1;
        }
        tx.execute("UPDATE conversations SET updated_at=unixepoch() WHERE id=?1", [conversation_id])?;
        tx.commit()?;
        if is_new || inserted > 0 { Ok(Some(conversation_id)) } else { Ok(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_db() -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("hydrogen-knowledge-{}-{stamp}.db", std::process::id()))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn additive_migration_preserves_existing_file_index() {
        let path = temporary_db();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE files(path TEXT PRIMARY KEY, mtime INTEGER NOT NULL, size INTEGER NOT NULL, is_dir INTEGER NOT NULL DEFAULT 0); INSERT INTO files(path,mtime,size) VALUES('/tmp/keep.txt',1,4);").unwrap();
        drop(conn);

        let repo = KnowledgeRepository::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE path='/tmp/keep.txt'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        assert_eq!(repo.list_conversations(10).unwrap().len(), 0);
        cleanup(&path);
    }

    #[test]
    fn conversations_are_persisted_and_searchable() {
        let path = temporary_db();
        let repo = KnowledgeRepository::open(&path).unwrap();
        let id = repo.create_conversation("Thermodynamics", "qwen3.5:2b-q4_K_M").unwrap();
        repo.append_message(id, "user", "What is a throttling device?", None, None).unwrap();
        repo.append_message(id, "assistant", "A throttling device reduces pressure.", Some("qwen3.5:2b-q4_K_M"), Some("[]")).unwrap();

        let conversation = repo.get_conversation(id).unwrap().unwrap();
        assert_eq!(conversation.messages.len(), 2);
        let hits = repo.search_knowledge("throttling device", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, Some(id));
        cleanup(&path);
    }

    #[test]
    fn captured_conversations_accept_incremental_updates_without_duplicates() {
        let path = temporary_db();
        let repo = KnowledgeRepository::open(&path).unwrap();
        let first = vec![("user".into(), "Explain pressure reduction".into())];
        let id = repo.import_captured_conversation("chatgpt:https://chatgpt.com/c/1", "Chat", "chatgpt", &first).unwrap().unwrap();
        let updated = vec![
            ("user".into(), "Explain pressure reduction".into()),
            ("assistant".into(), "A throttling device reduces pressure.".into()),
        ];
        assert_eq!(repo.import_captured_conversation("chatgpt:https://chatgpt.com/c/1", "Chat", "chatgpt", &updated).unwrap(), Some(id));
        assert_eq!(repo.import_captured_conversation("chatgpt:https://chatgpt.com/c/1", "Chat", "chatgpt", &updated).unwrap(), None);
        assert_eq!(repo.get_conversation(id).unwrap().unwrap().messages.len(), 2);
        cleanup(&path);
    }

    #[test]
    fn saved_web_pages_are_searchable_and_readable_offline() {
        let path = temporary_db();
        let repo = KnowledgeRepository::open(&path).unwrap();
        let id = repo.save_web_page(
            "https://example.com/throttling", "https://example.com/throttling",
            "Throttling devices explained", "Author", "2026-11-21", "Example",
            "A throttling device reduces pressure. It is commonly used in refrigeration systems.",
            "", "hash-1", false, true, true
        ).unwrap();
        let page = repo.get_web_page(id).unwrap().unwrap();
        assert_eq!(page["title"], "Throttling devices explained");
        assert_eq!(repo.search_knowledge("throttling pressure", 10).unwrap().len(), 1);
        cleanup(&path);
    }
}
