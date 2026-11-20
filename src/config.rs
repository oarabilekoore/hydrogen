use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub state_dir: PathBuf,
    pub db_path: PathBuf,
    pub cache_txt: PathBuf,
    pub rev_file: PathBuf,
    pub img_lock_path: PathBuf,
    pub models_dir: PathBuf,
    pub search_roots: Vec<PathBuf>,
    pub ics_globs: Vec<String>,
    pub evolution_db_globs: Vec<String>,
    pub default_event_duration_mins: i64,
    pub embeddings_enabled: bool,
}

impl AppConfig {
    pub fn load() -> Self {
        let home = env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let home_str = home.to_string_lossy();
        let state_dir = env::var("HYDROGEN_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/state/quickshell-launcher"));
        let models_dir = env::var("HYDROGEN_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/share/hydrogen/models"));
        let default_event_duration_mins = env::var("DURATION")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|&d| d > 0)
            .unwrap_or(60);

        Self {
            db_path: state_dir.join("content-index.db"),
            cache_txt: state_dir.join("file-index.txt"),
            rev_file: state_dir.join("index.rev"),
            img_lock_path: state_dir.join("image-sync.pid"),
            models_dir,
            search_roots: vec![
                home.join("Documents"),
                home.join("Downloads"),
                home.join("Pictures"),
                home.join("Videos"),
                home.join("Projects"),
            ],
            ics_globs: vec![
                format!("{home_str}/.local/share/evolution/calendar/*/calendar.ics"),
                format!("{home_str}/.calendars/**/*.ics"),
                format!("{home_str}/.local/share/vdirsyncer/**/*.ics"),
                format!("{home_str}/.local/share/khal/**/*.ics"),
                format!("{home_str}/.config/quickshell/calendars/**/*.ics"),
            ],
            evolution_db_globs: vec![format!("{home_str}/.cache/evolution/calendar/*/cache.db")],
            state_dir,
            default_event_duration_mins,
            embeddings_enabled: env::var("HYDROGEN_EMBEDDINGS").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
        }
    }
}
