use crate::config::{AppConfig, HydrogenSettings};
use crate::error::{HydrogenError, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the effective configuration as JSON
    Show,
    /// Print the active configuration file path
    Path,
    /// Create a default configuration without overwriting an existing file
    Init,
    /// Set a supported dotted configuration key
    Set { key: String, value: String },
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(HydrogenError::InvalidInput(format!(
            "expected boolean, got '{value}'"
        ))),
    }
}

fn parse_list(value: &str) -> Result<Vec<String>> {
    if let Ok(list) = serde_json::from_str::<Vec<String>>(value) {
        return Ok(list);
    }
    Ok(value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect())
}

fn set_value(settings: &mut HydrogenSettings, key: &str, value: &str) -> Result<()> {
    macro_rules! number {
        ($ty:ty) => {
            value.parse::<$ty>().map_err(|_| {
                HydrogenError::InvalidInput(format!("invalid value for {key}: {value}"))
            })?
        };
    }
    match key {
        "paths.state_dir" => settings.paths.state_dir = Some(PathBuf::from(value)),
        "paths.models_dir" => settings.paths.models_dir = Some(PathBuf::from(value)),
        "paths.roots" => {
            settings.paths.roots = parse_list(value)?.into_iter().map(PathBuf::from).collect()
        }
        "search.default_limit" => settings.search.default_limit = number!(usize),
        "search.include_hidden" => settings.search.include_hidden = parse_bool(value)?,
        "search.scan_max_depth" => settings.search.scan_max_depth = number!(usize),
        "search.scan_max_results" => settings.search.scan_max_results = number!(usize),
        "search.excluded_directories" => settings.search.excluded_directories = parse_list(value)?,
        "search.excluded_extensions" => settings.search.excluded_extensions = parse_list(value)?,
        "indexing.embeddings_enabled" => settings.indexing.embeddings_enabled = parse_bool(value)?,
        "indexing.pdf.enabled" => settings.indexing.pdf.enabled = parse_bool(value)?,
        "indexing.pdf.ocr_scanned_pages" => {
            settings.indexing.pdf.ocr_scanned_pages = parse_bool(value)?
        }
        "indexing.pdf.page_limit" => settings.indexing.pdf.page_limit = number!(usize),
        "indexing.images.enabled" => settings.indexing.images.enabled = parse_bool(value)?,
        "indexing.images.ocr" => settings.indexing.images.ocr = parse_bool(value)?,
        "indexing.images.captions" => settings.indexing.images.captions = parse_bool(value)?,
        "indexing.images.min_file_bytes" => settings.indexing.images.min_file_bytes = number!(u64),
        "indexing.images.max_file_bytes" => settings.indexing.images.max_file_bytes = number!(u64),
        "indexing.images.min_dimension" => settings.indexing.images.min_dimension = number!(u32),
        "indexing.images.batch_size" => settings.indexing.images.batch_size = number!(usize),
        "indexing.images.excluded_directories" => {
            settings.indexing.images.excluded_directories = parse_list(value)?
        }
        "models.inference_threads" => settings.models.inference_threads = number!(usize),
        "resources.max_inference_threads" => {
            settings.resources.max_inference_threads = number!(usize)
        }
        "models.ocr_printed" => settings.models.ocr_printed = value.to_string(),
        "models.ocr_formula" => settings.models.ocr_formula = value.to_string(),
        "answer.provider" => settings.answer.provider = value.to_string(),
        "answer.endpoint" => settings.answer.endpoint = value.to_string(),
        "answer.model" => settings.answer.model = value.to_string(),
        "answer.retrieval_limit" => settings.answer.retrieval_limit = number!(usize),
        "answer.candidate_limit" => settings.answer.candidate_limit = number!(usize),
        "answer.max_query_variants" => settings.answer.max_query_variants = number!(usize),
        "answer.min_query_chars" => settings.answer.min_query_chars = number!(usize),
        "answer.context_chars_per_passage" => {
            settings.answer.context_chars_per_passage = number!(usize)
        }
        "answer.context_window" => settings.answer.context_window = number!(usize),
        "answer.max_output_tokens" => settings.answer.max_output_tokens = number!(usize),
        "answer.temperature" => settings.answer.temperature = number!(f32),
        "answer.top_p" => settings.answer.top_p = number!(f32),
        "answer.repeat_penalty" => settings.answer.repeat_penalty = number!(f32),
        "answer.timeout_seconds" => settings.answer.timeout_seconds = number!(u64),
        "calendar.ics_globs" => settings.calendar.ics_globs = parse_list(value)?,
        "calendar.evolution_db_globs" => settings.calendar.evolution_db_globs = parse_list(value)?,
        "calendar.default_event_duration_mins" => {
            settings.calendar.default_event_duration_mins = number!(i64)
        }
        "watch.debounce_ms" => settings.watch.debounce_ms = number!(u64),
        "watch.reconcile_interval_seconds" => {
            settings.watch.reconcile_interval_seconds = number!(u64)
        }
        "watch.poll_interval_ms" => settings.watch.poll_interval_ms = number!(u64),
        "frontend.bind_address" => settings.frontend.bind_address = value.to_string(),
        "frontend.auto_start" => settings.frontend.auto_start = parse_bool(value)?,
        "frontend.open_existing_window" => {
            settings.frontend.open_existing_window = parse_bool(value)?
        }
        "web.enabled" => settings.web.enabled = parse_bool(value)?,
        "web.auto_index" => settings.web.auto_index = parse_bool(value)?,
        "web.minimum_text_length" => settings.web.minimum_text_length = number!(usize),
        "web.minimum_paragraphs" => settings.web.minimum_paragraphs = number!(usize),
        "web.save_readable_snapshot" => settings.web.save_readable_snapshot = parse_bool(value)?,
        "web.track_visits" => settings.web.track_visits = parse_bool(value)?,
        "web.version_pages" => settings.web.version_pages = parse_bool(value)?,
        "web.max_versions_per_page" => settings.web.max_versions_per_page = number!(usize),
        "web.max_page_size_mb" => settings.web.max_page_size_mb = number!(usize),
        "web.feed_enabled" => settings.web.feed_enabled = parse_bool(value)?,
        "web.feed_interval_hours" => settings.web.feed_interval_hours = number!(u64),
        "web.allow_domains" => settings.web.allow_domains = parse_list(value)?,
        "web.exclude_domains" => settings.web.exclude_domains = parse_list(value)?,
        "web.exclude_patterns" => settings.web.exclude_patterns = parse_list(value)?,
        "conversations.enabled" => settings.conversations.enabled = parse_bool(value)?,
        "conversations.index_messages" => {
            settings.conversations.index_messages = parse_bool(value)?
        }
        "conversations.index_attachments" => {
            settings.conversations.index_attachments = parse_bool(value)?
        }
        "conversations.deduplicate" => settings.conversations.deduplicate = parse_bool(value)?,
        "conversations.capture_ai_chats" => {
            settings.conversations.capture_ai_chats = parse_bool(value)?
        }
        _ => {
            return Err(HydrogenError::InvalidInput(format!(
                "unknown or non-writable config key: {key}"
            )))
        }
    }
    Ok(())
}

