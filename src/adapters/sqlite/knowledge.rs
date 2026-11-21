use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub sources: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebPageRecord {
    pub id: i64,
    pub url: String,
    pub canonical_url: String,
    pub title: String,
    pub content: String,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub site_name: Option<String>,
    pub content_hash: String,
    pub captured_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub url: Option<String>,
    pub updated_at: i64,
}

pub struct KnowledgeStore {
    db_path: PathBuf,
}

impl KnowledgeStore {
    pub fn open(db_path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        }
        let store = Self {
            db_path: db_path.to_path_buf(),
        };
        let conn = store.connect()?;
        ensure_schema(&conn)?;
        Ok(store)
    }

    fn connect(&self) -> rusqlite::Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(conn)
    }

    pub fn create_conversation(&self, title: &str) -> rusqlite::Result<String> {
        let id = new_id("conv");
        let title = if title.trim().is_empty() {
            "New conversation"
        } else {
            title.trim()
        };
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO conversations(id,title) VALUES(?1,?2)",
            params![id, title],
        )?;
        Ok(id)
    }

    pub fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        model: Option<&str>,
    ) -> rusqlite::Result<String> {
        self.append_message_with_sources(conversation_id, role, content, model, None)
    }

    pub fn append_message_with_sources(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        model: Option<&str>,
        sources: Option<&serde_json::Value>,
    ) -> rusqlite::Result<String> {
        if !matches!(role, "system" | "user" | "assistant") {
            return Err(rusqlite::Error::InvalidParameterName(
                "role must be system, user, or assistant".into(),
            ));
        }
        if content.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "message content cannot be empty".into(),
            ));
        }
        let id = new_id("msg");
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            params![conversation_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.execute(
            "INSERT INTO messages(id,conversation_id,role,content,model,sources_json) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, conversation_id, role, content, model, sources.map(serde_json::Value::to_string)],
        )?;
        tx.execute(
            "UPDATE conversations SET updated_at=unixepoch(), title=CASE
                WHEN title='New conversation' AND ?2='user' THEN substr(?3,1,96)
                ELSE title END
             WHERE id=?1",
            params![conversation_id, role, content],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn list_conversations(&self, limit: usize) -> rusqlite::Result<Vec<ConversationSummary>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT c.id,c.title,c.created_at,c.updated_at,COUNT(m.id)
             FROM conversations c LEFT JOIN messages m ON m.conversation_id=c.id
             GROUP BY c.id ORDER BY c.updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.clamp(1, 500) as i64], |row| {
            Ok(ConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                message_count: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_messages(&self, conversation_id: &str) -> rusqlite::Result<Vec<StoredMessage>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id,conversation_id,role,content,model,sources_json,created_at
             FROM messages WHERE conversation_id=?1 ORDER BY created_at,id",
        )?;
        let rows = stmt.query_map(params![conversation_id], |row| {
            Ok(StoredMessage {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                model: row.get(4)?,
                sources: row
                    .get::<_, Option<String>>(5)?
                    .and_then(|raw| serde_json::from_str(&raw).ok()),
                created_at: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Replace a captured browser conversation snapshot without creating duplicates.
    /// source_key should be a stable URL or extension-generated platform conversation key.
    pub fn capture_conversation(
        &self,
        source_key: &str,
        title: &str,
        messages: &[(String, String, Option<String>)],
    ) -> rusqlite::Result<String> {
        if source_key.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "source_key cannot be empty".into(),
            ));
        }
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT conversation_id FROM captured_conversations WHERE source_key=?1",
                params![source_key],
                |row| row.get(0),
            )
            .optional()?;
        let conversation_id = match existing {
            Some(id) => id,
            None => {
                let id = new_id("capture");
                tx.execute(
                    "INSERT INTO conversations(id,title) VALUES(?1,?2)",
                    params![
                        id,
                        if title.trim().is_empty() {
                            "Captured conversation"
                        } else {
                            title.trim()
                        }
                    ],
                )?;
                tx.execute(
                    "INSERT INTO captured_conversations(source_key,conversation_id) VALUES(?1,?2)",
                    params![source_key, id],
                )?;
                id
            }
        };
        tx.execute(
            "DELETE FROM messages WHERE conversation_id=?1",
            params![conversation_id],
        )?;
        for (role, content, model) in messages {
            if !matches!(role.as_str(), "user" | "assistant") || content.trim().is_empty() {
                continue;
            }
            tx.execute(
                "INSERT INTO messages(id,conversation_id,role,content,model) VALUES(?1,?2,?3,?4,?5)",
                params![new_id("msg"), conversation_id, role, content, model],
            )?;
        }
        tx.execute(
            "UPDATE conversations SET title=?2,updated_at=unixepoch() WHERE id=?1",
            params![
                conversation_id,
                if title.trim().is_empty() {
                    "Captured conversation"
                } else {
                    title.trim()
                }
            ],
        )?;
        tx.commit()?;
        Ok(conversation_id)
    }

    pub fn save_web_page(
        &self,
        url: &str,
        canonical_url: &str,
        title: &str,
        content: &str,
        author: Option<&str>,
        published_at: Option<&str>,
        site_name: Option<&str>,
        content_hash: &str,
        keep_version: bool,
        max_versions: usize,
    ) -> rusqlite::Result<i64> {
        let conn = self.connect()?;
        if keep_version {
            let previous: Option<(i64, String, String, String)> = conn
                .query_row(
                    "SELECT id,content_hash,title,content FROM web_pages WHERE canonical_url=?1",
                    params![canonical_url],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            if let Some((page_id, old_hash, old_title, old_content)) = previous {
                if old_hash != content_hash {
                    conn.execute(
                        "INSERT OR IGNORE INTO web_page_versions(page_id,content_hash,title,content)
                         VALUES(?1,?2,?3,?4)",
                        params![page_id,old_hash,old_title,old_content],
                    )?;
                }
            }
        }
        conn.execute(
            "INSERT INTO web_pages(url,canonical_url,title,content,author,published_at,site_name,content_hash)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(canonical_url) DO UPDATE SET
               url=excluded.url,title=excluded.title,content=excluded.content,
               author=excluded.author,published_at=excluded.published_at,
               site_name=excluded.site_name,content_hash=excluded.content_hash,
               updated_at=unixepoch()",
            params![url, canonical_url, title, content, author, published_at, site_name, content_hash],
        )?;
        let page_id: i64 = conn.query_row(
            "SELECT id FROM web_pages WHERE canonical_url=?1",
            params![canonical_url],
            |row| row.get(0),
        )?;
        conn.execute(
            "DELETE FROM web_page_versions
             WHERE page_id=?1 AND id NOT IN (
                SELECT id FROM web_page_versions
                WHERE page_id=?1 ORDER BY captured_at DESC,id DESC LIMIT ?2
             )",
            params![page_id, max_versions.clamp(1, 50) as i64],
        )?;
        Ok(page_id)
    }

    pub fn search_web_pages(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<WebPageRecord>> {
        let terms: Vec<String> = query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|term| term.chars().count() >= 2)
            .map(|term| format!("\"{}\"", term.replace('"', "")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = terms.join(" OR ");
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT web_pages.id,web_pages.url,web_pages.canonical_url,web_pages.title,
                    substr(web_pages.content,1,12000),web_pages.author,web_pages.published_at,web_pages.site_name,
                    web_pages.content_hash,web_pages.captured_at,web_pages.updated_at
             FROM web_pages_fts JOIN web_pages ON web_pages.id=web_pages_fts.rowid
             WHERE web_pages_fts MATCH ?1 ORDER BY bm25(web_pages_fts) LIMIT ?2",
        )?;
        let rows = statement.query_map(params![fts_query, limit.clamp(1, 100) as i64], |row| {
            Ok(WebPageRecord {
                id: row.get(0)?,
                url: row.get(1)?,
                canonical_url: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                author: row.get(5)?,
                published_at: row.get(6)?,
                site_name: row.get(7)?,
                content_hash: row.get(8)?,
                captured_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_web_page(&self, id: i64) -> rusqlite::Result<Option<WebPageRecord>> {
        let conn = self.connect()?;
        conn.query_row(
            "SELECT id,url,canonical_url,title,content,author,published_at,site_name,content_hash,captured_at,updated_at
             FROM web_pages WHERE id=?1",
            params![id],
            |row| Ok(WebPageRecord {
                id: row.get(0)?,
                url: row.get(1)?,
                canonical_url: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                author: row.get(5)?,
                published_at: row.get(6)?,
                site_name: row.get(7)?,
                content_hash: row.get(8)?,
                captured_at: row.get(9)?,
                updated_at: row.get(10)?,
            }),
        ).optional()
    }

    pub fn record_visit(&self, canonical_url: &str) -> rusqlite::Result<()> {
        let conn = self.connect()?;
        let page_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM web_pages WHERE canonical_url=?1",
                params![canonical_url],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(page_id) = page_id {
            conn.execute(
                "INSERT INTO web_visits(page_id) VALUES(?1)",
                params![page_id],
            )?;
            conn.execute(
                "UPDATE web_pages SET last_visited_at=unixepoch(),visit_count=visit_count+1 WHERE id=?1",
                params![page_id],
            )?;
        }
        Ok(())
    }

    pub fn search_saved_knowledge(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<KnowledgeHit>> {
        let conn = self.connect()?;
        let terms: Vec<String> = query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|term| term.chars().count() >= 2)
            .map(|term| format!("\"{}\"", term.replace('"', "")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = terms.join(" OR ");
        let mut hits = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT m.conversation_id,c.title,snippet(messages_fts,2,'','','…',24),c.updated_at
                 FROM messages_fts JOIN messages m ON m.rowid=messages_fts.rowid
                 JOIN conversations c ON c.id=m.conversation_id
                 WHERE messages_fts MATCH ?1
                 GROUP BY m.conversation_id
                 ORDER BY c.updated_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![fts_query, limit.clamp(1, 100) as i64], |row| {
                Ok(KnowledgeHit {
                    kind: "conversation".into(),
                    id: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                    url: None,
                    updated_at: row.get(3)?,
                })
            })?;
            hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        {
            let mut stmt = conn.prepare(
                "SELECT CAST(web_pages.id AS TEXT),web_pages.title,
                        snippet(web_pages_fts,2,'','','…',24),web_pages.updated_at,web_pages.url
                 FROM web_pages_fts JOIN web_pages ON web_pages.id=web_pages_fts.rowid
                 WHERE web_pages_fts MATCH ?1
                 ORDER BY bm25(web_pages_fts) LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![fts_query, limit.clamp(1, 100) as i64], |row| {
                Ok(KnowledgeHit {
                    kind: "web_page".into(),
                    id: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                    updated_at: row.get(3)?,
                    url: Some(row.get(4)?),
                })
            })?;
            hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(hits)
    }
}

pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversations(
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT 'New conversation',
            created_at INTEGER NOT NULL DEFAULT(unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT(unixepoch())
         );
         CREATE TABLE IF NOT EXISTS messages(
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK(role IN ('system','user','assistant')),
            content TEXT NOT NULL,
            model TEXT,
            sources_json TEXT,
            created_at INTEGER NOT NULL DEFAULT(unixepoch())
         );
         CREATE TABLE IF NOT EXISTS captured_conversations(
            source_key TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL UNIQUE REFERENCES conversations(id) ON DELETE CASCADE,
            captured_at INTEGER NOT NULL DEFAULT(unixepoch())
         );
         CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id,created_at);
         CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            conversation_id UNINDEXED, role UNINDEXED, content, tokenize='porter unicode61'
         );
         CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid,conversation_id,role,content)
            VALUES(new.rowid,new.conversation_id,new.role,new.content);
         END;
         CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
            DELETE FROM messages_fts WHERE rowid=old.rowid;
         END;
         CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
            DELETE FROM messages_fts WHERE rowid=old.rowid;
            INSERT INTO messages_fts(rowid,conversation_id,role,content)
            VALUES(new.rowid,new.conversation_id,new.role,new.content);
         END;
         CREATE TABLE IF NOT EXISTS web_pages(
            id INTEGER PRIMARY KEY,
            url TEXT NOT NULL,
            canonical_url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            author TEXT,
            published_at TEXT,
            site_name TEXT,
            content_hash TEXT NOT NULL,
            captured_at INTEGER NOT NULL DEFAULT(unixepoch()),
            updated_at INTEGER NOT NULL DEFAULT(unixepoch()),
            last_visited_at INTEGER,
            visit_count INTEGER NOT NULL DEFAULT 0
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS web_pages_fts USING fts5(
            title, url UNINDEXED, content, tokenize='porter unicode61'
         );
         CREATE TRIGGER IF NOT EXISTS web_pages_ai AFTER INSERT ON web_pages BEGIN
            INSERT INTO web_pages_fts(rowid,title,url,content)
            VALUES(new.id,new.title,new.url,new.content);
         END;
         CREATE TRIGGER IF NOT EXISTS web_pages_ad AFTER DELETE ON web_pages BEGIN
            DELETE FROM web_pages_fts WHERE rowid=old.id;
         END;
         CREATE TRIGGER IF NOT EXISTS web_pages_au AFTER UPDATE ON web_pages BEGIN
            DELETE FROM web_pages_fts WHERE rowid=old.id;
            INSERT INTO web_pages_fts(rowid,title,url,content)
            VALUES(new.id,new.title,new.url,new.content);
         END;
         CREATE TABLE IF NOT EXISTS web_visits(
            id INTEGER PRIMARY KEY,
            page_id INTEGER NOT NULL REFERENCES web_pages(id) ON DELETE CASCADE,
            visited_at INTEGER NOT NULL DEFAULT(unixepoch())
         );
         CREATE INDEX IF NOT EXISTS idx_web_visits_page ON web_visits(page_id,visited_at DESC);
         CREATE TABLE IF NOT EXISTS web_page_versions(
            id INTEGER PRIMARY KEY,
            page_id INTEGER NOT NULL REFERENCES web_pages(id) ON DELETE CASCADE,
            content_hash TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            captured_at INTEGER NOT NULL DEFAULT(unixepoch()),
            UNIQUE(page_id,content_hash)
         );"
    )?;
    // Additive compatibility migration for databases created by early builds of
    // this feature branch, before answer citations were persisted.
    let has_sources_column = {
        let mut statement = conn.prepare("PRAGMA table_info(messages)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        let mut found = false;
        for column in rows {
            if column? == "sources_json" {
                found = true;
                break;
            }
        }
        found
    };
    if !has_sources_column {
        conn.execute("ALTER TABLE messages ADD COLUMN sources_json TEXT", [])?;
    }
    Ok(())
}

fn new_id(prefix: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let material = format!("{prefix}:{now}:{}:{sequence}", std::process::id());
    format!(
        "{prefix}_{}",
        &blake3::hash(material.as_bytes()).to_hex()[..24]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store(name: &str) -> KnowledgeStore {
        let path = std::env::temp_dir().join(format!(
            "hydrogen-knowledge-{name}-{}-{}.sqlite",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        KnowledgeStore::open(&path).expect("create test knowledge store")
    }

    #[test]
    fn conversations_persist_and_are_searchable() {
        let store = test_store("conversation");
        let id = store.create_conversation("New conversation").unwrap();
        store
            .append_message(&id, "user", "What is a throttling device?", None)
            .unwrap();
        let sources = serde_json::json!([{"id":1,"name":"Lecture notes","page":14}]);
        store
            .append_message_with_sources(
                &id,
                "assistant",
                "A restriction that reduces pressure [1].",
                Some("qwen3.5:2b-q4_K_M"),
                Some(&sources),
            )
            .unwrap();
        let messages = store.get_messages(&id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].sources.as_ref(), Some(&sources));
        assert!(store
            .search_saved_knowledge("throttling", 10)
            .unwrap()
            .iter()
            .any(|hit| hit.id == id));
    }

    #[test]
    fn captured_conversations_update_in_place() {
        let store = test_store("capture");
        let first = vec![
            ("user".to_string(), "Explain throttling".to_string(), None),
            (
                "assistant".to_string(),
                "A pressure-reducing process.".to_string(),
                Some("model-a".to_string()),
            ),
        ];
        let id = store
            .capture_conversation("https://chatgpt.com/c/example", "Study chat", &first)
            .unwrap();
        let updated = vec![
            ("user".to_string(), "Explain throttling".to_string(), None),
            ("assistant".to_string(), "A pressure-reducing process.".to_string(), Some("model-a".to_string())),
            ("user".to_string(), "What is enthalpy?".to_string(), None),
        ];
        let same_id = store
            .capture_conversation("https://chatgpt.com/c/example", "Study chat", &updated)
            .unwrap();
        assert_eq!(id, same_id);
        assert_eq!(store.list_conversations(10).unwrap().len(), 1);
        assert_eq!(store.get_messages(&id).unwrap().len(), 3);
    }

    #[test]
    fn web_pages_are_deduplicated_by_canonical_url() {
        let store = test_store("web");
        let first = store
            .save_web_page(
                "https://example.test/post?utm_source=x",
                "https://example.test/post",
                "A useful article",
                "Readable article body about thermodynamics.",
                Some("Author"),
                None,
                Some("Example"),
                "hash-a",
                true,
                5,
            )
            .unwrap();
        let second = store
            .save_web_page(
                "https://example.test/post",
                "https://example.test/post",
                "Updated title",
                "Updated article body.",
                None,
                None,
                None,
                "hash-b",
                true,
                5,
            )
            .unwrap();
        assert_eq!(first, second);
    }
}
