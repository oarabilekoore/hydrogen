mod adapters;
mod application;
mod cli;
mod config;
mod domain;
mod error;
mod ports;

use clap::Parser;
use cli::Cli;
use config::AppConfig;
use serde_json::json;
use std::process;

fn main() {
    let cli = Cli::parse();
    let cfg = match AppConfig::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{}", json!({"error": err}));
            process::exit(2);
        }
    };

    if let Err(err) = cli::dispatch(cli, &cfg) {
        let payload = json!({
            "error": err.to_string(),
        });
        eprintln!("{payload}");
        process::exit(1);
    }
}
