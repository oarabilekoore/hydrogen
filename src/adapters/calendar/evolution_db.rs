use crate::adapters::calendar::ics_files::parse_vcalendar_text;
use crate::domain::calendar::model::RawVEvent;
use crate::ports::calendar_store::CalendarStore;
use glob::glob;
use rusqlite::{Connection, OpenFlags};

pub struct EvolutionDbStore {
    globs: Vec<String>,
}

impl EvolutionDbStore {
    pub fn new(globs: Vec<String>) -> Self {
        Self { globs }
    }
}

impl CalendarStore for EvolutionDbStore {
    fn load_events(&self) -> Vec<RawVEvent> {
        let mut out = Vec::new();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        for pattern in &self.globs {
            let Ok(paths) = glob(pattern) else {
                continue;
            };
            for path in paths.flatten() {
                let uri = format!("file:{}?mode=ro&immutable=1", path.to_string_lossy());
                let Ok(conn) = Connection::open_with_flags(&uri, flags) else {
                    continue;
                };
                let Ok(mut stmt) = conn.prepare("SELECT ECacheOBJ FROM ECacheObjects") else {
                    continue;
                };
                let Ok(rows) = stmt.query_map([], |r| r.get::<_, Option<String>>(0)) else {
                    continue;
                };
                for row in rows.flatten().flatten() {
                    out.extend(parse_vcalendar_text(&row));
                }
            }
        }
        out
    }
}