pub fn run(args: ConfigArgs, cfg: &AppConfig) -> Result<()> {
    match args.action {
        ConfigAction::Show => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "config_file": cfg.config_path,
                    "settings": cfg.settings,
                    "effective_paths": {
                        "state_dir": cfg.state_dir,
                        "models_dir": cfg.models_dir,
                        "roots": cfg.search_roots
                    }
                }))?
            );
        }
        ConfigAction::Path => println!("{}", cfg.config_path.display()),
        ConfigAction::Init => {
            if cfg.config_path.exists() {
                return Err(HydrogenError::Config(format!(
                    "{} already exists; edit it instead of overwriting it",
                    cfg.config_path.display()
                )));
            }
            if let Some(parent) = cfg.config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let contents = AppConfig::default_toml().map_err(HydrogenError::Config)?;
            std::fs::write(&cfg.config_path, contents)?;
            println!(
                "{}",
                json!({ "status": "created", "path": cfg.config_path })
            );
        }
        ConfigAction::Set { key, value } => {
            // Read the persisted TOML, not environment-overridden runtime settings.
            // This prevents temporary service overrides from being written back to disk.
            let mut settings = if cfg.config_path.is_file() {
                let raw = std::fs::read_to_string(&cfg.config_path)?;
                toml::from_str::<HydrogenSettings>(&raw).map_err(|error| {
                    HydrogenError::Config(format!("invalid {}: {error}", cfg.config_path.display()))
                })?
            } else {
                HydrogenSettings::default()
            };
            set_value(&mut settings, &key, &value)?;
            cfg.write_settings(&settings)
                .map_err(HydrogenError::Config)?;
            println!(
                "{}",
                json!({
                    "status": "updated",
                    "key": key,
                    "value": value,
                    "path": cfg.config_path
                })
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setter_updates_nested_model_and_indexing_values() {
        let mut settings = HydrogenSettings::default();
        set_value(&mut settings, "answer.model", "qwen3.5:2b-q4_K_M").unwrap();
        set_value(&mut settings, "indexing.pdf.ocr_scanned_pages", "false").unwrap();
        set_value(
            &mut settings,
            "paths.roots",
            "[\"/home/test/Documents\", \"~/Notes\"]",
        )
        .unwrap();
        assert_eq!(settings.answer.model, "qwen3.5:2b-q4_K_M");
        assert!(!settings.indexing.pdf.ocr_scanned_pages);
        assert_eq!(settings.paths.roots.len(), 2);
        assert_eq!(settings.paths.roots[1], PathBuf::from("~/Notes"));
    }

    #[test]
    fn setter_rejects_unknown_keys_and_invalid_booleans() {
        let mut settings = HydrogenSettings::default();
        assert!(set_value(&mut settings, "answer.unknown", "value").is_err());
        assert!(set_value(&mut settings, "indexing.pdf.enabled", "sometimes").is_err());
    }
}
