use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub config_path: PathBuf,
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
    pub inference_threads: usize,
    pub image_batch_size: usize,
    pub pdf_page_limit: usize,
    pub settings: HydrogenSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HydrogenSettings {
    pub paths: PathsConfig,
    pub search: SearchConfig,
    pub indexing: IndexingConfig,
    pub models: ModelsConfig,
    pub resources: ResourceConfig,
    pub answer: AnswerConfig,
    pub calendar: CalendarConfig,
    pub watch: WatchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    pub state_dir: Option<PathBuf>,
    pub models_dir: Option<PathBuf>,
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub default_limit: usize,
    pub include_hidden: bool,
    pub scan_max_depth: usize,
    pub scan_max_results: usize,
    pub excluded_directories: Vec<String>,
    pub excluded_extensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexingConfig {
    pub pdf: PdfIndexingConfig,
    pub images: ImageIndexingConfig,
    pub embeddings_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PdfIndexingConfig {
    pub enabled: bool,
    pub ocr_scanned_pages: bool,
    pub page_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageIndexingConfig {
    pub enabled: bool,
    pub ocr: bool,
    pub captions: bool,
    pub min_file_bytes: u64,
    pub max_file_bytes: u64,
    pub min_dimension: u32,
    pub batch_size: usize,
    pub excluded_directories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    pub inference_threads: usize,
    pub ocr_printed: String,
    pub ocr_formula: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceConfig {
    pub max_inference_threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnswerConfig {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub retrieval_limit: usize,
    pub candidate_limit: usize,
    pub max_query_variants: usize,
    pub min_query_chars: usize,
    pub context_chars_per_passage: usize,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
    pub top_p: f32,
    pub repeat_penalty: f32,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarConfig {
    pub default_event_duration_mins: i64,
    pub ics_globs: Vec<String>,
    pub evolution_db_globs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchConfig {
    pub debounce_ms: u64,
    pub reconcile_interval_seconds: u64,
    pub poll_interval_ms: u64,
}

fn expand_home(path: &Path, home: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        path.to_path_buf()
    }
}

fn default_roots(home: &Path) -> Vec<PathBuf> {
    ["Documents", "Downloads", "Pictures", "Videos", "Projects"]
        .iter()
        .map(|name| home.join(name))
        .collect()
}

fn default_ics_globs(home: &Path) -> Vec<String> {
    let h = home.to_string_lossy();
    vec![
        format!("{h}/.local/share/evolution/calendar/*/calendar.ics"),
        format!("{h}/.calendars/**/*.ics"),
        format!("{h}/.local/share/vdirsyncer/**/*.ics"),
        format!("{h}/.local/share/khal/**/*.ics"),
        format!("{h}/.config/quickshell/calendars/**/*.ics"),
    ]
}

fn default_evolution_globs(home: &Path) -> Vec<String> {
    vec![format!(
        "{}/.cache/evolution/calendar/*/cache.db",
        home.display()
    )]
}

impl Default for HydrogenSettings {
    fn default() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            paths: PathsConfig {
                state_dir: None,
                models_dir: None,
                roots: default_roots(&home),
            },
            search: SearchConfig::default(),
            indexing: IndexingConfig::default(),
            models: ModelsConfig::default(),
            resources: ResourceConfig::default(),
            answer: AnswerConfig::default(),
            watch: WatchConfig::default(),
            calendar: CalendarConfig {
                default_event_duration_mins: 60,
                ics_globs: default_ics_globs(&home),
                evolution_db_globs: default_evolution_globs(&home),
            },
        }
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            state_dir: None,
            models_dir: None,
            roots: default_roots(&home),
        }
    }
}
impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 20,
            include_hidden: false,
            scan_max_depth: 6,
            scan_max_results: 5000,
            excluded_directories: vec![
                ".git",
                "node_modules",
                "__pycache__",
                ".venv",
                "venv",
                "target",
                "build",
                ".cache",
                "dist",
                ".next",
                ".cargo",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            excluded_extensions: vec!["tmp", "part", "swp", "lock"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}
impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            pdf: PdfIndexingConfig::default(),
            images: ImageIndexingConfig::default(),
            embeddings_enabled: false,
        }
    }
}
impl Default for PdfIndexingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ocr_scanned_pages: true,
            page_limit: 500,
        }
    }
}
impl Default for ImageIndexingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ocr: true,
            captions: false,
            min_file_bytes: 10 * 1024,
            max_file_bytes: 25 * 1024 * 1024,
            min_dimension: 128,
            batch_size: 2,
            excluded_directories: vec![
                "assets",
                "asset",
                "cursors",
                "cursor",
                "icons",
                "icon",
                "themes",
                "theme",
                "sprites",
                "thumbnails",
                ".thumbnails",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}
impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            inference_threads: 2,
            ocr_printed: "ppocr".into(),
            ocr_formula: "texo-distill-f16.gguf".into(),
        }
    }
}
impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            max_inference_threads: 2,
        }
    }
}
impl Default for AnswerConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
            endpoint: "http://127.0.0.1:11434".into(),
            model: "qwen3.5:2b-q4_K_M".into(),
            retrieval_limit: 8,
            candidate_limit: 16,
            max_query_variants: 6,
            min_query_chars: 3,
            context_chars_per_passage: 1600,
            context_window: 4096,
            max_output_tokens: 180,
            temperature: 0.0,
            top_p: 0.7,
            repeat_penalty: 1.08,
            timeout_seconds: 60,
        }
    }
}
impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 350,
            reconcile_interval_seconds: 60,
            poll_interval_ms: 150,
        }
    }
}
impl Default for CalendarConfig {
    fn default() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Self {
            default_event_duration_mins: 60,
            ics_globs: default_ics_globs(&home),
            evolution_db_globs: default_evolution_globs(&home),
        }
    }
}

impl AppConfig {
    pub fn load() -> std::result::Result<Self, String> {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let config_path = env::var_os("HYDROGEN_CONFIG")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join("hydrogen/config.toml"))
            })
            .unwrap_or_else(|| home.join(".config/hydrogen/config.toml"));

        let mut settings = if config_path.is_file() {
            let raw = fs::read_to_string(&config_path)
                .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
            toml::from_str::<HydrogenSettings>(&raw)
                .map_err(|e| format!("invalid {}: {e}", config_path.display()))?
        } else {
            HydrogenSettings::default()
        };

        // Environment variables remain supported as explicit, temporary overrides.
        if let Some(value) = env::var_os("HYDROGEN_STATE_DIR") {
            settings.paths.state_dir = Some(value.into());
        }
        if let Some(value) = env::var_os("HYDROGEN_MODELS_DIR") {
            settings.paths.models_dir = Some(value.into());
        }
        if let Ok(value) = env::var("HYDROGEN_EMBEDDINGS") {
            settings.indexing.embeddings_enabled =
                matches!(value.as_str(), "1") || value.eq_ignore_ascii_case("true");
        }
        if let Ok(raw) = env::var("HYDROGEN_INFERENCE_THREADS") {
            if let Ok(value) = raw.parse::<usize>() {
                settings.models.inference_threads = value;
            }
        }
        if let Ok(raw) = env::var("HYDROGEN_IMAGE_BATCH_SIZE") {
            if let Ok(value) = raw.parse::<usize>() {
                settings.indexing.images.batch_size = value;
            }
        }
        if let Ok(raw) = env::var("DURATION") {
            if let Ok(value) = raw.parse::<i64>() {
                settings.calendar.default_event_duration_mins = value;
            }
        }
        if let Ok(raw) = env::var("HYDROGEN_CAPTIONS") {
            settings.indexing.images.captions = raw == "1" || raw.eq_ignore_ascii_case("true");
        }
        if let Ok(raw) = env::var("HYDROGEN_PDF_PAGE_LIMIT") {
            if let Ok(value) = raw.parse::<usize>() {
                settings.indexing.pdf.page_limit = value;
            }
        }

        settings.models.inference_threads = settings
            .models
            .inference_threads
            .clamp(1, settings.resources.max_inference_threads.clamp(1, 8));
        settings.indexing.pdf.page_limit = settings.indexing.pdf.page_limit.clamp(1, 500);
        settings.indexing.images.batch_size = settings.indexing.images.batch_size.clamp(1, 4);
        settings.indexing.images.min_file_bytes = settings.indexing.images.min_file_bytes.max(1);
        settings.indexing.images.max_file_bytes = settings
            .indexing
            .images
            .max_file_bytes
            .max(settings.indexing.images.min_file_bytes);
        settings.answer.retrieval_limit = settings.answer.retrieval_limit.clamp(1, 16);
        settings.answer.context_chars_per_passage =
            settings.answer.context_chars_per_passage.clamp(100, 12000);
        settings.answer.context_window = settings.answer.context_window.clamp(512, 32768);
        settings.answer.max_output_tokens = settings.answer.max_output_tokens.clamp(16, 2048);
        settings.answer.timeout_seconds = settings.answer.timeout_seconds.clamp(5, 300);
        if settings.answer.model.trim().is_empty() {
            settings.answer.model = AnswerConfig::default().model;
        }
        if settings.answer.endpoint.trim().is_empty() {
            settings.answer.endpoint = AnswerConfig::default().endpoint;
        }
        settings.answer.candidate_limit = settings.answer.candidate_limit.clamp(1, 100);
        settings.answer.max_query_variants = settings.answer.max_query_variants.clamp(1, 12);
        settings.watch.debounce_ms = settings.watch.debounce_ms.clamp(50, 5000);
        settings.watch.reconcile_interval_seconds =
            settings.watch.reconcile_interval_seconds.clamp(5, 3600);
        settings.watch.poll_interval_ms = settings.watch.poll_interval_ms.clamp(25, 2000);
        settings.search.default_limit = settings.search.default_limit.clamp(1, 100);
        settings.search.scan_max_depth = settings.search.scan_max_depth.clamp(1, 32);
        settings.search.scan_max_results = settings.search.scan_max_results.clamp(1, 100_000);

        let search_roots: Vec<PathBuf> = settings
            .paths
            .roots
            .iter()
            .map(|path| expand_home(path, &home))
            .collect();
        let state_dir = settings
            .paths
            .state_dir
            .as_deref()
            .map(|path| expand_home(path, &home))
            .unwrap_or_else(|| home.join(".local/state/quickshell-launcher"));
        let models_dir = settings
            .paths
            .models_dir
            .as_deref()
            .map(|path| expand_home(path, &home))
            .unwrap_or_else(|| home.join(".local/share/hydrogen/models"));
        let default_event_duration_mins = settings.calendar.default_event_duration_mins.max(1);
        let ics_globs = settings.calendar.ics_globs.clone();
        let evolution_db_globs = settings.calendar.evolution_db_globs.clone();

        Ok(Self {
            config_path,
            db_path: state_dir.join("content-index.db"),
            cache_txt: state_dir.join("file-index.txt"),
            rev_file: state_dir.join("index.rev"),
            img_lock_path: state_dir.join("image-sync.pid"),
            models_dir,
            search_roots,
            ics_globs,
            evolution_db_globs,
            state_dir,
            default_event_duration_mins,
            embeddings_enabled: settings.indexing.embeddings_enabled,
            inference_threads: settings.models.inference_threads,
            image_batch_size: settings.indexing.images.batch_size,
            pdf_page_limit: settings.indexing.pdf.page_limit,
            settings,
        })
    }

    pub fn default_toml() -> std::result::Result<String, String> {
        toml::to_string_pretty(&HydrogenSettings::default()).map_err(|e| e.to_string())
    }

    pub fn write_settings(&self, settings: &HydrogenSettings) -> std::result::Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string_pretty(settings).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, content).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_toml_preserves_unmentioned_defaults() {
        let settings: HydrogenSettings =
            toml::from_str("[answer]\nmodel = \"qwen3.5:2b-q4_K_M\"\n").unwrap();
        assert_eq!(settings.answer.model, "qwen3.5:2b-q4_K_M");
        assert!(settings.indexing.pdf.enabled);
        assert!(settings.indexing.pdf.ocr_scanned_pages);
        assert!(settings.indexing.images.enabled);
        assert!(settings.indexing.images.ocr);
        assert_eq!(settings.search.scan_max_depth, 6);
        assert_eq!(settings.answer.retrieval_limit, 8);
    }

    #[test]
    fn default_configuration_is_local_and_resource_bounded() {
        let settings = HydrogenSettings::default();
        assert_eq!(settings.answer.provider, "ollama");
        assert_eq!(settings.answer.endpoint, "http://127.0.0.1:11434");
        assert_eq!(settings.models.inference_threads, 2);
        assert_eq!(settings.resources.max_inference_threads, 2);
        assert!(settings.indexing.pdf.ocr_scanned_pages);
        assert!(!settings.indexing.embeddings_enabled);
    }

    #[test]
    fn home_relative_paths_expand_without_touching_absolute_paths() {
        let home = PathBuf::from("/home/tester");
        assert_eq!(expand_home(Path::new("~/Notes"), &home), home.join("Notes"));
        assert_eq!(
            expand_home(Path::new("/mnt/data"), &home),
            PathBuf::from("/mnt/data")
        );
    }
}
