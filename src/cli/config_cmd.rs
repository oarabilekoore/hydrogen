use crate::config::AppConfig;
use crate::error::{HydrogenError, Result};
use clap::{Args, Subcommand};
use serde_json::json;

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
}

pub fn run(args: ConfigArgs, cfg: &AppConfig) -> Result<()> {
    match args.action {
        ConfigAction::Show => {
            println!("{}", serde_json::to_string_pretty(&json!({
                "config_file": cfg.config_path,
                "settings": cfg.settings
            }))?);
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
            println!("{}", json!({
                "status": "created",
                "path": cfg.config_path
            }));
        }
    }
    Ok(())
}
