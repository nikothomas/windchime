// src/config.rs
use serde::Deserialize;
use std::error::Error;
use config::{Config, File};

#[derive(Debug, Deserialize)]
pub struct WindchimeConfig {
    pub demultiplex_barcodes: Option<String>,
    pub pipeline_env: Option<String>,
    pub skip_existing: Option<bool>,
}

impl Default for WindchimeConfig {
    fn default() -> Self {
        WindchimeConfig {
            demultiplex_barcodes: None,
            pipeline_env: None,
            skip_existing: None,
        }
    }
}

pub fn load_config(path: &str) -> Result<WindchimeConfig, Box<dyn Error>> {
    // Build configuration from the given file (supports TOML/JSON/etc.)
    let settings = Config::builder()
        .add_source(File::with_name(path))
        .build()?;
    let cfg: WindchimeConfig = settings.try_deserialize()?;
    Ok(cfg)
}
